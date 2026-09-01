use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migrations_0055_and_0056_preserve_existing_data_and_add_supervisor_ledgers() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("generation-supervisor-upgrade.db");
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
    let first_fifty_four = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(54).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_fifty_four
        .run(&mut connection)
        .await
        .expect("pre-0055 schema");

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

    full.run(&mut connection).await.expect("0055 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_definitions WHERE id = ?")
            .bind(dataset_id)
            .fetch_one(&mut connection)
            .await
            .expect("dataset count"),
        1
    );
    for table in [
        "generation_quality_contracts",
        "generation_supervisor_runs",
        "generation_supervisor_run_events",
        "generation_supervisor_prompt_versions",
        "generation_supervisor_strategy_sets",
        "generation_supervisor_strategy_assignments",
        "generation_supervisor_child_reservations",
        "generation_supervisor_child_outcomes",
        "generation_supervisor_row_observations",
        "generation_supervisor_quality_manifests",
        "generation_supervisor_quality_windows",
        "generation_supervisor_decisions",
        "generation_supervisor_advisor_sessions",
        "generation_supervisor_model_calls",
        "generation_supervisor_tool_calls",
        "generation_supervisor_revision_proposals",
        "generation_supervisor_revision_reviews",
        "generation_supervisor_revision_authorizations",
        "generation_supervisor_revision_activations",
        "generation_supervisor_qualification_handoffs",
        "generation_supervisor_qualification_entries",
        "generation_supervisor_qualification_applications",
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
