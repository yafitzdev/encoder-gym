//! Production pre-training composition: exact inputs -> Agent -> generation ->
//! immutable dataset. The returned version is NOT permission to train: complete
//! task-owned benchmark isolation must still precede the training handoff.
pub(super) mod control;
mod inspection;
mod iteration_loop;
mod progress;
mod providers;
mod qualification;
mod registration;
mod training;
pub(super) use iteration_loop::execute as drive_loop;

use std::{collections::BTreeSet, path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use dataset_quality_core::{
    native_assessment::{
        NativeAdmissionRecord, NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest,
        NativeTargetFitEvidence, NativeTargetFitRequest,
    },
    ports::{NativeReviewStore, NativeSemanticReviewer},
};
use encoder_experiment_nomos::{NomosNativeAssessmentBinding, nomos_native_target_brief};
use encoder_optimization_core::{
    agent::{AgentTurnRecord, DatasetEditProposal, InspectionPage},
    fingerprint,
    generation::{
        GenerationCanaryPolicy, GenerationPhase, GenerationTask, RepairNotExecuted,
        RepairNotExecutedReason, V3CanaryStatus, evaluate_v3_canaries,
    },
    ports::{OptimizationAgentStore, OptimizationGenerationStore},
    repair_strategy::{RepairOperation, RepairPlan},
};
use encoder_optimization_runner::{
    OptimizationAgent,
    generation::OptimizationGenerator,
    semantic_review::{DurableNativeReviewRunner, PiNativeSemanticReviewer},
};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    optimization_iteration::ProjectOptimizationIteration,
};
use project_workspace_local::{
    AppendActivity, append_activity, initialize_activity, optimization_agent::ProjectAgentStore,
    optimization_dataset, optimization_generation::ProjectGenerationStore, optimization_launch,
    optimization_native_review::ProjectNativeReviewStore, optimization_repair_execution,
    optimization_runs,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetStepResult {
    iteration_id: Uuid,
    proposal: DatasetEditProposal,
    publication: Option<optimization_dataset::OptimizationDatasetPublication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qualification: Option<qualification::QualifiedDataset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    development: Option<
        project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult,
    >,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<registration::IterationCandidate>,
}

pub(super) async fn execute(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, false, false).await
}

pub(super) async fn prepare_candidate(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, true, false).await
}

pub(super) async fn complete_iteration(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, true, true).await
}

async fn execute_step(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
    qualify: bool,
    train: bool,
) -> Result<()> {
    let database_url = super::super::sqlite_file_url(&folder.join("project.sqlite"));
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )
    .await?;
    if optimization_runs::show(folder, run_id)
        .await?
        .preparation
        .is_none()
    {
        super::prepare_inputs(folder, run_id).await?;
    }
    let iteration = super::iteration_inputs::bind_inputs(folder, run_id).await?;
    let (action_id, result) = run_step(folder, &iteration, runtime, qualify, train).await?;
    super::super::print(&serde_json::json!({"actionId": action_id, "datasetStep": result}))
}

