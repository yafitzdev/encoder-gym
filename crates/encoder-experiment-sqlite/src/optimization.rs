use chrono::{DateTime, Utc};
use encoder_campaign_core::optimization::{
    OptimizationEvent, OptimizationLaunchStore, OptimizationStoreError,
    ProductionOptimizationDefinition, ProductionOptimizationRun, replay_optimization,
};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_repair_core::ports::NativeRepairTrainingStore;
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::replay_benchmark_generation, ports::BenchmarkGenerationStore,
};

use crate::SqliteExperimentStore;

#[derive(sqlx::FromRow)]
struct DefinitionRow {
    id: Uuid,
    manifest_fingerprint: String,
    fingerprint: String,
    project_snapshot_id: Uuid,
    metric_source_protocol_id: Uuid,
    training_snapshot_id: Uuid,
    benchmark_generation_id: Uuid,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Uuid,
    definition_id: Uuid,
    fingerprint: String,
    reserved_campaign_id: Uuid,
    artifact_json: String,
    last_sequence: i64,
    last_event_fingerprint: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl OptimizationLaunchStore for SqliteExperimentStore {
    fn create_optimization(
        &self,
        definition: ProductionOptimizationDefinition,
        run: ProductionOptimizationRun,
        first_event: OptimizationEvent,
    ) -> encoder_campaign_core::optimization::BoxFuture<'_, Result<(), OptimizationStoreError>>
    {
        Box::pin(async move {
            self.validate_definition(&definition).await?;
            run.validate_integrity(&definition).map_err(store_error)?;
            first_event.validate_integrity(&run).map_err(store_error)?;
            if first_event.sequence != 1 || first_event.previous_event_fingerprint.is_some() {
                return Err(OptimizationStoreError(
                    "first production optimization event is invalid".into(),
                ));
            }
            let definition_json = serde_json::to_string(&definition).map_err(store_error)?;
            let run_json = serde_json::to_string(&run).map_err(store_error)?;
            let event_json = serde_json::to_string(&first_event).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_production_optimization_definitions \
                 (id, manifest_fingerprint, fingerprint, project_snapshot_id, \
                  metric_source_protocol_id, training_snapshot_id, benchmark_generation_id, \
                  artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(definition.id)
            .bind(&definition.manifest_fingerprint)
            .bind(&definition.fingerprint)
            .bind(definition.project.id)
            .bind(definition.metric_source_protocol.id)
            .bind(definition.training_snapshot.id)
            .bind(definition.benchmark.generation_id)
            .bind(definition_json)
            .bind(definition.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_production_optimization_runs \
                 (id, definition_id, fingerprint, reserved_campaign_id, artifact_json, \
                  last_sequence, last_event_fingerprint, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, 1, ?, ?, ?)",
            )
            .bind(run.id)
            .bind(run.definition_id)
            .bind(&run.fingerprint)
            .bind(run.reserved_campaign_id)
            .bind(run_json)
            .bind(&first_event.fingerprint)
            .bind(run.created_at)
            .bind(first_event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_production_optimization_events \
                 (id, run_id, sequence, previous_event_fingerprint, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, 1, NULL, ?, ?, ?)",
            )
            .bind(first_event.id)
            .bind(first_event.run_id)
            .bind(&first_event.fingerprint)
            .bind(event_json)
            .bind(first_event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)
        })
    }

    fn find_optimization_by_manifest(
        &self,
        manifest_fingerprint: String,
    ) -> encoder_campaign_core::optimization::BoxFuture<
        '_,
        Result<
            Option<(ProductionOptimizationDefinition, ProductionOptimizationRun)>,
            OptimizationStoreError,
        >,
    > {
        Box::pin(async move {
            let definition_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_production_optimization_definitions \
                 WHERE manifest_fingerprint = ?",
            )
            .bind(manifest_fingerprint)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match definition_id {
                Some(id) => self.load_optimization_by_definition(id).await.map(Some),
                None => Ok(None),
            }
        })
    }

    fn get_optimization_run(
        &self,
        run_id: Uuid,
    ) -> encoder_campaign_core::optimization::BoxFuture<
        '_,
        Result<
            Option<(ProductionOptimizationDefinition, ProductionOptimizationRun)>,
            OptimizationStoreError,
        >,
    > {
        Box::pin(async move {
            let definition_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT definition_id FROM encoder_production_optimization_runs WHERE id = ?",
            )
            .bind(run_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match definition_id {
                Some(id) => self.load_optimization_by_definition(id).await.map(Some),
                None => Ok(None),
            }
        })
    }

    fn append_optimization_event(
        &self,
        event: OptimizationEvent,
    ) -> encoder_campaign_core::optimization::BoxFuture<'_, Result<(), OptimizationStoreError>>
    {
        Box::pin(async move {
            let (_, run) = self
                .get_optimization_run(event.run_id)
                .await?
                .ok_or_else(|| {
                    OptimizationStoreError("production optimization run does not exist".into())
                })?;
            event.validate_integrity(&run).map_err(store_error)?;
            let event_json = serde_json::to_string(&event).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let result = sqlx::query(
                "UPDATE encoder_production_optimization_runs \
                 SET last_sequence = ?, last_event_fingerprint = ?, updated_at = ? \
                 WHERE id = ? AND last_sequence = ? AND last_event_fingerprint = ?",
            )
            .bind(i64::from(event.sequence))
            .bind(&event.fingerprint)
            .bind(event.created_at)
            .bind(event.run_id)
            .bind(i64::from(event.sequence - 1))
            .bind(
                event
                    .previous_event_fingerprint
                    .as_deref()
                    .unwrap_or_default(),
            )
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(OptimizationStoreError(
                    "production optimization journal head changed".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO encoder_production_optimization_events \
                 (id, run_id, sequence, previous_event_fingerprint, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(event.id)
            .bind(event.run_id)
            .bind(i64::from(event.sequence))
            .bind(&event.previous_event_fingerprint)
            .bind(&event.fingerprint)
            .bind(event_json)
            .bind(event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)
        })
    }

    fn list_optimization_events(
        &self,
        run_id: Uuid,
    ) -> encoder_campaign_core::optimization::BoxFuture<
        '_,
        Result<Vec<OptimizationEvent>, OptimizationStoreError>,
    > {
        Box::pin(async move {
            let (_, run) = self.get_optimization_run(run_id).await?.ok_or_else(|| {
                OptimizationStoreError("production optimization run does not exist".into())
            })?;
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_production_optimization_events \
                 WHERE run_id = ? ORDER BY sequence",
            )
            .bind(run_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let events = rows
                .into_iter()
                .map(|value| serde_json::from_str(&value).map_err(store_error))
                .collect::<Result<Vec<_>, _>>()?;
            let view = replay_optimization(&run, &events).map_err(store_error)?;
            let stored: RunRow = sqlx::query_as(
                "SELECT id, definition_id, fingerprint, reserved_campaign_id, artifact_json, \
                 last_sequence, last_event_fingerprint, created_at, updated_at \
                 FROM encoder_production_optimization_runs WHERE id = ?",
            )
            .bind(run_id)
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?;
            if stored.last_sequence != i64::from(view.last_sequence)
                || stored.last_event_fingerprint != view.last_event_fingerprint
                || stored.updated_at != view.updated_at
            {
                return Err(OptimizationStoreError(
                    "production optimization journal header changed".into(),
                ));
            }
            Ok(events)
        })
    }
}

impl SqliteExperimentStore {
    async fn validate_definition(
        &self,
        definition: &ProductionOptimizationDefinition,
    ) -> Result<(), OptimizationStoreError> {
        let project = self
            .get_project(definition.project.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| OptimizationStoreError("optimization project does not exist".into()))?;
        let protocol = self
            .get_protocol(definition.metric_source_protocol.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                OptimizationStoreError("optimization metric source does not exist".into())
            })?;
        let snapshot = self
            .get_native_repair_training_snapshot(definition.training_snapshot.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                OptimizationStoreError("optimization training snapshot does not exist".into())
            })?;
        if snapshot.fingerprint != definition.training_snapshot.fingerprint
            || snapshot.specification_fingerprint
                != definition.training_snapshot_specification_fingerprint
            || snapshot.proposal.id != definition.proposal.id
            || snapshot.proposal.fingerprint != definition.proposal.fingerprint
            || snapshot.selection.id != definition.delta_selection.id
            || snapshot.selection.fingerprint != definition.delta_selection.fingerprint
            || snapshot.execution_project.id != definition.project.id
            || snapshot.execution_project.fingerprint != definition.project.fingerprint
        {
            return Err(OptimizationStoreError(
                "optimization repair lineage differs from its immutable definition".into(),
            ));
        }
        let generation = self
            .get_benchmark_generation(definition.benchmark.generation_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                OptimizationStoreError("optimization benchmark generation does not exist".into())
            })?;
        let generation_events = self
            .list_benchmark_generation_events(generation.id)
            .await
            .map_err(store_error)?;
        let generation_view =
            replay_benchmark_generation(&generation, &generation_events).map_err(store_error)?;
        let observed_benchmark =
            encoder_campaign_core::CampaignBenchmarkBinding::from_active_generation(
                &generation,
                &generation_view,
                definition.benchmark.sealed_suite_key.clone(),
            )
            .map_err(store_error)?;
        if observed_benchmark != definition.benchmark {
            return Err(OptimizationStoreError(
                "optimization benchmark generation authority changed".into(),
            ));
        }
        definition
            .validate_integrity(&project, &protocol)
            .map_err(store_error)
    }

    async fn load_optimization_by_definition(
        &self,
        definition_id: Uuid,
    ) -> Result<(ProductionOptimizationDefinition, ProductionOptimizationRun), OptimizationStoreError>
    {
        let stored: DefinitionRow = sqlx::query_as(
            "SELECT id, manifest_fingerprint, fingerprint, project_snapshot_id, \
             metric_source_protocol_id, training_snapshot_id, benchmark_generation_id, \
             artifact_json, created_at FROM encoder_production_optimization_definitions WHERE id = ?",
        )
        .bind(definition_id)
        .fetch_one(self.pool())
        .await
        .map_err(store_error)?;
        let definition: ProductionOptimizationDefinition =
            serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
        if stored.id != definition.id
            || stored.manifest_fingerprint != definition.manifest_fingerprint
            || stored.fingerprint != definition.fingerprint
            || stored.project_snapshot_id != definition.project.id
            || stored.metric_source_protocol_id != definition.metric_source_protocol.id
            || stored.training_snapshot_id != definition.training_snapshot.id
            || stored.benchmark_generation_id != definition.benchmark.generation_id
            || stored.created_at != definition.created_at
        {
            return Err(OptimizationStoreError(
                "production optimization definition storage envelope changed".into(),
            ));
        }
        self.validate_definition(&definition).await?;
        let stored_run: RunRow = sqlx::query_as(
            "SELECT id, definition_id, fingerprint, reserved_campaign_id, artifact_json, \
             last_sequence, last_event_fingerprint, created_at, updated_at \
             FROM encoder_production_optimization_runs WHERE definition_id = ?",
        )
        .bind(definition.id)
        .fetch_one(self.pool())
        .await
        .map_err(store_error)?;
        let run: ProductionOptimizationRun =
            serde_json::from_str(&stored_run.artifact_json).map_err(store_error)?;
        if stored_run.id != run.id
            || stored_run.definition_id != run.definition_id
            || stored_run.fingerprint != run.fingerprint
            || stored_run.reserved_campaign_id != run.reserved_campaign_id
            || stored_run.created_at != run.created_at
            || stored_run.last_sequence < 1
            || stored_run.last_event_fingerprint.is_empty()
            || stored_run.updated_at < run.created_at
        {
            return Err(OptimizationStoreError(
                "production optimization run storage envelope changed".into(),
            ));
        }
        run.validate_integrity(&definition).map_err(store_error)?;
        Ok((definition, run))
    }
}

fn store_error(error: impl std::fmt::Display) -> OptimizationStoreError {
    OptimizationStoreError(error.to_string())
}
