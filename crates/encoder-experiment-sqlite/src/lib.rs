//! Dedicated SQLite persistence for production encoder experiment journals.

mod benchmark_generation;
mod campaign;
mod native_delta;
mod repair;

use std::{str::FromStr, time::Duration};

use encoder_experiment_core::{
    domain::ExternalProjectSnapshot,
    journal::ExperimentEvent,
    ports::{BoxFuture, ExperimentStore, ExperimentStoreError},
    protocol::ExperimentProtocol,
};
use sqlx::{
    Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone)]
pub struct SqliteExperimentStore {
    pool: SqlitePool,
}

impl SqliteExperimentStore {
    pub async fn connect(database_url: &str) -> Result<Self, ExperimentStoreError> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(store_error)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(if database_url.contains(":memory:") {
                1
            } else {
                5
            })
            .connect_with(options)
            .await
            .map_err(store_error)?;
        MIGRATOR.run(&pool).await.map_err(store_error)?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Recovery lookup used by campaign orchestration after a process exits
    /// between durable experiment-run creation and campaign journal linking.
    pub async fn run_ids_for_protocol(
        &self,
        protocol_id: Uuid,
    ) -> Result<Vec<Uuid>, ExperimentStoreError> {
        sqlx::query_scalar(
            "SELECT id FROM encoder_experiment_runs WHERE protocol_id = ? ORDER BY created_at, id",
        )
        .bind(protocol_id)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn protocol_project(
        &self,
        protocol: &ExperimentProtocol,
    ) -> Result<ExternalProjectSnapshot, ExperimentStoreError> {
        let project = self
            .get_project(protocol.project_snapshot_id)
            .await?
            .ok_or_else(|| ExperimentStoreError("experiment project does not exist".into()))?;
        protocol.validate_integrity(&project).map_err(store_error)?;
        Ok(project)
    }
}

impl ExperimentStore for SqliteExperimentStore {
    fn create_project(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>> {
        Box::pin(async move {
            project.validate_integrity().map_err(store_error)?;
            let artifact_json = serde_json::to_string(&project).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_experiment_projects \
                 (id, fingerprint, source_fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(project.id)
            .bind(&project.fingerprint)
            .bind(&project.source_fingerprint)
            .bind(artifact_json)
            .bind(project.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_project(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_experiment_projects WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            artifact.map(decode_project).transpose()
        })
    }

    fn find_project_by_fingerprint(
        &self,
        fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_experiment_projects WHERE fingerprint = ?",
            )
            .bind(fingerprint)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            artifact.map(decode_project).transpose()
        })
    }

    fn find_project_by_source_fingerprint(
        &self,
        source_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_experiment_projects \
                 WHERE source_fingerprint = ?",
            )
            .bind(source_fingerprint)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            artifact.map(decode_project).transpose()
        })
    }

    fn create_protocol(
        &self,
        protocol: ExperimentProtocol,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>> {
        Box::pin(async move {
            self.protocol_project(&protocol).await?;
            let artifact_json = serde_json::to_string(&protocol).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_experiment_protocols \
                 (id, project_snapshot_id, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(protocol.id)
            .bind(protocol.project_snapshot_id)
            .bind(&protocol.fingerprint)
            .bind(artifact_json)
            .bind(protocol.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_protocol(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ExperimentProtocol>, ExperimentStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_experiment_protocols WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            let Some(artifact) = artifact else {
                return Ok(None);
            };
            let protocol: ExperimentProtocol =
                serde_json::from_str(&artifact).map_err(store_error)?;
            self.protocol_project(&protocol).await?;
            Ok(Some(protocol))
        })
    }

    fn create_run(
        &self,
        first_event: ExperimentEvent,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>> {
        Box::pin(async move {
            first_event.validate_integrity().map_err(store_error)?;
            if first_event.sequence != 1 || first_event.previous_event_fingerprint.is_some() {
                return Err(ExperimentStoreError(
                    "first experiment event must start a new journal".into(),
                ));
            }
            let protocol = self
                .get_protocol(first_event.protocol_id)
                .await?
                .ok_or_else(|| ExperimentStoreError("experiment protocol does not exist".into()))?;
            if first_event.protocol_fingerprint != protocol.fingerprint {
                return Err(ExperimentStoreError(
                    "first event does not match its immutable protocol".into(),
                ));
            }
            let event_json = serde_json::to_string(&first_event).map_err(store_error)?;
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_experiment_runs \
                 (id, protocol_id, last_sequence, last_event_fingerprint, created_at, updated_at) \
                 VALUES (?, ?, 1, ?, ?, ?)",
            )
            .bind(first_event.run_id)
            .bind(first_event.protocol_id)
            .bind(&first_event.fingerprint)
            .bind(first_event.created_at)
            .bind(first_event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            insert_event(&mut transaction, &first_event, &event_json).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn append_event(
        &self,
        event: ExperimentEvent,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>> {
        Box::pin(async move {
            event.validate_integrity().map_err(store_error)?;
            let previous = event.previous_event_fingerprint.as_deref().ok_or_else(|| {
                ExperimentStoreError("appended experiment event requires a predecessor".into())
            })?;
            let event_json = serde_json::to_string(&event).map_err(store_error)?;
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            let run = sqlx::query(
                "SELECT protocol_id, last_sequence, last_event_fingerprint \
                 FROM encoder_experiment_runs WHERE id = ?",
            )
            .bind(event.run_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(store_error)?
            .ok_or_else(|| ExperimentStoreError("experiment run does not exist".into()))?;
            let protocol_id: Uuid = run.try_get("protocol_id").map_err(store_error)?;
            let last_sequence: i64 = run.try_get("last_sequence").map_err(store_error)?;
            let last_fingerprint: String =
                run.try_get("last_event_fingerprint").map_err(store_error)?;
            if protocol_id != event.protocol_id
                || last_sequence + 1 != i64::from(event.sequence)
                || last_fingerprint != previous
            {
                return Err(ExperimentStoreError(
                    "experiment event append conflicted with the durable journal head".into(),
                ));
            }
            insert_event(&mut transaction, &event, &event_json).await?;
            let updated = sqlx::query(
                "UPDATE encoder_experiment_runs SET last_sequence = ?, \
                 last_event_fingerprint = ?, updated_at = ? \
                 WHERE id = ? AND last_sequence = ? AND last_event_fingerprint = ?",
            )
            .bind(i64::from(event.sequence))
            .bind(&event.fingerprint)
            .bind(event.created_at)
            .bind(event.run_id)
            .bind(last_sequence)
            .bind(previous)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if updated.rows_affected() != 1 {
                return Err(ExperimentStoreError(
                    "experiment event append lost an optimistic concurrency race".into(),
                ));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn load_events(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ExperimentEvent>, ExperimentStoreError>> {
        Box::pin(async move {
            let artifacts: Vec<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_experiment_events \
                 WHERE run_id = ? ORDER BY sequence",
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            artifacts
                .into_iter()
                .map(|artifact| {
                    let event: ExperimentEvent =
                        serde_json::from_str(&artifact).map_err(store_error)?;
                    event.validate_integrity().map_err(store_error)?;
                    Ok(event)
                })
                .collect()
        })
    }
}

async fn insert_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: &ExperimentEvent,
    artifact_json: &str,
) -> Result<(), ExperimentStoreError> {
    sqlx::query(
        "INSERT INTO encoder_experiment_events \
         (id, run_id, sequence, previous_event_fingerprint, fingerprint, artifact_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(event.id)
    .bind(event.run_id)
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

fn decode_project(artifact: String) -> Result<ExternalProjectSnapshot, ExperimentStoreError> {
    let project: ExternalProjectSnapshot = serde_json::from_str(&artifact).map_err(store_error)?;
    project.validate_integrity().map_err(store_error)?;
    Ok(project)
}

pub(crate) fn store_error(error: impl std::fmt::Display) -> ExperimentStoreError {
    ExperimentStoreError(error.to_string())
}
