use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0044_preserves_existing_data_and_adds_quality_history() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("quality-upgrade.db");
    let options = SqliteConnectOptions::from_str(&format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    ))
    .expect("database URL")
    .create_if_missing(true)
    .foreign_keys(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .expect("connect");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let full = Migrator::new(migrations.as_path())
        .await
        .expect("load migrations");
    let first_forty_three = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(43).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_three
        .run(&mut connection)
        .await
        .expect("pre-0044 schema");

    let dataset_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy', 'classify', '[\"a\"]', '[]', ?)",
    )
    .bind(dataset_id)
    .bind(chrono::Utc::now())
    .execute(&mut connection)
    .await
    .expect("legacy dataset");

    full.run(&mut connection).await.expect("0044 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_definitions WHERE id = ?")
            .bind(dataset_id)
            .fetch_one(&mut connection)
            .await
            .expect("dataset count"),
        1
    );
    for table in [
        "dataset_quality_audit_plans",
        "dataset_quality_audit_guidance",
        "dataset_quality_audit_runs",
        "dataset_quality_reports",
        "dataset_curation_manifests",
        "dataset_curation_applications",
    ] {
        let count = sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut connection)
            .await
            .unwrap_or_else(|error| panic!("query {table}: {error}"));
        assert_eq!(count, 0, "{table} should start empty");
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}
