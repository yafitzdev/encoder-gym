use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0037_preserves_existing_projects_and_adds_bootstrap_history() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("bootstrap-upgrade.db");
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
    let first_thirty_six = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(36).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_thirty_six
        .run(&mut connection)
        .await
        .expect("pre-0037 schema");
    let dataset_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy project', 'classify', '[\"a\",\"b\"]', '[]', ?)",
    )
    .bind(dataset_id)
    .bind(chrono::Utc::now())
    .execute(&mut connection)
    .await
    .expect("legacy dataset");

    full.run(&mut connection).await.expect("0037 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_definitions WHERE id = ?")
            .bind(dataset_id)
            .fetch_one(&mut connection)
            .await
            .expect("legacy dataset count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM project_bootstraps")
            .fetch_one(&mut connection)
            .await
            .expect("bootstrap count"),
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
