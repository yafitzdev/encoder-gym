use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migrations_0041_and_0042_preserve_existing_data_and_add_research_history() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("research-upgrade.db");
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
    let first_forty = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(40).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty
        .run(&mut connection)
        .await
        .expect("pre-0041 schema");
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

    full.run(&mut connection).await.expect("0041 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_definitions WHERE id = ?")
            .bind(dataset_id)
            .fetch_one(&mut connection)
            .await
            .expect("dataset count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM research_runs")
            .fetch_one(&mut connection)
            .await
            .expect("research count"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generation_job_authenticity")
            .fetch_one(&mut connection)
            .await
            .expect("generation authenticity count"),
        0
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}
