//! Immutable benchmark suites and deterministic acceptance contracts.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotSplit;
use evaluation_core::comparison::comparison_fingerprint;
use evaluation_core::domain::{
    ClassificationMetrics, EvaluationComparisonReport, EvaluationMetrics, EvaluationProtocol,
    EvaluationRun, EvaluationRunState, LabelMetrics, SliceIdentity,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    contamination::{ContaminationOverride, ContaminationReport, ContaminationStatus},
    governance::{
        CohortDisposition, CohortRole, CohortRoleDecision, DisclosureLevel, EvaluationCohort,
        ExposurePurpose,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkSuiteKind {
    Development,
    SealedAcceptance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSuiteRequest {
    pub name: String,
    pub kind: BenchmarkSuiteKind,
    pub task: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub required_model_formats: Vec<String>,
    pub cohorts: Vec<BenchmarkCohortRequest>,
    pub contract: AcceptanceContract,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSuiteDefinition {
    pub contamination_report_id: Uuid,
    #[serde(flatten)]
    pub request: BenchmarkSuiteRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkCohortRequest {
    pub cohort_id: Uuid,
    pub protocol: EvaluationProtocol,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkCohortEvidence {
    pub cohort: EvaluationCohort,
    pub role: CohortRoleDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkCohort {
    pub cohort_id: Uuid,
    pub cohort_fingerprint: String,
    pub evaluation_cohort_fingerprint: String,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub split: SnapshotSplit,
    pub role: CohortRole,
    pub role_decision_id: Uuid,
    pub role_decision_fingerprint: String,
    pub protocol: EvaluationProtocol,
    pub protocol_fingerprint: String,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceContract {
    #[serde(default)]
    pub metric_requirements: Vec<MetricRequirement>,
    #[serde(default)]
    pub regression: Option<RegressionRequirement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricRequirement {
    pub target: MetricTarget,
    pub metric: BenchmarkMetric,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    #[serde(default = "one")]
    pub minimum_support: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetricTarget {
    Overall,
    Label { label: String },
    Slice { key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMetric {
    Accuracy,
    MacroPrecision,
    MacroRecall,
    MacroF1,
    WeightedF1,
    Precision,
    Recall,
    F1,
    LogLoss,
    BrierScore,
    ExpectedCalibrationError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionRequirement {
    #[serde(default)]
    pub max_accuracy_drop: Option<f64>,
    #[serde(default)]
    pub max_macro_f1_drop: Option<f64>,
    #[serde(default)]
    pub minimum_accuracy_delta_lower_bound: Option<f64>,
    #[serde(default)]
    pub minimum_macro_f1_delta_lower_bound: Option<f64>,
    #[serde(default)]
    pub require_mcnemar_significance: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub id: Uuid,
    pub name: String,
    pub kind: BenchmarkSuiteKind,
    pub task: String,
    pub labels: Vec<String>,
    pub required_model_formats: Vec<String>,
    pub cohorts: Vec<BenchmarkCohort>,
    pub contract: AcceptanceContract,
    pub contamination_report_id: Uuid,
    pub contamination_report_fingerprint: String,
    pub contamination_override_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkSuite {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkError> {
        suite_fingerprint(self)
    }

    /// Revalidates every self-contained invariant after deserialization.
    /// References to cohorts, role decisions, and contamination artifacts are
    /// additionally checked by the persistence adapter that owns those facts.
    pub fn validate_integrity(&self) -> Result<(), BenchmarkError> {
        self.validate_integrity_inner(false)
    }

    /// Reads historical suites whose empty contract predates strict decision
    /// criteria. Such suites remain inspectable and assess as inconclusive,
    /// but cannot enter a new benchmark bundle.
    pub fn validate_legacy_compatible_integrity(&self) -> Result<(), BenchmarkError> {
        self.validate_integrity_inner(true)
    }

    fn validate_integrity_inner(&self, allow_vacuous_contract: bool) -> Result<(), BenchmarkError> {
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkError::FingerprintMismatch);
        }
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.trim() != self.name
            || self.task.trim().is_empty()
            || self.task.trim() != self.task
            || self.contamination_report_id.is_nil()
            || !is_canonical_fingerprint(&self.contamination_report_fingerprint)
            || self
                .contamination_override_fingerprint
                .as_deref()
                .is_some_and(|value| !is_canonical_fingerprint(value))
        {
            return Err(BenchmarkError::InvalidSuite(
                "identity, text, or contamination pins are malformed".into(),
            ));
        }
        validate_labels(&self.labels)?;
        if let Err(error) = validate_contract(&self.contract, &self.labels, self.kind) {
            if !(allow_vacuous_contract && error == BenchmarkError::VacuousContract) {
                return Err(error);
            }
        }
        if self.cohorts.is_empty() {
            return Err(BenchmarkError::NoCohorts);
        }
        if self
            .required_model_formats
            .iter()
            .any(|value| value.trim().is_empty() || value.trim() != value)
        {
            return Err(BenchmarkError::InvalidSuite(
                "model formats must be non-empty and trimmed".into(),
            ));
        }
        let mut canonical_formats = self.required_model_formats.clone();
        canonical_formats.sort();
        canonical_formats.dedup();
        if canonical_formats != self.required_model_formats {
            return Err(BenchmarkError::InvalidSuite(
                "model formats are not canonical".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut previous = None;
        for cohort in &self.cohorts {
            if !seen.insert(cohort.cohort_id) {
                return Err(BenchmarkError::DuplicateCohort(cohort.cohort_id));
            }
            if previous.is_some_and(|id| id >= cohort.cohort_id) {
                return Err(BenchmarkError::InvalidSuite(
                    "cohorts are not in canonical identity order".into(),
                ));
            }
            previous = Some(cohort.cohort_id);
            if cohort.cohort_id.is_nil()
                || cohort.snapshot_id.is_nil()
                || cohort.role_decision_id.is_nil()
                || !is_canonical_fingerprint(&cohort.cohort_fingerprint)
                || !is_canonical_fingerprint(&cohort.evaluation_cohort_fingerprint)
                || !is_canonical_fingerprint(&cohort.snapshot_fingerprint)
                || !is_canonical_fingerprint(&cohort.role_decision_fingerprint)
                || !is_canonical_fingerprint(&cohort.protocol_fingerprint)
            {
                return Err(BenchmarkError::InvalidSuite(format!(
                    "cohort pins are malformed: {}",
                    cohort.cohort_id
                )));
            }
            cohort
                .protocol
                .validate_for_labels(&self.labels)
                .map_err(|error| BenchmarkError::InvalidSuite(error.to_string()))?;
            if cohort.protocol.split != cohort.split
                || cohort.protocol.fingerprint().map_err(map_fingerprint)?
                    != cohort.protocol_fingerprint
                || artifact_core::fingerprint(&(cohort.snapshot_fingerprint.as_str(), cohort.split))
                    .map_err(map_fingerprint)?
                    != cohort.evaluation_cohort_fingerprint
            {
                return Err(BenchmarkError::InvalidSuite(format!(
                    "cohort protocol or source identity differs: {}",
                    cohort.cohort_id
                )));
            }
            let role_is_compatible = match self.kind {
                BenchmarkSuiteKind::Development => matches!(
                    cohort.role,
                    CohortRole::Development
                        | CohortRole::Diagnostic
                        | CohortRole::ExternalBenchmark
                ),
                BenchmarkSuiteKind::SealedAcceptance => matches!(
                    cohort.role,
                    CohortRole::SealedAcceptance | CohortRole::ExternalBenchmark
                ),
            };
            if !role_is_compatible {
                return Err(BenchmarkError::IncompatibleRole(cohort.cohort_id));
            }
            if self.kind == BenchmarkSuiteKind::SealedAcceptance
                && (cohort.disclosure != DisclosureLevel::Aggregate || cohort.adaptation_eligible)
            {
                return Err(BenchmarkError::SealedDisclosure);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CohortAssessmentInput {
    pub cohort_id: Uuid,
    pub run: Option<EvaluationRun>,
    pub comparison: Option<EvaluationComparisonReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceState {
    Pass,
    Fail,
    Inconclusive,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceReasonState {
    Passed,
    Failed,
    Inconclusive,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceReason {
    pub cohort_id: Option<Uuid>,
    pub code: String,
    pub state: AcceptanceReasonState,
    pub message: String,
    pub observed: Option<f64>,
    pub required: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceAssessment {
    pub id: Uuid,
    pub suite_id: Uuid,
    pub suite_fingerprint: String,
    pub checkpoint_id: Option<Uuid>,
    pub evaluation_run_ids: BTreeMap<Uuid, Uuid>,
    pub comparison_ids: BTreeMap<Uuid, Uuid>,
    pub state: AcceptanceState,
    pub reasons: Vec<AcceptanceReason>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AcceptanceAssessment {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkError> {
        assessment_fingerprint(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum BenchmarkError {
    #[error("{0} must not be empty")]
    Empty(&'static str),
    #[error("labels must contain at least two unique non-empty values")]
    Labels,
    #[error("benchmark suite requires at least one cohort")]
    NoCohorts,
    #[error("cohort request appears more than once: {0}")]
    DuplicateCohort(Uuid),
    #[error("missing resolved cohort: {0}")]
    MissingCohort(Uuid),
    #[error("cohort role is incompatible with suite kind: {0}")]
    IncompatibleRole(Uuid),
    #[error("cohort is retired: {0}")]
    RetiredCohort(Uuid),
    #[error("sealed suite must be aggregate-only and adaptation-ineligible")]
    SealedDisclosure,
    #[error(
        "benchmark cohort {cohort_id} permits {permitted:?} disclosure but {requested:?} was requested"
    )]
    DisclosureExceeded {
        cohort_id: Uuid,
        requested: DisclosureLevel,
        permitted: DisclosureLevel,
    },
    #[error("benchmark cohort is ineligible for adaptive purpose {purpose:?}: {cohort_id}")]
    AdaptiveAccess {
        cohort_id: Uuid,
        purpose: ExposurePurpose,
    },
    #[error("contamination report does not cover exactly the suite cohorts")]
    ContaminationCohorts,
    #[error("blocked contamination report requires a matching override")]
    ContaminationBlocked,
    #[error("contamination artifact fingerprint mismatch")]
    ContaminationFingerprint,
    #[error("cohort or role evidence fingerprint mismatch: {0}")]
    CohortEvidenceFingerprint(Uuid),
    #[error("acceptance contract has no effective requirement")]
    VacuousContract,
    #[error("invalid metric requirement: {0}")]
    MetricRequirement(String),
    #[error("invalid regression requirement: {0}")]
    RegressionRequirement(String),
    #[error("assessment inputs contain an unknown or duplicate cohort: {0}")]
    AssessmentCohort(Uuid),
    #[error("artifact fingerprint mismatch")]
    FingerprintMismatch,
    #[error("invalid benchmark suite: {0}")]
    InvalidSuite(String),
    #[error("could not fingerprint benchmark artifact: {0}")]
    Fingerprint(String),
}

/// Validates a prospective use of every cohort pinned by a benchmark suite.
///
/// This is deliberately a core policy rather than a CLI check so preparation,
/// runtime orchestration, and future application surfaces enforce the same
/// disclosure and adaptation boundary before evidence is accessed.
pub fn validate_suite_access(
    suite: &BenchmarkSuite,
    purpose: ExposurePurpose,
    requested_disclosure: Option<DisclosureLevel>,
) -> Result<(), BenchmarkError> {
    suite.validate_integrity()?;
    for cohort in &suite.cohorts {
        let requested = requested_disclosure.unwrap_or(cohort.disclosure);
        if requested > cohort.disclosure {
            return Err(BenchmarkError::DisclosureExceeded {
                cohort_id: cohort.cohort_id,
                requested,
                permitted: cohort.disclosure,
            });
        }
        if purpose.is_adaptive() && !cohort.adaptation_eligible {
            return Err(BenchmarkError::AdaptiveAccess {
                cohort_id: cohort.cohort_id,
                purpose,
            });
        }
    }
    Ok(())
}

pub fn build_benchmark_suite(
    request: BenchmarkSuiteRequest,
    evidence: Vec<BenchmarkCohortEvidence>,
    contamination: &ContaminationReport,
    contamination_override: Option<&ContaminationOverride>,
) -> Result<BenchmarkSuite, BenchmarkError> {
    let name = required(request.name, "suite name")?;
    let task = required(request.task, "suite task")?;
    validate_labels(&request.labels)?;
    validate_contract(&request.contract, &request.labels, request.kind)?;
    if request.cohorts.is_empty() {
        return Err(BenchmarkError::NoCohorts);
    }
    if contamination
        .reproduce_fingerprint()
        .map_err(map_contamination)?
        != contamination.fingerprint
    {
        return Err(BenchmarkError::ContaminationFingerprint);
    }
    let evidence = evidence
        .into_iter()
        .map(|item| (item.cohort.id, item))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut cohorts = Vec::with_capacity(request.cohorts.len());
    for cohort_request in request.cohorts {
        if !seen.insert(cohort_request.cohort_id) {
            return Err(BenchmarkError::DuplicateCohort(cohort_request.cohort_id));
        }
        let resolved = evidence
            .get(&cohort_request.cohort_id)
            .ok_or(BenchmarkError::MissingCohort(cohort_request.cohort_id))?;
        if resolved
            .cohort
            .reproduce_fingerprint()
            .map_err(|_| BenchmarkError::CohortEvidenceFingerprint(resolved.cohort.id))?
            != resolved.cohort.fingerprint
            || resolved
                .role
                .reproduce_fingerprint()
                .map_err(|_| BenchmarkError::CohortEvidenceFingerprint(resolved.cohort.id))?
                != resolved.role.fingerprint
        {
            return Err(BenchmarkError::CohortEvidenceFingerprint(
                resolved.cohort.id,
            ));
        }
        validate_suite_role(request.kind, resolved)?;
        if request.kind == BenchmarkSuiteKind::SealedAcceptance
            && (cohort_request.disclosure != DisclosureLevel::Aggregate
                || cohort_request.adaptation_eligible)
        {
            return Err(BenchmarkError::SealedDisclosure);
        }
        cohort_request
            .protocol
            .validate_for_labels(&request.labels)
            .map_err(|error| BenchmarkError::MetricRequirement(error.to_string()))?;
        if cohort_request.protocol.split != resolved.cohort.split {
            return Err(BenchmarkError::IncompatibleRole(resolved.cohort.id));
        }
        cohorts.push(BenchmarkCohort {
            cohort_id: resolved.cohort.id,
            cohort_fingerprint: resolved.cohort.fingerprint.clone(),
            evaluation_cohort_fingerprint: artifact_core::fingerprint(&(
                resolved.cohort.snapshot_fingerprint.as_str(),
                resolved.cohort.split,
            ))
            .map_err(map_fingerprint)?,
            snapshot_id: resolved.cohort.snapshot_id,
            snapshot_fingerprint: resolved.cohort.snapshot_fingerprint.clone(),
            split: resolved.cohort.split,
            role: resolved.role.role,
            role_decision_id: resolved.role.id,
            role_decision_fingerprint: resolved.role.fingerprint.clone(),
            protocol_fingerprint: cohort_request
                .protocol
                .fingerprint()
                .map_err(map_fingerprint)?,
            protocol: cohort_request.protocol,
            disclosure: cohort_request.disclosure,
            adaptation_eligible: cohort_request.adaptation_eligible,
        });
    }
    cohorts.sort_by_key(|cohort| cohort.cohort_id);
    let suite_cohort_ids = cohorts
        .iter()
        .map(|cohort| cohort.cohort_id)
        .collect::<BTreeSet<_>>();
    let report_cohort_ids = contamination
        .cohort_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if suite_cohort_ids != report_cohort_ids {
        return Err(BenchmarkError::ContaminationCohorts);
    }
    let override_fingerprint = match contamination.status {
        ContaminationStatus::Clean => None,
        ContaminationStatus::Blocked => {
            let value = contamination_override.ok_or(BenchmarkError::ContaminationBlocked)?;
            if value.report_id != contamination.id
                || value.report_fingerprint != contamination.fingerprint
                || value.reproduce_fingerprint().map_err(map_contamination)? != value.fingerprint
            {
                return Err(BenchmarkError::ContaminationBlocked);
            }
            Some(value.fingerprint.clone())
        }
    };
    let mut required_model_formats = request
        .required_model_formats
        .into_iter()
        .map(|value| required(value, "model format"))
        .collect::<Result<Vec<_>, _>>()?;
    required_model_formats.sort();
    required_model_formats.dedup();
    let mut suite = BenchmarkSuite {
        id: Uuid::new_v4(),
        name,
        kind: request.kind,
        task,
        labels: request.labels,
        required_model_formats,
        cohorts,
        contract: request.contract,
        contamination_report_id: contamination.id,
        contamination_report_fingerprint: contamination.fingerprint.clone(),
        contamination_override_fingerprint: override_fingerprint,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    suite.fingerprint = suite_fingerprint(&suite)?;
    suite.validate_integrity()?;
    Ok(suite)
}

pub fn assess_benchmark(
    suite: &BenchmarkSuite,
    inputs: Vec<CohortAssessmentInput>,
) -> Result<AcceptanceAssessment, BenchmarkError> {
    suite.validate_legacy_compatible_integrity()?;
    let mut by_cohort = BTreeMap::new();
    for input in inputs {
        let cohort_id = input.cohort_id;
        if !suite
            .cohorts
            .iter()
            .any(|cohort| cohort.cohort_id == cohort_id)
        {
            return Err(BenchmarkError::AssessmentCohort(cohort_id));
        }
        if by_cohort.insert(cohort_id, input).is_some() {
            return Err(BenchmarkError::AssessmentCohort(cohort_id));
        }
    }
    let mut reasons = Vec::new();
    if let Err(error) = validate_contract(&suite.contract, &suite.labels, suite.kind) {
        let (code, state) = if error == BenchmarkError::VacuousContract {
            ("vacuous_contract", AcceptanceReasonState::Inconclusive)
        } else {
            ("invalid_contract", AcceptanceReasonState::Invalid)
        };
        reason(
            &mut reasons,
            None,
            code,
            state,
            error.to_string(),
            None,
            None,
        );
    }
    let mut run_ids = BTreeMap::new();
    let mut comparison_ids = BTreeMap::new();
    let mut checkpoint_id = None;
    for cohort in &suite.cohorts {
        let Some(input) = by_cohort.get(&cohort.cohort_id) else {
            reason(
                &mut reasons,
                Some(cohort.cohort_id),
                "missing_run",
                AcceptanceReasonState::Inconclusive,
                "suite cohort has no evaluation run",
                None,
                None,
            );
            continue;
        };
        let Some(run) = &input.run else {
            reason(
                &mut reasons,
                Some(cohort.cohort_id),
                "missing_run",
                AcceptanceReasonState::Inconclusive,
                "suite cohort has no evaluation run",
                None,
                None,
            );
            continue;
        };
        run_ids.insert(cohort.cohort_id, run.id);
        if let Some(expected) = checkpoint_id {
            if expected != run.checkpoint_id {
                reason(
                    &mut reasons,
                    Some(cohort.cohort_id),
                    "checkpoint_mismatch",
                    AcceptanceReasonState::Invalid,
                    "suite runs do not evaluate one checkpoint",
                    None,
                    None,
                );
            }
        } else {
            checkpoint_id = Some(run.checkpoint_id);
        }
        validate_run(suite, cohort, run, &mut reasons);
        if let Some(metrics) = &run.metrics {
            evaluate_requirements(cohort.cohort_id, metrics, &suite.contract, &mut reasons);
        }
        evaluate_regression(
            cohort,
            run,
            input.comparison.as_ref(),
            &suite.contract,
            &mut reasons,
        );
        if let Some(comparison) = &input.comparison {
            comparison_ids.insert(cohort.cohort_id, comparison.id);
        }
    }
    if reasons.is_empty() {
        reason(
            &mut reasons,
            None,
            "all_requirements",
            AcceptanceReasonState::Passed,
            "all deterministic acceptance requirements passed",
            None,
            None,
        );
    }
    let state = derive_state(&reasons);
    let mut assessment = AcceptanceAssessment {
        id: Uuid::new_v4(),
        suite_id: suite.id,
        suite_fingerprint: suite.fingerprint.clone(),
        checkpoint_id,
        evaluation_run_ids: run_ids,
        comparison_ids,
        state,
        reasons,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    assessment.fingerprint = assessment_fingerprint(&assessment)?;
    Ok(assessment)
}

fn validate_run(
    suite: &BenchmarkSuite,
    cohort: &BenchmarkCohort,
    run: &EvaluationRun,
    reasons: &mut Vec<AcceptanceReason>,
) {
    let invalid = run.state != EvaluationRunState::Completed
        || run.metrics.is_none()
        || run.snapshot_id != cohort.snapshot_id
        || run.split != cohort.split
        || run.protocol_fingerprint != cohort.protocol_fingerprint
        || run.source_identity.snapshot_fingerprint != cohort.snapshot_fingerprint
        || run.source_identity.cohort_fingerprint != cohort.evaluation_cohort_fingerprint
        || run.source_identity.labels != suite.labels
        || (!suite.required_model_formats.is_empty()
            && !suite
                .required_model_formats
                .contains(&run.source_identity.checkpoint_model_format));
    if invalid {
        reason(
            reasons,
            Some(cohort.cohort_id),
            "incompatible_run",
            AcceptanceReasonState::Invalid,
            "evaluation run is incomplete or incompatible with the immutable suite",
            None,
            None,
        );
    }
}

fn evaluate_requirements(
    cohort_id: Uuid,
    metrics: &EvaluationMetrics,
    contract: &AcceptanceContract,
    reasons: &mut Vec<AcceptanceReason>,
) {
    for requirement in &contract.metric_requirements {
        let resolved = resolve_metric(metrics, requirement);
        let Some((support, observed)) = resolved else {
            reason(
                reasons,
                Some(cohort_id),
                "missing_metric_target",
                AcceptanceReasonState::Inconclusive,
                "required label or slice metrics are absent",
                None,
                None,
            );
            continue;
        };
        if support < requirement.minimum_support {
            reason(
                reasons,
                Some(cohort_id),
                "insufficient_support",
                AcceptanceReasonState::Inconclusive,
                format!(
                    "metric target support {support} is below {}",
                    requirement.minimum_support
                ),
                Some(support as f64),
                Some(requirement.minimum_support as f64),
            );
            continue;
        }
        if let Some(minimum) = requirement.minimum {
            if observed < minimum {
                reason(
                    reasons,
                    Some(cohort_id),
                    "metric_below_minimum",
                    AcceptanceReasonState::Failed,
                    format!(
                        "{:?} for {:?} is below its minimum",
                        requirement.metric, requirement.target
                    ),
                    Some(observed),
                    Some(minimum),
                );
            }
        }
        if let Some(maximum) = requirement.maximum {
            if observed > maximum {
                reason(
                    reasons,
                    Some(cohort_id),
                    "metric_above_maximum",
                    AcceptanceReasonState::Failed,
                    format!(
                        "{:?} for {:?} exceeds its maximum",
                        requirement.metric, requirement.target
                    ),
                    Some(observed),
                    Some(maximum),
                );
            }
        }
    }
}

fn evaluate_regression(
    cohort: &BenchmarkCohort,
    candidate_run: &EvaluationRun,
    comparison: Option<&EvaluationComparisonReport>,
    contract: &AcceptanceContract,
    reasons: &mut Vec<AcceptanceReason>,
) {
    let Some(requirement) = &contract.regression else {
        if comparison.is_some() {
            reason(
                reasons,
                Some(cohort.cohort_id),
                "unexpected_comparison",
                AcceptanceReasonState::Invalid,
                "paired comparison was supplied without a regression requirement",
                None,
                None,
            );
        }
        return;
    };
    let Some(comparison) = comparison else {
        reason(
            reasons,
            Some(cohort.cohort_id),
            "missing_baseline_comparison",
            AcceptanceReasonState::Inconclusive,
            "regression contract requires a paired baseline comparison",
            None,
            None,
        );
        return;
    };
    let fingerprint_matches = comparison_fingerprint(comparison)
        .is_ok_and(|fingerprint| fingerprint == comparison.fingerprint);
    let candidate_metrics_match = candidate_run
        .metrics
        .as_ref()
        .is_some_and(|metrics| metrics == &comparison.right_metrics);
    if !fingerprint_matches
        || !comparison_is_consistent(comparison, candidate_run)
        || comparison.left_run_id == comparison.right_run_id
        || comparison.right_run_id != candidate_run.id
        || !candidate_metrics_match
        || comparison.cohort_fingerprint != cohort.evaluation_cohort_fingerprint
        || comparison.protocol_fingerprint != cohort.protocol_fingerprint
    {
        reason(
            reasons,
            Some(cohort.cohort_id),
            "incompatible_comparison",
            AcceptanceReasonState::Invalid,
            "paired comparison is not authentic or is incompatible with the candidate run, cohort, or protocol",
            None,
            None,
        );
        return;
    }
    if let Some(maximum) = requirement.max_accuracy_drop {
        if comparison.accuracy_delta < -maximum {
            reason(
                reasons,
                Some(cohort.cohort_id),
                "accuracy_regression",
                AcceptanceReasonState::Failed,
                "accuracy regression exceeds tolerance",
                Some(comparison.accuracy_delta),
                Some(-maximum),
            );
        }
    }
    if let Some(maximum) = requirement.max_macro_f1_drop {
        if comparison.macro_f1_delta < -maximum {
            reason(
                reasons,
                Some(cohort.cohort_id),
                "macro_f1_regression",
                AcceptanceReasonState::Failed,
                "macro-F1 regression exceeds tolerance",
                Some(comparison.macro_f1_delta),
                Some(-maximum),
            );
        }
    }
    if let Some(minimum) = requirement.minimum_accuracy_delta_lower_bound {
        if comparison.accuracy_delta_interval.lower < minimum {
            reason(
                reasons,
                Some(cohort.cohort_id),
                "accuracy_interval",
                AcceptanceReasonState::Inconclusive,
                "accuracy interval lower bound does not establish the required improvement",
                Some(comparison.accuracy_delta_interval.lower),
                Some(minimum),
            );
        }
    }
    if let Some(minimum) = requirement.minimum_macro_f1_delta_lower_bound {
        if comparison.macro_f1_delta_interval.lower < minimum {
            reason(
                reasons,
                Some(cohort.cohort_id),
                "macro_f1_interval",
                AcceptanceReasonState::Inconclusive,
                "macro-F1 interval lower bound does not establish the required improvement",
                Some(comparison.macro_f1_delta_interval.lower),
                Some(minimum),
            );
        }
    }
    if requirement.require_mcnemar_significance && !comparison.mcnemar.significant {
        reason(
            reasons,
            Some(cohort.cohort_id),
            "mcnemar_not_significant",
            AcceptanceReasonState::Inconclusive,
            "paired errors do not meet the configured significance requirement",
            Some(comparison.mcnemar.two_sided_p_value),
            None,
        );
    }
}

fn comparison_is_consistent(
    comparison: &EvaluationComparisonReport,
    candidate_run: &EvaluationRun,
) -> bool {
    let left = &comparison.left_metrics.overall;
    let right = &comparison.right_metrics.overall;
    let total = comparison
        .both_correct
        .checked_add(comparison.both_wrong)
        .and_then(|value| value.checked_add(comparison.left_only_correct))
        .and_then(|value| value.checked_add(comparison.right_only_correct));
    let fixed = comparison
        .fixed_snapshot_member_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let regressed = comparison
        .regressed_snapshot_member_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let label_deltas_match = left.per_label.len() == right.per_label.len()
        && right.per_label.iter().all(|(label, right_metrics)| {
            left.per_label.get(label).is_some_and(|left_metrics| {
                comparison
                    .per_label_f1_delta
                    .get(label)
                    .is_some_and(|delta| {
                        approximately_equal(*delta, right_metrics.f1 - left_metrics.f1)
                    })
            })
        })
        && comparison.per_label_f1_delta.len() == right.per_label.len();
    let intervals = [
        &comparison.accuracy_delta_interval,
        &comparison.macro_f1_delta_interval,
    ];
    let intervals_valid = intervals.iter().all(|interval| {
        interval.level.is_finite()
            && interval.level > 0.0
            && interval.level < 1.0
            && interval.lower.is_finite()
            && interval.upper.is_finite()
            && interval.lower <= interval.upper
    });
    let expected_significance =
        comparison.mcnemar.left_only_correct + comparison.mcnemar.right_only_correct > 0
            && comparison.mcnemar.two_sided_p_value < 1.0 - candidate_run.protocol.confidence_level;

    left.total == right.total
        && total == Some(right.total)
        && approximately_equal(comparison.accuracy_delta, right.accuracy - left.accuracy)
        && approximately_equal(comparison.macro_f1_delta, right.macro_f1 - left.macro_f1)
        && label_deltas_match
        && fixed.len() == comparison.fixed_snapshot_member_ids.len()
        && regressed.len() == comparison.regressed_snapshot_member_ids.len()
        && fixed.len() as u64 == comparison.right_only_correct
        && regressed.len() as u64 == comparison.left_only_correct
        && fixed.is_disjoint(&regressed)
        && comparison.mcnemar.left_only_correct == comparison.left_only_correct
        && comparison.mcnemar.right_only_correct == comparison.right_only_correct
        && comparison.mcnemar.two_sided_p_value.is_finite()
        && (0.0..=1.0).contains(&comparison.mcnemar.two_sided_p_value)
        && comparison.mcnemar.significant == expected_significance
        && intervals_valid
}

fn approximately_equal(left: f64, right: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= 1e-12
}

fn resolve_metric(
    metrics: &EvaluationMetrics,
    requirement: &MetricRequirement,
) -> Option<(u64, f64)> {
    match &requirement.target {
        MetricTarget::Overall => Some((
            metrics.overall.total,
            classification_value(requirement.metric, &metrics.overall)?,
        )),
        MetricTarget::Label { label } => {
            let value = metrics.overall.per_label.get(label)?;
            Some((value.support, label_value(requirement.metric, value)?))
        }
        MetricTarget::Slice { key } => {
            let value = metrics.slices.get(key)?;
            Some((
                value.support,
                classification_value(requirement.metric, &value.metrics)?,
            ))
        }
    }
}

fn classification_value(metric: BenchmarkMetric, value: &ClassificationMetrics) -> Option<f64> {
    Some(match metric {
        BenchmarkMetric::Accuracy => value.accuracy,
        BenchmarkMetric::MacroPrecision => value.macro_precision,
        BenchmarkMetric::MacroRecall => value.macro_recall,
        BenchmarkMetric::MacroF1 => value.macro_f1,
        BenchmarkMetric::WeightedF1 => value.weighted_f1,
        BenchmarkMetric::LogLoss => value.log_loss,
        BenchmarkMetric::BrierScore => value.brier_score,
        BenchmarkMetric::ExpectedCalibrationError => value.expected_calibration_error,
        BenchmarkMetric::Precision | BenchmarkMetric::Recall | BenchmarkMetric::F1 => return None,
    })
}

fn label_value(metric: BenchmarkMetric, value: &LabelMetrics) -> Option<f64> {
    Some(match metric {
        BenchmarkMetric::Precision => value.precision,
        BenchmarkMetric::Recall => value.recall,
        BenchmarkMetric::F1 => value.f1,
        _ => return None,
    })
}

fn validate_suite_role(
    kind: BenchmarkSuiteKind,
    evidence: &BenchmarkCohortEvidence,
) -> Result<(), BenchmarkError> {
    if evidence.role.cohort_id != evidence.cohort.id
        || evidence.role.disposition == CohortDisposition::Retired
    {
        return Err(BenchmarkError::RetiredCohort(evidence.cohort.id));
    }
    let compatible = match kind {
        BenchmarkSuiteKind::Development => matches!(
            evidence.role.role,
            CohortRole::Development | CohortRole::Diagnostic | CohortRole::ExternalBenchmark
        ),
        BenchmarkSuiteKind::SealedAcceptance => matches!(
            evidence.role.role,
            CohortRole::SealedAcceptance | CohortRole::ExternalBenchmark
        ),
    };
    if !compatible {
        return Err(BenchmarkError::IncompatibleRole(evidence.cohort.id));
    }
    Ok(())
}

fn validate_labels(labels: &[String]) -> Result<(), BenchmarkError> {
    if labels.len() < 2
        || labels.iter().any(|label| label.trim().is_empty())
        || labels.iter().collect::<BTreeSet<_>>().len() != labels.len()
    {
        return Err(BenchmarkError::Labels);
    }
    Ok(())
}

fn validate_contract(
    contract: &AcceptanceContract,
    labels: &[String],
    suite_kind: BenchmarkSuiteKind,
) -> Result<(), BenchmarkError> {
    let regression_is_effective = contract.regression.as_ref().is_some_and(|requirement| {
        requirement.max_accuracy_drop.is_some()
            || requirement.max_macro_f1_drop.is_some()
            || requirement.minimum_accuracy_delta_lower_bound.is_some()
            || requirement.minimum_macro_f1_delta_lower_bound.is_some()
    });
    if contract.metric_requirements.is_empty() && !regression_is_effective {
        return Err(BenchmarkError::VacuousContract);
    }

    let mut seen = BTreeSet::new();
    for requirement in &contract.metric_requirements {
        if suite_kind == BenchmarkSuiteKind::SealedAcceptance
            && requirement.target != MetricTarget::Overall
        {
            return Err(BenchmarkError::MetricRequirement(
                "sealed acceptance contracts may target only aggregate overall metrics".into(),
            ));
        }
        if requirement.minimum_support == 0
            || requirement.minimum.is_some_and(|value| !value.is_finite())
            || requirement.maximum.is_some_and(|value| !value.is_finite())
            || requirement
                .minimum
                .zip(requirement.maximum)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(BenchmarkError::MetricRequirement(format!(
                "invalid bounds for {:?}",
                requirement.target
            )));
        }
        if !seen.insert((requirement.target.clone(), requirement.metric)) {
            return Err(BenchmarkError::MetricRequirement(format!(
                "duplicate requirement for {:?} and {:?}",
                requirement.target, requirement.metric
            )));
        }

        let lower_is_better = matches!(
            requirement.metric,
            BenchmarkMetric::LogLoss
                | BenchmarkMetric::BrierScore
                | BenchmarkMetric::ExpectedCalibrationError
        );
        if lower_is_better {
            let Some(maximum) = requirement.maximum else {
                return Err(BenchmarkError::MetricRequirement(format!(
                    "{:?} requires a maximum",
                    requirement.metric
                )));
            };
            if requirement.minimum.is_some() || maximum < 0.0 {
                return Err(BenchmarkError::MetricRequirement(format!(
                    "{:?} accepts only a non-negative maximum",
                    requirement.metric
                )));
            }
            let bounded_maximum = match requirement.metric {
                BenchmarkMetric::BrierScore => Some(2.0),
                BenchmarkMetric::ExpectedCalibrationError => Some(1.0),
                _ => None,
            };
            if let Some(bound) = bounded_maximum {
                if maximum >= bound {
                    return Err(BenchmarkError::MetricRequirement(format!(
                        "{:?} maximum must be below {bound}",
                        requirement.metric
                    )));
                }
            }
        } else {
            let Some(minimum) = requirement.minimum else {
                return Err(BenchmarkError::MetricRequirement(format!(
                    "{:?} requires a positive minimum",
                    requirement.metric
                )));
            };
            if minimum <= 0.0
                || minimum > 1.0
                || requirement.maximum.is_some_and(|maximum| maximum > 1.0)
            {
                return Err(BenchmarkError::MetricRequirement(format!(
                    "{:?} minimum must be in (0, 1] and maximum, if present, at most 1",
                    requirement.metric
                )));
            }
        }

        match (&requirement.target, requirement.metric) {
            (
                MetricTarget::Label { label },
                BenchmarkMetric::Precision | BenchmarkMetric::Recall | BenchmarkMetric::F1,
            ) if labels.contains(label) => {}
            (
                MetricTarget::Overall | MetricTarget::Slice { .. },
                BenchmarkMetric::Precision | BenchmarkMetric::Recall | BenchmarkMetric::F1,
            ) => {
                return Err(BenchmarkError::MetricRequirement(
                    "precision/recall/F1 require a label target".into(),
                ));
            }
            (MetricTarget::Label { .. }, _) => {
                return Err(BenchmarkError::MetricRequirement(
                    "label targets support precision, recall, or F1".into(),
                ));
            }
            (MetricTarget::Slice { key }, _) => {
                SliceIdentity::parse_key_for_labels(key, labels).map_err(|error| {
                    BenchmarkError::MetricRequirement(format!(
                        "invalid canonical slice target: {error}"
                    ))
                })?;
            }
            _ => {}
        }
    }
    if let Some(regression) = &contract.regression {
        if suite_kind == BenchmarkSuiteKind::SealedAcceptance {
            return Err(BenchmarkError::RegressionRequirement(
                "sealed acceptance contracts cannot request paired comparisons".into(),
            ));
        }
        let values = [
            regression.max_accuracy_drop,
            regression.max_macro_f1_drop,
            regression.minimum_accuracy_delta_lower_bound,
            regression.minimum_macro_f1_delta_lower_bound,
        ];
        if !regression_is_effective {
            return Err(BenchmarkError::RegressionRequirement(
                "regression requires at least one quantitative bound".into(),
            ));
        }
        if values.into_iter().flatten().any(|value| !value.is_finite())
            || regression
                .max_accuracy_drop
                .is_some_and(|value| !(0.0..1.0).contains(&value))
            || regression
                .max_macro_f1_drop
                .is_some_and(|value| !(0.0..1.0).contains(&value))
            || regression
                .minimum_accuracy_delta_lower_bound
                .is_some_and(|value| !(-1.0..=1.0).contains(&value) || value == -1.0)
            || regression
                .minimum_macro_f1_delta_lower_bound
                .is_some_and(|value| !(-1.0..=1.0).contains(&value) || value == -1.0)
        {
            return Err(BenchmarkError::RegressionRequirement(
                "bounds must be finite and non-vacuous rate deltas".into(),
            ));
        }
    }
    Ok(())
}

fn derive_state(reasons: &[AcceptanceReason]) -> AcceptanceState {
    if reasons
        .iter()
        .any(|reason| reason.state == AcceptanceReasonState::Invalid)
    {
        AcceptanceState::Invalid
    } else if reasons
        .iter()
        .any(|reason| reason.state == AcceptanceReasonState::Failed)
    {
        AcceptanceState::Fail
    } else if reasons
        .iter()
        .any(|reason| reason.state == AcceptanceReasonState::Inconclusive)
    {
        AcceptanceState::Inconclusive
    } else {
        AcceptanceState::Pass
    }
}

#[allow(clippy::too_many_arguments)]
fn reason(
    reasons: &mut Vec<AcceptanceReason>,
    cohort_id: Option<Uuid>,
    code: &str,
    state: AcceptanceReasonState,
    message: impl Into<String>,
    observed: Option<f64>,
    required: Option<f64>,
) {
    reasons.push(AcceptanceReason {
        cohort_id,
        code: code.into(),
        state,
        message: message.into(),
        observed,
        required,
    });
}

fn suite_fingerprint(value: &BenchmarkSuite) -> Result<String, BenchmarkError> {
    artifact_core::fingerprint(&serde_json::json!({
        "name": value.name, "kind": value.kind, "task": value.task, "labels": value.labels,
        "required_model_formats": value.required_model_formats, "cohorts": value.cohorts,
        "contract": value.contract, "contamination_report_id": value.contamination_report_id,
        "contamination_report_fingerprint": value.contamination_report_fingerprint,
        "contamination_override_fingerprint": value.contamination_override_fingerprint,
    }))
    .map_err(map_fingerprint)
}

fn assessment_fingerprint(value: &AcceptanceAssessment) -> Result<String, BenchmarkError> {
    artifact_core::fingerprint(&serde_json::json!({
        "suite_id": value.suite_id, "suite_fingerprint": value.suite_fingerprint,
        "checkpoint_id": value.checkpoint_id, "evaluation_run_ids": value.evaluation_run_ids,
        "comparison_ids": value.comparison_ids, "state": value.state, "reasons": value.reasons,
    }))
    .map_err(map_fingerprint)
}

fn required(value: String, field: &'static str) -> Result<String, BenchmarkError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(BenchmarkError::Empty(field))
    } else {
        Ok(value)
    }
}

fn is_canonical_fingerprint(value: &str) -> bool {
    !value.trim().is_empty() && value.trim() == value
}

const fn one() -> u64 {
    1
}

fn map_fingerprint(error: artifact_core::FingerprintError) -> BenchmarkError {
    BenchmarkError::Fingerprint(error.to_string())
}
fn map_contamination(error: crate::contamination::ContaminationError) -> BenchmarkError {
    BenchmarkError::Fingerprint(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use evaluation_core::{
        domain::{
            ConfidenceInterval, EvaluationPrediction, EvaluationSourceIdentity, McNemarResult,
        },
        metrics::calculate_metrics_with_protocol,
    };
    use training_core::domain::LabelProbability;

    use super::*;
    use crate::{
        contamination::{
            CohortContaminationInput, ContaminationMember, ContaminationPolicy, check_contamination,
        },
        governance::CohortOrigin,
    };

    fn cohort(name: &str) -> BenchmarkCohortEvidence {
        let cohort = EvaluationCohort::new(
            name,
            Uuid::new_v4(),
            format!("sha256:{name}"),
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
        )
        .expect("cohort");
        let role =
            CohortRoleDecision::initial(&cohort, CohortRole::Development, "fixture").expect("role");
        BenchmarkCohortEvidence { cohort, role }
    }

    fn suite_fixture() -> BenchmarkSuite {
        let evidence = vec![cohort("fold-a"), cohort("fold-b")];
        let contamination_inputs = evidence
            .iter()
            .enumerate()
            .map(|(index, item)| CohortContaminationInput {
                cohort: item.cohort.clone(),
                role: item.role.clone(),
                members: vec![ContaminationMember {
                    snapshot_member_id: Uuid::new_v4(),
                    snapshot_id: item.cohort.snapshot_id,
                    source_row_id: Uuid::new_v4(),
                    text: format!("unique example {index}"),
                    group_id: None,
                }],
            })
            .collect();
        let contamination =
            check_contamination(contamination_inputs, None, ContaminationPolicy::default())
                .expect("clean report");
        let protocol = EvaluationProtocol {
            split: SnapshotSplit::Validation,
            ..EvaluationProtocol::default()
        };
        let request = BenchmarkSuiteRequest {
            name: "development".into(),
            kind: BenchmarkSuiteKind::Development,
            task: "classify support".into(),
            labels: vec!["billing".into(), "fraud".into()],
            required_model_formats: vec!["fixture".into()],
            cohorts: evidence
                .iter()
                .map(|item| BenchmarkCohortRequest {
                    cohort_id: item.cohort.id,
                    protocol: protocol.clone(),
                    disclosure: DisclosureLevel::Slices,
                    adaptation_eligible: true,
                })
                .collect(),
            contract: AcceptanceContract {
                metric_requirements: vec![MetricRequirement {
                    target: MetricTarget::Overall,
                    metric: BenchmarkMetric::Accuracy,
                    minimum: Some(0.9),
                    maximum: None,
                    minimum_support: 1,
                }],
                regression: None,
            },
        };
        build_benchmark_suite(request, evidence, &contamination, None).expect("suite")
    }

    fn completed_run(
        suite: &BenchmarkSuite,
        cohort: &BenchmarkCohort,
        correct: bool,
    ) -> EvaluationRun {
        let source_identity = EvaluationSourceIdentity {
            checkpoint_checksum: "sha256:checkpoint".into(),
            checkpoint_model_format: "fixture".into(),
            base_model_fingerprint: None,
            tokenizer_fingerprint: None,
            snapshot_fingerprint: cohort.snapshot_fingerprint.clone(),
            cohort_fingerprint: cohort.evaluation_cohort_fingerprint.clone(),
            labels: suite.labels.clone(),
        };
        let mut run = EvaluationRun::queued_with_protocol(
            Uuid::from_u128(99),
            cohort.snapshot_id,
            cohort.protocol.clone(),
            source_identity,
            1,
            "sha256:input",
        )
        .expect("run");
        let prediction = EvaluationPrediction {
            id: Uuid::new_v4(),
            evaluation_run_id: run.id,
            snapshot_member_id: Uuid::new_v4(),
            source_row_id: Uuid::new_v4(),
            text: "example".into(),
            expected_label: "billing".into(),
            predicted_label: if correct { "billing" } else { "fraud" }.into(),
            confidence: 0.9,
            probabilities: vec![
                LabelProbability {
                    label: "billing".into(),
                    probability: if correct { 0.9 } else { 0.1 },
                },
                LabelProbability {
                    label: "fraud".into(),
                    probability: if correct { 0.1 } else { 0.9 },
                },
            ],
            dimensions: BTreeMap::new(),
            created_at: Utc::now(),
        };
        run.metrics = Some(calculate_metrics_with_protocol(
            &suite.labels,
            &[prediction],
            &run.protocol,
        ));
        run.example_count = 1;
        run.processed_examples = 1;
        run.transition(EvaluationRunState::Running)
            .expect("running");
        run.transition(EvaluationRunState::Completed)
            .expect("completed");
        run
    }

    fn no_change_comparison(
        run: &EvaluationRun,
        cohort: &BenchmarkCohort,
    ) -> EvaluationComparisonReport {
        let metrics = run.metrics.clone().expect("completed metrics");
        let total = metrics.overall.total;
        let mut comparison = EvaluationComparisonReport {
            id: Uuid::new_v4(),
            left_run_id: Uuid::new_v4(),
            right_run_id: run.id,
            cohort_fingerprint: cohort.evaluation_cohort_fingerprint.clone(),
            protocol_fingerprint: cohort.protocol_fingerprint.clone(),
            left_metrics: metrics.clone(),
            right_metrics: metrics,
            accuracy_delta: 0.0,
            macro_f1_delta: 0.0,
            per_label_f1_delta: BTreeMap::from([("billing".into(), 0.0), ("fraud".into(), 0.0)]),
            both_correct: total,
            both_wrong: 0,
            left_only_correct: 0,
            right_only_correct: 0,
            fixed_snapshot_member_ids: Vec::new(),
            regressed_snapshot_member_ids: Vec::new(),
            accuracy_delta_interval: ConfidenceInterval {
                level: run.protocol.confidence_level,
                lower: 0.0,
                upper: 0.0,
            },
            macro_f1_delta_interval: ConfidenceInterval {
                level: run.protocol.confidence_level,
                lower: 0.0,
                upper: 0.0,
            },
            mcnemar: McNemarResult {
                left_only_correct: 0,
                right_only_correct: 0,
                two_sided_p_value: 1.0,
                significant: false,
            },
            slice_deltas: BTreeMap::new(),
            fingerprint: String::new(),
            created_at: Utc::now(),
        };
        comparison.fingerprint = comparison_fingerprint(&comparison).expect("fingerprint");
        comparison
    }

    #[test]
    fn suite_access_policy_enforces_disclosure_and_adaptive_eligibility() {
        let mut suite = suite_fixture();
        validate_suite_access(
            &suite,
            ExposurePurpose::Optimization,
            Some(DisclosureLevel::Slices),
        )
        .expect("slice-level adaptive use is permitted");
        assert!(matches!(
            validate_suite_access(
                &suite,
                ExposurePurpose::Diagnosis,
                Some(DisclosureLevel::RowContent),
            ),
            Err(BenchmarkError::DisclosureExceeded { .. })
        ));

        suite.cohorts[0].adaptation_eligible = false;
        suite.fingerprint = suite_fingerprint(&suite).expect("suite fingerprint");
        assert!(matches!(
            validate_suite_access(
                &suite,
                ExposurePurpose::Optimization,
                Some(DisclosureLevel::Slices),
            ),
            Err(BenchmarkError::AdaptiveAccess { .. })
        ));
    }

    #[test]
    fn deterministic_assessment_distinguishes_fail_and_inconclusive() {
        let suite = suite_fixture();
        let inputs = suite
            .cohorts
            .iter()
            .enumerate()
            .map(|(index, cohort)| CohortAssessmentInput {
                cohort_id: cohort.cohort_id,
                run: Some(completed_run(&suite, cohort, index == 0)),
                comparison: None,
            })
            .collect();
        let failed = assess_benchmark(&suite, inputs).expect("assessment");
        assert_eq!(failed.state, AcceptanceState::Fail);
        assert!(
            failed
                .reasons
                .iter()
                .any(|reason| reason.code == "metric_below_minimum")
        );
        assert_eq!(
            failed.reproduce_fingerprint().expect("fingerprint"),
            failed.fingerprint
        );

        let inconclusive = assess_benchmark(
            &suite,
            vec![CohortAssessmentInput {
                cohort_id: suite.cohorts[0].cohort_id,
                run: Some(completed_run(&suite, &suite.cohorts[0], true)),
                comparison: None,
            }],
        )
        .expect("assessment");
        assert_eq!(inconclusive.state, AcceptanceState::Inconclusive);
    }

    #[test]
    fn contracts_must_contain_meaningful_decision_criteria() {
        let labels = vec!["billing".into(), "fraud".into()];
        assert_eq!(
            validate_contract(
                &AcceptanceContract {
                    metric_requirements: Vec::new(),
                    regression: None,
                },
                &labels,
                BenchmarkSuiteKind::Development,
            ),
            Err(BenchmarkError::VacuousContract)
        );

        let zero_floor = AcceptanceContract {
            metric_requirements: vec![MetricRequirement {
                target: MetricTarget::Overall,
                metric: BenchmarkMetric::Accuracy,
                minimum: Some(0.0),
                maximum: None,
                minimum_support: 1,
            }],
            regression: None,
        };
        assert!(matches!(
            validate_contract(&zero_floor, &labels, BenchmarkSuiteKind::Development),
            Err(BenchmarkError::MetricRequirement(_))
        ));

        let sealed_label = AcceptanceContract {
            metric_requirements: vec![MetricRequirement {
                target: MetricTarget::Label {
                    label: "billing".into(),
                },
                metric: BenchmarkMetric::F1,
                minimum: Some(0.5),
                maximum: None,
                minimum_support: 1,
            }],
            regression: None,
        };
        assert!(matches!(
            validate_contract(&sealed_label, &labels, BenchmarkSuiteKind::SealedAcceptance),
            Err(BenchmarkError::MetricRequirement(_))
        ));
    }

    #[test]
    fn legacy_vacuous_contracts_can_never_pass_assessment() {
        let mut suite = suite_fixture();
        suite.contract = AcceptanceContract {
            metric_requirements: Vec::new(),
            regression: None,
        };
        suite.fingerprint = suite_fingerprint(&suite).expect("legacy fingerprint");
        let inputs = suite
            .cohorts
            .iter()
            .map(|cohort| CohortAssessmentInput {
                cohort_id: cohort.cohort_id,
                run: Some(completed_run(&suite, cohort, true)),
                comparison: None,
            })
            .collect();
        let assessment = assess_benchmark(&suite, inputs).expect("legacy assessment");
        assert_eq!(assessment.state, AcceptanceState::Inconclusive);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.code == "vacuous_contract")
        );
    }

    #[test]
    fn paired_comparison_must_bind_to_the_candidate_run() {
        let mut suite = suite_fixture();
        suite.contract.regression = Some(RegressionRequirement {
            max_accuracy_drop: Some(0.1),
            max_macro_f1_drop: None,
            minimum_accuracy_delta_lower_bound: None,
            minimum_macro_f1_delta_lower_bound: None,
            require_mcnemar_significance: false,
        });
        suite.fingerprint = suite_fingerprint(&suite).expect("suite fingerprint");
        let cohort = &suite.cohorts[0];
        let run = completed_run(&suite, cohort, true);
        let mut comparison = no_change_comparison(&run, cohort);
        comparison.right_run_id = Uuid::new_v4();
        comparison.fingerprint = comparison_fingerprint(&comparison).expect("forged fingerprint");

        let assessment = assess_benchmark(
            &suite,
            vec![CohortAssessmentInput {
                cohort_id: cohort.cohort_id,
                run: Some(run),
                comparison: Some(comparison),
            }],
        )
        .expect("assessment");
        assert_eq!(assessment.state, AcceptanceState::Invalid);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.code == "incompatible_comparison")
        );
    }
}
