//! Immutable workflow approval decisions and bounded envelope consumption.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::workflow::{ApprovalEnvelope, WorkflowBudgetUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowApprovalMode {
    HumanReview,
    PreauthorizedEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowApprovalDecision {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub workflow_iteration: u32,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub proposal_review_id: Uuid,
    pub proposal_review_fingerprint: String,
    pub mode: WorkflowApprovalMode,
    pub approved_additional_rows: u64,
    pub usage_before: WorkflowBudgetUsage,
    pub envelope: Option<ApprovalEnvelope>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl WorkflowApprovalDecision {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workflow_run_id: Uuid,
        workflow_iteration: u32,
        proposal_id: Uuid,
        proposal_fingerprint: String,
        proposal_review_id: Uuid,
        proposal_review_fingerprint: String,
        mode: WorkflowApprovalMode,
        approved_additional_rows: u64,
        usage_before: WorkflowBudgetUsage,
        envelope: Option<ApprovalEnvelope>,
        note: Option<String>,
    ) -> Result<Self, ApprovalError> {
        if proposal_fingerprint.trim().is_empty()
            || proposal_review_fingerprint.trim().is_empty()
            || approved_additional_rows == 0
            || (mode == WorkflowApprovalMode::PreauthorizedEnvelope) != envelope.is_some()
        {
            return Err(ApprovalError::InvalidDecision);
        }
        if let Some(envelope) = &envelope {
            if workflow_iteration >= envelope.maximum_iterations
                || approved_additional_rows > envelope.maximum_additional_rows
            {
                return Err(ApprovalError::EnvelopeExceeded);
            }
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            workflow_run_id,
            workflow_iteration,
            proposal_id,
            proposal_fingerprint,
            proposal_review_id,
            proposal_review_fingerprint,
            mode,
            approved_additional_rows,
            usage_before,
            envelope,
            note: note
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ApprovalError> {
        artifact_core::fingerprint(&serde_json::json!({
            "workflow_run_id": self.workflow_run_id,
            "workflow_iteration": self.workflow_iteration,
            "proposal_id": self.proposal_id,
            "proposal_fingerprint": self.proposal_fingerprint,
            "proposal_review_id": self.proposal_review_id,
            "proposal_review_fingerprint": self.proposal_review_fingerprint,
            "mode": self.mode,
            "approved_additional_rows": self.approved_additional_rows,
            "usage_before": self.usage_before,
            "envelope": self.envelope,
            "note": self.note,
        }))
        .map_err(|error| ApprovalError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    #[error("workflow approval decision is invalid")]
    InvalidDecision,
    #[error("workflow approval exceeds the persisted preauthorization envelope")]
    EnvelopeExceeded,
    #[error("workflow approval fingerprint failed: {0}")]
    Fingerprint(String),
}