async fn run_step(
    folder: &Path,
    iteration: &ProjectOptimizationIteration,
    runtime: crate::cli::ResearchRuntimeArgs,
    qualify: bool,
    train: bool,
) -> Result<(Uuid, DatasetStepResult)> {
    let run_id = iteration.scope.run_id;
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.agent".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references: vec![
            ActivityReference::new("run", run_id.to_string()).expect("UUID"),
            ActivityReference::new("iteration", iteration.scope.iteration.to_string())
                .expect("iteration"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result: Result<_> = async {
        if qualify {
            qualification::preflight(folder, run_id, iteration).await?;
        }
        let mut result = drive(folder, iteration, action_id, runtime).await?;
        ensure!(
            !project_workspace_local::optimization_execution::stopped(folder, run_id).await?,
            "Run stopped before qualification"
        );
        if qualify {
            if let Some(publication) = &result.publication {
                result.qualification =
                    Some(qualification::prepare(folder, run_id, publication).await?);
            }
        }
        if train {
            ensure!(
                !project_workspace_local::optimization_execution::stopped(folder, run_id).await?,
                "Run stopped before training"
            );
            if let Some(qualified) = &result.qualification {
                let (development, candidate) =
                    training::complete(folder, run_id, qualified).await?;
                result.development = Some(development);
                result.candidate = Some(candidate);
            }
        }
        Ok(result)
    }
    .await;
    let mut terminal = if result.is_ok() {
        event(ActivityEventState::Succeeded, None)
    } else {
        event(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "agent_dataset_failed",
                "Agent iteration stopped; inspect its recorded calls, dataset and scientific journal before resuming.",
            )?),
        )
    };
    if let Ok(step) = &result {
        terminal.references.push(ActivityReference::new(
            "iteration_id",
            step.iteration_id.to_string(),
        )?);
        if let Some(candidate) = &step.candidate {
            terminal.references.extend([
                ActivityReference::new("model", candidate.model.id.to_string())?,
                ActivityReference::new(
                    "dataset_version",
                    candidate.dataset.version.id.to_string(),
                )?,
            ]);
        }
        if let Some(development) = &step.development {
            terminal.references.push(ActivityReference::new(
                "experiment_run",
                development.experiment_run_id.to_string(),
            )?);
            for report in development.reports.values() {
                terminal.references.push(ActivityReference::new(
                    "evaluation_report",
                    report.id.to_string(),
                )?);
            }
        }
    }
    append_activity(folder, terminal).await?;
    Ok((action_id, result?))
}

