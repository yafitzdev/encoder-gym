//! Compose verified plan, publication and development facts into immutable
//! per-target feedback. This layer does not alter scientific acceptance.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Result, ensure};
use encoder_experiment_nomos::NomosRepairMetricPoint;
use encoder_optimization_core::{
    agent::DatasetEditProposal,
    fingerprint,
    repair_outcome::{
        OutcomeIdentity, REPAIR_OUTCOME_SCHEMA_VERSION, RepairEditCounts, RepairGlobalVerdict,
        RepairMetricObservation, RepairOutcome, aggregate_change, classify_change,
        intervention_fingerprint, intervention_summary,
    },
    repair_strategy::{RepairOperation, RepairPlan},
};
use project_workspace_core::{
    optimization_iteration::ProjectOptimizationIteration,
    optimization_iteration_execution::IterationDevelopmentResult,
};
use project_workspace_local::optimization_dataset::OptimizationDatasetPublication;

use super::registration::IterationCandidate;

pub(super) struct OutcomeSources<'a> {
    pub iteration: &'a ProjectOptimizationIteration,
    pub plan: &'a RepairPlan,
    pub proposal: &'a DatasetEditProposal,
    pub publication: &'a OptimizationDatasetPublication,
    pub input_points: &'a [NomosRepairMetricPoint],
    pub candidate_points: &'a [NomosRepairMetricPoint],
    pub development: &'a IterationDevelopmentResult,
    pub output_development_evidence_fingerprint: &'a str,
    pub candidate: &'a IterationCandidate,
}

