use std::{path::Path, time::Duration};

use analysis_core::{
    domain::FindingIdentity,
    ports::{AnalysisFindingQuery, AnalysisStore, FindingEvidenceQuery},
    runner::{reproduce_report_fingerprint, verify_report_evidence},
};
use dataset_core::ports::SnapshotStore;
use evaluation_core::{
    comparison::{comparison_fingerprint, selection_fingerprint},
    domain::EvaluationRunState,
    ports::EvaluationStore,
};
use generation_core::ports::{
    BackendConfigurationStore, DatasetStore, JobStore, PlanStore, RowStore,
};
use optimization_core::{
    campaigns::{
        CampaignArtifactKind, CampaignArtifactLink, OptimizationCampaign,
        link_candidate_evaluation, link_checkpoint, link_comparison, link_follow_up_analysis,
        link_generation_job, link_generation_plan, link_snapshot, link_training_run,
    },
    planning::verify_constrained_proposal,
    ports::{CampaignQuery, OptimizationStore, ProposalReviewQuery},
    scenarios::{OptimizationScenario, ScenarioPairComparison},
};
use project_config::{GenerationBackendKind, ResolvedProjectConfig};
use serde::{Serialize, de::DeserializeOwned};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::{EncoderRegistry, TrainingStore};
use training_linear::verify_file;
use training_transformer::BertBundle;
use workflow_core::{
    advisor::AdvisoryAssessment,
    allocation::InitialAllocationRecord,
    approval::WorkflowApprovalDecision,
    benchmark::{AcceptanceAssessment, BenchmarkSuite},
    governance::EvidenceExposure,
    ports::{WorkflowDefinitionQuery, WorkflowRunQuery, WorkflowRunStore},
    promotion::ModelPromotion,
    stop::StopDecision,
};

use crate::{cli::DoctorArgs, presentation};

use super::config;

#[derive(Debug, Serialize)]
struct DoctorReport {
    healthy: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: &'static str,
    status: CheckStatus,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Pass,
    Warning,
    Fail,
    Skipped,
}

pub async fn execute(args: DoctorArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let mut checks = database_checks(store).await;
    let configured = match args.config.as_deref() {
        Some(path) => match config::resolve_path(path) {
            Ok(config) => {
                checks.push(pass(
                    "configuration",
                    format!("{} is valid ({})", path.display(), config.fingerprint()?),
                ));
                Some(config)
            }
            Err(error) => {
                checks.push(fail("configuration", error.to_string()));
                None
            }
        },
        None => {
            checks.push(skipped(
                "configuration",
                "no --config file was supplied".into(),
            ));
            None
        }
    };
    checks.push(artifact_check(configured.as_ref()));
    checks.extend(model_artifact_checks(store).await);
    checks.push(backend_check(args.check_backend, configured.as_ref(), store).await);
    let healthy = checks
        .iter()
        .all(|check| !matches!(check.status, CheckStatus::Fail));
    presentation::print(&DoctorReport { healthy, checks })?;
    anyhow::ensure!(healthy, "one or more doctor checks failed");
    Ok(())
}

async fn model_artifact_checks(store: &SqliteStore) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    match store.list_encoders().await {
        Ok(encoders) if encoders.is_empty() => checks.push(skipped(
            "registered_encoders",
            "no local encoders are registered".into(),
        )),
        Ok(encoders) => {
            let mut failures = Vec::new();
            for encoder in &encoders {
                if let Err(error) = BertBundle::verify(encoder) {
                    failures.push(format!("{} ({}): {error}", encoder.name, encoder.id));
                }
            }
            if failures.is_empty() {
                checks.push(pass(
                    "registered_encoders",
                    format!("{} registered bundle(s) verified", encoders.len()),
                ));
            } else {
                checks.push(fail("registered_encoders", failures.join("; ")));
            }
        }
        Err(error) => checks.push(fail("registered_encoders", error.to_string())),
    }

    match store.list_training_runs().await {
        Ok(runs) => {
            let mut checked = 0_usize;
            let mut failures = Vec::new();
            for run in runs {
                match store.list_checkpoints(run.id).await {
                    Ok(checkpoints) => {
                        for checkpoint in checkpoints {
                            checked += 1;
                            if let Err(error) = verify_file(
                                &checkpoint.artifact_path,
                                &checkpoint.artifact_checksum,
                                checkpoint.artifact_size_bytes,
                            ) {
                                failures.push(format!("{}: {error}", checkpoint.id));
                            }
                        }
                    }
                    Err(error) => failures.push(format!("run {}: {error}", run.id)),
                }
            }
            if !failures.is_empty() {
                checks.push(fail("training_checkpoints", failures.join("; ")));
            } else if checked == 0 {
                checks.push(skipped(
                    "training_checkpoints",
                    "no checkpoint artifacts are registered".into(),
                ));
            } else {
                checks.push(pass(
                    "training_checkpoints",
                    format!("{checked} checkpoint artifact(s) verified"),
                ));
            }
        }
        Err(error) => checks.push(fail("training_checkpoints", error.to_string())),
    }
    checks
}

