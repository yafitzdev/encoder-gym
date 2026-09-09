//! Passive connections never create, migrate, reconcile, or change journal mode.
use super::{ExperimentStoreError, MIGRATOR, SqliteExperimentStore, store_error};
use serde::Serialize;
use sqlx::{
    Connection, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::Path, str::FromStr, time::Duration};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScientificStoreInventory {
    pub projects: u64,
    pub protocols: u64,
    pub experiment_runs: u64,
    pub benchmark_generations: u64,
    pub diagnoses: u64,
    pub proposals: u64,
    pub approved_delta_selections: u64,
    pub training_snapshots: u64,
    pub optimization_runs: u64,
}

impl SqliteExperimentStore {
    /// Return the immutable approved-delta identities associated with one exact
    /// execution-project snapshot. Callers must still load each selection
    /// through its owning store contract before using it.
    pub async fn native_delta_selection_ids_for_project(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<Uuid>, ExperimentStoreError> {
        sqlx::query_scalar(
            "SELECT selections.id \
             FROM encoder_native_delta_selections AS selections \
             JOIN encoder_repair_proposals AS proposals \
               ON proposals.id = selections.proposal_id \
             WHERE proposals.execution_project_snapshot_id = ? \
             ORDER BY selections.created_at DESC, selections.id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    /// Return optimization runs for one exact scientific project, newest
    /// durable transition first. Callers still replay every returned run
    /// through the optimization owner before presenting it.
    pub async fn optimization_run_ids_for_project(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<Uuid>, ExperimentStoreError> {
        sqlx::query_scalar(
            "SELECT runs.id \
             FROM encoder_production_optimization_runs AS runs \
             JOIN encoder_production_optimization_definitions AS definitions \
               ON definitions.id = runs.definition_id \
             WHERE definitions.project_snapshot_id = ? \
             ORDER BY runs.updated_at DESC, runs.id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    /// Open an existing, current-schema database with SQLite-enforced read-only access.
    /// WAL remains visible: concurrent writers may commit while status/watch runs.
    pub async fn connect_read_only(database_url: &str) -> Result<Self, ExperimentStoreError> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(store_error)?
            .create_if_missing(false)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .pragma("query_only", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(store_error)?;
        let applied = sqlx::query_as::<_, (i64, Vec<u8>, bool)>(
            "SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version",
        ).fetch_all(&pool).await.map_err(|error| ExperimentStoreError(format!(
            "could not verify production database schema: {error}; initialize or upgrade explicitly with `synth database migrate --kind production`"
        )))?;
        let expected = MIGRATOR
            .iter()
            .filter(|migration| !migration.migration_type.is_down_migration())
            .collect::<Vec<_>>();
        if applied.len() != expected.len()
            || applied
                .iter()
                .zip(expected)
                .any(|((version, checksum, success), migration)| {
                    !success
                        || *version != migration.version
                        || checksum.as_slice() != migration.checksum.as_ref()
                })
        {
            return Err(ExperimentStoreError(
                "database schema does not match this executable; use a matching executable or explicitly upgrade with `synth database migrate --kind production`".into(),
            ));
        }
        Ok(Self { pool })
    }

    /// Run SQLite's complete structural check without migrating or reconciling
    /// the selected scientific history.
    pub async fn verify_integrity(&self) -> Result<(), ExperimentStoreError> {
        let rows = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        if rows.as_slice() != ["ok"] {
            return Err(ExperimentStoreError(format!(
                "scientific database integrity check failed: {}",
                rows.join("; ")
            )));
        }
        Ok(())
    }

    /// Small row-free inventory for an explicit history-import preview. Every
    /// artifact is still verified by its owning read path before later use.
    pub async fn inventory(&self) -> Result<ScientificStoreInventory, ExperimentStoreError> {
        async fn count(pool: &sqlx::SqlitePool, table: &str) -> Result<u64, ExperimentStoreError> {
            let query = format!("SELECT COUNT(*) FROM {table}");
            let value = sqlx::query_scalar::<_, i64>(&query)
                .fetch_one(pool)
                .await
                .map_err(store_error)?;
            u64::try_from(value).map_err(store_error)
        }
        Ok(ScientificStoreInventory {
            projects: count(&self.pool, "encoder_experiment_projects").await?,
            protocols: count(&self.pool, "encoder_experiment_protocols").await?,
            experiment_runs: count(&self.pool, "encoder_experiment_runs").await?,
            benchmark_generations: count(&self.pool, "encoder_benchmark_generations").await?,
            diagnoses: count(&self.pool, "encoder_repair_diagnoses").await?,
            proposals: count(&self.pool, "encoder_repair_proposals").await?,
            approved_delta_selections: count(&self.pool, "encoder_native_delta_selections").await?,
            training_snapshots: count(&self.pool, "encoder_native_repair_training_snapshots")
                .await?,
            optimization_runs: count(&self.pool, "encoder_production_optimization_runs").await?,
        })
    }

    /// Create one transactionally consistent standalone SQLite image. This
    /// reads the selected database and its WAL but never modifies either.
    pub async fn snapshot_database(
        database_url: &str,
        destination: &Path,
    ) -> Result<(), ExperimentStoreError> {
        if destination.try_exists().map_err(store_error)? {
            return Err(ExperimentStoreError(
                "scientific history destination already exists".into(),
            ));
        }
        let parent = destination.parent().ok_or_else(|| {
            ExperimentStoreError("scientific history destination has no parent".into())
        })?;
        if !parent.is_dir() {
            return Err(ExperimentStoreError(
                "scientific history destination parent does not exist".into(),
            ));
        }
        // Require the exact current schema before producing any output.
        let verified = Self::connect_read_only(database_url).await?;
        verified.verify_integrity().await?;
        verified.pool.close().await;

        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(store_error)?
            .create_if_missing(false)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options)
            .await
            .map_err(store_error)?;
        let result = sqlx::query("VACUUM INTO ?")
            .bind(destination.to_string_lossy().into_owned())
            .execute(&mut connection)
            .await
            .map_err(store_error);
        connection.close().await.map_err(store_error)?;
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use encoder_experiment_core::{
        domain::{
            BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
            ExternalProjectSnapshot, ModelArtifactIdentity,
        },
        ports::ExperimentStore,
    };

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "snapshot test",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            digest('1'),
            BackendIdentity::new("adapter", "v1", digest('2')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('3'))
                    .unwrap(),
                ExternalArtifactIdentity::new("dev", EvidenceRole::Development, 1, digest('4'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('5'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, digest('6'))
                .unwrap(),
            serde_json::json!({"test": true}),
            chrono::Utc::now(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn consistent_snapshot_includes_wal_and_reopens_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.sqlite");
        let destination = directory.path().join("snapshot.sqlite");
        let source_url = format!("sqlite://{}", source.to_string_lossy().replace('\\', "/"));
        let destination_url = format!(
            "sqlite://{}",
            destination.to_string_lossy().replace('\\', "/")
        );
        let store = SqliteExperimentStore::connect(&source_url).await.unwrap();
        let project = project();
        store.create_project(project.clone()).await.unwrap();

        SqliteExperimentStore::snapshot_database(&source_url, &destination)
            .await
            .unwrap();
        let copy = SqliteExperimentStore::connect_read_only(&destination_url)
            .await
            .unwrap();
        copy.verify_integrity().await.unwrap();
        assert_eq!(copy.get_project(project.id).await.unwrap(), Some(project));
        assert_eq!(copy.inventory().await.unwrap().projects, 1);
        copy.pool.close().await;
        store.pool.close().await;
    }
}