async fn drive(
    folder: &Path,
    iteration: &ProjectOptimizationIteration,
    action_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<DatasetStepResult> {
    let run_id = iteration.scope.run_id;
    let run = optimization_runs::show(folder, run_id).await?;
    ensure!(
        !run.state.is_terminal() && run.materialization.is_none() && run.experiment.is_none(),
        "Agent edits cannot run after a fixed candidate was materialized or the run ended"
    );
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|launch| {
            launch.id.to_string() == run.run.launch.id
                && launch.fingerprint == run.run.launch.fingerprint
        })
        .context("Pinned launch is missing")?;
    let settings = launch
        .scope
        .agentic
        .as_ref()
        .context("Run has no Agent execution authorization")?;
    let inspection = Arc::new(inspection::IterationInspection::load(folder, iteration).await?);
    let agent_store = Arc::new(ProjectAgentStore::open(folder, run_id, action_id).await?);
    let generation_store = Arc::new(
        ProjectGenerationStore::open(folder, run_id)
            .await?
            .with_activity(),
    );
    // Exact PID/start-time checks reject an invocation while its predecessor is
    // still alive. A missing response does not itself authorize a replacement.
    agent_store.recover_interrupted(&iteration.scope).await?;
    generation_store.recover_interrupted().await?;
    let native_review_store = if iteration.scope.analysis_protocol == 3 {
        let store = Arc::new(ProjectNativeReviewStore::open(folder, run_id).await?);
        store.recover_interrupted().await?;
        Some(store)
    } else {
        None
    };
    let selected = providers::agent(
        run.run.project_id,
        agent_store.provider(),
        &launch.scope.advisor,
        runtime,
    )?;
    let agent = OptimizationAgent::new(
        selected.runtime.clone(),
        agent_store.clone(),
        inspection.clone(),
        selected.selection.clone(),
    );
    let proposal = agent.analyze(iteration.scope.clone()).await?;
    if proposal.stop {
        return Ok(DatasetStepResult {
            iteration_id: iteration.id,
            proposal,
            publication: None,
            qualification: None,
            development: None,
            candidate: None,
        });
    }
    let history = agent_store.history(iteration.scope.clone()).await?;
    let mut evidence = BTreeSet::new();
    for tool in history.iter().flat_map(|turn| &turn.tools).filter(|tool| {
        !tool.failed
            && matches!(
                tool.name.as_str(),
                "inspect_development_failures" | "inspect_dataset_landscape"
            )
    }) {
        let page: InspectionPage = serde_json::from_value(tool.result.clone())?;
        page.validate(20)?;
        evidence.extend(page.items.into_iter().map(|item| item.id));
    }
    let templates = Arc::new(inspection.templates(&proposal)?);
    let v3 = (iteration.scope.analysis_protocol == 3)
        .then(|| v3_plan_and_briefs(&history, &proposal))
        .transpose()?;
    let per_combination_canaries =
        settings.generation_canary == Some(GenerationCanaryPolicy::PerCombinationSemanticV3);
    let tasks = if let (Some((plan, _)), true) = (&v3, per_combination_canaries) {
        templates.tasks_v3(
            &iteration.scope,
            &proposal,
            plan,
            &evidence,
            providers::output_limit(&launch.scope.generation),
        )?
    } else {
        templates.tasks(
            &iteration.scope,
            &proposal,
            &evidence,
            providers::output_limit(&launch.scope.generation),
        )?
    };
    if !tasks.is_empty() {
        let backend = providers::generation(run.run.project_id, generation_store.provider())?;
        let generator = OptimizationGenerator {
            backend,
            admission: templates,
            store: generation_store.clone(),
            concurrency: settings.generation_concurrency,
            maximum_cost_microusd_per_call: providers::cost_limit(&launch.scope.generation),
        };
        if let (Some(review_store), Some((plan, target_briefs))) = (native_review_store, &v3) {
            let reviewer = Arc::new(PiNativeSemanticReviewer::new(
                selected.runtime,
                selected.selection.clone(),
            )?);
            let reviewer_identity = reviewer.native_identity();
            let review_runner =
                DurableNativeReviewRunner::new(reviewer, review_store.clone(), selected.selection)?;
            if !per_combination_canaries {
                generator
                    .generate_with_canary(tasks.clone(), settings.generation_canary)
                    .await?;
                review_generated_rows(
                    &tasks,
                    target_briefs,
                    generation_store,
                    review_store,
                    &review_runner,
                    &reviewer_identity.fingerprint,
                )
                .await?;
            } else {
                let canary_tasks = tasks
                    .iter()
                    .filter(|task| {
                        task.execution_v3
                            .as_ref()
                            .is_some_and(|value| value.phase == GenerationPhase::Canary)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let bulk_tasks = tasks
                    .iter()
                    .filter(|task| {
                        task.execution_v3
                            .as_ref()
                            .is_some_and(|value| value.phase == GenerationPhase::Bulk)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                ensure!(
                    canary_tasks.len() + bulk_tasks.len() == tasks.len(),
                    "Protocol V3 generation tasks have no execution phase"
                );
                let canary_outcomes = generator.generate(canary_tasks.clone()).await?;
                let canary_admissions = review_generated_rows(
                    &canary_tasks,
                    target_briefs,
                    generation_store.clone(),
                    review_store.clone(),
                    &review_runner,
                    &reviewer_identity.fingerprint,
                )
                .await?;
                let decisions = canary_admissions
                    .iter()
                    .map(|record| (record.authority.row_id.clone(), record.admission.admitted()))
                    .collect();
                let gate = evaluate_v3_canaries(&canary_tasks, &canary_outcomes, &decisions)?;
                if gate.status == V3CanaryStatus::Rejected {
                    optimization_repair_execution::record_not_executed(
                        folder,
                        &RepairNotExecuted {
                            schema_version: 1,
                            run_id,
                            iteration: iteration.scope.iteration,
                            proposal_fingerprint: fingerprint(&proposal)?,
                            repair_plan_fingerprint: fingerprint(plan)?,
                            reason: RepairNotExecutedReason::CanaryRejected,
                            canary: gate,
                        },
                    )
                    .await?;
                    return Ok(DatasetStepResult {
                        iteration_id: iteration.id,
                        proposal,
                        publication: None,
                        qualification: None,
                        development: None,
                        candidate: None,
                    });
                }
                let bulk_outcomes = generator.generate(bulk_tasks.clone()).await?;
                ensure!(
                    bulk_outcomes.len() == bulk_tasks.len(),
                    "Protocol V3 bulk generation did not complete"
                );
                review_generated_rows(
                    &bulk_tasks,
                    target_briefs,
                    generation_store,
                    review_store,
                    &review_runner,
                    &reviewer_identity.fingerprint,
                )
                .await?;
            }
        } else {
            generator
                .generate_with_canary(tasks.clone(), settings.generation_canary)
                .await?;
        }
    }
    ensure!(
        !agent_store.stopped(run_id).await?,
        "Run stopped before dataset publication"
    );
    let publication =
        optimization_dataset::publish(folder, run_id, iteration.scope.iteration).await?;
    ensure!(
        publication.parent == iteration.dataset,
        "Published dataset has another parent"
    );
    Ok(DatasetStepResult {
        iteration_id: iteration.id,
        proposal,
        publication: Some(publication),
        qualification: None,
        development: None,
        candidate: None,
    })
}

fn v3_plan_and_briefs(
    history: &[AgentTurnRecord],
    proposal: &DatasetEditProposal,
) -> Result<(
    RepairPlan,
    Vec<dataset_quality_core::native_assessment::NativeTargetBrief>,
)> {
    let submitted = history
        .iter()
        .flat_map(|turn| &turn.tools)
        .find(|tool| tool.name == "submit_repair_plan" && !tool.failed)
        .context("Protocol V3 has no accepted structured repair plan")?;
    let plan: RepairPlan = serde_json::from_value(
        submitted
            .arguments
            .get("plan")
            .cloned()
            .context("Submitted repair plan payload is missing")?,
    )?;
    let mut result = Vec::new();
    for target in &plan.targets {
        let brief = nomos_native_target_brief(target)?;
        match &target.operation {
            RepairOperation::LabelPreservingVariants { anchors, .. } => {
                for anchor in anchors {
                    result.push((anchor.row_id.clone(), anchor.additions, brief.clone()));
                }
            }
            RepairOperation::ExistingAnchorContrast { pairs, .. } => {
                for pair in pairs {
                    for anchor in [&pair.left, &pair.right] {
                        result.push((
                            anchor.row_id.clone(),
                            pair.additions_per_side,
                            brief.clone(),
                        ));
                    }
                }
            }
            RepairOperation::ProvenRedundantRowRemoval { .. } => {}
        }
    }
    ensure!(
        result.len() == proposal.additions.len()
            && result
                .iter()
                .zip(&proposal.additions)
                .all(
                    |((row_id, count, _), addition)| row_id == &addition.template_row_id
                        && count == &addition.count
                ),
        "Structured repair targets do not reproduce the compiled generation plan"
    );
    Ok((
        plan,
        result.into_iter().map(|(_, _, brief)| brief).collect(),
    ))
}

async fn review_generated_rows(
    tasks: &[GenerationTask],
    target_briefs: &[dataset_quality_core::native_assessment::NativeTargetBrief],
    generation_store: Arc<ProjectGenerationStore>,
    review_store: Arc<ProjectNativeReviewStore>,
    review_runner: &DurableNativeReviewRunner,
    reviewer_identity_fingerprint: &str,
) -> Result<Vec<NativeAdmissionRecord>> {
    type ReviewItem = (
        NomosNativeAssessmentBinding,
        dataset_quality_core::native_assessment::NativeTargetBrief,
        Uuid,
    );
    let legacy_requests = tasks.iter().all(|task| task.execution_v3.is_none());
    let mut groups = std::collections::BTreeMap::<String, Vec<ReviewItem>>::new();
    let mut run_and_iteration = None;
    for (task_index, task) in tasks.iter().enumerate() {
        let history = generation_store.history(task.clone()).await?;
        let completed = history
            .iter()
            .filter_map(|outcome| outcome.admission.as_ref())
            .collect::<Vec<_>>();
        ensure!(
            completed.len() == 1,
            "Native semantic review requires one completed structural generation outcome"
        );
        let target = target_briefs
            .get(task.target_index as usize)
            .context("Generated task has no structured repair target")?
            .clone();
        run_and_iteration.get_or_insert((task.run_id, task.iteration));
        for row in &completed[0].accepted {
            let binding = NomosNativeAssessmentBinding::from_generated(
                encoder_optimization_core::generation::generated_semantic_row_id(task, row.index),
                &row.content,
            )?;
            let group =
                task.execution_v3
                    .as_ref()
                    .and_then(|execution| {
                        execution.contrast_pair_id.as_ref().map(|pair| {
                            format!("contrast:{pair}:{:08}", task.first_row + row.index)
                        })
                    })
                    .unwrap_or_else(|| format!("{task_index:08}:task"));
            groups
                .entry(group)
                .or_default()
                .push((binding, target.clone(), task.id));
        }
    }
    let mut batches = Vec::<Vec<ReviewItem>>::new();
    let mut batch = Vec::new();
    for group in groups.into_values() {
        ensure!(
            group.len() <= 8,
            "Native semantic review group exceeds eight rows"
        );
        if batch.len() + group.len() > 8 {
            batches.push(std::mem::take(&mut batch));
        }
        batch.extend(group);
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    let Some((run_id, iteration)) = run_and_iteration else {
        return Ok(Vec::new());
    };
    let mut recorded = Vec::new();
    for items in batches {
        let bindings = items
            .iter()
            .map(|(binding, _, _)| binding.clone())
            .collect::<Vec<_>>();
        let blind_request_id = if legacy_requests {
            encoder_optimization_core::child_id(&serde_json::json!({
                "protocol": "native-blind-review-v1",
                "task": items[0].2,
                "rows": bindings.iter().map(|binding| &binding.blind_row.row_fingerprint).collect::<Vec<_>>(),
                "reviewer": reviewer_identity_fingerprint,
            }))?
        } else {
            encoder_optimization_core::child_id(&serde_json::json!({
                "protocol": "native-blind-review-batch-v3",
                "rows": bindings.iter().map(|binding| (&binding.blind_row.row_id, &binding.blind_row.row_fingerprint)).collect::<Vec<_>>(),
                "reviewer": reviewer_identity_fingerprint,
            }))?
        };
        let blind_request = NativeBlindAssessmentRequest::new(
            blind_request_id,
            run_id,
            iteration,
            reviewer_identity_fingerprint,
            bindings
                .iter()
                .map(|binding| binding.blind_row.clone())
                .collect(),
        )?;
        // Re-running the explicit CLI command is the only authorization for
        // the single replacement of a recovered interrupted provider call.
        let blind_output = review_runner
            .assess_blind(blind_request.clone(), true)
            .await?;
        let blind_evidence = blind_output
            .assessments
            .into_iter()
            .map(|draft| NativeBlindAssessmentEvidence::record(&blind_request, draft))
            .collect::<Result<Vec<_>, _>>()?;
        let target_request_id = if legacy_requests {
            encoder_optimization_core::child_id(&serde_json::json!({
                "protocol": "native-target-fit-review-v1",
                "blindRequest": blind_request.fingerprint,
                "target": items[0].1.fingerprint,
                "reviewer": reviewer_identity_fingerprint,
            }))?
        } else {
            encoder_optimization_core::child_id(&serde_json::json!({
                "protocol": "native-target-fit-review-batch-v3",
                "blindRequest": blind_request.fingerprint,
                "targets": items.iter().map(|(_, target, _)| &target.fingerprint).collect::<Vec<_>>(),
                "reviewer": reviewer_identity_fingerprint,
            }))?
        };
        let target_request = NativeTargetFitRequest::new(
            target_request_id,
            &blind_request,
            reviewer_identity_fingerprint,
            blind_evidence
                .iter()
                .cloned()
                .zip(items.iter().map(|(_, target, _)| target.clone()))
                .collect(),
        )?;
        let fit_output = review_runner
            .assess_target_fit(target_request.clone(), true)
            .await?;
        let fit_evidence = fit_output
            .assessments
            .into_iter()
            .map(|draft| NativeTargetFitEvidence::record(&target_request, draft))
            .collect::<Result<Vec<_>, _>>()?;
        let records = bindings
            .into_iter()
            .zip(blind_evidence)
            .zip(fit_evidence)
            .map(|((binding, blind), target_fit)| {
                NativeAdmissionRecord::new(
                    run_id,
                    iteration,
                    reviewer_identity_fingerprint,
                    binding.label_authority,
                    blind,
                    target_fit,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        review_store.record_admissions(records.clone()).await?;
        recorded.extend(records);
    }
    Ok(recorded)
}
