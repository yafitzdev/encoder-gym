use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0043_preserves_existing_generation_and_adds_architect_history() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("architect-upgrade.db");
    let options = SqliteConnectOptions::from_str(&format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    ))
    .unwrap()
    .create_if_missing(true)
    .foreign_keys(false);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let full = Migrator::new(migrations.as_path()).await.unwrap();
    let first_forty_two = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(42).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_two.run(&mut connection).await.unwrap();
    let dataset_id = Uuid::new_v4();
    sqlx::query("INSERT INTO dataset_definitions (id, name, task_description, labels_json, dimensions_json, created_at) VALUES (?, 'legacy', 'classify', '[\"a\"]', '[]', ?)")
        .bind(dataset_id).bind(chrono::Utc::now()).execute(&mut connection).await.unwrap();

    full.run(&mut connection).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_definitions WHERE id = ?")
            .bind(dataset_id)
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_architect_runs")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generation_job_strategies")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .unwrap()
            .is_empty()
    );
}