async fn database_checks(store: &SqliteStore) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    match sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_one(store.pool())
        .await
    {
        Ok(result) if result == "ok" => checks.push(pass("database_integrity", result)),
        Ok(result) => checks.push(fail("database_integrity", result)),
        Err(error) => checks.push(fail("database_integrity", error.to_string())),
    }
    match sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(store.pool())
        .await
    {
        Ok(rows) if rows.is_empty() => checks.push(pass("foreign_keys", "no violations".into())),
        Ok(rows) => checks.push(fail("foreign_keys", format!("{} violation(s)", rows.len()))),
        Err(error) => checks.push(fail("foreign_keys", error.to_string())),
    }
    match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 0")
        .fetch_one(store.pool())
        .await
    {
        Ok(0) => checks.push(pass(
            "migrations",
            "all applied migrations succeeded".into(),
        )),
        Ok(count) => checks.push(fail("migrations", format!("{count} failed migration(s)"))),
        Err(error) => checks.push(fail("migrations", error.to_string())),
    }
    checks.push(evaluation_facts_check(store).await);
    checks.push(analysis_facts_check(store).await);
    checks.push(optimization_facts_check(store).await);
    checks.push(workflow_facts_check(store).await);
    checks
}

async fn workflow_facts_check(store: &SqliteStore) -> DoctorCheck {
    let definitions = match store
        .query_workflow_definitions(WorkflowDefinitionQuery {
            dataset_id: None,
            limit: 10_000,
            offset: 0,
        })
        .await
    {
        Ok(values) => values,
        Err(error) => return fail("workflow_facts", error.to_string()),
    };
    let runs = match store
        .query_workflow_runs(WorkflowRunQuery {
            definition_id: None,
            state: None,
            limit: 10_000,
            offset: 0,
        })
        .await
    {
        Ok(values) => values,
        Err(error) => return fail("workflow_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for definition in &definitions {
        if definition.reproduce_fingerprint().ok().as_deref()
            != Some(definition.fingerprint.as_str())
        {
            failures.push(format!("definition {} fingerprint mismatch", definition.id));
        }
    }
    for run in &runs {
        let Some(definition) = definitions
            .iter()
            .find(|definition| definition.id == run.definition_id)
        else {
            failures.push(format!("run {} definition is missing", run.id));
            continue;
        };
        if run.definition_fingerprint != definition.fingerprint {
            failures.push(format!("run {} definition fingerprint mismatch", run.id));
        }
        let attempts = match store.list_workflow_attempts(run.id).await {
            Ok(values) => values,
            Err(error) => {
                failures.push(format!("run {}: {error}", run.id));
                continue;
            }
        };
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt.reproduce_fingerprint().ok().as_deref() != Some(attempt.fingerprint.as_str())
                || usize::try_from(attempt.sequence).ok() != Some(index)
                || index > 0
                    && (attempt.predecessor_id != Some(attempts[index - 1].id)
                        || attempt.predecessor_fingerprint.as_deref()
                            != Some(attempts[index - 1].fingerprint.as_str()))
            {
                failures.push(format!("run {} attempt chain is invalid", run.id));
                break;
            }
        }
        if attempts.last().map(|attempt| attempt.id) != run.latest_attempt_id
            || attempts.last().map(|attempt| attempt.fingerprint.as_str())
                != run.latest_attempt_fingerprint.as_deref()
        {
            failures.push(format!("run {} latest attempt projection mismatch", run.id));
        }
        if run.usage.validate_against(&definition.budget).is_err() {
            failures.push(format!("run {} exceeds its resolved budget", run.id));
        }
        let normalized_links = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_artifact_links links \
             JOIN workflow_stage_attempts attempts \
               ON attempts.id = links.workflow_stage_attempt_id \
             WHERE attempts.workflow_run_id = ?",
        )
        .bind(run.id)
        .fetch_one(store.pool())
        .await
        .unwrap_or(-1);
        let expected_links = attempts
            .iter()
            .map(|attempt| attempt.artifacts.len())
            .sum::<usize>();
        if usize::try_from(normalized_links).ok() != Some(expected_links) {
            failures.push(format!("run {} normalized artifact links differ", run.id));
        }
    }
    let artifact_checks = [
        verify_artifact_json::<InitialAllocationRecord, _>(
            store,
            "workflow_initial_allocations",
            |value| {
                value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
                    && value.result.reproduce_fingerprint().ok().as_deref()
                        == Some(&value.result.fingerprint)
            },
        )
        .await,
        verify_artifact_json::<BenchmarkSuite, _>(store, "workflow_benchmark_suites", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<AcceptanceAssessment, _>(
            store,
            "workflow_acceptance_assessments",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<EvidenceExposure, _>(
            store,
            "workflow_evidence_exposures",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<AdvisoryAssessment, _>(store, "advisory_assessments", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<WorkflowApprovalDecision, _>(
            store,
            "workflow_approval_decisions",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<StopDecision, _>(store, "workflow_stop_decisions", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<ModelPromotion, _>(store, "model_promotions", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
    ];
    let mut checked_artifacts = 0;
    for result in artifact_checks {
        match result {
            Ok(count) => checked_artifacts += count,
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        pass(
            "workflow_facts",
            format!(
                "{} definition(s), {} run chain(s), and {} workflow artifact(s) verified",
                definitions.len(),
                runs.len(),
                checked_artifacts,
            ),
        )
    } else {
        fail("workflow_facts", failures.join("; "))
    }
}

async fn verify_artifact_json<T, F>(
    store: &SqliteStore,
    table: &'static str,
    verify: F,
) -> Result<usize, String>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    let rows = sqlx::query_scalar::<_, String>(&format!("SELECT artifact_json FROM {table}"))
        .fetch_all(store.pool())
        .await
        .map_err(|error| format!("{table}: {error}"))?;
    for (index, json) in rows.iter().enumerate() {
        let artifact = serde_json::from_str::<T>(json)
            .map_err(|error| format!("{table} row {index}: {error}"))?;
        if !verify(&artifact) {
            return Err(format!("{table} row {index}: fingerprint mismatch"));
        }
    }
    Ok(rows.len())
}

async fn optimization_facts_check(store: &SqliteStore) -> DoctorCheck {
    let proposals = match store.list_optimization_proposals().await {
        Ok(proposals) => proposals,
        Err(error) => return fail("optimization_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for proposal in &proposals {
        if let Some(source) = &proposal.source_identity {
            if let Err(error) = verify_constrained_proposal(proposal) {
                failures.push(format!("{}: {error}", proposal.id));
                continue;
            }
            match store.get_analysis_report(source.analysis_report_id).await {
                Ok(Some(report))
                    if report.fingerprint == source.analysis_fingerprint
                        && report.protocol_fingerprint == source.analysis_protocol_fingerprint => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: analysis identity mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source analysis missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            match store.get_dataset(source.dataset_id).await {
                Ok(Some(dataset))
                    if artifact_core::fingerprint(&dataset).ok().as_deref()
                        == Some(source.dataset_fingerprint.as_str()) => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: dataset fingerprint mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source dataset missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            match store.get_snapshot(source.snapshot_id).await {
                Ok(Some(snapshot))
                    if snapshot.fingerprint == source.snapshot_fingerprint
                        && snapshot.source_dataset_id == source.dataset_id => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: snapshot identity mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source snapshot missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            let normalized_counts = [
                (
                    "decisions",
                    "optimization_decision_cells",
                    proposal.decision_cells.len(),
                ),
                (
                    "data recommendations",
                    "optimization_recommendations",
                    proposal.normalized_recommendations.len(),
                ),
                (
                    "review recommendations",
                    "optimization_review_only_recommendations",
                    proposal.review_only_recommendations.len(),
                ),
                (
                    "coverage cells",
                    "optimization_evidence_coverage",
                    proposal
                        .decision_evidence
                        .as_ref()
                        .map_or(0, |evidence| evidence.current_coverage.len()),
                ),
            ];
            for (name, table, expected) in normalized_counts {
                let sql = format!("SELECT COUNT(*) FROM {table} WHERE proposal_id = ?");
                let actual = sqlx::query_scalar::<_, i64>(&sql)
                    .bind(proposal.id)
                    .fetch_one(store.pool())
                    .await
                    .unwrap_or(-1);
                if usize::try_from(actual).ok() != Some(expected) {
                    failures.push(format!("{}: normalized {name} differ", proposal.id));
                }
            }
            if let Some(set) = &proposal.training_candidate_set {
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM optimization_training_candidates WHERE proposal_id = ?",
                )
                .bind(proposal.id)
                .fetch_one(store.pool())
                .await
                .unwrap_or(-1);
                if usize::try_from(count).ok() != Some(set.candidates.len()) {
                    failures.push(format!(
                        "{}: normalized training candidates differ",
                        proposal.id
                    ));
                }
                if store
                    .get_training_run(set.configuration_space.baseline.training_run_id)
                    .await
                    .ok()
                    .flatten()
                    .is_none()
                    || store
                        .get_checkpoint(set.configuration_space.baseline.checkpoint_id)
                        .await
                        .ok()
                        .flatten()
                        .is_none()
                {
                    failures.push(format!(
                        "{}: training baseline artifact missing",
                        proposal.id
                    ));
                }
            }
            if let Some(lineage) = &proposal.rebase_lineage {
                match store
                    .get_optimization_proposal(lineage.previous_proposal_id)
                    .await
                {
                    Ok(Some(previous))
                        if previous.fingerprint == lineage.previous_proposal_fingerprint => {}
                    _ => failures.push(format!("{}: rebase predecessor mismatch", proposal.id)),
                }
            }
        }
        let reviews = match store
            .query_proposal_reviews(ProposalReviewQuery {
                proposal_id: Some(proposal.id),
                state: None,
                limit: u32::MAX,
                offset: 0,
            })
            .await
        {
            Ok(reviews) => reviews,
            Err(error) => {
                failures.push(format!("{}: {error}", proposal.id));
                Vec::new()
            }
        };
        for review in &reviews {
            if let Err(error) = review.validate(proposal) {
                failures.push(format!("{} review {}: {error}", proposal.id, review.id));
            }
        }
        if let Ok(Some(application)) = store.get_proposal_application(proposal.id).await {
            if let Some(review_id) = application.approval_review_id {
                let approval = reviews.iter().find(|review| review.id == review_id);
                if approval.is_none_or(|review| {
                    application.approval_fingerprint.as_deref() != Some(review.fingerprint.as_str())
                        || review
                            .selected_data_recommendation_ids(proposal)
                            .ok()
                            .as_ref()
                            != Some(&application.selected_recommendation_ids)
                }) {
                    failures.push(format!("{}: approved application mismatch", proposal.id));
                }
            } else if proposal.source_identity.is_some() {
                failures.push(format!(
                    "{}: decision-grade application lacks approval",
                    proposal.id
                ));
            }
        }
    }

    let scenario_groups = match store.list_scenario_groups().await {
        Ok(groups) => groups,
        Err(error) => {
            failures.push(error.to_string());
            Vec::new()
        }
    };
    for group in &scenario_groups {
        if let Err(error) = group.validate() {
            failures.push(format!("scenario group {}: {error}", group.id));
        }
        let scenarios = sqlx::query_scalar::<_, String>(
            "SELECT scenario_json FROM optimization_scenarios \
             WHERE group_id = ? ORDER BY scenario_index",
        )
        .bind(group.id)
        .fetch_all(store.pool())
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .map(|json| serde_json::from_str::<OptimizationScenario>(&json).ok())
                .collect::<Option<Vec<_>>>()
        });
        if scenarios.as_ref() != Some(&group.scenarios) {
            failures.push(format!(
                "scenario group {}: normalized scenarios differ",
                group.id
            ));
        }
        let comparisons = sqlx::query_scalar::<_, String>(
            "SELECT comparison_json FROM optimization_scenario_comparisons \
             WHERE group_id = ? ORDER BY comparison_index",
        )
        .bind(group.id)
        .fetch_all(store.pool())
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .map(|json| serde_json::from_str::<ScenarioPairComparison>(&json).ok())
                .collect::<Option<Vec<_>>>()
        });
        if comparisons.as_ref() != Some(&group.comparisons) {
            failures.push(format!(
                "scenario group {}: normalized comparisons differ",
                group.id
            ));
        }
    }

    let campaigns = match store
        .query_campaigns(CampaignQuery {
            proposal_id: None,
            limit: u32::MAX,
            offset: 0,
        })
        .await
    {
        Ok(campaigns) => campaigns,
        Err(error) => {
            failures.push(error.to_string());
            Vec::new()
        }
    };
    for campaign in &campaigns {
        if let Err(error) = campaign.validate() {
            failures.push(format!("campaign {}: {error}", campaign.id));
        }
        let proposal = store
            .get_optimization_proposal(campaign.proposal_id)
            .await
            .ok()
            .flatten();
        let approval = store
            .query_proposal_reviews(ProposalReviewQuery {
                proposal_id: Some(campaign.proposal_id),
                state: None,
                limit: u32::MAX,
                offset: 0,
            })
            .await
            .ok()
            .and_then(|reviews| {
                reviews
                    .into_iter()
                    .find(|review| review.id == campaign.approval_review_id)
            });
        match (proposal.as_ref(), approval.as_ref()) {
            (Some(proposal), Some(approval)) => {
                if let Err(error) = campaign.validate_decision(proposal, approval) {
                    failures.push(format!("campaign {} decision: {error}", campaign.id));
                }
            }
            _ => failures.push(format!(
                "campaign {}: proposal or approval missing",
                campaign.id
            )),
        }
        let links = match store.list_campaign_links(campaign.id).await {
            Ok(links) => {
                for link in &links {
                    if let Err(error) = link.validate(campaign) {
                        failures.push(format!("campaign link {}: {error}", link.id));
                    }
                    if let Err(error) = verify_campaign_link(store, campaign, &links, link).await {
                        failures.push(format!("campaign link {}: {error}", link.id));
                    }
                }
                links
            }
            Err(error) => {
                failures.push(format!("campaign {}: {error}", campaign.id));
                Vec::new()
            }
        };
        if let Ok(Some(outcome)) = store.get_campaign_outcome(campaign.id).await {
            match store.get_comparison(outcome.comparison_id).await {
                Ok(Some(comparison)) => {
                    if let Some(proposal) = proposal.as_ref() {
                        let coverage = store
                            .dataset_cell_counts(campaign.dataset_id)
                            .await
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(key, counts)| (key, counts.accepted))
                            .collect();
                        if let Err(error) = outcome.validate_reproduction(
                            campaign,
                            proposal,
                            &links,
                            &comparison,
                            &coverage,
                        ) {
                            failures.push(format!("outcome {}: {error}", outcome.id));
                        }
                    } else {
                        failures.push(format!("outcome {}: proposal missing", outcome.id));
                    }
                }
                _ => failures.push(format!("outcome {}: comparison missing", outcome.id)),
            }
        }
    }
    if failures.is_empty() {
        pass(
            "optimization_facts",
            format!(
                "{} proposal(s), {} scenario group(s), and {} campaign(s) verified",
                proposals.len(),
                scenario_groups.len(),
                campaigns.len()
            ),
        )
    } else {
        fail("optimization_facts", failures.join("; "))
    }
}

async fn verify_campaign_link(
    store: &SqliteStore,
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    actual: &CampaignArtifactLink,
) -> Result<(), String> {
    let expected = match actual.artifact_kind {
        CampaignArtifactKind::GenerationPlan => {
            let plan = required_artifact(store.get_plan(actual.artifact_id).await, "plan")?;
            let application = required_artifact(
                store.get_proposal_application(campaign.proposal_id).await,
                "proposal application",
            )?;
            link_generation_plan(campaign, &plan, &application)
        }
        CampaignArtifactKind::GenerationJob => {
            let job = required_artifact(store.get_job(actual.artifact_id).await, "job")?;
            link_generation_job(campaign, links, &job)
        }
        CampaignArtifactKind::DatasetSnapshot => {
            let snapshot =
                required_artifact(store.get_snapshot(actual.artifact_id).await, "snapshot")?;
            let members = store
                .list_snapshot_members(snapshot.id)
                .await
                .map_err(|error| error.to_string())?;
            link_snapshot(campaign, links, &snapshot, &members)
        }
        CampaignArtifactKind::TrainingRun => {
            let run = required_artifact(
                store.get_training_run(actual.artifact_id).await,
                "training run",
            )?;
            link_training_run(campaign, links, &run)
        }
        CampaignArtifactKind::TrainingCheckpoint => {
            let checkpoint =
                required_artifact(store.get_checkpoint(actual.artifact_id).await, "checkpoint")?;
            link_checkpoint(campaign, links, &checkpoint)
        }
        CampaignArtifactKind::CandidateEvaluation => {
            let evaluation = required_artifact(
                store.get_evaluation_run(actual.artifact_id).await,
                "evaluation run",
            )?;
            link_candidate_evaluation(campaign, links, &evaluation)
        }
        CampaignArtifactKind::EvaluationComparison => {
            let comparison =
                required_artifact(store.get_comparison(actual.artifact_id).await, "comparison")?;
            link_comparison(campaign, links, &comparison)
        }
        CampaignArtifactKind::FollowUpAnalysis => {
            let report = required_artifact(
                store.get_analysis_report(actual.artifact_id).await,
                "analysis report",
            )?;
            link_follow_up_analysis(campaign, links, &report)
        }
    }
    .map_err(|error| error.to_string())?;
    if expected.artifact_kind != actual.artifact_kind
        || expected.artifact_id != actual.artifact_id
        || expected.artifact_fingerprint != actual.artifact_fingerprint
        || expected.details != actual.details
    {
        return Err("persisted link does not reproduce from its artifact".into());
    }
    Ok(())
}

fn required_artifact<T, E: std::fmt::Display>(
    result: Result<Option<T>, E>,
    name: &str,
) -> Result<T, String> {
    result
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{name} is missing"))
}

async fn analysis_facts_check(store: &SqliteStore) -> DoctorCheck {
    let reports = match store.list_analysis_reports().await {
        Ok(reports) => reports,
        Err(error) => return fail("analysis_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for report in &reports {
        let legacy = report.protocol_fingerprint.is_empty()
            || report
                .source_identity
                .as_ref()
                .is_none_or(|source| source.evaluation_input_fingerprint.starts_with("legacy:"));
        if legacy {
            continue;
        }
        if let Err(error) = verify_report_evidence(store, report).await {
            failures.push(format!("{}: {error}", report.id));
        }
        if report.protocol.validate().is_err()
            || report.protocol.fingerprint().ok().as_deref()
                != Some(report.protocol_fingerprint.as_str())
        {
            failures.push(format!("{}: protocol fingerprint mismatch", report.id));
        }
        if reproduce_report_fingerprint(report).ok().as_deref() != Some(report.fingerprint.as_str())
        {
            failures.push(format!("{}: report fingerprint mismatch", report.id));
        }
        let Some(source) = &report.source_identity else {
            failures.push(format!("{}: source identity missing", report.id));
            continue;
        };
        match store.get_evaluation_run(report.evaluation_run_id).await {
            Ok(Some(run)) => {
                let count = store.count_predictions(run.id).await.unwrap_or(u64::MAX);
                if run.state != EvaluationRunState::Completed
                    || count != report.prediction_count
                    || source.prediction_count != count
                    || source.evaluation_input_fingerprint != run.input_fingerprint
                    || source.evaluation_protocol_fingerprint != run.protocol_fingerprint
                    || source.cohort_fingerprint != run.source_identity.cohort_fingerprint
                {
                    failures.push(format!(
                        "{}: evaluation source identity mismatch",
                        report.id
                    ));
                }
            }
            Ok(None) => failures.push(format!("{}: evaluation run missing", report.id)),
            Err(error) => failures.push(format!("{}: {error}", report.id)),
        }
        if let Some(comparison_id) = source.comparison_id {
            match store.get_comparison(comparison_id).await {
                Ok(Some(comparison))
                    if source.comparison_fingerprint.as_deref()
                        == Some(comparison.fingerprint.as_str())
                        && (comparison.left_run_id == report.evaluation_run_id
                            || comparison.right_run_id == report.evaluation_run_id) => {}
                Ok(Some(_)) => failures.push(format!(
                    "{}: comparison source identity mismatch",
                    report.id
                )),
                Ok(None) => failures.push(format!("{}: comparison missing", report.id)),
                Err(error) => failures.push(format!("{}: {error}", report.id)),
            }
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut previous_cumulative = 0_u64;
        for (index, finding) in report.findings.iter().enumerate() {
            let identity = FindingIdentity {
                kind: finding.kind,
                attributes: finding.attributes.clone(),
            };
            if finding.rank != index as u64 + 1
                || identity.key() != finding.key
                || finding.reproduce_fingerprint().ok().as_deref()
                    != Some(finding.fingerprint.as_str())
                || !keys.insert(&finding.key)
                || finding.error_count > finding.support
                || finding.cumulative_error_count
                    != previous_cumulative.saturating_add(finding.marginal_error_count)
                || finding.cumulative_error_count > report.error_count
            {
                failures.push(format!("{}: invalid finding {}", report.id, finding.key));
            }
            previous_cumulative = finding.cumulative_error_count;
            let normalized_evidence = store
                .query_finding_evidence(FindingEvidenceQuery {
                    analysis_report_id: report.id,
                    finding_key: finding.key.clone(),
                    category: None,
                    limit: 1_000,
                    offset: 0,
                })
                .await;
            let expected_evidence = report
                .finding_evidence
                .get(&finding.key)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if normalized_evidence.as_deref() != Ok(expected_evidence) {
                failures.push(format!(
                    "{}: normalized evidence differs for {}",
                    report.id, finding.key
                ));
            }
        }
        let normalized = store
            .query_analysis_findings(AnalysisFindingQuery {
                analysis_report_id: report.id,
                kind: None,
                minimum_support: None,
                minimum_error_count: None,
                sort: Default::default(),
                limit: u32::MAX,
                offset: 0,
            })
            .await;
        if normalized.as_ref().ok() != Some(&report.findings) {
            failures.push(format!("{}: normalized findings differ", report.id));
        }
        let invalid_evidence: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM analysis_finding_evidence e \
             JOIN analysis_reports a ON a.id = e.analysis_report_id \
             JOIN evaluation_predictions p ON p.id = e.prediction_id \
             WHERE e.analysis_report_id = ? AND (p.evaluation_run_id <> a.evaluation_run_id \
             OR p.snapshot_member_id <> e.snapshot_member_id \
             OR p.source_row_id <> e.source_row_id)",
        )
        .bind(report.id)
        .fetch_one(store.pool())
        .await
        .unwrap_or(-1);
        if invalid_evidence != 0 {
            failures.push(format!("{}: invalid evidence links", report.id));
        }
    }
    let orphan_reviews: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM analysis_finding_reviews r \
         LEFT JOIN analysis_findings f ON f.analysis_report_id = r.analysis_report_id \
         AND f.finding_key = r.finding_key WHERE f.finding_key IS NULL",
    )
    .fetch_one(store.pool())
    .await
    .unwrap_or(-1);
    if orphan_reviews != 0 {
        failures.push(format!("orphan finding reviews: {orphan_reviews}"));
    }
    if failures.is_empty() {
        pass(
            "analysis_facts",
            format!("{} analysis report(s) verified", reports.len()),
        )
    } else {
        fail("analysis_facts", failures.join("; "))
    }
}

async fn evaluation_facts_check(store: &SqliteStore) -> DoctorCheck {
    let runs = match store.list_evaluation_runs().await {
        Ok(runs) => runs,
        Err(error) => return fail("evaluation_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for run in &runs {
        let count = match store.count_predictions(run.id).await {
            Ok(count) => count,
            Err(error) => {
                failures.push(format!("{}: {error}", run.id));
                continue;
            }
        };
        if run.state == EvaluationRunState::Completed
            && (count != run.total_examples || run.example_count != count || run.metrics.is_none())
        {
            failures.push(format!(
                "{}: completed run has inconsistent count or metrics",
                run.id
            ));
        }
    }
    let duplicate_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM (SELECT evaluation_run_id, snapshot_member_id FROM evaluation_predictions GROUP BY evaluation_run_id, snapshot_member_id HAVING COUNT(*) > 1)")
        .fetch_one(store.pool()).await.unwrap_or(-1);
    if duplicate_count != 0 {
        failures.push(format!("duplicate prediction groups: {duplicate_count}"));
    }
    let invalid_comparisons = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM evaluation_comparisons c JOIN evaluation_runs l ON l.id = c.left_run_id JOIN evaluation_runs r ON r.id = c.right_run_id WHERE l.state <> 'completed' OR r.state <> 'completed' OR l.snapshot_id <> r.snapshot_id OR l.split <> r.split OR l.protocol_fingerprint <> r.protocol_fingerprint")
        .fetch_one(store.pool()).await.unwrap_or(-1);
    if invalid_comparisons != 0 {
        failures.push(format!("invalid comparison reports: {invalid_comparisons}"));
    }
    match store.list_comparisons(u32::MAX, 0).await {
        Ok(reports) => {
            for report in reports {
                if comparison_fingerprint(&report).ok().as_deref() != Some(&report.fingerprint) {
                    failures.push(format!("{}: comparison fingerprint mismatch", report.id));
                }
            }
        }
        Err(error) => failures.push(error.to_string()),
    }
    match store.list_selections(u32::MAX, 0).await {
        Ok(reports) => {
            for report in reports {
                if selection_fingerprint(&report).ok().as_deref() != Some(&report.fingerprint) {
                    failures.push(format!("{}: selection fingerprint mismatch", report.id));
                }
                for candidate in &report.candidate_run_ids {
                    if !runs.iter().any(|run| {
                        run.id == *candidate && run.state == EvaluationRunState::Completed
                    }) {
                        failures.push(format!(
                            "{}: incomplete or missing candidate {candidate}",
                            report.id
                        ));
                    }
                }
            }
        }
        Err(error) => failures.push(error.to_string()),
    }
    if failures.is_empty() {
        pass(
            "evaluation_facts",
            format!("{} evaluation run(s) verified", runs.len()),
        )
    } else {
        fail("evaluation_facts", failures.join("; "))
    }
}

fn artifact_check(configured: Option<&ResolvedProjectConfig>) -> DoctorCheck {
    let path = configured.map_or_else(
        || Path::new("artifacts/training"),
        |config| config.training.artifact_root.as_path(),
    );
    if path.is_dir() {
        pass("artifact_directory", format!("{} exists", path.display()))
    } else if path.exists() {
        fail(
            "artifact_directory",
            format!("{} exists but is not a directory", path.display()),
        )
    } else {
        warning(
            "artifact_directory",
            format!(
                "{} will be created by the first training run",
                path.display()
            ),
        )
    }
}

async fn backend_check(
    requested: bool,
    configured: Option<&ResolvedProjectConfig>,
    store: &SqliteStore,
) -> DoctorCheck {
    if !requested {
        return skipped(
            "backend_connectivity",
            "use --check-backend to make a connectivity request".into(),
        );
    }
    if configured.is_some_and(|config| config.generation.backend == GenerationBackendKind::Fake) {
        return pass(
            "backend_connectivity",
            "the deterministic fake backend is local and available".into(),
        );
    }
    let configuration = match configured.and_then(ResolvedProjectConfig::backend_configuration) {
        Some(configuration) => Some(configuration),
        None => match store.get_backend_configuration("openai-compatible").await {
            Ok(configuration) => configuration,
            Err(error) => return fail("backend_connectivity", error.to_string()),
        },
    };
    let Some(configuration) = configuration else {
        return fail(
            "backend_connectivity",
            "the OpenAI-compatible backend is not configured".into(),
        );
    };
    let Some(base_url) = configuration.base_url else {
        return fail("backend_connectivity", "backend base URL is missing".into());
    };
    let api_key_env = configured.map_or("SYNTH_OPENAI_API_KEY", |config| {
        config.generation.api_key_env.as_str()
    });
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(error) => return fail("backend_connectivity", error.to_string()),
    };
    let mut request = client.get(&endpoint);
    if let Ok(api_key) = std::env::var(api_key_env) {
        request = request.bearer_auth(api_key);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => pass(
            "backend_connectivity",
            format!("{} responded successfully", safe_origin(&endpoint)),
        ),
        Ok(response) => fail(
            "backend_connectivity",
            format!(
                "{} returned HTTP {}",
                safe_origin(&endpoint),
                response.status()
            ),
        ),
        Err(error) => fail(
            "backend_connectivity",
            format!("{} could not be reached: {error}", safe_origin(&endpoint)),
        ),
    }
}

fn safe_origin(url: &str) -> String {
    reqwest::Url::parse(url).map_or_else(
        |_| "configured backend".into(),
        |url| {
            let host = url.host_str().unwrap_or("configured backend");
            match url.port() {
                Some(port) => format!("{}://{host}:{port}", url.scheme()),
                None => format!("{}://{host}", url.scheme()),
            }
        },
    )
}

fn pass(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Pass,
        message,
    }
}

fn warning(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Warning,
        message,
    }
}

fn fail(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Fail,
        message,
    }
}

fn skipped(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Skipped,
        message,
    }
}
