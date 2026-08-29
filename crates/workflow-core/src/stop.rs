//! Deterministic bounded stop/continue decisions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    benchmark::{AcceptanceAssessment, AcceptanceState},
    workflow::{WorkflowBudget, WorkflowBudgetUsage, WorkflowPolicy},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    DevelopmentAcceptanceSatisfied,
    StatisticallySupportedImprovement,
    Regression,
    InconclusiveEvidence,
    InvalidEvidence,
    MinimumImprovementNotMet,
    IterationBudgetReached,
    RowBudgetReached,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopDecision {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub workflow_iteration: u32,
    pub acceptance_assessment_id: Uuid,
    pub acceptance_assessment_fingerprint: String,
    pub comparison_ids: Vec<Uuid>,
    pub comparison_fingerprints: Vec<String>,
    pub accuracy_delta: Option<f64>,
    pub macro_f1_delta: Option<f64>,
    pub should_continue: bool,
    pub reason: StopReason,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[allow(clippy::too_many_arguments)]
pub fn decide(
    workflow_run_id: Uuid,
    workflow_iteration: u32,
    acceptance: &AcceptanceAssessment,
    comparisons: &[evaluation_core::domain::EvaluationComparisonReport],
    usage: &WorkflowBudgetUsage,
    budget: &WorkflowBudget,
    policy: &WorkflowPolicy,
) -> Result<StopDecision, StopDecisionError> {
    if comparisons.is_empty()
        || comparisons
            .iter()
            .any(|value| !value.accuracy_delta.is_finite() || !value.macro_f1_delta.is_finite())
    {
        return Err(StopDecisionError::InvalidComparison);
    }
    let accuracy_delta = comparisons
        .iter()
        .map(|value| value.accuracy_delta)
        .reduce(f64::min);
    let macro_f1_delta = comparisons
        .iter()
        .map(|value| value.macro_f1_delta)
        .reduce(f64::min);
    let (should_continue, reason) = match acceptance.state {
        AcceptanceState::Pass => (false, StopReason::DevelopmentAcceptanceSatisfied),
        AcceptanceState::Invalid if policy.stop_on_invalid => (false, StopReason::InvalidEvidence),
        AcceptanceState::Inconclusive if policy.stop_on_inconclusive => {
            (false, StopReason::InconclusiveEvidence)
        }
        _ if workflow_iteration >= budget.maximum_iterations => {
            (false, StopReason::IterationBudgetReached)
        }
        _ if usage.accepted_rows >= budget.maximum_cumulative_rows => {
            (false, StopReason::RowBudgetReached)
        }
        _ if accuracy_delta.is_some_and(|delta| delta < -policy.maximum_tolerated_regression)
            || macro_f1_delta.is_some_and(|delta| delta < -policy.maximum_tolerated_regression) =>
        {
            (false, StopReason::Regression)
        }
        _ if comparisons.iter().all(|comparison| {
            comparison.accuracy_delta_interval.lower >= policy.minimum_improvement
                || comparison.macro_f1_delta_interval.lower >= policy.minimum_improvement
                || (comparison.mcnemar.significant
                    && (comparison.accuracy_delta >= policy.minimum_improvement
                        || comparison.macro_f1_delta >= policy.minimum_improvement))
        }) =>
        {
            (true, StopReason::StatisticallySupportedImprovement)
        }
        _ => (false, StopReason::MinimumImprovementNotMet),
    };
    let mut value = StopDecision {
        id: Uuid::new_v4(),
        workflow_run_id,
        workflow_iteration,
        acceptance_assessment_id: acceptance.id,
        acceptance_assessment_fingerprint: acceptance.fingerprint.clone(),
        comparison_ids: comparisons.iter().map(|value| value.id).collect(),
        comparison_fingerprints: comparisons
            .iter()
            .map(|value| value.fingerprint.clone())
            .collect(),
        accuracy_delta,
        macro_f1_delta,
        should_continue,
        reason,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    value.fingerprint = value.reproduce_fingerprint()?;
    Ok(value)
}

impl StopDecision {
    pub fn reproduce_fingerprint(&self) -> Result<String, StopDecisionError> {
        artifact_core::fingerprint(&serde_json::json!({
            "workflow_run_id": self.workflow_run_id,
            "workflow_iteration": self.workflow_iteration,
            "acceptance_assessment_id": self.acceptance_assessment_id,
            "acceptance_assessment_fingerprint": self.acceptance_assessment_fingerprint,
            "comparison_ids": self.comparison_ids,
            "comparison_fingerprints": self.comparison_fingerprints,
            "accuracy_delta": self.accuracy_delta,
            "macro_f1_delta": self.macro_f1_delta,
            "should_continue": self.should_continue,
            "reason": self.reason,
        }))
        .map_err(|error| StopDecisionError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StopDecisionError {
    #[error("stop decision requires compatible finite comparison evidence")]
    InvalidComparison,
    #[error("stop decision fingerprint failed: {0}")]
    Fingerprint(String),
}
