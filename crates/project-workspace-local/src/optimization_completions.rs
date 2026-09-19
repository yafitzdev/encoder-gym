//! Append-only completion and best-dataset selection for the bounded Agent loop.
use crate::{
    connect, optimization_agent, optimization_iteration_execution as execution,
    optimization_iterations, optimization_launch, optimization_runs,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    domain::ExternalProjectSnapshot, journal::ExperimentEvent, protocol::ExperimentProtocol,
};
use project_workspace_core::{
    OptimizationLaunchAuthorization,
    optimization_iteration::{IterationDevelopmentEvidence, ProjectOptimizationIteration},
    optimization_iteration_execution::{IterationDevelopmentResult, IterationTrainingBinding},
    optimization_loop::{EvaluatedIteration, IterationCompletion, IterationDecision},
};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

/// Exact source journals loaded by the CLI from the run's pinned scientific store.
pub struct IterationScientificEvidence {
    pub iteration_id: Uuid,
    pub project: ExternalProjectSnapshot,
    pub protocol: ExperimentProtocol,
    pub events: Vec<ExperimentEvent>,
}

pub async fn list(folder: &Path, run_id: Uuid) -> Result<Vec<IterationCompletion>> {
    // Validate completion choices against the original proposal/result records,
    // not merely their self-consistent serialized hashes.
    optimization_iterations::list(folder, run_id).await?;
    let mut db = connect(folder, true, false).await?;
    let result = read(&mut db, run_id).await;
    db.close().await?;
    result
}

pub(crate) async fn read(
    db: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<IterationCompletion>> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_iteration_completions'").fetch_one(&mut *db).await?;
    if exists == 0 {
        return Ok(vec![]);
    }
    let rows = sqlx::query("SELECT c.iteration_id,c.iteration,c.fingerprint,c.metadata_json,i.fingerprint AS input_fingerprint FROM optimization_iteration_completions c LEFT JOIN project_optimization_iterations i ON i.id=c.iteration_id AND i.run_id=c.run_id AND i.iteration=c.iteration WHERE c.run_id=? ORDER BY c.iteration")
        .bind(run_id.to_string()).fetch_all(db).await?;
    let mut result = Vec::<IterationCompletion>::new();
    for row in rows {
        let value: IterationCompletion =
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        value.validate_identity()?;
        ensure!(
            value.run.id == run_id.to_string()
                && value.iteration.id == row.get::<String, _>("iteration_id")
                && Some(&value.iteration.fingerprint)
                    == row.get::<Option<String>, _>("input_fingerprint").as_ref()
                && i64::from(value.number) == row.get::<i64, _>("iteration")
                && value.fingerprint == row.get::<String, _>("fingerprint")
                && value.number as usize == result.len() + 1
                && (value.number != 1 || value.total_row_changes == value.row_changes)
                && result.last().is_none_or(|previous| previous.end.is_none()
                    && previous.total_row_changes.checked_add(value.row_changes)
                        == Some(value.total_row_changes)),
            "Iteration completion storage or ordering changed"
        );
        result.push(value);
    }
    Ok(result)
}

pub(crate) async fn validate_records(
    db: &mut SqliteConnection,
    iterations: &[project_workspace_core::optimization_iteration::ProjectOptimizationIteration],
    launch: &project_workspace_core::OptimizationLaunchAuthorization,
    completions: &[IterationCompletion],
) -> Result<()> {
    ensure!(
        completions.len() <= iterations.len(),
        "Completion has no iteration inputs"
    );
    let mut evaluations = Vec::new();
    for (index, completion) in completions.iter().enumerate() {
        let iteration = &iterations[index];
        let record = decision(db, iteration).await?;
        let proposal = record
            .proposal
            .as_ref()
            .context("Recorded decision missing")?;
        let not_executed = crate::optimization_repair_execution::read_not_executed(
            db,
            iteration.scope.run_id,
            iteration.scope.iteration,
        )
        .await?;
        if !proposal.stop && not_executed.is_none() {
            let training: IterationTrainingBinding =
                execution::read(db, "optimization_iteration_training", iteration.id)
                    .await?
                    .context("Completed training missing")?;
            let result: IterationDevelopmentResult =
                execution::read(db, "optimization_iteration_results", iteration.id)
                    .await?
                    .context("Completed result missing")?;
            validate_v3_repair_outcomes(db, launch, iteration, &record, &training, &result).await?;
            evaluations.push((iteration, training, result));
        }
        let history: Vec<_> = evaluations
            .iter()
            .map(|(iteration, training, result)| EvaluatedIteration {
                iteration,
                training,
                result,
            })
            .collect();
        let expected = IterationCompletion::create(
            iteration,
            launch,
            IterationDecision {
                proposal_call_id: record.call.id,
                proposal,
                not_executed: not_executed.as_ref(),
            },
            &history,
            index.checked_sub(1).map(|i| &completions[i]),
            completion.created_at,
        )?;
        ensure!(
            *completion == expected,
            "Completion differs from its recorded proposal, results or selection"
        );
    }
    Ok(())
}

