use sqlx::FromRow;
use uuid::Uuid;
use workflow_core::{
    ports::{BoxFuture, StopDecisionStore, WorkflowStoreError},
    stop::StopDecision,
};

use crate::SqliteStore;

impl StopDecisionStore for SqliteStore {
    fn create_stop_decision(
        &self,
        decision: &StopDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let decision = decision.clone();
        Box::pin(async move {
            validate(&decision)?;
            sqlx::query(
                "INSERT INTO workflow_stop_decisions \
                 (id, workflow_run_id, workflow_iteration, acceptance_assessment_id, \
                  should_continue, reason, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(decision.id)
            .bind(decision.workflow_run_id)
            .bind(decision.workflow_iteration)
            .bind(decision.acceptance_assessment_id)
            .bind(decision.should_continue)
            .bind(format!("{:?}", decision.reason).to_ascii_lowercase())
            .bind(&decision.fingerprint)
            .bind(serde_json::to_string(&decision).map_err(store_error)?)
            .bind(decision.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_stop_decision(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<StopDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, StopRow>(
                    "SELECT artifact_json FROM workflow_stop_decisions WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
        })
    }

    fn get_iteration_stop_decision(
        &self,
        workflow_run_id: Uuid,
        workflow_iteration: u32,
    ) -> BoxFuture<'_, Result<Option<StopDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, StopRow>(
                    "SELECT artifact_json FROM workflow_stop_decisions \
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
struct StopRow {
    artifact_json: String,
}

fn load_optional(row: Option<StopRow>) -> Result<Option<StopDecision>, WorkflowStoreError> {
    row.map(|row| {
        let value: StopDecision = serde_json::from_str(&row.artifact_json).map_err(store_error)?;
        validate(&value)?;
        Ok(value)
    })
    .transpose()
}

fn validate(value: &StopDecision) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "workflow stop decision fingerprint does not reproduce".into(),
        ));
    }
    Ok(())
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
