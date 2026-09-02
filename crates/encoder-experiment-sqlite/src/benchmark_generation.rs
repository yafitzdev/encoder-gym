use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::{
        BenchmarkGeneration, BenchmarkGenerationEvent, BenchmarkGenerationEventKind,
        replay_benchmark_generation,
    },
    ports::{BenchmarkGenerationStore, BoxFuture, WorkflowStoreError},
};

use crate::SqliteExperimentStore;

impl BenchmarkGenerationStore for SqliteExperimentStore {
    fn create_benchmark_generation(
        &self,
        generation: &BenchmarkGeneration,
        first_event: &BenchmarkGenerationEvent,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let generation = generation.clone();
        let first_event = first_event.clone();
        Box::pin(async move {
            generation.validate_integrity().map_err(store_error)?;
            replay_benchmark_generation(&generation, std::slice::from_ref(&first_event))
                .map_err(store_error)?;
            let generation_json = serde_json::to_string(&generation).map_err(store_error)?;
            let event_json = serde_json::to_string(&first_event).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_benchmark_generations \
                 (id, fingerprint, artifact_json, last_sequence, last_event_fingerprint, \
                  created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?, ?)",
            )
            .bind(generation.id)
            .bind(&generation.fingerprint)
            .bind(generation_json)
            .bind(&first_event.fingerprint)
            .bind(generation.created_at)
            .bind(first_event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            insert_event(&mut transaction, &first_event, &event_json).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_benchmark_generation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkGeneration>, WorkflowStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_benchmark_generations WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            artifact
                .map(|value| {
                    let generation: BenchmarkGeneration =
                        serde_json::from_str(&value).map_err(store_error)?;
                    generation.validate_integrity().map_err(store_error)?;
                    Ok(generation)
                })
                .transpose()
        })
    }

    fn append_benchmark_generation_event(
        &self,
        event: &BenchmarkGenerationEvent,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let event = event.clone();
        Box::pin(async move {
            validate_candidate_journal(self, &event).await?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            append_in_transaction(&mut transaction, &event).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_benchmark_generation_events(
        &self,
        generation_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkGenerationEvent>, WorkflowStoreError>> {
        Box::pin(async move {
            let artifacts: Vec<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_benchmark_generation_events \
                 WHERE generation_id = ? ORDER BY sequence",
            )
            .bind(generation_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            artifacts
                .into_iter()
                .map(|artifact| {
                    let event: BenchmarkGenerationEvent =
                        serde_json::from_str(&artifact).map_err(store_error)?;
                    event.validate_integrity().map_err(store_error)?;
                    Ok(event)
                })
                .collect()
        })
    }

    fn activate_successor_generation(
        &self,
        predecessor_superseded: &BenchmarkGenerationEvent,
        successor_activated: &BenchmarkGenerationEvent,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let predecessor_superseded = predecessor_superseded.clone();
        let successor_activated = successor_activated.clone();
        Box::pin(async move {
            let linked = matches!(
                &predecessor_superseded.event,
                BenchmarkGenerationEventKind::Superseded {
                    successor_generation_id,
                    successor_generation_fingerprint,
                } if *successor_generation_id == successor_activated.generation_id
                    && *successor_generation_fingerprint == successor_activated.generation_fingerprint
            ) && matches!(
                successor_activated.event,
                BenchmarkGenerationEventKind::Activated { .. }
            );
            if !linked {
                return Err(WorkflowStoreError(
                    "successor activation events do not form an exact pair".into(),
                ));
            }
            validate_candidate_journal(self, &predecessor_superseded).await?;
            validate_candidate_journal(self, &successor_activated).await?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            append_in_transaction(&mut transaction, &predecessor_superseded).await?;
            append_in_transaction(&mut transaction, &successor_activated).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }
}

pub(crate) async fn validate_candidate_journal(
    store: &SqliteExperimentStore,
    event: &BenchmarkGenerationEvent,
) -> Result<(), WorkflowStoreError> {
    event.validate_integrity().map_err(store_error)?;
    let generation = store
        .get_benchmark_generation(event.generation_id)
        .await?
        .ok_or_else(|| WorkflowStoreError("benchmark generation does not exist".into()))?;
    let mut events = store
        .list_benchmark_generation_events(event.generation_id)
        .await?;
    events.push(event.clone());
    replay_benchmark_generation(&generation, &events).map_err(store_error)?;
    Ok(())
}

pub(crate) async fn append_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &BenchmarkGenerationEvent,
) -> Result<(), WorkflowStoreError> {
    let previous = event.previous_event_fingerprint.as_deref().ok_or_else(|| {
        WorkflowStoreError("appended benchmark generation event requires a predecessor".into())
    })?;
    let head = sqlx::query(
        "SELECT last_sequence, last_event_fingerprint \
         FROM encoder_benchmark_generations WHERE id = ?",
    )
    .bind(event.generation_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(store_error)?
    .ok_or_else(|| WorkflowStoreError("benchmark generation does not exist".into()))?;
    let last_sequence: i64 = head.try_get("last_sequence").map_err(store_error)?;
    let last_fingerprint: String = head
        .try_get("last_event_fingerprint")
        .map_err(store_error)?;
    if last_sequence + 1 != i64::from(event.sequence) || last_fingerprint != previous {
        return Err(WorkflowStoreError(
            "benchmark generation append conflicted with the durable journal head".into(),
        ));
    }
    let event_json = serde_json::to_string(event).map_err(store_error)?;
    insert_event(transaction, event, &event_json).await?;
    let updated = sqlx::query(
        "UPDATE encoder_benchmark_generations SET last_sequence = ?, \
         last_event_fingerprint = ?, updated_at = ? \
         WHERE id = ? AND last_sequence = ? AND last_event_fingerprint = ?",
    )
    .bind(i64::from(event.sequence))
    .bind(&event.fingerprint)
    .bind(event.created_at)
    .bind(event.generation_id)
    .bind(last_sequence)
    .bind(previous)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(WorkflowStoreError(
            "benchmark generation append lost an optimistic concurrency race".into(),
        ));
    }
    Ok(())
}

async fn insert_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &BenchmarkGenerationEvent,
    artifact_json: &str,
) -> Result<(), WorkflowStoreError> {
    sqlx::query(
        "INSERT INTO encoder_benchmark_generation_events \
         (id, generation_id, sequence, previous_event_fingerprint, fingerprint, \
          artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(event.id)
    .bind(event.generation_id)
    .bind(i64::from(event.sequence))
    .bind(&event.previous_event_fingerprint)
    .bind(&event.fingerprint)
    .bind(artifact_json)
    .bind(event.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