async fn decision(
    db: &mut SqliteConnection,
    iteration: &project_workspace_core::optimization_iteration::ProjectOptimizationIteration,
) -> Result<encoder_optimization_core::agent::AgentTurnRecord> {
    let calls = optimization_agent::read_history(
        db,
        iteration.scope.run_id,
        Some(iteration.scope.iteration),
    )
    .await?;
    ensure!(
        calls.iter().all(|(_, record)| record.is_some()),
        "An Agent call is still pending"
    );
    let mut decisions = calls
        .into_iter()
        .filter_map(|(_, record)| record)
        .filter(|record| !record.interrupted && record.proposal.is_some());
    let record = decisions
        .next()
        .context("Completion requires one recorded Agent decision")?;
    ensure!(
        decisions.next().is_none()
            && record.call.scope_fingerprint == iteration.scope.fingerprint()?,
        "Completion requires one decision from its exact iteration"
    );
    Ok(record)
}

async fn validate_v3_repair_outcomes(
    db: &mut SqliteConnection,
    launch: &OptimizationLaunchAuthorization,
    iteration: &ProjectOptimizationIteration,
    record: &encoder_optimization_core::agent::AgentTurnRecord,
    training: &IterationTrainingBinding,
    result: &IterationDevelopmentResult,
) -> Result<()> {
    use encoder_optimization_core::{
        fingerprint,
        generation::GenerationCanaryPolicy,
        repair_outcome::{
            OutcomeIdentity, RepairGlobalVerdict, intervention_fingerprint, intervention_summary,
        },
        repair_strategy::RepairPlan,
    };

    let required = launch.scope.agentic.as_ref().is_some_and(|settings| {
        settings.analysis_protocol == 3
            && settings.generation_canary == Some(GenerationCanaryPolicy::PerCombinationSemanticV3)
    });
    if !required {
        return Ok(());
    }
    ensure!(
        iteration.scope.analysis_protocol == 3,
        "Protocol V3 repair outcomes belong to another iteration"
    );
    let submissions = record
        .tools
        .iter()
        .filter(|tool| tool.name == "submit_repair_plan" && !tool.failed)
        .collect::<Vec<_>>();
    ensure!(
        submissions.len() == 1,
        "Executed protocol V3 iteration has no exact repair plan"
    );
    let plan: RepairPlan = serde_json::from_value(
        submissions[0]
            .arguments
            .get("plan")
            .cloned()
            .context("Protocol V3 submission plan missing")?,
    )?;
    ensure!(
        !plan.stop && !plan.targets.is_empty(),
        "Executed protocol V3 iteration has no repair targets"
    );
    let outcomes = crate::optimization_repair_outcomes::read_iteration(
        db,
        iteration.scope.run_id,
        iteration.scope.iteration,
    )
    .await?;
    ensure!(
        outcomes.len() == plan.targets.len(),
        "Executed protocol V3 iteration requires one outcome per repair target"
    );
    let plan_fingerprint = fingerprint(&plan)?;
    let proposal_fingerprint = fingerprint(
        record
            .proposal
            .as_ref()
            .context("Protocol V3 compiled proposal missing")?,
    )?;
    let output_evidence = IterationDevelopmentEvidence::completed(
        training,
        result,
        &iteration.development.benchmark_definition_fingerprint,
    )?;
    let input_dataset = OutcomeIdentity {
        id: iteration.dataset.id.to_string(),
        fingerprint: iteration.dataset.fingerprint.clone(),
    };
    let output_dataset = OutcomeIdentity {
        id: training.qualified_dataset.id.to_string(),
        fingerprint: training.qualified_dataset.fingerprint.clone(),
    };
    let expected_reports = result
        .reports
        .values()
        .map(|report| (report.id.to_string(), report.fingerprint.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    let expected_verdict = if result.development_passed {
        RepairGlobalVerdict::Keep
    } else {
        RepairGlobalVerdict::Reject
    };
    let targets = plan
        .targets
        .iter()
        .map(|target| (target.target_id.as_str(), target))
        .collect::<std::collections::BTreeMap<_, _>>();
    for outcome in &outcomes {
        let target = targets
            .get(outcome.target_id.as_str())
            .context("Repair outcome target is absent from its plan")?;
        let mut cluster_keys = target.cluster_keys.clone();
        cluster_keys.sort();
        let reports = outcome
            .reports
            .iter()
            .map(|report| (report.id.clone(), report.fingerprint.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        ensure!(
            outcome.repair_plan_fingerprint == plan_fingerprint
                && outcome.proposal_fingerprint == proposal_fingerprint
                && outcome.intervention_fingerprint == intervention_fingerprint(target)?
                && outcome.cluster_keys == cluster_keys
                && outcome.intervention == intervention_summary(target)
                && outcome.input_dataset == input_dataset
                && outcome.output_dataset == output_dataset
                && outcome.input_development_evidence_fingerprint
                    == iteration.development.fingerprint
                && outcome.output_development_evidence_fingerprint == output_evidence.fingerprint
                && outcome.global_verdict == expected_verdict
                && reports == expected_reports
                && outcome.observations.iter().all(|observation| {
                    target.cluster_keys.contains(&observation.cluster_key)
                        && observation.metric == target.target_metric.name
                        && observation.expected_direction == target.target_metric.direction
                }),
            "Repair outcome differs from its plan, dataset, or development result"
        );
    }
    Ok(())
}

/// Complete one exact iteration, or revalidate/reuse an already completed one.
/// Every earlier selection is reconstructed from its original scientific journal.
pub async fn finish(
    folder: &Path,
    run_id: Uuid,
    number: u32,
    scientific: &[IterationScientificEvidence],
) -> Result<IterationCompletion> {
    Box::pin(complete(folder, run_id, number, scientific, true)).await
}

/// Replay existing completions against all original journals without migrating
/// or writing the project database. Missing completions are never synthesized.
pub async fn verify_completed(
    folder: &Path,
    run_id: Uuid,
    number: u32,
    scientific: &[IterationScientificEvidence],
) -> Result<IterationCompletion> {
    Box::pin(complete(folder, run_id, number, scientific, false)).await
}

async fn complete(
    folder: &Path,
    run_id: Uuid,
    number: u32,
    scientific: &[IterationScientificEvidence],
    create: bool,
) -> Result<IterationCompletion> {
    let view = optimization_runs::show(folder, run_id).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|item| item.id.to_string() == view.run.launch.id)
        .context("Launch missing")?;
    let iterations = optimization_iterations::list(folder, run_id).await?;
    ensure!(
        number > 0 && number as usize <= iterations.len(),
        "Iteration does not exist"
    );
    // Recheck full dataset/publication custody as well as the scientific result.
    for iteration in iterations.iter().take(number as usize) {
        execution::training(folder, run_id, iteration.id).await?;
    }
    let mut db = connect(folder, !create, false).await?;
    if create {
        sqlx::migrate!("./migrations").run(&mut db).await?;
    }
    let existing = read(&mut db, run_id).await?;
    ensure!(
        existing.len() + usize::from(create) >= number as usize,
        "Cannot skip an iteration completion"
    );
    let mut completed = Vec::new();
    let mut evaluations = Vec::new();
    for iteration in iterations.iter().take(number as usize) {
        let record = decision(&mut db, iteration).await?;
        let proposal = record.proposal.as_ref().expect("filtered proposal");
        let not_executed = crate::optimization_repair_execution::read_not_executed(
            &mut db,
            iteration.scope.run_id,
            iteration.scope.iteration,
        )
        .await?;
        if !proposal.stop && not_executed.is_none() {
            let binding: IterationTrainingBinding =
                execution::read(&mut db, "optimization_iteration_training", iteration.id)
                    .await?
                    .context("Training receipt missing")?;
            let saved: IterationDevelopmentResult =
                execution::read(&mut db, "optimization_iteration_results", iteration.id)
                    .await?
                    .context("Development result missing")?;
            let source = scientific
                .iter()
                .find(|source| source.iteration_id == iteration.id)
                .context("Scientific completion evidence missing")?;
            let actual = IterationDevelopmentResult::from_journal(
                &binding,
                &source.project,
                &source.protocol,
                &source.events,
            )?;
            ensure!(
                actual == saved,
                "Iteration result differs from its scientific journal"
            );
            validate_v3_repair_outcomes(&mut db, &launch, iteration, &record, &binding, &saved)
                .await?;
            evaluations.push((iteration, binding, saved));
        }
        let history: Vec<_> = evaluations
            .iter()
            .map(|(iteration, training, result)| EvaluatedIteration {
                iteration,
                training,
                result,
            })
            .collect();
        let prior = existing
            .iter()
            .find(|value| value.number == iteration.scope.iteration);
        let value = IterationCompletion::create(
            iteration,
            &launch,
            IterationDecision {
                proposal_call_id: record.call.id,
                proposal,
                not_executed: not_executed.as_ref(),
            },
            &history,
            completed.last(),
            prior.map_or_else(Utc::now, |value| value.created_at),
        )?;
        if let Some(prior) = prior {
            ensure!(*prior == value, "Completed iteration selection changed");
        }
        completed.push(value);
    }
    let result = completed.pop().context("Completion missing")?;
    if existing.iter().any(|value| value.number == number) {
        db.close().await?;
        return Ok(result);
    }
    ensure!(
        !view.state.is_terminal(),
        "Run stopped before iteration completion"
    );
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let head:String = sqlx::query_scalar("SELECT fingerprint FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1").bind(run_id.to_string()).fetch_one(&mut *tx).await?;
    ensure!(
        head == view.head_fingerprint && read(&mut tx, run_id).await? == existing,
        "Run changed before iteration completion"
    );
    sqlx::query("INSERT INTO optimization_iteration_completions(iteration_id,run_id,iteration,fingerprint,metadata_json) VALUES(?,?,?,?,?)")
        .bind(&result.iteration.id).bind(run_id.to_string()).bind(i64::from(result.number)).bind(&result.fingerprint).bind(serde_json::to_string(&result)?).execute(&mut *tx).await?;
    tx.commit().await?;
    db.close().await?;
    Ok(result)
}
