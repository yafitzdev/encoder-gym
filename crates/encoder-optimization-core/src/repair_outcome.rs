//! Measured feedback for one accepted repair target. These records describe
//! observed development changes; they do not claim causality or alter the
//! benchmark's existing candidate-selection rules.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    OptimizationError, fingerprint,
    repair_strategy::{MetricDirection, RepairOperation, RepairTarget},
    require,
};

pub const REPAIR_OUTCOME_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutcomeIdentity {
    pub id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetMetricChange {
    Improved,
    Regressed,
    Unchanged,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairGlobalVerdict {
    Keep,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairEditCounts {
    pub requested_additions: u64,
    pub generated: u64,
    pub structurally_admitted: u64,
    pub semantically_admitted: u64,
    pub published_additions: u64,
    pub requested_removals: u64,
    pub published_removals: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairMetricObservation {
    pub cluster_key: String,
    pub suite: String,
    pub metric: String,
    pub expected_direction: MetricDirection,
    pub support: Option<u64>,
    pub original_baseline: Option<f64>,
    pub preceding_candidate: Option<f64>,
    pub candidate: Option<f64>,
    pub delta_from_original_baseline: Option<f64>,
    pub delta_from_preceding_candidate: Option<f64>,
    pub change: TargetMetricChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairRemovalSummary {
    pub row_id: String,
    pub retained_row_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RepairInterventionSummary {
    LabelPreservingVariants { anchor_ids: Vec<String> },
    ExistingAnchorContrast { pair_ids: Vec<String> },
    ProvenRedundantRowRemoval { rows: Vec<RepairRemovalSummary> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairOutcome {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub iteration: u32,
    pub target_id: String,
    pub repair_plan_fingerprint: String,
    pub proposal_fingerprint: String,
    pub intervention_fingerprint: String,
    pub cluster_keys: Vec<String>,
    pub intervention: RepairInterventionSummary,
    pub hypothesis: String,
    pub input_dataset: OutcomeIdentity,
    pub output_dataset: OutcomeIdentity,
    pub candidate: OutcomeIdentity,
    pub input_development_evidence_fingerprint: String,
    pub output_development_evidence_fingerprint: String,
    pub reports: Vec<OutcomeIdentity>,
    pub edits: RepairEditCounts,
    pub metric_change: TargetMetricChange,
    pub observations: Vec<RepairMetricObservation>,
    pub global_verdict: RepairGlobalVerdict,
    pub limitations: Vec<String>,
}

impl RepairOutcome {
    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            self.schema_version == REPAIR_OUTCOME_SCHEMA_VERSION
                && !self.run_id.is_nil()
                && (1..=10).contains(&self.iteration)
                && valid_text(&self.target_id, 128)
                && valid_text(&self.hypothesis, 1_000)
                && !self.reports.is_empty()
                && !self.observations.is_empty()
                && self.observations.len() <= 128
                && !self.limitations.is_empty()
                && self.limitations.len() <= 8,
            "Invalid repair outcome identity",
        )?;
        require(
            !self.cluster_keys.is_empty()
                && self.cluster_keys.len() <= 4
                && sorted_unique_text(&self.cluster_keys, 160)
                && self.intervention.valid(),
            "Invalid repair outcome intervention",
        )?;
        for value in [
            &self.repair_plan_fingerprint,
            &self.proposal_fingerprint,
            &self.intervention_fingerprint,
            &self.input_development_evidence_fingerprint,
            &self.output_development_evidence_fingerprint,
        ] {
            valid_hash(value)?;
        }
        let mut report_identities = BTreeSet::new();
        for identity in
            self.reports
                .iter()
                .chain([&self.input_dataset, &self.output_dataset, &self.candidate])
        {
            require(
                valid_text(&identity.id, 256),
                "Invalid repair outcome artifact",
            )?;
            valid_hash(&identity.fingerprint)?;
        }
        require(
            self.reports.iter().all(|report| {
                report_identities.insert((report.id.as_str(), report.fingerprint.as_str()))
            }) && self.input_dataset != self.output_dataset,
            "Repeated repair report or unchanged output dataset",
        )?;
        require(
            self.edits.generated == self.edits.requested_additions
                && self.edits.structurally_admitted <= self.edits.generated
                && self.edits.semantically_admitted <= self.edits.structurally_admitted
                && self.edits.published_additions <= self.edits.semantically_admitted
                && self.edits.published_removals <= self.edits.requested_removals,
            "Invalid repair outcome edit counts",
        )?;
        let mut observations = BTreeSet::new();
        for observation in &self.observations {
            let expected_original_delta = observation
                .candidate
                .zip(observation.original_baseline)
                .map(|(candidate, baseline)| candidate - baseline);
            let expected_preceding_delta = observation
                .candidate
                .zip(observation.preceding_candidate)
                .map(|(candidate, preceding)| candidate - preceding);
            require(
                valid_text(&observation.cluster_key, 160)
                    && valid_text(&observation.suite, 128)
                    && valid_text(&observation.metric, 128)
                    && observations.insert((
                        observation.cluster_key.as_str(),
                        observation.suite.as_str(),
                        observation.metric.as_str(),
                    ))
                    && [
                        observation.original_baseline,
                        observation.preceding_candidate,
                        observation.candidate,
                        observation.delta_from_original_baseline,
                        observation.delta_from_preceding_candidate,
                    ]
                    .into_iter()
                    .flatten()
                    .all(f64::is_finite)
                    && same_number(
                        observation.delta_from_original_baseline,
                        expected_original_delta,
                    )
                    && same_number(
                        observation.delta_from_preceding_candidate,
                        expected_preceding_delta,
                    )
                    && observation.change
                        == classify_change(
                            observation.expected_direction,
                            observation.preceding_candidate,
                            observation.candidate,
                        ),
                "Invalid repair metric observation",
            )?;
        }
        require(
            self.limitations
                .iter()
                .all(|limitation| valid_text(limitation, 1_000))
                && self.metric_change
                    == aggregate_change(
                        &self
                            .observations
                            .iter()
                            .map(|observation| observation.change)
                            .collect::<Vec<_>>(),
                    ),
            "Invalid repair outcome summary",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= 262_144,
            "Repair outcome exceeds 256 KiB",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairOutcomeSummary {
    pub iteration: u32,
    pub target_id: String,
    pub intervention_fingerprint: String,
    pub cluster_keys: Vec<String>,
    pub intervention: RepairInterventionSummary,
    pub hypothesis: String,
    pub edits: RepairEditCounts,
    pub metric_change: TargetMetricChange,
    pub global_verdict: RepairGlobalVerdict,
    pub output_development_evidence_fingerprint: String,
    pub limitations: Vec<String>,
}

impl From<&RepairOutcome> for RepairOutcomeSummary {
    fn from(value: &RepairOutcome) -> Self {
        Self {
            iteration: value.iteration,
            target_id: value.target_id.clone(),
            intervention_fingerprint: value.intervention_fingerprint.clone(),
            cluster_keys: value.cluster_keys.clone(),
            intervention: value.intervention.clone(),
            hypothesis: value.hypothesis.clone(),
            edits: value.edits.clone(),
            metric_change: value.metric_change,
            global_verdict: value.global_verdict,
            output_development_evidence_fingerprint: value
                .output_development_evidence_fingerprint
                .clone(),
            limitations: value.limitations.clone(),
        }
    }
}

pub fn intervention_fingerprint(target: &RepairTarget) -> Result<String, OptimizationError> {
    let mut clusters = target.cluster_keys.clone();
    clusters.sort();
    fingerprint(&serde_json::json!({
        "protocol":"repair-intervention-v1",
        "clusters":clusters,
        "intervention":intervention_summary(target),
    }))
}

pub fn intervention_summary(target: &RepairTarget) -> RepairInterventionSummary {
    match &target.operation {
        RepairOperation::LabelPreservingVariants { anchors, .. } => {
            let mut anchors = anchors
                .iter()
                .map(|anchor| anchor.row_id.clone())
                .collect::<Vec<_>>();
            anchors.sort();
            RepairInterventionSummary::LabelPreservingVariants {
                anchor_ids: anchors,
            }
        }
        RepairOperation::ExistingAnchorContrast { pairs, .. } => {
            let mut pairs = pairs
                .iter()
                .map(|pair| pair.pair_id.clone())
                .collect::<Vec<_>>();
            pairs.sort();
            RepairInterventionSummary::ExistingAnchorContrast { pair_ids: pairs }
        }
        RepairOperation::ProvenRedundantRowRemoval { removals } => {
            let mut rows = removals
                .iter()
                .map(|removal| RepairRemovalSummary {
                    row_id: removal.row_id.clone(),
                    retained_row_id: removal.retained_row_id.clone(),
                })
                .collect::<Vec<_>>();
            rows.sort_by(|left, right| {
                (&left.row_id, &left.retained_row_id).cmp(&(&right.row_id, &right.retained_row_id))
            });
            RepairInterventionSummary::ProvenRedundantRowRemoval { rows }
        }
    }
}

pub fn classify_change(
    direction: MetricDirection,
    preceding: Option<f64>,
    candidate: Option<f64>,
) -> TargetMetricChange {
    let (Some(preceding), Some(candidate)) = (preceding, candidate) else {
        return TargetMetricChange::Unavailable;
    };
    let delta = candidate - preceding;
    if delta.abs() <= 1e-12 {
        TargetMetricChange::Unchanged
    } else if matches!(direction, MetricDirection::Increase) == (delta > 0.0) {
        TargetMetricChange::Improved
    } else {
        TargetMetricChange::Regressed
    }
}

pub fn aggregate_change(values: &[TargetMetricChange]) -> TargetMetricChange {
    if values.is_empty() || values.contains(&TargetMetricChange::Unavailable) {
        TargetMetricChange::Unavailable
    } else if values.contains(&TargetMetricChange::Regressed) {
        TargetMetricChange::Regressed
    } else if values.contains(&TargetMetricChange::Improved) {
        TargetMetricChange::Improved
    } else {
        TargetMetricChange::Unchanged
    }
}

fn valid_hash(value: &str) -> Result<(), OptimizationError> {
    require(
        value.strip_prefix("sha256:").is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }),
        "Invalid repair outcome fingerprint",
    )
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= maximum
        && !value.chars().any(|character| character.is_control())
}

impl RepairInterventionSummary {
    fn valid(&self) -> bool {
        match self {
            Self::LabelPreservingVariants { anchor_ids } => {
                !anchor_ids.is_empty()
                    && anchor_ids.len() <= 8
                    && sorted_unique_text(anchor_ids, 128)
            }
            Self::ExistingAnchorContrast { pair_ids } => {
                !pair_ids.is_empty() && pair_ids.len() <= 4 && sorted_unique_text(pair_ids, 128)
            }
            Self::ProvenRedundantRowRemoval { rows } => {
                !rows.is_empty()
                    && rows.len() <= 32
                    && rows.iter().all(|row| {
                        valid_text(&row.row_id, 128)
                            && valid_text(&row.retained_row_id, 128)
                            && row.row_id != row.retained_row_id
                    })
                    && rows.windows(2).all(|pair| {
                        (&pair[0].row_id, &pair[0].retained_row_id)
                            < (&pair[1].row_id, &pair[1].retained_row_id)
                    })
            }
        }
    }
}

fn sorted_unique_text(values: &[String], maximum: usize) -> bool {
    values.iter().all(|value| valid_text(value, maximum))
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn same_number(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() <= 1e-12,
        (None, None) => true,
        _ => false,
    }
}
