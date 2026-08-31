use chrono::{DateTime, Utc};
use sqlx::FromRow;
use training_core::{domain::TrainingRunState, ports::TrainingStore};
use uuid::Uuid;
use workflow_core::{
    benchmark::{AcceptanceAssessment, AcceptanceState},
    ports::{
        BenchmarkBundleStore, BenchmarkStore, BoxFuture, PromotionStore,
        TrainingBenchmarkCheckStore, WorkflowRunStore, WorkflowStoreError,
    },
    promotion::{ModelPromotion, PromotionState},
    workflow::StageAttemptState,
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
            validate_check_binding(self, &promotion, true).await?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let check_id = promotion.training_benchmark_check_id.ok_or_else(|| {
                WorkflowStoreError("new model promotion has no training-benchmark check".into())
            })?;
            let current_check = crate::training_benchmark::load_training_benchmark_check(
                &mut transaction,
                check_id,
                true,
            )
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                WorkflowStoreError(
                    "model promotion training-benchmark check is no longer executable".into(),
                )
            })?;
            if current_check.fingerprint
                != promotion
                    .training_benchmark_check_fingerprint
                    .as_deref()
                    .unwrap_or_default()
                || !current_check.training_allowed()
            {
                return Err(WorkflowStoreError(
                    "model promotion training-benchmark authority changed before commit".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO model_promotions \
                 (id, workflow_run_id, checkpoint_id, training_snapshot_id, \
                  training_benchmark_check_id, training_benchmark_check_fingerprint, \
                  development_assessment_id, sealed_assessment_id, state, fingerprint, \
                  artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(promotion.id)
            .bind(promotion.workflow_run_id)
            .bind(promotion.checkpoint_id)
            .bind(promotion.training_snapshot_id)
            .bind(promotion.training_benchmark_check_id)
            .bind(&promotion.training_benchmark_check_fingerprint)
            .bind(promotion.development_assessment_id)
            .bind(promotion.sealed_assessment_id)
            .bind(state(promotion.state))
            .bind(&promotion.fingerprint)
            .bind(serde_json::to_string(&promotion).map_err(store_error)?)
            .bind(promotion.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_promotion(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelPromotion>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                self,
                sqlx::query_as::<_, PromotionRow>(
                    "SELECT id, workflow_run_id, checkpoint_id, training_snapshot_id, \
                     training_benchmark_check_id, training_benchmark_check_fingerprint, \
                     development_assessment_id, sealed_assessment_id, state, fingerprint, \
                     artifact_json, created_at FROM model_promotions WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
            .await
        })
    }

    fn get_workflow_promotion(
        &self,
        workflow_run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelPromotion>, WorkflowStoreError>> {
        Box::pin(async move {
            load_optional(
                self,
                sqlx::query_as::<_, PromotionRow>(
                    "SELECT id, workflow_run_id, checkpoint_id, training_snapshot_id, \
                     training_benchmark_check_id, training_benchmark_check_fingerprint, \
                     development_assessment_id, sealed_assessment_id, state, fingerprint, \
                     artifact_json, created_at FROM model_promotions WHERE workflow_run_id = ?",
                )
                .bind(workflow_run_id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
            )
            .await
        })
    }
}

#[derive(Debug, FromRow)]
struct PromotionRow {
    id: Uuid,
    workflow_run_id: Uuid,
    checkpoint_id: Uuid,
    training_snapshot_id: Uuid,
    training_benchmark_check_id: Option<Uuid>,
    training_benchmark_check_fingerprint: Option<String>,
    development_assessment_id: Uuid,
    sealed_assessment_id: Uuid,
    state: String,
    fingerprint: String,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

async fn load_optional(
    store: &SqliteStore,
    row: Option<PromotionRow>,
) -> Result<Option<ModelPromotion>, WorkflowStoreError> {
    let value = row
        .map(|row| {
            let value: ModelPromotion =
                serde_json::from_str(&row.artifact_json).map_err(store_error)?;
            if value.id != row.id
                || value.workflow_run_id != row.workflow_run_id
                || value.checkpoint_id != row.checkpoint_id
                || value.training_snapshot_id != row.training_snapshot_id
                || value.training_benchmark_check_id != row.training_benchmark_check_id
                || value.training_benchmark_check_fingerprint
                    != row.training_benchmark_check_fingerprint
                || value.development_assessment_id != row.development_assessment_id
                || value.sealed_assessment_id != row.sealed_assessment_id
                || state(value.state) != row.state
                || value.fingerprint != row.fingerprint
                || value.created_at != row.created_at
            {
                return Err(WorkflowStoreError(
                    "model promotion normalized projection differs from artifact".into(),
                ));
            }
            validate(&value)?;
            Ok(value)
        })
        .transpose()?;
    if let Some(value) = &value {
        validate_check_binding(store, value, false).await?;
    }
    Ok(value)
}

async fn validate_check_binding(
    store: &SqliteStore,
    value: &ModelPromotion,
    require_current_roles: bool,
) -> Result<(), WorkflowStoreError> {
    let (Some(id), Some(fingerprint)) = (
        value.training_benchmark_check_id,
        value.training_benchmark_check_fingerprint.as_deref(),
    ) else {
        if value.training_benchmark_check_id.is_some()
            || value.training_benchmark_check_fingerprint.is_some()
        {
            return Err(WorkflowStoreError(
                "model promotion has an incomplete training-benchmark binding".into(),
            ));
        }
        if require_current_roles {
            return Err(WorkflowStoreError(
                "new model promotion requires a training-benchmark binding".into(),
            ));
        }
        return Ok(());
    };
    let check = if require_current_roles {
        store.get_executable_training_benchmark_check(id).await?
    } else {
        store.get_training_benchmark_check(id).await?
    }
    .ok_or_else(|| WorkflowStoreError("model promotion training-benchmark check missing".into()))?;
    if check.fingerprint != fingerprint
        || !check.training_allowed()
        || check.training_snapshot_id != value.training_snapshot_id
        || check.training_snapshot_fingerprint != value.training_snapshot_fingerprint
    {
        return Err(WorkflowStoreError(
            "model promotion training-benchmark binding is not clean or differs".into(),
        ));
    }
    let checkpoint = TrainingStore::get_checkpoint(store, value.checkpoint_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| WorkflowStoreError("model promotion checkpoint missing".into()))?;
    let training_run = TrainingStore::get_training_run(store, checkpoint.run_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| WorkflowStoreError("model promotion training run missing".into()))?;
    let input_binding = training_run.input_binding.as_ref().ok_or_else(|| {
        WorkflowStoreError("model promotion training run has no input authority".into())
    })?;
    if !checkpoint.is_final
        || checkpoint.epoch != training_run.configuration.epochs
        || checkpoint.model_format != training_run.model_format
        || checkpoint.artifact_checksum != value.checkpoint_fingerprint
        || training_run.state != TrainingRunState::Completed
        || training_run.snapshot_id != value.training_snapshot_id
        || input_binding.authority_kind != "training_benchmark_check"
        || input_binding.authority_id != check.id
        || input_binding.authority_fingerprint != check.fingerprint
        || input_binding.protocol != check.training_input_protocol.stable_name()
        || input_binding.population_fingerprint != check.training_population_fingerprint
        || input_binding.member_count != check.training_member_count
    {
        return Err(WorkflowStoreError(
            "model promotion checkpoint is not the completed output authorized by its training-benchmark check"
                .into(),
        ));
    }
    let mut training_connection = store.pool().acquire().await.map_err(store_error)?;
    crate::training::validate_training_input_authority(
        &mut training_connection,
        &training_run,
        require_current_roles,
    )
    .await
    .map_err(store_error)?;
    let bundle = store
        .get_benchmark_bundle(check.benchmark_bundle_id)
        .await?
        .ok_or_else(|| {
            WorkflowStoreError("model promotion training-benchmark bundle missing".into())
        })?;
    if bundle.fingerprint != check.benchmark_bundle_fingerprint
        || bundle.development_suite_id != value.development_suite_id
        || bundle.development_suite_fingerprint != value.development_suite_fingerprint
        || bundle.sealed_suite_id != Some(value.sealed_suite_id)
        || bundle.sealed_suite_fingerprint.as_deref()
            != Some(value.sealed_suite_fingerprint.as_str())
    {
        return Err(WorkflowStoreError(
            "model promotion benchmark suites differ from its leakage authority".into(),
        ));
    }
    let run = store
        .get_workflow_run(value.workflow_run_id)
        .await?
        .ok_or_else(|| WorkflowStoreError("model promotion workflow run missing".into()))?;
    let definition = store
        .get_workflow_definition(run.definition_id)
        .await?
        .ok_or_else(|| WorkflowStoreError("model promotion workflow definition missing".into()))?;
    let binding = definition.benchmark_bundle.as_ref().ok_or_else(|| {
        WorkflowStoreError("model promotion workflow has no benchmark authority".into())
    })?;
    if binding.bundle_id != bundle.id
        || binding.bundle_fingerprint != bundle.fingerprint
        || definition.development_suite_id != value.development_suite_id
        || definition.development_suite_fingerprint != value.development_suite_fingerprint
        || definition.sealed_suite_id != Some(value.sealed_suite_id)
        || definition.sealed_suite_fingerprint.as_deref()
            != Some(value.sealed_suite_fingerprint.as_str())
    {
        return Err(WorkflowStoreError(
            "model promotion leakage authority differs from its workflow definition".into(),
        ));
    }
    if artifact_core::fingerprint(&definition.policy).map_err(store_error)?
        != value.policy_fingerprint
    {
        return Err(WorkflowStoreError(
            "model promotion policy differs from its workflow definition".into(),
        ));
    }

    let development =
        BenchmarkStore::get_acceptance_assessment(store, value.development_assessment_id)
            .await?
            .ok_or_else(|| {
                WorkflowStoreError("model promotion development assessment missing".into())
            })?;
    let sealed = BenchmarkStore::get_acceptance_assessment(store, value.sealed_assessment_id)
        .await?
        .ok_or_else(|| WorkflowStoreError("model promotion sealed assessment missing".into()))?;
    validate_assessment_binding(
        &development,
        value.development_assessment_fingerprint.as_str(),
        value.checkpoint_id,
        value.development_suite_id,
        value.development_suite_fingerprint.as_str(),
        "development",
    )?;
    validate_assessment_binding(
        &sealed,
        value.sealed_assessment_fingerprint.as_str(),
        value.checkpoint_id,
        value.sealed_suite_id,
        value.sealed_suite_fingerprint.as_str(),
        "sealed",
    )?;
    let (expected_state, expected_reason) = match sealed.state {
        AcceptanceState::Pass => (
            PromotionState::Promoted,
            "sealed acceptance contract passed".to_owned(),
        ),
        AcceptanceState::Fail => (
            PromotionState::Rejected,
            "sealed acceptance contract failed".to_owned(),
        ),
        AcceptanceState::Inconclusive | AcceptanceState::Invalid => (
            PromotionState::Inconclusive,
            format!("sealed acceptance contract returned {:?}", sealed.state).to_ascii_lowercase(),
        ),
    };
    if value.state != expected_state || value.reason != expected_reason {
        return Err(WorkflowStoreError(
            "model promotion outcome differs from its sealed assessment".into(),
        ));
    }

    let attempts = store.list_workflow_attempts(value.workflow_run_id).await?;
    let has_link = |kinds: &[&str], artifact_id: Uuid, artifact_fingerprint: &str| {
        attempts
            .iter()
            .filter(|attempt| attempt.state == StageAttemptState::Completed)
            .flat_map(|attempt| &attempt.artifacts)
            .any(|artifact| {
                kinds.contains(&artifact.kind.as_str())
                    && artifact.artifact_id == artifact_id
                    && artifact.artifact_fingerprint == artifact_fingerprint
            })
    };
    let iteration = has_link(
        &["iteration_checkpoint"],
        value.checkpoint_id,
        &value.checkpoint_fingerprint,
    );
    let linked = if iteration {
        has_link(
            &["iteration_snapshot"],
            value.training_snapshot_id,
            &value.training_snapshot_fingerprint,
        ) && has_link(
            &["iteration_training_benchmark_check"],
            check.id,
            &check.fingerprint,
        ) && has_link(
            &["development_acceptance_assessment"],
            development.id,
            &development.fingerprint,
        )
    } else {
        has_link(
            &["checkpoint"],
            value.checkpoint_id,
            &value.checkpoint_fingerprint,
        ) && has_link(
            &["snapshot"],
            value.training_snapshot_id,
            &value.training_snapshot_fingerprint,
        ) && has_link(&["training_benchmark_check"], check.id, &check.fingerprint)
            && has_link(
                &["acceptance_assessment"],
                development.id,
                &development.fingerprint,
            )
    };
    if !linked
        || !has_link(
            &["sealed_acceptance_assessment"],
            sealed.id,
            &sealed.fingerprint,
        )
    {
        return Err(WorkflowStoreError(
            "model promotion artifacts are not the completed final cycle of its workflow run"
                .into(),
        ));
    }
    Ok(())
}

fn validate_assessment_binding(
    assessment: &AcceptanceAssessment,
    expected_fingerprint: &str,
    checkpoint_id: Uuid,
    suite_id: Uuid,
    suite_fingerprint: &str,
    name: &str,
) -> Result<(), WorkflowStoreError> {
    if assessment.fingerprint != expected_fingerprint
        || assessment.reproduce_fingerprint().map_err(store_error)? != assessment.fingerprint
        || assessment.checkpoint_id != Some(checkpoint_id)
        || assessment.suite_id != suite_id
        || assessment.suite_fingerprint != suite_fingerprint
    {
        return Err(WorkflowStoreError(format!(
            "model promotion {name} assessment differs from its selected checkpoint or suite"
        )));
    }
    Ok(())
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
