//! Cross-workflow interruption records and the persistence contract for local recovery.

use std::{future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    Generation,
    Training,
    Evaluation,
    EncoderWorkflow,
}

impl WorkflowKind {
    pub const ALL: [Self; 4] = [
        Self::Generation,
        Self::Training,
        Self::Evaluation,
        Self::EncoderWorkflow,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generation => "generation",
            Self::Training => "training",
            Self::Evaluation => "evaluation",
            Self::EncoderWorkflow => "encoder_workflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Pending,
    Resumed,
    Restarted,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub workflow_kind: WorkflowKind,
    pub workflow_id: Uuid,
    pub state: RecoveryState,
    pub resumable_in_place: bool,
    pub message: String,
    pub detected_at: DateTime<Utc>,
    pub replacement_id: Option<Uuid>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("workflow recovery persistence failed: {0}")]
pub struct RecoveryStoreError(pub String);

pub trait RecoveryStore: Send + Sync {
    /// Records the current process as the owner before a runner is launched.
    fn acquire_execution_lease(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>>;

    fn release_execution_lease(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>>;

    /// Marks running workflows whose owning process no longer exists as interrupted.
    fn detect_interrupted_workflows(
        &self,
    ) -> BoxFuture<'_, Result<Vec<RecoveryRecord>, RecoveryStoreError>>;

    fn list_recovery_records(
        &self,
        include_resolved: bool,
    ) -> BoxFuture<'_, Result<Vec<RecoveryRecord>, RecoveryStoreError>>;

    /// Resets a pending interrupted generation job to queued in one transaction.
    fn prepare_generation_resume(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>>;

    fn resolve_recovery(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
        state: RecoveryState,
        replacement_id: Option<Uuid>,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>>;
}
