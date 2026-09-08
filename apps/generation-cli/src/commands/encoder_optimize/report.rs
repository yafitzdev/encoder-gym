//! Management reports derived from immutable optimization and experiment facts.
use super::{load_launch, load_optional_campaign, next_command};
use crate::{
    cli::EncoderOptimizeRunArgs, commands::production_repair::load_approved_delta_facts,
    presentation,
};
use anyhow::Context;
use encoder_campaign_core::{
    CampaignStore,
    optimization::{OptimizationLaunchStore, OptimizationRunState},
};
use encoder_experiment_core::{journal::ExperimentEventKind, ports::ExperimentStore};
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::ports::{NativeRepairTrainingStore, RepairEvidenceStore};

pub(super) async fn execute(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let campaign = load_optional_campaign(store, &context).await?;
    let experiment = campaign
        .as_ref()
        .and_then(|value| value.experiment.as_ref());
    let project = store
        .get_project(context.definition.project.id)
        .await?
        .context("optimization project does not exist")?;
    let protocol = store.get_protocol(context.run.reserved_protocol_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    let diagnosis = store
        .get_diagnosis(approved.proposal.context.diagnosis.id)
        .await?
        .context("optimization repair diagnosis does not exist")?;
    let experiment_events = store
        .load_events(context.run.reserved_experiment_run_id)
        .await?;
    let campaign_events = store
        .list_campaign_events(context.run.reserved_campaign_id)
        .await?;
    let optimization_events = store.list_optimization_events(context.run.id).await?;
    let baseline_by_suite = protocol
        .as_ref()
        .into_iter()
        .flat_map(|protocol| protocol.baseline_development_reports())
        .map(|value| (value.suite_key.as_str(), value))
        .collect::<std::collections::BTreeMap<_, _>>();
    let candidate_results = experiment
        .map(|value| {
            value
                .candidates
                .iter()
                .map(|(id, execution)| {
                    let development = execution
                        .development_assessments
                        .iter()
                        .map(|(suite, assessment)| {
                            let baseline = baseline_by_suite
                                .get(suite.as_str())
                                .expect("deep replay requires a baseline development suite");
                            let candidate = execution
                                .development_reports
                                .get(suite)
                                .expect("deep replay requires a candidate development report");
                            serde_json::json!({
                                "suite": suite,
                                "baseline_report_id": baseline.id,
                                "baseline_metrics": baseline.metrics,
                                "candidate_report_id": candidate.id,
                                "candidate_metrics": candidate.metrics,
                                "assessment_id": assessment.id,
                                "verdict": assessment.verdict,
                                "primary_improvement": assessment.primary_improvement,
                                "failed_gates": assessment.gates.iter().filter(|gate| !gate.passed).collect::<Vec<_>>(),
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!({
                        "candidate_id": id,
                        "state": execution.state,
                        "checkpoint": execution.train_output.as_ref().map(|output| serde_json::json!({
                            "key": output.model.key,
                            "format": output.model.format,
                            "bytes": output.model.bytes,
                            "fingerprint": output.model.fingerprint,
                            "training_duration_seconds": output.duration_seconds,
                            "metadata": output.metadata,
                        })),
                        "development_suites": development,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let training_seconds = experiment
        .map(|value| {
            value
                .candidates
                .values()
                .filter_map(|candidate| candidate.train_output.as_ref())
                .map(|output| output.duration_seconds)
                .sum::<u64>()
        })
        .unwrap_or_default();
    let candidate_failures = experiment_events
        .iter()
        .filter(|event| matches!(event.event, ExperimentEventKind::CandidateFailed { .. }))
        .count();
    let adopted = campaign_events.iter().any(|event| {
        matches!(
            event.event,
            encoder_campaign_core::CampaignEventKind::RunAdopted { .. }
        )
    });
    let sealed_used = experiment.is_some_and(|value| value.sealed_report.is_some());
    let development_evaluations = experiment
        .map(|value| {
            value
                .candidates
                .values()
                .map(|candidate| candidate.development_reports.len())
                .sum::<usize>()
        })
        .unwrap_or_default();
    let trained_candidates = experiment
        .map(|value| {
            value
                .candidates
                .values()
                .filter(|candidate| candidate.train_output.is_some())
                .count()
        })
        .unwrap_or_default();
    let mut evidence_limits = vec![
        format!("{} candidate(s) were configured; {} produced a completed training artifact.", context.definition.candidates.len(), trained_candidates),
        "Results apply to the pinned project, training snapshot, benchmark generation, and metric contract.".to_owned(),
        if sealed_used {
            "Candidate sealed evidence covers only the development-selected candidate.".to_owned()
        } else {
            "No candidate sealed result is available for this run.".to_owned()
        },
    ];
    if context.view.state != OptimizationRunState::Completed {
        evidence_limits.push("The optimization has not completed its final decision.".to_owned());
    }
    presentation::print(&serde_json::json!({
        "schema_version": 1,
        "run_id": context.run.id,
        "name": context.definition.name,
        "state": context.view.state,
        "decision": context.view.decision.or_else(|| experiment.and_then(|value| value.final_decision)),
        "production_baseline_changed": false,
        "project": {
            "id": project.id,
            "revision": project.source_revision,
            "fingerprint": project.fingerprint,
            "baseline_model": project.baseline_model,
        },
        "diagnosis": {
            "id": diagnosis.id,
            "fingerprint": diagnosis.fingerprint,
            "weaknesses": diagnosis.weaknesses,
            "source_campaign_id": diagnosis.source_campaign_id,
            "source_experiment_run_id": diagnosis.source_experiment_run_id,
        },
        "approved_repair": {
            "proposal_id": approved.proposal.id,
            "proposal_fingerprint": approved.proposal.fingerprint,
            "targets": approved.proposal.targets,
            "actions": approved.proposal.actions,
            "candidate_hypotheses": approved.proposal.candidates,
            "proposal_application": approved.selection.application,
            "delta_approval": approved.approval,
            "delta_selection_id": approved.selection.id,
            "delta_selection_fingerprint": approved.selection.fingerprint,
        },
        "training_data_change": {
            "snapshot_id": snapshot.id,
            "snapshot_fingerprint": snapshot.fingerprint,
            "base_rows": snapshot.base_rows,
            "delta_rows": snapshot.delta_rows,
            "total_rows": snapshot.total_rows,
            "combined_membership_fingerprint": snapshot.combined_membership_fingerprint,
            "inputs": snapshot.inputs,
        },
        "selected_candidate_id": experiment.and_then(|value| value.selected_candidate_id),
        "sealed_evidence": {
            "used": sealed_used,
            "candidate_exposures": usize::from(sealed_used),
            "generation_id": context.definition.benchmark.generation_id,
            "generation_state": campaign.as_ref().and_then(|value| value.generation.as_ref()).map(|value| value.1.state),
            "authorization": experiment.and_then(|value| value.sealed_authorized_by.as_deref()),
        },
        "candidate_results": candidate_results,
        "budget_and_recovery": {
            "maximum": context.definition.campaign_budget,
            "observed_training_seconds": training_seconds,
            "observed_development_evaluations": development_evaluations,
            "observed_sealed_evaluations": usize::from(sealed_used),
            "candidate_failure_events": candidate_failures,
            "adopted_previously_proven_run": adopted,
            "optimization_event_count": optimization_events.len(),
            "campaign_event_count": campaign_events.len(),
            "experiment_event_count": experiment_events.len(),
        },
        "known_evidence_limits": evidence_limits,
        "next_command": next_command(&context, campaign.as_ref()),
        "provenance_head": context.view.last_event_fingerprint,
    }))
}
