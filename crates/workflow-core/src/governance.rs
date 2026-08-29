//! Append-only cohort roles, evidence exposures, and adaptive-overfitting risk.

use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotSplit;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortOrigin {
    InternalSnapshot,
    ExternalBenchmark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationCohort {
    pub id: Uuid,
    pub name: String,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub split: SnapshotSplit,
    pub origin: CohortOrigin,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl EvaluationCohort {
    pub fn new(
        name: impl Into<String>,
        snapshot_id: Uuid,
        snapshot_fingerprint: impl Into<String>,
        split: SnapshotSplit,
        origin: CohortOrigin,
    ) -> Result<Self, GovernanceError> {
        let name = required(name.into(), "cohort name")?;
        let snapshot_fingerprint = required(snapshot_fingerprint.into(), "snapshot fingerprint")?;
        let mut cohort = Self {
            id: Uuid::new_v4(),
            name,
            snapshot_id,
            snapshot_fingerprint,
            split,
            origin,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        cohort.fingerprint = cohort_fingerprint(&cohort)?;
        Ok(cohort)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, GovernanceError> {
        cohort_fingerprint(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortRole {
    Training,
    Development,
    Diagnostic,
    SealedAcceptance,
    ExternalBenchmark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortDisposition {
    Active,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CohortRoleDecision {
    pub id: Uuid,
    pub cohort_id: Uuid,
    pub role: CohortRole,
    pub disposition: CohortDisposition,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CohortRoleDecision {
    pub fn initial(
        cohort: &EvaluationCohort,
        role: CohortRole,
        reason: impl Into<String>,
    ) -> Result<Self, GovernanceError> {
        validate_cohort(cohort)?;
        if cohort.origin == CohortOrigin::ExternalBenchmark && role != CohortRole::ExternalBenchmark
        {
            return Err(GovernanceError::ExternalOriginRole);
        }
        if cohort.origin == CohortOrigin::InternalSnapshot && role == CohortRole::ExternalBenchmark
        {
            return Err(GovernanceError::InternalOriginRole);
        }
        let mut decision = Self {
            id: Uuid::new_v4(),
            cohort_id: cohort.id,
            role,
            disposition: CohortDisposition::Active,
            predecessor_id: None,
            predecessor_fingerprint: None,
            reason: required(reason.into(), "role reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        decision.fingerprint = role_fingerprint(&decision)?;
        Ok(decision)
    }

    pub fn transition(
        previous: &Self,
        role: CohortRole,
        disposition: CohortDisposition,
        reason: impl Into<String>,
    ) -> Result<Self, GovernanceError> {
        validate_role_decision(previous)?;
        if previous.disposition == CohortDisposition::Retired {
            return Err(GovernanceError::RetiredCohort);
        }
        if role == CohortRole::SealedAcceptance && previous.role != CohortRole::SealedAcceptance {
            return Err(GovernanceError::CannotPromoteToSealed);
        }
        if previous.role == CohortRole::ExternalBenchmark && role != previous.role {
            return Err(GovernanceError::ExternalRoleImmutable);
        }
        if role != previous.role && disposition == CohortDisposition::Retired {
            return Err(GovernanceError::RetirementCannotChangeRole);
        }
        if role == previous.role && disposition == previous.disposition {
            return Err(GovernanceError::NoRoleChange);
        }
        let mut decision = Self {
            id: Uuid::new_v4(),
            cohort_id: previous.cohort_id,
            role,
            disposition,
            predecessor_id: Some(previous.id),
            predecessor_fingerprint: Some(previous.fingerprint.clone()),
            reason: required(reason.into(), "role reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        decision.fingerprint = role_fingerprint(&decision)?;
        Ok(decision)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, GovernanceError> {
        role_fingerprint(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposurePurpose {
    Training,
    DevelopmentEvaluation,
    Diagnosis,
    Comparison,
    Acceptance,
    ManualInspection,
    Advisor,
    Optimization,
}

impl ExposurePurpose {
    pub const fn is_adaptive(self) -> bool {
        matches!(
            self,
            Self::Training
                | Self::DevelopmentEvaluation
                | Self::Diagnosis
                | Self::Comparison
                | Self::Advisor
                | Self::Optimization
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisclosureLevel {
    Aggregate,
    Slices,
    Predictions,
    RowContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceExposureRequest {
    pub evaluation_run_id: Option<Uuid>,
    pub workflow_run_id: Option<Uuid>,
    pub workflow_iteration: Option<u32>,
    pub purpose: ExposurePurpose,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceExposure {
    pub id: Uuid,
    pub cohort_id: Uuid,
    pub role_decision_id: Uuid,
    pub role: CohortRole,
    pub evaluation_run_id: Option<Uuid>,
    pub workflow_run_id: Option<Uuid>,
    pub workflow_iteration: Option<u32>,
    pub purpose: ExposurePurpose,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub requires_retirement: bool,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl EvidenceExposure {
    pub fn new(
        cohort: &EvaluationCohort,
        role: &CohortRoleDecision,
        request: EvidenceExposureRequest,
    ) -> Result<Self, GovernanceError> {
        validate_cohort(cohort)?;
        validate_role_decision(role)?;
        if role.cohort_id != cohort.id {
            return Err(GovernanceError::CohortRoleMismatch);
        }
        if role.disposition != CohortDisposition::Active {
            return Err(GovernanceError::RetiredCohort);
        }
        validate_exposure_policy(role.role, &request)?;
        let requires_retirement = role.role == CohortRole::SealedAcceptance
            && (request.disclosure > DisclosureLevel::Aggregate
                || request.purpose == ExposurePurpose::ManualInspection);
        let note = request.note.and_then(|note| {
            let note = note.trim().to_owned();
            (!note.is_empty()).then_some(note)
        });
        let mut exposure = Self {
            id: Uuid::new_v4(),
            cohort_id: cohort.id,
            role_decision_id: role.id,
            role: role.role,
            evaluation_run_id: request.evaluation_run_id,
            workflow_run_id: request.workflow_run_id,
            workflow_iteration: request.workflow_iteration,
            purpose: request.purpose,
            disclosure: request.disclosure,
            adaptation_eligible: request.adaptation_eligible,
            requires_retirement,
            note,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        exposure.fingerprint = exposure_fingerprint(&exposure)?;
        Ok(exposure)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, GovernanceError> {
        exposure_fingerprint(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveRiskLevel {
    None,
    Observed,
    Elevated,
    Compromised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureRiskSummary {
    pub cohort_id: Uuid,
    pub total_exposures: u64,
    pub adaptive_exposures: u64,
    pub sealed_aggregate_acceptance_exposures: u64,
    pub maximum_disclosure: Option<DisclosureLevel>,
    pub retirement_required: bool,
    pub risk: AdaptiveRiskLevel,
}

pub fn summarize_exposure_risk(
    cohort_id: Uuid,
    exposures: &[EvidenceExposure],
) -> Result<ExposureRiskSummary, GovernanceError> {
    if exposures
        .iter()
        .any(|exposure| exposure.cohort_id != cohort_id)
    {
        return Err(GovernanceError::ForeignExposure);
    }
    for exposure in exposures {
        if exposure.reproduce_fingerprint()? != exposure.fingerprint {
            return Err(GovernanceError::FingerprintMismatch);
        }
    }
    let adaptive_exposures = exposures
        .iter()
        .filter(|exposure| exposure.adaptation_eligible || exposure.purpose.is_adaptive())
        .count() as u64;
    let sealed_aggregate_acceptance_exposures = exposures
        .iter()
        .filter(|exposure| {
            exposure.role == CohortRole::SealedAcceptance
                && exposure.purpose == ExposurePurpose::Acceptance
                && exposure.disclosure == DisclosureLevel::Aggregate
        })
        .count() as u64;
    let maximum_disclosure = exposures.iter().map(|exposure| exposure.disclosure).max();
    let retirement_required = exposures
        .iter()
        .any(|exposure| exposure.requires_retirement);
    let risk = if retirement_required || adaptive_exposures > 0 {
        AdaptiveRiskLevel::Compromised
    } else if sealed_aggregate_acceptance_exposures > 1 {
        AdaptiveRiskLevel::Elevated
    } else if exposures.is_empty() {
        AdaptiveRiskLevel::None
    } else {
        AdaptiveRiskLevel::Observed
    };
    Ok(ExposureRiskSummary {
        cohort_id,
        total_exposures: exposures.len() as u64,
        adaptive_exposures,
        sealed_aggregate_acceptance_exposures,
        maximum_disclosure,
        retirement_required,
        risk,
    })
}

pub fn validate_exposure_resolution(
    exposure: &EvidenceExposure,
    current_role: &CohortRoleDecision,
    successor: Option<&CohortRoleDecision>,
) -> Result<(), GovernanceError> {
    if exposure.role_decision_id != current_role.id || exposure.cohort_id != current_role.cohort_id
    {
        return Err(GovernanceError::ExposureRoleMismatch);
    }
    if !exposure.requires_retirement {
        if successor.is_some() {
            return Err(GovernanceError::UnnecessaryExposureResolution);
        }
        return Ok(());
    }
    let successor = successor.ok_or(GovernanceError::ExposureResolutionRequired)?;
    if successor.cohort_id != current_role.cohort_id
        || successor.predecessor_id != Some(current_role.id)
        || successor.predecessor_fingerprint.as_deref() != Some(&current_role.fingerprint)
    {
        return Err(GovernanceError::ExposureResolutionMismatch);
    }
    if successor.disposition != CohortDisposition::Retired
        && successor.role == CohortRole::SealedAcceptance
    {
        return Err(GovernanceError::ExposureResolutionRequired);
    }
    Ok(())
}

fn validate_exposure_policy(
    role: CohortRole,
    request: &EvidenceExposureRequest,
) -> Result<(), GovernanceError> {
    if request.workflow_iteration.is_some() && request.workflow_run_id.is_none() {
        return Err(GovernanceError::IterationWithoutWorkflow);
    }
    if request.adaptation_eligible && !request.purpose.is_adaptive() {
        return Err(GovernanceError::NonAdaptivePurposeMarkedAdaptive);
    }
    if role == CohortRole::SealedAcceptance {
        if request.adaptation_eligible || request.purpose.is_adaptive() {
            return Err(GovernanceError::SealedAdaptiveExposure);
        }
        if !matches!(
            request.purpose,
            ExposurePurpose::Acceptance | ExposurePurpose::ManualInspection
        ) {
            return Err(GovernanceError::SealedPurpose);
        }
    }
    if role == CohortRole::Training && request.purpose != ExposurePurpose::Training {
        return Err(GovernanceError::TrainingPurpose);
    }
    Ok(())
}

fn validate_cohort(cohort: &EvaluationCohort) -> Result<(), GovernanceError> {
    if cohort.reproduce_fingerprint()? != cohort.fingerprint {
        Err(GovernanceError::FingerprintMismatch)
    } else {
        Ok(())
    }
}

fn validate_role_decision(decision: &CohortRoleDecision) -> Result<(), GovernanceError> {
    if decision.reproduce_fingerprint()? != decision.fingerprint {
        Err(GovernanceError::FingerprintMismatch)
    } else {
        Ok(())
    }
}

fn cohort_fingerprint(cohort: &EvaluationCohort) -> Result<String, GovernanceError> {
    #[derive(Serialize)]
    struct Input<'a> {
        id: Uuid,
        name: &'a str,
        snapshot_id: Uuid,
        snapshot_fingerprint: &'a str,
        split: SnapshotSplit,
        origin: CohortOrigin,
        created_at: DateTime<Utc>,
    }
    artifact_core::fingerprint(&Input {
        id: cohort.id,
        name: &cohort.name,
        snapshot_id: cohort.snapshot_id,
        snapshot_fingerprint: &cohort.snapshot_fingerprint,
        split: cohort.split,
        origin: cohort.origin,
        created_at: cohort.created_at,
    })
    .map_err(|error| GovernanceError::Fingerprint(error.to_string()))
}

fn role_fingerprint(decision: &CohortRoleDecision) -> Result<String, GovernanceError> {
    #[derive(Serialize)]
    struct Input<'a> {
        id: Uuid,
        cohort_id: Uuid,
        role: CohortRole,
        disposition: CohortDisposition,
        predecessor_id: Option<Uuid>,
        predecessor_fingerprint: Option<&'a str>,
        reason: &'a str,
        created_at: DateTime<Utc>,
    }
    artifact_core::fingerprint(&Input {
        id: decision.id,
        cohort_id: decision.cohort_id,
        role: decision.role,
        disposition: decision.disposition,
        predecessor_id: decision.predecessor_id,
        predecessor_fingerprint: decision.predecessor_fingerprint.as_deref(),
        reason: &decision.reason,
        created_at: decision.created_at,
    })
    .map_err(|error| GovernanceError::Fingerprint(error.to_string()))
}

fn exposure_fingerprint(exposure: &EvidenceExposure) -> Result<String, GovernanceError> {
    #[derive(Serialize)]
    struct Input<'a> {
        id: Uuid,
        cohort_id: Uuid,
        role_decision_id: Uuid,
        role: CohortRole,
        evaluation_run_id: Option<Uuid>,
        workflow_run_id: Option<Uuid>,
        workflow_iteration: Option<u32>,
        purpose: ExposurePurpose,
        disclosure: DisclosureLevel,
        adaptation_eligible: bool,
        requires_retirement: bool,
        note: Option<&'a str>,
        created_at: DateTime<Utc>,
    }
    artifact_core::fingerprint(&Input {
        id: exposure.id,
        cohort_id: exposure.cohort_id,
        role_decision_id: exposure.role_decision_id,
        role: exposure.role,
        evaluation_run_id: exposure.evaluation_run_id,
        workflow_run_id: exposure.workflow_run_id,
        workflow_iteration: exposure.workflow_iteration,
        purpose: exposure.purpose,
        disclosure: exposure.disclosure,
        adaptation_eligible: exposure.adaptation_eligible,
        requires_retirement: exposure.requires_retirement,
        note: exposure.note.as_deref(),
        created_at: exposure.created_at,
    })
    .map_err(|error| GovernanceError::Fingerprint(error.to_string()))
}

fn required(value: String, field: &'static str) -> Result<String, GovernanceError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(GovernanceError::EmptyField(field))
    } else {
        Ok(value)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GovernanceError {
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    #[error("external benchmark cohorts require the external_benchmark role")]
    ExternalOriginRole,
    #[error("internal snapshot cohorts cannot use the external_benchmark role")]
    InternalOriginRole,
    #[error("retired cohorts cannot transition or receive new exposures")]
    RetiredCohort,
    #[error("an exposed non-sealed cohort cannot be promoted into sealed acceptance")]
    CannotPromoteToSealed,
    #[error("external benchmark role is immutable")]
    ExternalRoleImmutable,
    #[error("retirement preserves the cohort's current role")]
    RetirementCannotChangeRole,
    #[error("role transition does not change role or disposition")]
    NoRoleChange,
    #[error("cohort and role decision identities differ")]
    CohortRoleMismatch,
    #[error("workflow iteration requires a workflow run identity")]
    IterationWithoutWorkflow,
    #[error("a non-adaptive purpose cannot be marked adaptation eligible")]
    NonAdaptivePurposeMarkedAdaptive,
    #[error("sealed acceptance evidence cannot be adaptation eligible")]
    SealedAdaptiveExposure,
    #[error("sealed acceptance permits only acceptance or explicit manual inspection")]
    SealedPurpose,
    #[error("training cohorts permit only training-purpose exposure")]
    TrainingPurpose,
    #[error("exposure list contains another cohort")]
    ForeignExposure,
    #[error("exposure and current role identities differ")]
    ExposureRoleMismatch,
    #[error("row-level sealed disclosure requires retirement or demotion")]
    ExposureResolutionRequired,
    #[error("exposure resolution does not continue the current role decision")]
    ExposureResolutionMismatch,
    #[error("an exposure that preserves sealing must not append a role resolution")]
    UnnecessaryExposureResolution,
    #[error("artifact fingerprint does not reproduce")]
    FingerprintMismatch,
    #[error("artifact fingerprint failed: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use dataset_core::domain::SnapshotSplit;
    use uuid::Uuid;

    use super::{
        AdaptiveRiskLevel, CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision,
        DisclosureLevel, EvaluationCohort, EvidenceExposure, EvidenceExposureRequest,
        ExposurePurpose, GovernanceError, summarize_exposure_risk, validate_exposure_resolution,
    };

    fn cohort(origin: CohortOrigin) -> EvaluationCohort {
        EvaluationCohort::new(
            "acceptance",
            Uuid::new_v4(),
            "sha256:snapshot",
            SnapshotSplit::Test,
            origin,
        )
        .expect("cohort")
    }

    #[test]
    fn sealed_cohort_rejects_every_adaptive_path() {
        let cohort = cohort(CohortOrigin::InternalSnapshot);
        let role = CohortRoleDecision::initial(
            &cohort,
            CohortRole::SealedAcceptance,
            "fresh final holdout",
        )
        .expect("role");
        for purpose in [
            ExposurePurpose::DevelopmentEvaluation,
            ExposurePurpose::Diagnosis,
            ExposurePurpose::Comparison,
            ExposurePurpose::Advisor,
            ExposurePurpose::Optimization,
        ] {
            assert_eq!(
                EvidenceExposure::new(
                    &cohort,
                    &role,
                    EvidenceExposureRequest {
                        evaluation_run_id: None,
                        workflow_run_id: None,
                        workflow_iteration: None,
                        purpose,
                        disclosure: DisclosureLevel::Aggregate,
                        adaptation_eligible: true,
                        note: None,
                    },
                ),
                Err(GovernanceError::SealedAdaptiveExposure)
            );
        }
    }

    #[test]
    fn row_level_sealed_inspection_requires_retirement() {
        let cohort = cohort(CohortOrigin::InternalSnapshot);
        let role = CohortRoleDecision::initial(
            &cohort,
            CohortRole::SealedAcceptance,
            "fresh final holdout",
        )
        .expect("role");
        let exposure = EvidenceExposure::new(
            &cohort,
            &role,
            EvidenceExposureRequest {
                evaluation_run_id: Some(Uuid::new_v4()),
                workflow_run_id: None,
                workflow_iteration: None,
                purpose: ExposurePurpose::ManualInspection,
                disclosure: DisclosureLevel::RowContent,
                adaptation_eligible: false,
                note: Some("inspect release failure".into()),
            },
        )
        .expect("exposure");
        assert!(exposure.requires_retirement);
        let risk =
            summarize_exposure_risk(cohort.id, std::slice::from_ref(&exposure)).expect("risk");
        assert_eq!(risk.risk, AdaptiveRiskLevel::Compromised);
        assert!(risk.retirement_required);
        let retired = CohortRoleDecision::transition(
            &role,
            CohortRole::SealedAcceptance,
            CohortDisposition::Retired,
            "row content was disclosed",
        )
        .expect("retirement");
        validate_exposure_resolution(&exposure, &role, Some(&retired)).expect("resolution");
        assert_eq!(retired.disposition, CohortDisposition::Retired);
    }

    #[test]
    fn repeated_aggregate_acceptance_is_visible_as_elevated_risk() {
        let cohort = cohort(CohortOrigin::InternalSnapshot);
        let role = CohortRoleDecision::initial(
            &cohort,
            CohortRole::SealedAcceptance,
            "fresh final holdout",
        )
        .expect("role");
        let request = || EvidenceExposureRequest {
            evaluation_run_id: Some(Uuid::new_v4()),
            workflow_run_id: None,
            workflow_iteration: None,
            purpose: ExposurePurpose::Acceptance,
            disclosure: DisclosureLevel::Aggregate,
            adaptation_eligible: false,
            note: None,
        };
        let exposures = vec![
            EvidenceExposure::new(&cohort, &role, request()).expect("first"),
            EvidenceExposure::new(&cohort, &role, request()).expect("second"),
        ];
        assert_eq!(
            summarize_exposure_risk(cohort.id, &exposures)
                .expect("risk")
                .risk,
            AdaptiveRiskLevel::Elevated
        );
    }

    #[test]
    fn development_cannot_be_rebranded_as_sealed() {
        let cohort = cohort(CohortOrigin::InternalSnapshot);
        let role =
            CohortRoleDecision::initial(&cohort, CohortRole::Development, "iteration evidence")
                .expect("role");
        assert_eq!(
            CohortRoleDecision::transition(
                &role,
                CohortRole::SealedAcceptance,
                CohortDisposition::Active,
                "try to reuse it",
            ),
            Err(GovernanceError::CannotPromoteToSealed)
        );
    }
}
