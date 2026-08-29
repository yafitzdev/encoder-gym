use sqlx::FromRow;
use uuid::Uuid;
use workflow_core::{
    ports::{BoxFuture, PromotionStore, WorkflowStoreError},
    promotion::{ModelPromotion, PromotionState},
};

use crate::SqliteStore;

impl PromotionStore for SqliteStore {
    fn create_promotion(
        &self,
        promotion: &ModelPromotion,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let promotion = promotion.clone();
        Box::pin(async move {
            validate(&promotion)?;
            sqlx::query(
                "INSERT INTO model_promotions \
                 (id, workflow_run_id, checkpoint_id, training_snapshot_id, \
                  development_assessment_id, sealed_assessment_id, state, fingerprint, \
                  artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(promotion.id)
            .bind(promotion.workflow_run_id)
            .bind(promotion.checkpoint_id)
            .bind(promotion.training_snapshot_id)
            .bind(promotion.development_assessment_id)
            .bind(promotion.sealed_assessment_id)
            .bind(state(promotion.state))
            .bind(&promotion.fingerprint)
            .bind(serde_json::to_string(&promotion).map_err(store_error)?)
            .bind(promotion.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_promotion(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelPromotion>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, PromotionRow>(
                    "SELECT artifact_json FROM model_promotions WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
        })
    }

    fn get_workflow_promotion(
        &self,
        workflow_run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelPromotion>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                sqlx::query_as::<_, PromotionRow>(
                    "SELECT artifact_json FROM model_promotions WHERE workflow_run_id = ?",
                )
                .bind(workflow_run_id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
        })
    }
}

#[derive(Debug, FromRow)]
struct PromotionRow {
    artifact_json: String,
}

fn load_optional(row: Option<PromotionRow>) -> Result<Option<ModelPromotion>, WorkflowStoreError> {
    row.map(|row| {
        let value: ModelPromotion =
            serde_json::from_str(&row.artifact_json).map_err(store_error)?;
        validate(&value)?;
        Ok(value)
    })
    .transpose()
}

fn validate(value: &ModelPromotion) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "model promotion fingerprint does not reproduce".into(),
        ));
    }
    Ok(())
}

const fn state(value: PromotionState) -> &'static str {
    match value {
        PromotionState::Promoted => "promoted",
        PromotionState::Rejected => "rejected",
        PromotionState::Inconclusive => "inconclusive",
    }
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
