use artifact_core::fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::workflow::{StageAttemptState, WorkflowStage, WorkflowStageAttempt};

/// Identifies a long-running slice execution controlled by one workflow stage.
///
/// These links are control-plane facts, distinct from the immutable output
/// artifacts attached when a stage finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowChildKind {
    GenerationJob,
    GenerationSupervisorRun,
    QualityAuditRun,
    TrainingRun,
    EvaluationRun,
}

impl WorkflowChildKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GenerationJob => "generation_job",
            Self::GenerationSupervisorRun => "generation_supervisor_run",
            Self::QualityAuditRun => "quality_audit_run",
            Self::TrainingRun => "training_run",
            Self::EvaluationRun => "evaluation_run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowChildExecution {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub workflow_stage_attempt_id: Uuid,
    pub stage: WorkflowStage,
    pub ordinal: u32,
    pub child_kind: WorkflowChildKind,
    pub logical_key: String,
    pub child_execution_id: Uuid,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl WorkflowChildExecution {
    pub fn new(
        attempt: &WorkflowStageAttempt,
        ordinal: u32,
        child_kind: WorkflowChildKind,
        logical_key: impl Into<String>,
        child_execution_id: Uuid,
    ) -> Result<Self, WorkflowExecutionError> {
        let logical_key = logical_key.into();
        let mut value = Self {
            id: Uuid::new_v4(),
            workflow_run_id: attempt.workflow_run_id,
            workflow_stage_attempt_id: attempt.id,
            stage: attempt.stage,
            ordinal,
            child_kind,
            logical_key,
            child_execution_id,
            fingerprint: String::new(),
            created_at: Utc::now(),
        };
        value.validate_shape()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_for_attempt(
        &self,
        attempt: &WorkflowStageAttempt,
    ) -> Result<(), WorkflowExecutionError> {
        self.validate_integrity()?;
        if attempt.state != StageAttemptState::Running
            || self.workflow_run_id != attempt.workflow_run_id
            || self.workflow_stage_attempt_id != attempt.id
            || self.stage != attempt.stage
        {
            return Err(WorkflowExecutionError::AttemptMismatch);
        }
        Ok(())
    }

    pub fn validate_integrity(&self) -> Result<(), WorkflowExecutionError> {
        self.validate_shape()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(WorkflowExecutionError::FingerprintMismatch);
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, WorkflowExecutionError> {
        #[derive(Serialize)]
        struct FingerprintPayload<'a> {
            workflow_run_id: Uuid,
            workflow_stage_attempt_id: Uuid,
            stage: WorkflowStage,
            ordinal: u32,
            child_kind: WorkflowChildKind,
            logical_key: &'a str,
            child_execution_id: Uuid,
        }

        fingerprint(&FingerprintPayload {
            workflow_run_id: self.workflow_run_id,
            workflow_stage_attempt_id: self.workflow_stage_attempt_id,
            stage: self.stage,
            ordinal: self.ordinal,
            child_kind: self.child_kind,
            logical_key: &self.logical_key,
            child_execution_id: self.child_execution_id,
        })
        .map_err(|error| WorkflowExecutionError::Serialization(error.to_string()))
    }

    fn validate_shape(&self) -> Result<(), WorkflowExecutionError> {
        if self.id.is_nil()
            || self.workflow_run_id.is_nil()
            || self.workflow_stage_attempt_id.is_nil()
            || self.child_execution_id.is_nil()
            || self.ordinal == 0
        {
            return Err(WorkflowExecutionError::InvalidIdentity);
        }
        if self.logical_key.is_empty()
            || self.logical_key.trim() != self.logical_key
            || self.logical_key.len() > 256
        {
            return Err(WorkflowExecutionError::InvalidLogicalKey);
        }
        if !kind_is_allowed(self.stage, self.child_kind) {
            return Err(WorkflowExecutionError::KindNotAllowed {
                stage: self.stage,
                kind: self.child_kind,
            });
        }
        Ok(())
    }
}

const fn kind_is_allowed(stage: WorkflowStage, kind: WorkflowChildKind) -> bool {
    matches!(
        (stage, kind),
        (
            WorkflowStage::Generation | WorkflowStage::DatasetDiffGeneration,
            WorkflowChildKind::GenerationJob
        ) | (
            WorkflowStage::Generation | WorkflowStage::DatasetDiffGeneration,
            WorkflowChildKind::GenerationSupervisorRun
        ) | (
            WorkflowStage::QualityAudit | WorkflowStage::IterationQualityAudit,
            WorkflowChildKind::QualityAuditRun
        ) | (
            WorkflowStage::Training | WorkflowStage::IterationTraining,
            WorkflowChildKind::TrainingRun
        ) | (
            WorkflowStage::DevelopmentEvaluation
                | WorkflowStage::IterationEvaluation
                | WorkflowStage::SealedEvaluation,
            WorkflowChildKind::EvaluationRun
        )
    )
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkflowExecutionError {
    #[error("workflow child execution identity or ordinal is invalid")]
    InvalidIdentity,
    #[error("workflow child execution logical key is invalid")]
    InvalidLogicalKey,
    #[error("{kind:?} is not a permitted child of workflow stage {stage:?}")]
    KindNotAllowed {
        stage: WorkflowStage,
        kind: WorkflowChildKind,
    },
    #[error("workflow child execution does not match its running stage attempt")]
    AttemptMismatch,
    #[error("workflow child execution fingerprint mismatch")]
    FingerprintMismatch,
    #[error("workflow child execution serialization failed: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{WorkflowBudgetUsage, WorkflowStageAttempt};

    fn running_attempt(stage: WorkflowStage) -> WorkflowStageAttempt {
        WorkflowStageAttempt {
            id: Uuid::new_v4(),
            workflow_run_id: Uuid::new_v4(),
            sequence: 0,
            iteration: 0,
            stage,
            attempt: 1,
            state: StageAttemptState::Running,
            predecessor_id: None,
            predecessor_fingerprint: None,
            reason: None,
            retryable: false,
            artifacts: Vec::new(),
            usage_after: WorkflowBudgetUsage::zero(),
            started_at: Utc::now(),
            finished_at: None,
            fingerprint: "running-attempt".into(),
        }
    }

    #[test]
    fn child_kind_is_scoped_to_the_owning_stage() {
        let attempt = running_attempt(WorkflowStage::Training);
        let link = WorkflowChildExecution::new(
            &attempt,
            1,
            WorkflowChildKind::TrainingRun,
            "primary",
            Uuid::new_v4(),
        )
        .expect("training link");
        assert_eq!(link.reproduce_fingerprint().unwrap(), link.fingerprint);
        assert!(matches!(
            WorkflowChildExecution::new(
                &attempt,
                1,
                WorkflowChildKind::GenerationJob,
                "primary",
                Uuid::new_v4(),
            ),
            Err(WorkflowExecutionError::KindNotAllowed { .. })
        ));
    }

    #[test]
    fn supervisor_run_is_an_exact_generation_stage_child() {
        let attempt = running_attempt(WorkflowStage::Generation);
        let link = WorkflowChildExecution::new(
            &attempt,
            1,
            WorkflowChildKind::GenerationSupervisorRun,
            "supervision",
            Uuid::new_v4(),
        )
        .expect("supervisor child");
        link.validate_for_attempt(&attempt).expect("valid link");

        let training = running_attempt(WorkflowStage::Training);
        assert!(matches!(
            WorkflowChildExecution::new(
                &training,
                1,
                WorkflowChildKind::GenerationSupervisorRun,
                "supervision",
                Uuid::new_v4(),
            ),
            Err(WorkflowExecutionError::KindNotAllowed { .. })
        ));
    }
}
