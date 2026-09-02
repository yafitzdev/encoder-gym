use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalProjectSnapshot},
    metrics::{
        CandidateAssessment, CandidateVerdict, EvaluationReport, MetricContract, MetricGateResult,
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderRepairError, canonical_sha256, fingerprint,
    observation::{DevelopmentObservation, DevelopmentObservationSet},
};

pub const COMPARATIVE_DIAGNOSIS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairArtifactBinding {
    pub id: Uuid,
    pub fingerprint: String,
}

impl RepairArtifactBinding {
    fn observation_set(value: &DevelopmentObservationSet) -> Self {
        Self {
            id: value.id,
            fingerprint: value.fingerprint.clone(),
        }
    }

    fn validate(&self) -> Result<(), EncoderRepairError> {
        if self.id.is_nil() || !canonical_sha256(&self.fingerprint) {
            return Err(EncoderRepairError::Validation(
                "repair artifact binding is invalid".into(),
            ));
        }
        Ok(())
    }
}

/// Exact aggregate assessment evidence for one candidate and one development suite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSuiteOutcome {
    pub candidate_id: Uuid,
    pub model_fingerprint: String,
    pub suite_key: String,
    pub suite_fingerprint: String,
    pub baseline_report_id: Uuid,
    pub baseline_report_fingerprint: String,
    pub candidate_report_id: Uuid,
    pub candidate_report_fingerprint: String,
    pub assessment_id: Uuid,
    pub assessment_fingerprint: String,
    pub primary_improvement: f64,
    pub verdict: CandidateVerdict,
    pub metric_deltas: BTreeMap<String, f64>,
    pub gates: Vec<MetricGateResult>,
    pub fingerprint: String,
}

