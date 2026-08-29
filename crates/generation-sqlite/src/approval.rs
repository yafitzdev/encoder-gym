use sqlx::FromRow;
use uuid::Uuid;
use workflow_core::{
    approval::{WorkflowApprovalDecision, WorkflowApprovalMode},
    ports::{BoxFuture, WorkflowApprovalStore, WorkflowStoreError},
};

use crate::SqliteStore;

impl WorkflowApprovalStore for SqliteStore {
    fn create_workflow_approval(
        &self,
        decision: &WorkflowApprovalDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let decision = decision.clone();
        Box::pin(async move {
            validate(&decision)?;
            sqlx::query(
                "INSERT INTO workflow_approval_decisions \
                 (id, workflow_run_id, workflow_iteration, proposal_id, proposal_review_id, \
                  mode, approved_additional_rows, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(decision.id)
            .bind(decision.workflow_run_id)
            .bind(decision.workflow_iteration)
            .bind(decision.proposal_id)
            .bind(decision.proposal_review_id)
            .bind(mode(decision.mode))
            .bind(i64::try_from(decision.approved_additional_rows).map_err(store_error)?)
            .bind(&decision.fingerprint)
            .bind(serde_json::to_string(&decision).map_err(store_error)?)
            .bind(decision.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_workflow_approval(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowApprovalDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, ApprovalRow>(
                    "SELECT artifact_json FROM workflow_approval_decisions WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
        })
    }

    fn get_iteration_workflow_approval(
        &self,
        workflow_run_id: Uuid,
        workflow_iteration: u32,
    ) -> BoxFuture<'_, Result<Option<WorkflowApprovalDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, ApprovalRow>(
                    "SELECT artifact_json FROM workflow_approval_decisions \
                     WHERE workflow_run_id = ? AND workflow_iteration = ?",
                )
                .bind(workflow_run_id)
                .bind(workflow_iteration)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
        })
    }
}

#[derive(Debug, FromRow)]
struct ApprovalRow {
    artifact_json: String,
}

fn load_optional(
    row: Option<ApprovalRow>,
) -> Result<Option<WorkflowApprovalDecision>, WorkflowStoreError> {
    row.map(|row| {
        let value: WorkflowApprovalDecision =
            serde_json::from_str(&row.artifact_json).map_err(store_error)?;
        validate(&value)?;
        Ok(value)
    })
    .transpose()
}

fn validate(value: &WorkflowApprovalDecision) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "workflow approval fingerprint does not reproduce".into(),
        ));
    }
    Ok(())
}

const fn mode(value: WorkflowApprovalMode) -> &'static str {
    match value {
        WorkflowApprovalMode::HumanReview => "human_review",
        WorkflowApprovalMode::PreauthorizedEnvelope => "preauthorized_envelope",
    }
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
