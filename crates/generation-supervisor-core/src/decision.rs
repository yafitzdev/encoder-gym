//! Pure deterministic policy for immediate weakness and longitudinal drift.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use dataset_quality_core::policy::BasisPoints;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    SupervisorError,
    contract::GenerationQualityContract,
    fingerprint,
    observation::{BatchQualityObservation, QualityScope, QualityWindowKind, RowCriterionFailure},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorDecisionState {
    Healthy,
    InsufficientEvidence,
    PauseForDiagnosis,
    AwaitingReview,
    RevisionCanaryRequired,
    RevisionPassed,
    RevisionFailed,
    Escalate,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityFailureKind {
    ImmediateWeakness,
    LongitudinalDrift,
    RevisionCanaryFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorIssueCode {
    InsufficientAssessedRows,
    WeakQualifiedRate,
    ExcessBorderlineRate,
    ExcessQuarantinedRate,
    ExcessInvalidRate,
    NormalizedDuplication,
    TemplateModeCollapse,
    ShortcutConcentration,
    MissingRequiredPattern,
    AssignedLabelWeakness,
    LabelAmbiguity,
    DimensionWeakness,
    DifficultyWeakness,
    AuthenticityWeakness,
    StrategyWeakness,
    LabelLeakage,
    ShortcutRisk,
    EvaluatorConfidence,
    EvaluatorQuarantine,
    QualifiedRateDrift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdComparison {
    AtLeast,
    AtMost,
    MaximumDrop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdObservation {
    pub issue: SupervisorIssueCode,
    pub metric: String,
    pub observed_basis_points: u16,
    pub required_basis_points: u16,
    pub comparison: ThresholdComparison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermittedAction {
    CollectAuthorizedEvidence,
    ContinueNextSegment,
    DiagnoseWithPi,
    EscalateWithoutRevision,
    ReviewRevision,
    RunRevisionCanary,
    ActivateRevision,
    RejectRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeterministicQualityDecision {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_fingerprint: String,
    pub supervisor_run_id: Uuid,
    pub window_id: Uuid,
    pub window_fingerprint: String,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    pub scope: QualityScope,
    pub state: SupervisorDecisionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<QualityFailureKind>,
    pub issues: BTreeSet<SupervisorIssueCode>,
    pub threshold_observations: Vec<ThresholdObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_window_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_window_fingerprint: Option<String>,
    pub permitted_actions: BTreeSet<PermittedAction>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DeterministicQualityDecision {
    pub fn evaluate(
        id: Uuid,
        contract: &GenerationQualityContract,
        window: &BatchQualityObservation,
        baseline: Option<&BatchQualityObservation>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        contract.validate()?;
        if id.is_nil()
            || window.contract_id != contract.id
            || window.contract_fingerprint != contract.fingerprint
            || window.fingerprint.is_empty()
            || window.reproduce_fingerprint()? != window.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "decision window is not bound to the exact contract".into(),
            ));
        }
        if let Some(baseline) = baseline {
            if baseline.contract_id != contract.id
                || baseline.supervisor_run_id != window.supervisor_run_id
                || baseline.scope != window.scope
                || baseline.id == window.id
                || baseline.created_at > window.created_at
                || baseline.fingerprint.is_empty()
                || baseline.reproduce_fingerprint()? != baseline.fingerprint
            {
                return Err(SupervisorError::Integrity(
                    "baseline is not an earlier valid window in the same scope".into(),
                ));
            }
        }

        let mut issues = BTreeSet::new();
        let mut threshold_observations = Vec::new();
        if window.counts.assessed < contract.monitoring.minimum_evidence_rows_per_scope {
            issues.insert(SupervisorIssueCode::InsufficientAssessedRows);
            let mut value = Self {
                id,
                contract_id: contract.id,
                contract_fingerprint: contract.fingerprint.clone(),
                supervisor_run_id: window.supervisor_run_id,
                window_id: window.id,
                window_fingerprint: window.fingerprint.clone(),
                prompt_version_id: window.prompt_version_id,
                prompt_version_fingerprint: window.prompt_version_fingerprint.clone(),
                scope: window.scope.clone(),
                state: SupervisorDecisionState::InsufficientEvidence,
                failure_kind: None,
                issues,
                threshold_observations,
                baseline_window_id: baseline.map(|value| value.id),
                baseline_window_fingerprint: baseline.map(|value| value.fingerprint.clone()),
                permitted_actions: BTreeSet::from([
                    PermittedAction::CollectAuthorizedEvidence,
                    PermittedAction::EscalateWithoutRevision,
                ]),
                created_at,
                fingerprint: String::new(),
            };
            value.fingerprint = value.reproduce_fingerprint()?;
            return Ok(value);
        }

        let batch = &contract.batch_thresholds;
        compare_minimum(
            &mut issues,
            &mut threshold_observations,
            SupervisorIssueCode::WeakQualifiedRate,
            "qualified_rate",
            window.rates.qualified,
            batch.minimum_qualified_rate,
        );
        for (issue, metric, observed, maximum) in [
            (
                SupervisorIssueCode::ExcessBorderlineRate,
                "borderline_rate",
                window.rates.borderline,
                batch.maximum_borderline_rate,
            ),
            (
                SupervisorIssueCode::ExcessQuarantinedRate,
                "quarantined_rate",
                window.rates.quarantined,
                batch.maximum_quarantined_rate,
            ),
            (
                SupervisorIssueCode::ExcessInvalidRate,
                "invalid_rate",
                window.rates.invalid,
                batch.maximum_invalid_rate,
            ),
            (
                SupervisorIssueCode::NormalizedDuplication,
                "normalized_duplicate_rate",
                window.rates.normalized_duplicates,
                batch.maximum_normalized_duplicate_rate,
            ),
            (
                SupervisorIssueCode::TemplateModeCollapse,
                "template_repetition_rate",
                window.rates.template_repetitions,
                batch.maximum_template_repetition_rate,
            ),
            (
                SupervisorIssueCode::ShortcutConcentration,
                "shortcut_concentration",
                window.rates.shortcut_concentration,
                batch.maximum_shortcut_concentration,
            ),
        ] {
            compare_maximum(
                &mut issues,
                &mut threshold_observations,
                issue,
                metric,
                observed,
                maximum,
            );
        }
        if !window.missing_required_patterns.is_empty() {
            issues.insert(SupervisorIssueCode::MissingRequiredPattern);
        }
        if window.rates.qualified < batch.minimum_qualified_rate {
            for criterion in window.criterion_failures.keys() {
                issues.insert(issue_for_criterion(*criterion));
            }
        }

        if let Some(baseline) = baseline {
            let drop = baseline
                .rates
                .qualified
                .get()
                .saturating_sub(window.rates.qualified.get());
            if drop > batch.maximum_qualified_rate_drop.get() {
                issues.insert(SupervisorIssueCode::QualifiedRateDrift);
                threshold_observations.push(ThresholdObservation {
                    issue: SupervisorIssueCode::QualifiedRateDrift,
                    metric: "qualified_rate_drop".into(),
                    observed_basis_points: drop,
                    required_basis_points: batch.maximum_qualified_rate_drop.get(),
                    comparison: ThresholdComparison::MaximumDrop,
                });
            }
        }

        let has_failures = !issues.is_empty();
        let (state, failure_kind, permitted_actions) = if has_failures {
            if window.kind == QualityWindowKind::RevisionCanary {
                (
                    SupervisorDecisionState::RevisionFailed,
                    Some(QualityFailureKind::RevisionCanaryFailure),
                    BTreeSet::from([
                        PermittedAction::RejectRevision,
                        PermittedAction::EscalateWithoutRevision,
                    ]),
                )
            } else {
                (
                    SupervisorDecisionState::PauseForDiagnosis,
                    Some(
                        if baseline.is_some() && window.kind == QualityWindowKind::Rolling {
                            QualityFailureKind::LongitudinalDrift
                        } else {
                            QualityFailureKind::ImmediateWeakness
                        },
                    ),
                    BTreeSet::from([
                        PermittedAction::DiagnoseWithPi,
                        PermittedAction::EscalateWithoutRevision,
                    ]),
                )
            }
        } else if window.kind == QualityWindowKind::RevisionCanary {
            (
                SupervisorDecisionState::RevisionPassed,
                None,
                BTreeSet::from([PermittedAction::ActivateRevision]),
            )
        } else {
            (
                SupervisorDecisionState::Healthy,
                None,
                BTreeSet::from([PermittedAction::ContinueNextSegment]),
            )
        };
        let mut value = Self {
            id,
            contract_id: contract.id,
            contract_fingerprint: contract.fingerprint.clone(),
            supervisor_run_id: window.supervisor_run_id,
            window_id: window.id,
            window_fingerprint: window.fingerprint.clone(),
            prompt_version_id: window.prompt_version_id,
            prompt_version_fingerprint: window.prompt_version_fingerprint.clone(),
            scope: window.scope.clone(),
            state,
            failure_kind,
            issues,
            threshold_observations,
            baseline_window_id: baseline.map(|value| value.id),
            baseline_window_fingerprint: baseline.map(|value| value.fingerprint.clone()),
            permitted_actions,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self, contract: &GenerationQualityContract) -> Result<(), SupervisorError> {
        if self.contract_id != contract.id
            || self.contract_fingerprint != contract.fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "deterministic decision does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemicPauseDecision {
    pub contract_id: Uuid,
    pub supervisor_run_id: Uuid,
    pub failing_scope_decision_ids: Vec<Uuid>,
    pub global_pause: bool,
    pub fingerprint: String,
}

impl SystemicPauseDecision {
    pub fn evaluate(
        contract: &GenerationQualityContract,
        run_id: Uuid,
        decisions: &[DeterministicQualityDecision],
    ) -> Result<Self, SupervisorError> {
        if decisions.iter().any(|decision| {
            decision.contract_id != contract.id || decision.supervisor_run_id != run_id
        }) {
            return Err(SupervisorError::Integrity(
                "systemic decision received another contract or run".into(),
            ));
        }
        let mut failing_scope_decision_ids = decisions
            .iter()
            .filter(|decision| decision.state == SupervisorDecisionState::PauseForDiagnosis)
            .map(|decision| decision.id)
            .collect::<Vec<_>>();
        failing_scope_decision_ids.sort();
        let minimum = contract.monitoring.systemic_pause_minimum_scopes;
        let global_pause = minimum > 0
            && u32::try_from(failing_scope_decision_ids.len()).unwrap_or(u32::MAX) >= minimum;
        let mut value = Self {
            contract_id: contract.id,
            supervisor_run_id: run_id,
            failing_scope_decision_ids,
            global_pause,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn compare_minimum(
    issues: &mut BTreeSet<SupervisorIssueCode>,
    observations: &mut Vec<ThresholdObservation>,
    issue: SupervisorIssueCode,
    metric: &str,
    observed: BasisPoints,
    minimum: BasisPoints,
) {
    if observed < minimum {
        issues.insert(issue);
        observations.push(ThresholdObservation {
            issue,
            metric: metric.into(),
            observed_basis_points: observed.get(),
            required_basis_points: minimum.get(),
            comparison: ThresholdComparison::AtLeast,
        });
    }
}

fn compare_maximum(
    issues: &mut BTreeSet<SupervisorIssueCode>,
    observations: &mut Vec<ThresholdObservation>,
    issue: SupervisorIssueCode,
    metric: &str,
    observed: BasisPoints,
    maximum: BasisPoints,
) {
    if observed > maximum {
        issues.insert(issue);
        observations.push(ThresholdObservation {
            issue,
            metric: metric.into(),
            observed_basis_points: observed.get(),
            required_basis_points: maximum.get(),
            comparison: ThresholdComparison::AtMost,
        });
    }
}

fn issue_for_criterion(criterion: RowCriterionFailure) -> SupervisorIssueCode {
    match criterion {
        RowCriterionFailure::AssignedLabel => SupervisorIssueCode::AssignedLabelWeakness,
        RowCriterionFailure::LabelMargin => SupervisorIssueCode::LabelAmbiguity,
        RowCriterionFailure::DimensionAdherence => SupervisorIssueCode::DimensionWeakness,
        RowCriterionFailure::DifficultyAdherence => SupervisorIssueCode::DifficultyWeakness,
        RowCriterionFailure::AuthenticityAdherence => SupervisorIssueCode::AuthenticityWeakness,
        RowCriterionFailure::StrategyAdherence => SupervisorIssueCode::StrategyWeakness,
        RowCriterionFailure::LabelLeakage => SupervisorIssueCode::LabelLeakage,
        RowCriterionFailure::ShortcutRisk => SupervisorIssueCode::ShortcutRisk,
        RowCriterionFailure::EvaluatorConfidence => SupervisorIssueCode::EvaluatorConfidence,
        RowCriterionFailure::ProviderQuarantine => SupervisorIssueCode::EvaluatorQuarantine,
    }
}