pub(super) async fn record(folder: &Path, sources: OutcomeSources<'_>) -> Result<()> {
    let OutcomeSources {
        iteration,
        plan,
        proposal,
        publication,
        input_points,
        candidate_points,
        development,
        output_development_evidence_fingerprint,
        candidate,
    } = sources;
    ensure!(
        publication.run_id == iteration.scope.run_id
            && publication.iteration == iteration.scope.iteration
            && publication.proposal_fingerprint == fingerprint(proposal)?,
        "Repair outcome publication belongs to another proposal"
    );
    let reports = development
        .reports
        .values()
        .map(|report| OutcomeIdentity {
            id: report.id.to_string(),
            fingerprint: report.fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    let global_verdict = if development.development_passed {
        RepairGlobalVerdict::Keep
    } else {
        RepairGlobalVerdict::Reject
    };
    let input_by_key = input_points
        .iter()
        .map(|point| (point_key(point), point))
        .collect::<BTreeMap<_, _>>();
    let candidate_by_key = candidate_points
        .iter()
        .map(|point| (point_key(point), point))
        .collect::<BTreeMap<_, _>>();
    let mut outcomes = Vec::with_capacity(plan.targets.len());
    for target in &plan.targets {
        let mut observations = Vec::new();
        for candidate_point in candidate_points
            .iter()
            .filter(|point| point.target_id == target.target_id)
        {
            let key = point_key(candidate_point);
            let preceding = input_by_key.get(&key).copied();
            let original_baseline = candidate_point
                .original_baseline
                .or_else(|| preceding.and_then(|point| point.original_baseline));
            if let (Some(left), Some(right)) = (
                candidate_point.original_baseline,
                preceding.and_then(|point| point.original_baseline),
            ) {
                ensure!(
                    (left - right).abs() <= 1e-12,
                    "Repair outcome original baseline changed"
                );
            }
            let preceding_value = preceding.and_then(|point| point.value);
            let candidate_value = candidate_point.value;
            observations.push(RepairMetricObservation {
                cluster_key: candidate_point.cluster_key.clone(),
                suite: candidate_point.suite.clone(),
                metric: candidate_point.metric.clone(),
                expected_direction: target.target_metric.direction,
                support: candidate_point.support,
                original_baseline,
                preceding_candidate: preceding_value,
                candidate: candidate_value,
                delta_from_original_baseline: candidate_value
                    .zip(original_baseline)
                    .map(|(candidate, baseline)| candidate - baseline),
                delta_from_preceding_candidate: candidate_value
                    .zip(preceding_value)
                    .map(|(candidate, preceding)| candidate - preceding),
                change: classify_change(
                    target.target_metric.direction,
                    preceding_value,
                    candidate_value,
                ),
            });
        }
        for input_point in input_points
            .iter()
            .filter(|point| point.target_id == target.target_id)
        {
            ensure!(
                candidate_by_key.contains_key(&point_key(input_point)),
                "Repair outcome candidate population is missing a target metric point"
            );
        }
        let requested_removals = match &target.operation {
            RepairOperation::ProvenRedundantRowRemoval { removals } => removals.len() as u64,
            _ => 0,
        };
        let published_removals = match &target.operation {
            RepairOperation::ProvenRedundantRowRemoval { removals } => removals
                .iter()
                .filter(|removal| publication.removed.contains(&removal.row_id))
                .count()
                as u64,
            _ => 0,
        };
        let counts = publication
            .target_counts
            .iter()
            .filter(|counts| counts.target_id.as_deref() == Some(&target.target_id))
            .fold(
                RepairEditCounts {
                    requested_additions: 0,
                    generated: 0,
                    structurally_admitted: 0,
                    semantically_admitted: 0,
                    published_additions: 0,
                    requested_removals,
                    published_removals,
                },
                |mut total, counts| {
                    total.requested_additions += counts.requested;
                    total.generated += counts.generated;
                    total.structurally_admitted += counts.structurally_admitted;
                    total.semantically_admitted += counts.semantically_admitted;
                    total.published_additions += counts.published;
                    total
                },
            );
        let requested_from_plan = match &target.operation {
            RepairOperation::LabelPreservingVariants { anchors, .. } => anchors
                .iter()
                .map(|anchor| u64::from(anchor.additions))
                .sum(),
            RepairOperation::ExistingAnchorContrast { pairs, .. } => pairs
                .iter()
                .map(|pair| u64::from(pair.additions_per_side) * 2)
                .sum(),
            RepairOperation::ProvenRedundantRowRemoval { .. } => 0,
        };
        ensure!(
            counts.requested_additions == requested_from_plan,
            "Repair outcome counts do not reproduce the target allocation"
        );
        let metric_change = aggregate_change(
            &observations
                .iter()
                .map(|observation| observation.change)
                .collect::<Vec<_>>(),
        );
        let mut limitations = vec![
            target.evidence_limitations.clone(),
            "Observed development deltas are descriptive and do not establish causality or statistical significance.".into(),
        ];
        if observations.iter().any(|observation| {
            observation.change
                == encoder_optimization_core::repair_outcome::TargetMetricChange::Unavailable
        }) {
            limitations.push(
                "At least one target metric was unavailable on a comparable saved population."
                    .into(),
            );
        }
        let outcome = RepairOutcome {
            schema_version: REPAIR_OUTCOME_SCHEMA_VERSION,
            run_id: iteration.scope.run_id,
            iteration: iteration.scope.iteration,
            target_id: target.target_id.clone(),
            repair_plan_fingerprint: fingerprint(plan)?,
            proposal_fingerprint: fingerprint(proposal)?,
            intervention_fingerprint: intervention_fingerprint(target)?,
            cluster_keys: {
                let mut keys = target.cluster_keys.clone();
                keys.sort();
                keys
            },
            intervention: intervention_summary(target),
            hypothesis: target.hypothesis.clone(),
            input_dataset: OutcomeIdentity {
                id: publication.parent.id.to_string(),
                fingerprint: publication.parent.fingerprint.clone(),
            },
            output_dataset: OutcomeIdentity {
                id: publication.version.id.to_string(),
                fingerprint: publication.version.fingerprint.clone(),
            },
            candidate: OutcomeIdentity {
                id: candidate.model.id.to_string(),
                fingerprint: candidate.model.fingerprint.clone(),
            },
            input_development_evidence_fingerprint: iteration.development.fingerprint.clone(),
            output_development_evidence_fingerprint: output_development_evidence_fingerprint.into(),
            reports: reports.clone(),
            edits: counts,
            metric_change,
            observations,
            global_verdict,
            limitations,
        };
        outcome.validate()?;
        outcomes.push(outcome);
    }
    outcomes.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    project_workspace_local::optimization_repair_outcomes::record(folder, &outcomes).await
}

fn point_key(point: &NomosRepairMetricPoint) -> (String, String, String, String) {
    (
        point.target_id.clone(),
        point.cluster_key.clone(),
        point.suite.clone(),
        point.metric.clone(),
    )
}