impl CandidateSuiteOutcome {
    pub fn create(
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        candidate_id: Uuid,
        baseline: &EvaluationReport,
        candidate: &EvaluationReport,
        assessment: &CandidateAssessment,
    ) -> Result<Self, EncoderRepairError> {
        if candidate_id.is_nil()
            || baseline.evidence_role != EvidenceRole::Development
            || candidate.evidence_role != EvidenceRole::Development
        {
            return Err(EncoderRepairError::Validation(
                "repair outcomes require a candidate and development evidence".into(),
            ));
        }
        assessment
            .validate_integrity(project, contract, baseline, candidate)
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        let metric_deltas = candidate
            .metrics
            .iter()
            .map(|(key, value)| (key.clone(), stable_decimal(value - baseline.metrics[key])))
            .collect();
        let mut value = Self {
            candidate_id,
            model_fingerprint: candidate.model.fingerprint.clone(),
            suite_key: candidate.suite_key.clone(),
            suite_fingerprint: candidate.suite_fingerprint.clone(),
            baseline_report_id: baseline.id,
            baseline_report_fingerprint: baseline.fingerprint.clone(),
            candidate_report_id: candidate.id,
            candidate_report_fingerprint: candidate.fingerprint.clone(),
            assessment_id: assessment.id,
            assessment_fingerprint: assessment.fingerprint.clone(),
            primary_improvement: assessment.primary_improvement,
            verdict: assessment.verdict,
            metric_deltas,
            gates: assessment.gates.clone(),
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "candidate suite outcome fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "candidate_id": self.candidate_id,
            "model_fingerprint": self.model_fingerprint,
            "suite_key": self.suite_key,
            "suite_fingerprint": self.suite_fingerprint,
            "baseline_report_id": self.baseline_report_id,
            "baseline_report_fingerprint": self.baseline_report_fingerprint,
            "candidate_report_id": self.candidate_report_id,
            "candidate_report_fingerprint": self.candidate_report_fingerprint,
            "assessment_id": self.assessment_id,
            "assessment_fingerprint": self.assessment_fingerprint,
            "primary_improvement": self.primary_improvement,
            "verdict": self.verdict,
            "metric_deltas": self.metric_deltas,
            "gates": self.gates,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if self.candidate_id.is_nil()
            || self.baseline_report_id.is_nil()
            || self.candidate_report_id.is_nil()
            || self.assessment_id.is_nil()
            || self.suite_key.is_empty()
            || self.suite_key.trim() != self.suite_key
            || !self.primary_improvement.is_finite()
            || self.metric_deltas.is_empty()
            || self.metric_deltas.values().any(|value| !value.is_finite())
            || self.gates.is_empty()
            || [
                &self.model_fingerprint,
                &self.suite_fingerprint,
                &self.baseline_report_fingerprint,
                &self.candidate_report_fingerprint,
                &self.assessment_fingerprint,
            ]
            .iter()
            .any(|value| !canonical_sha256(value))
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "candidate suite outcome is not canonical or complete".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaknessKind {
    SharedAcrossSuites,
    SuiteSpecific,
    InsufficientSupport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteWeaknessEvidence {
    pub suite_key: String,
    pub eligible_support: u64,
    pub expected_abstentions: u64,
    pub baseline_top_one_errors: u64,
    pub baseline_top_one_error_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossSuiteWeakness {
    pub slice: BTreeMap<String, String>,
    pub slice_fingerprint: String,
    pub kind: WeaknessKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_key: Option<String>,
    pub suites: Vec<SuiteWeaknessEvidence>,
    pub total_baseline_top_one_errors: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSliceComparison {
    pub candidate_id: Uuid,
    pub model_fingerprint: String,
    pub suite_key: String,
    pub slice: BTreeMap<String, String>,
    pub slice_fingerprint: String,
    pub eligible_support: u64,
    pub expected_abstentions: u64,
    pub baseline_top_one_errors: u64,
    pub candidate_top_one_errors: u64,
    pub baseline_top_two_errors: u64,
    pub candidate_top_two_errors: u64,
    pub fixed_at_one: u64,
    pub regressed_at_one: u64,
    pub persistent_top_one_errors: u64,
    pub rank_improved: u64,
    pub rank_worsened: u64,
    pub baseline_top_one_error_rate: f64,
    pub candidate_top_one_error_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTradeoff {
    pub candidate_id: Uuid,
    pub model_fingerprint: String,
    pub passed_suites: Vec<String>,
    pub failed_suites: Vec<String>,
    pub outcomes: Vec<CandidateSuiteOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparativeDiagnosis {
    pub schema_version: u32,
    pub id: Uuid,
    /// Stable identity of the exact evidence and derivation policy, excluding record UUID/time.
    pub derivation_fingerprint: String,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub source_campaign_id: Uuid,
    pub source_experiment_run_id: Uuid,
    pub minimum_support: u64,
    pub slice_dimensions: Vec<String>,
    pub observation_sets: Vec<RepairArtifactBinding>,
    pub weaknesses: Vec<CrossSuiteWeakness>,
    pub candidate_comparisons: Vec<CandidateSliceComparison>,
    pub candidate_tradeoffs: Vec<CandidateTradeoff>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ComparativeDiagnosis {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project: &ExternalProjectSnapshot,
        source_campaign_id: Uuid,
        source_experiment_run_id: Uuid,
        minimum_support: u64,
        mut slice_dimensions: Vec<String>,
        baseline_sets: &[DevelopmentObservationSet],
        candidate_sets: &[DevelopmentObservationSet],
        outcomes: &[CandidateSuiteOutcome],
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        slice_dimensions.sort();
        slice_dimensions.dedup();
        let derived = derive(
            project,
            source_campaign_id,
            source_experiment_run_id,
            minimum_support,
            &slice_dimensions,
            baseline_sets,
            candidate_sets,
            outcomes,
        )?;
        let mut value = Self {
            schema_version: COMPARATIVE_DIAGNOSIS_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            derivation_fingerprint: String::new(),
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            source_campaign_id,
            source_experiment_run_id,
            minimum_support,
            slice_dimensions,
            observation_sets: derived.observation_sets,
            weaknesses: derived.weaknesses,
            candidate_comparisons: derived.candidate_comparisons,
            candidate_tradeoffs: derived.candidate_tradeoffs,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.derivation_fingerprint = value.reproduce_derivation_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn verify_derivation(
        &self,
        project: &ExternalProjectSnapshot,
        baseline_sets: &[DevelopmentObservationSet],
        candidate_sets: &[DevelopmentObservationSet],
        outcomes: &[CandidateSuiteOutcome],
    ) -> Result<(), EncoderRepairError> {
        self.validate_integrity()?;
        let mut expected = Self::create(
            project,
            self.source_campaign_id,
            self.source_experiment_run_id,
            self.minimum_support,
            self.slice_dimensions.clone(),
            baseline_sets,
            candidate_sets,
            outcomes,
            self.created_at,
        )?;
        expected.id = self.id;
        expected.fingerprint = expected.reproduce_fingerprint()?;
        if expected != *self {
            return Err(EncoderRepairError::Integrity(
                "comparative diagnosis does not reproduce from its observation sets".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_derivation_fingerprint()? != self.derivation_fingerprint {
            return Err(EncoderRepairError::Integrity(
                "comparative diagnosis derivation fingerprint changed".into(),
            ));
        }
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "comparative diagnosis fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_derivation_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "source_campaign_id": self.source_campaign_id,
            "source_experiment_run_id": self.source_experiment_run_id,
            "minimum_support": self.minimum_support,
            "slice_dimensions": self.slice_dimensions,
            "observation_sets": self.observation_sets,
            "candidate_tradeoffs": self.candidate_tradeoffs,
        }))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "derivation_fingerprint": self.derivation_fingerprint,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "source_campaign_id": self.source_campaign_id,
            "source_experiment_run_id": self.source_experiment_run_id,
            "minimum_support": self.minimum_support,
            "slice_dimensions": self.slice_dimensions,
            "observation_sets": self.observation_sets,
            "weaknesses": self.weaknesses,
            "candidate_comparisons": self.candidate_comparisons,
            "candidate_tradeoffs": self.candidate_tradeoffs,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if self.schema_version != COMPARATIVE_DIAGNOSIS_SCHEMA_VERSION
            || self.id.is_nil()
            || !self.derivation_fingerprint.is_empty()
                && !canonical_sha256(&self.derivation_fingerprint)
            || self.project_snapshot_id.is_nil()
            || !canonical_sha256(&self.project_snapshot_fingerprint)
            || self.source_campaign_id.is_nil()
            || self.source_experiment_run_id.is_nil()
            || self.minimum_support == 0
            || self.slice_dimensions.is_empty()
            || self.observation_sets.is_empty()
            || self.weaknesses.is_empty()
            || self.candidate_comparisons.is_empty()
            || self.candidate_tradeoffs.is_empty()
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "comparative diagnosis is incomplete or invalid".into(),
            ));
        }
        if self
            .slice_dimensions
            .iter()
            .any(|value| value.is_empty() || value.trim() != value)
            || self
                .slice_dimensions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(EncoderRepairError::Validation(
                "diagnosis dimensions must be unique and canonically ordered".into(),
            ));
        }
        for binding in &self.observation_sets {
            binding.validate()?;
        }
        Ok(())
    }
}

struct DerivedDiagnosis {
    observation_sets: Vec<RepairArtifactBinding>,
    weaknesses: Vec<CrossSuiteWeakness>,
    candidate_comparisons: Vec<CandidateSliceComparison>,
    candidate_tradeoffs: Vec<CandidateTradeoff>,
}

#[allow(clippy::too_many_arguments)]
fn derive(
    project: &ExternalProjectSnapshot,
    source_campaign_id: Uuid,
    source_experiment_run_id: Uuid,
    minimum_support: u64,
    slice_dimensions: &[String],
    baseline_sets: &[DevelopmentObservationSet],
    candidate_sets: &[DevelopmentObservationSet],
    outcomes: &[CandidateSuiteOutcome],
) -> Result<DerivedDiagnosis, EncoderRepairError> {
    if source_campaign_id.is_nil()
        || source_experiment_run_id.is_nil()
        || minimum_support == 0
        || slice_dimensions.is_empty()
        || baseline_sets.len() < 2
        || candidate_sets.is_empty()
        || outcomes.is_empty()
    {
        return Err(EncoderRepairError::Validation(
            "comparative diagnosis requires two suites, candidates, and finite support".into(),
        ));
    }
    let mut suites = BTreeMap::new();
    for set in baseline_sets {
        validate_set_scope(set, project, source_campaign_id, source_experiment_run_id)?;
        if set.candidate_id.is_some() || suites.insert(set.suite_key.as_str(), set).is_some() {
            return Err(EncoderRepairError::Validation(
                "diagnosis requires exactly one baseline observation set per suite".into(),
            ));
        }
    }
    let suite_keys = suites.keys().copied().collect::<BTreeSet<_>>();
    let mut candidates: BTreeMap<Uuid, BTreeMap<&str, &DevelopmentObservationSet>> =
        BTreeMap::new();
    for set in candidate_sets {
        validate_set_scope(set, project, source_campaign_id, source_experiment_run_id)?;
        let candidate_id = set.candidate_id.ok_or_else(|| {
            EncoderRepairError::Validation("candidate observation set has no candidate".into())
        })?;
        if candidates
            .entry(candidate_id)
            .or_default()
            .insert(set.suite_key.as_str(), set)
            .is_some()
        {
            return Err(EncoderRepairError::Validation(
                "candidate observation suite is duplicated".into(),
            ));
        }
    }
    if candidates
        .values()
        .any(|sets| sets.keys().copied().collect::<BTreeSet<_>>() != suite_keys)
    {
        return Err(EncoderRepairError::Validation(
            "every candidate requires complete observations for every development suite".into(),
        ));
    }
    let mut outcome_map = BTreeMap::new();
    for outcome in outcomes {
        outcome.validate_integrity()?;
        if outcome_map
            .insert((outcome.candidate_id, outcome.suite_key.as_str()), outcome)
            .is_some()
        {
            return Err(EncoderRepairError::Validation(
                "candidate suite outcome is duplicated".into(),
            ));
        }
    }
    if outcome_map.len() != candidates.len() * suites.len() {
        return Err(EncoderRepairError::Validation(
            "candidate outcomes must cover every candidate-suite pair".into(),
        ));
    }

    let mut comparisons = Vec::new();
    for (candidate_id, candidate_suites) in &candidates {
        for (suite_key, baseline) in &suites {
            let candidate = candidate_suites[suite_key];
            let outcome = outcome_map
                .get(&(*candidate_id, *suite_key))
                .ok_or_else(|| {
                    EncoderRepairError::Validation(
                        "candidate outcome does not cover an observation suite".into(),
                    )
                })?;
            if outcome.model_fingerprint != candidate.model_fingerprint
                || outcome.suite_fingerprint != candidate.suite_fingerprint
                || outcome.baseline_report_id != baseline.evaluation_report_id
                || outcome.baseline_report_fingerprint != baseline.evaluation_report_fingerprint
                || outcome.candidate_report_id != candidate.evaluation_report_id
                || outcome.candidate_report_fingerprint != candidate.evaluation_report_fingerprint
            {
                return Err(EncoderRepairError::Validation(
                    "candidate outcome does not bind the exact observation reports".into(),
                ));
            }
            comparisons.extend(compare_sets(
                *candidate_id,
                &candidate.model_fingerprint,
                baseline,
                candidate,
                slice_dimensions,
            )?);
        }
    }
    comparisons.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then_with(|| left.suite_key.cmp(&right.suite_key))
            .then_with(|| left.slice_fingerprint.cmp(&right.slice_fingerprint))
    });

    let first_candidate = *candidates
        .keys()
        .next()
        .expect("candidate map was checked non-empty");
    let baseline_cells = comparisons
        .iter()
        .filter(|value| value.candidate_id == first_candidate)
        .collect::<Vec<_>>();
    let mut by_slice: BTreeMap<&str, Vec<&CandidateSliceComparison>> = BTreeMap::new();
    for cell in baseline_cells {
        by_slice
            .entry(cell.slice_fingerprint.as_str())
            .or_default()
            .push(cell);
    }
    let mut weaknesses = Vec::new();
    for cells in by_slice.values() {
        let total_errors = cells
            .iter()
            .map(|value| value.baseline_top_one_errors)
            .sum::<u64>();
        if total_errors == 0 {
            continue;
        }
        let insufficient = cells
            .iter()
            .any(|value| value.eligible_support < minimum_support);
        let error_suites = cells
            .iter()
            .filter(|value| value.baseline_top_one_errors > 0)
            .map(|value| value.suite_key.as_str())
            .collect::<Vec<_>>();
        let (kind, suite_key) = if insufficient {
            (WeaknessKind::InsufficientSupport, None)
        } else if error_suites.len() >= 2 {
            (WeaknessKind::SharedAcrossSuites, None)
        } else {
            (
                WeaknessKind::SuiteSpecific,
                Some(error_suites[0].to_owned()),
            )
        };
        let first = cells[0];
        weaknesses.push(CrossSuiteWeakness {
            slice: first.slice.clone(),
            slice_fingerprint: first.slice_fingerprint.clone(),
            kind,
            suite_key,
            suites: cells
                .iter()
                .map(|value| SuiteWeaknessEvidence {
                    suite_key: value.suite_key.clone(),
                    eligible_support: value.eligible_support,
                    expected_abstentions: value.expected_abstentions,
                    baseline_top_one_errors: value.baseline_top_one_errors,
                    baseline_top_one_error_rate: value.baseline_top_one_error_rate,
                })
                .collect(),
            total_baseline_top_one_errors: total_errors,
        });
    }
    weaknesses.sort_by(|left, right| {
        right
            .total_baseline_top_one_errors
            .cmp(&left.total_baseline_top_one_errors)
            .then_with(|| left.slice_fingerprint.cmp(&right.slice_fingerprint))
    });

    let mut tradeoffs = Vec::new();
    for candidate_id in candidates.keys() {
        let mut values = outcomes
            .iter()
            .filter(|value| value.candidate_id == *candidate_id)
            .cloned()
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.suite_key.cmp(&right.suite_key));
        let model_fingerprint = values[0].model_fingerprint.clone();
        if values
            .iter()
            .any(|value| value.model_fingerprint != model_fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "candidate outcomes disagree on model identity".into(),
            ));
        }
        tradeoffs.push(CandidateTradeoff {
            candidate_id: *candidate_id,
            model_fingerprint,
            passed_suites: values
                .iter()
                .filter(|value| value.verdict == CandidateVerdict::Passed)
                .map(|value| value.suite_key.clone())
                .collect(),
            failed_suites: values
                .iter()
                .filter(|value| value.verdict == CandidateVerdict::Failed)
                .map(|value| value.suite_key.clone())
                .collect(),
            outcomes: values,
        });
    }
    tradeoffs.sort_by_key(|value| value.candidate_id);

    let mut observation_sets = baseline_sets
        .iter()
        .chain(candidate_sets)
        .map(RepairArtifactBinding::observation_set)
        .collect::<Vec<_>>();
    observation_sets.sort_by_key(|value| value.id);
    Ok(DerivedDiagnosis {
        observation_sets,
        weaknesses,
        candidate_comparisons: comparisons,
        candidate_tradeoffs: tradeoffs,
    })
}

fn validate_set_scope(
    set: &DevelopmentObservationSet,
    project: &ExternalProjectSnapshot,
    campaign_id: Uuid,
    run_id: Uuid,
) -> Result<(), EncoderRepairError> {
    set.validate_integrity()?;
    if set.project_snapshot_id != project.id
        || set.project_snapshot_fingerprint != project.fingerprint
        || set.source_campaign_id != campaign_id
        || set.source_experiment_run_id != run_id
    {
        return Err(EncoderRepairError::Validation(
            "observation set is outside the diagnosis evidence scope".into(),
        ));
    }
    Ok(())
}

fn compare_sets(
    candidate_id: Uuid,
    model_fingerprint: &str,
    baseline: &DevelopmentObservationSet,
    candidate: &DevelopmentObservationSet,
    dimensions: &[String],
) -> Result<Vec<CandidateSliceComparison>, EncoderRepairError> {
    if baseline.suite_key != candidate.suite_key
        || baseline.suite_fingerprint != candidate.suite_fingerprint
        || baseline.observations.len() != candidate.observations.len()
    {
        return Err(EncoderRepairError::Validation(
            "candidate observations do not cover the baseline suite".into(),
        ));
    }
    let mut accumulators: BTreeMap<String, (BTreeMap<String, String>, Accumulator)> =
        BTreeMap::new();
    for (left, right) in baseline.observations.iter().zip(&candidate.observations) {
        if left.source_row_id != right.source_row_id
            || left.source_row_fingerprint != right.source_row_fingerprint
            || left.slices != right.slices
            || left.expected_abstention != right.expected_abstention
        {
            return Err(EncoderRepairError::Validation(
                "candidate row observations do not match baseline row identity and slices".into(),
            ));
        }
        for slice in observation_slices(left, dimensions)? {
            let slice_fingerprint = fingerprint(&slice)?;
            accumulators
                .entry(slice_fingerprint)
                .or_insert_with(|| (slice, Accumulator::default()))
                .1
                .observe(left, right);
        }
    }
    accumulators
        .into_iter()
        .map(|(slice_fingerprint, (slice, value))| {
            let baseline_error_rate = ratio(value.baseline_top_one_errors, value.eligible_support);
            let candidate_error_rate =
                ratio(value.candidate_top_one_errors, value.eligible_support);
            Ok(CandidateSliceComparison {
                candidate_id,
                model_fingerprint: model_fingerprint.to_owned(),
                suite_key: baseline.suite_key.clone(),
                slice,
                slice_fingerprint,
                eligible_support: value.eligible_support,
                expected_abstentions: value.expected_abstentions,
                baseline_top_one_errors: value.baseline_top_one_errors,
                candidate_top_one_errors: value.candidate_top_one_errors,
                baseline_top_two_errors: value.baseline_top_two_errors,
                candidate_top_two_errors: value.candidate_top_two_errors,
                fixed_at_one: value.fixed_at_one,
                regressed_at_one: value.regressed_at_one,
                persistent_top_one_errors: value.persistent_top_one_errors,
                rank_improved: value.rank_improved,
                rank_worsened: value.rank_worsened,
                baseline_top_one_error_rate: baseline_error_rate,
                candidate_top_one_error_rate: candidate_error_rate,
            })
        })
        .collect()
}

fn observation_slices(
    observation: &DevelopmentObservation,
    dimensions: &[String],
) -> Result<Vec<BTreeMap<String, String>>, EncoderRepairError> {
    let mut result = vec![BTreeMap::new()];
    let mut full = BTreeMap::new();
    for dimension in dimensions {
        let value = observation.slices.get(dimension).ok_or_else(|| {
            EncoderRepairError::Validation(format!(
                "observation omitted requested diagnosis dimension {dimension}"
            ))
        })?;
        result.push(BTreeMap::from([(dimension.clone(), value.clone())]));
        full.insert(dimension.clone(), value.clone());
    }
    if full.len() > 1 {
        result.push(full);
    }
    Ok(result)
}

#[derive(Default)]
struct Accumulator {
    eligible_support: u64,
    expected_abstentions: u64,
    baseline_top_one_errors: u64,
    candidate_top_one_errors: u64,
    baseline_top_two_errors: u64,
    candidate_top_two_errors: u64,
    fixed_at_one: u64,
    regressed_at_one: u64,
    persistent_top_one_errors: u64,
    rank_improved: u64,
    rank_worsened: u64,
}

impl Accumulator {
    fn observe(&mut self, baseline: &DevelopmentObservation, candidate: &DevelopmentObservation) {
        if baseline.expected_abstention {
            self.expected_abstentions += 1;
            return;
        }
        self.eligible_support += 1;
        let baseline_error = !baseline.top_one_correct();
        let candidate_error = !candidate.top_one_correct();
        self.baseline_top_one_errors += u64::from(baseline_error);
        self.candidate_top_one_errors += u64::from(candidate_error);
        self.baseline_top_two_errors += u64::from(!baseline.top_two_correct());
        self.candidate_top_two_errors += u64::from(!candidate.top_two_correct());
        self.fixed_at_one += u64::from(baseline_error && !candidate_error);
        self.regressed_at_one += u64::from(!baseline_error && candidate_error);
        self.persistent_top_one_errors += u64::from(baseline_error && candidate_error);
        let baseline_rank = baseline.first_relevant_rank.unwrap_or(u32::MAX);
        let candidate_rank = candidate.first_relevant_rank.unwrap_or(u32::MAX);
        self.rank_improved += u64::from(candidate_rank < baseline_rank);
        self.rank_worsened += u64::from(candidate_rank > baseline_rank);
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        (numerator as f64 / denominator as f64 * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
    }
}

fn stable_decimal(value: f64) -> f64 {
    (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use encoder_experiment_core::{
        domain::{
            BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, ModelArtifactIdentity,
        },
        metrics::{
            MetricDefinition, MetricDirection, MetricGate, MetricGateCondition, assess_candidate,
        },
    };
    use serde_json::json;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos repair",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            digest('a'),
            BackendIdentity::new("nomos", "v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('1'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "development-a",
                    EvidenceRole::Development,
                    1,
                    digest('2'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "development-b",
                    EvidenceRole::Development,
                    1,
                    digest('3'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('4'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "fake", 1, digest('5')).unwrap(),
            json!({"task":"ranking"}),
            time(0),
        )
        .unwrap()
    }

    fn contract() -> MetricContract {
        MetricContract::create(
            vec![
                MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap(),
                MetricDefinition::new("recall_at_2", MetricDirection::HigherIsBetter).unwrap(),
            ],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MinimumImprovement { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "recall_at_2",
                    EvidenceRole::Development,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "mrr",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn report(
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        model: ModelArtifactIdentity,
        suite: &str,
        mrr: f64,
        recall: f64,
    ) -> EvaluationReport {
        EvaluationReport::create(
            project,
            model,
            EvidenceRole::Development,
            suite,
            if suite == "generic" {
                digest('6')
            } else {
                digest('7')
            },
            contract,
            BTreeMap::from([("mrr".into(), mrr), ("recall_at_2".into(), recall)]),
            3,
            time(1),
        )
        .unwrap()
    }

    fn observations(ranks: [Option<u32>; 3]) -> Vec<DevelopmentObservation> {
        ranks
            .into_iter()
            .enumerate()
            .map(|(index, rank)| {
                DevelopmentObservation::create(
                    format!("row-{index}"),
                    digest(char::from(b'a' + u8::try_from(index).unwrap())),
                    BTreeMap::from([
                        ("target_capability".into(), "search".into()),
                        (
                            "workflow".into(),
                            if index == 2 { "audit" } else { "lookup" }.into(),
                        ),
                    ]),
                    false,
                    rank,
                    rank.map_or(0.0, |value| 1.0 / f64::from(value)),
                    Some(0.1),
                    Some(digest('f')),
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn diagnosis_distinguishes_cross_suite_tradeoffs_and_row_regressions() {
        let project = project();
        let contract = contract();
        let campaign_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let candidate_id = Uuid::new_v4();
        let candidate_model =
            ModelArtifactIdentity::new("candidate", "fake", 1, digest('8')).unwrap();
        let observer = BackendIdentity::new("fake-observer", "v1", digest('9')).unwrap();
        let artifact = |key: &str| {
            ExternalArtifactIdentity::new(key, EvidenceRole::Development, 1, digest('0')).unwrap()
        };
        let mut baseline_sets = Vec::new();
        let mut candidate_sets = Vec::new();
        let mut outcomes = Vec::new();
        for (suite, baseline_ranks, candidate_ranks, candidate_mrr, candidate_recall) in [
            (
                "generic",
                [Some(1), Some(2), Some(1)],
                [Some(1), Some(1), Some(2)],
                0.81,
                0.79,
            ),
            (
                "retired",
                [Some(2), Some(2), Some(1)],
                [Some(1), Some(3), Some(1)],
                0.79,
                0.81,
            ),
        ] {
            let baseline_report = report(
                &project,
                &contract,
                project.baseline_model.clone(),
                suite,
                0.8,
                0.8,
            );
            let candidate_report = report(
                &project,
                &contract,
                candidate_model.clone(),
                suite,
                candidate_mrr,
                candidate_recall,
            );
            let assessment = assess_candidate(
                &project,
                &contract,
                &baseline_report,
                &candidate_report,
                time(2),
            )
            .unwrap();
            baseline_sets.push(
                DevelopmentObservationSet::create(
                    &project,
                    campaign_id,
                    run_id,
                    None,
                    &baseline_report,
                    &contract,
                    observer.clone(),
                    artifact(&format!("baseline-{suite}")),
                    observations(baseline_ranks),
                    time(3),
                )
                .unwrap(),
            );
            candidate_sets.push(
                DevelopmentObservationSet::create(
                    &project,
                    campaign_id,
                    run_id,
                    Some(candidate_id),
                    &candidate_report,
                    &contract,
                    observer.clone(),
                    artifact(&format!("candidate-{suite}")),
                    observations(candidate_ranks),
                    time(3),
                )
                .unwrap(),
            );
            outcomes.push(
                CandidateSuiteOutcome::create(
                    &project,
                    &contract,
                    candidate_id,
                    &baseline_report,
                    &candidate_report,
                    &assessment,
                )
                .unwrap(),
            );
        }
        let diagnosis = ComparativeDiagnosis::create(
            &project,
            campaign_id,
            run_id,
            1,
            vec!["target_capability".into(), "workflow".into()],
            &baseline_sets,
            &candidate_sets,
            &outcomes,
            time(4),
        )
        .unwrap();
        diagnosis
            .verify_derivation(&project, &baseline_sets, &candidate_sets, &outcomes)
            .unwrap();
        assert!(
            diagnosis
                .weaknesses
                .iter()
                .any(|value| value.kind == WeaknessKind::SharedAcrossSuites)
        );
        assert!(
            diagnosis
                .candidate_comparisons
                .iter()
                .any(|value| value.fixed_at_one > 0)
        );
        assert!(
            diagnosis
                .candidate_comparisons
                .iter()
                .any(|value| value.regressed_at_one > 0)
        );
        assert_eq!(diagnosis.candidate_tradeoffs[0].passed_suites.len(), 0);
        assert_eq!(diagnosis.candidate_tradeoffs[0].failed_suites.len(), 2);
    }

    #[test]
    fn sealed_reports_cannot_become_repair_observations() {
        let project = project();
        let contract = contract();
        let sealed = EvaluationReport::create(
            &project,
            project.baseline_model.clone(),
            EvidenceRole::SealedAcceptance,
            "sealed",
            digest('4'),
            &contract,
            BTreeMap::from([("mrr".into(), 0.8), ("recall_at_2".into(), 0.8)]),
            3,
            time(1),
        )
        .unwrap();
        let result = DevelopmentObservationSet::create(
            &project,
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            &sealed,
            &contract,
            BackendIdentity::new("fake", "v1", digest('9')).unwrap(),
            ExternalArtifactIdentity::new("sealed", EvidenceRole::Development, 1, digest('0'))
                .unwrap(),
            observations([Some(1), Some(2), Some(1)]),
            time(2),
        );
        assert!(matches!(result, Err(EncoderRepairError::Validation(_))));
    }
}
