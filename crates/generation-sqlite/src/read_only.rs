//! Passive connections never create, migrate, reconcile, or change journal mode.
use super::{MIGRATOR, SqliteStore, StoreError, store_error};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{str::FromStr, time::Duration};

impl SqliteStore {
    /// Open an existing, current-schema database with SQLite-enforced read-only access.
    /// WAL remains visible: concurrent writers may commit while status/watch runs.
    pub async fn connect_read_only(database_url: &str) -> Result<Self, StoreError> {
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
        ).fetch_all(&pool).await.map_err(|error| StoreError(format!(
            "could not verify classification database schema: {error}; initialize or upgrade explicitly with `synth database migrate --kind classification`"
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
            return Err(StoreError(
                "database schema does not match this executable; use a matching executable or explicitly upgrade with `synth database migrate --kind classification`".into(),
            ));
        }
        Ok(Self { pool })
    }
}
