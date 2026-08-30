use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0038_preserves_existing_jobs_and_adds_empty_semantic_history() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("semantic-upgrade.db");
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
    let first_thirty_seven = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(37).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_thirty_seven
        .run(&mut connection)
        .await
        .expect("pre-0038 schema");
    let dataset_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy', 'classify', '[\"a\"]', '[]', ?)",
    )
    .bind(dataset_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("dataset");
    sqlx::query(
        "INSERT INTO generation_plans (id, dataset_id, cells_json, created_at) \
         VALUES (?, ?, '[]', ?)",
    )
    .bind(plan_id)
    .bind(dataset_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("plan");
    sqlx::query(
        "INSERT INTO generation_jobs \
         (id, dataset_id, plan_id, backend_name, backend_model, state, requested_rows, \
          generated_rows, accepted_rows, rejected_rows, failed_requests, cancel_requested, \
          created_at, updated_at) \
         VALUES (?, ?, ?, 'fake', 'v1', 'completed', 0, 0, 0, 0, 0, 0, ?, ?)",
    )
    .bind(job_id)
    .bind(dataset_id)
    .bind(plan_id)
    .bind(now)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("job");

    full.run(&mut connection).await.expect("0038 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generation_jobs WHERE id = ?")
            .bind(job_id)
            .fetch_one(&mut connection)
            .await
            .expect("job count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM semantic_profiles")
            .fetch_one(&mut connection)
            .await
            .expect("profile count"),
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
