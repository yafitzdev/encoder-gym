use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0040_backfills_empty_fields_without_losing_legacy_rows() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("hybrid-construction-upgrade.db");
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
    let first_thirty_nine = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(39).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_thirty_nine
        .run(&mut connection)
        .await
        .expect("pre-0040 schema");

    let dataset_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let row_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let member_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy', 'classify', '[\"billing\"]', '[]', ?)",
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
         VALUES (?, ?, ?, 'fake', 'v1', 'completed', 1, 1, 1, 0, 0, 0, ?, ?)",
    )
    .bind(job_id)
    .bind(dataset_id)
    .bind(plan_id)
    .bind(now)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("job");
    sqlx::query(
        "INSERT INTO generated_rows \
         (id, dataset_id, plan_id, generation_job_id, cell_key, text, normalized_text, \
          label, dimensions_json, generator_backend, generator_model, created_at, \
          validation_status, validation_errors_json, generation_metadata_json) \
         VALUES (?, ?, ?, ?, 'label=billing', 'legacy row', 'legacy row', 'billing', '{}', \
                 'fake', 'v1', ?, 'accepted', '[]', '{}')",
    )
    .bind(row_id)
    .bind(dataset_id)
    .bind(plan_id)
    .bind(job_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("generated row");
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) \
         VALUES (?, ?, 'generated', ?, 'label=billing', 'legacy row', 'legacy row', \
                 'billing', '{}', '{}', ?)",
    )
    .bind(row_id)
    .bind(dataset_id)
    .bind(row_id.to_string())
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("source row");
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, split_configuration_json, member_count, created_at) \
         VALUES (?, ?, 'legacy', '{}', 1, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("snapshot");
    sqlx::query(
        "INSERT INTO dataset_snapshot_members \
         (id, snapshot_id, source_row_id, split, text, label, dimensions_json, \
          source_provenance_json, source_created_at) \
         VALUES (?, ?, ?, 'train', 'legacy row', 'billing', '{}', '{}', ?)",
    )
    .bind(member_id)
    .bind(snapshot_id)
    .bind(row_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("snapshot member");

    full.run(&mut connection).await.expect("0040 upgrade");

    for table in [
        "generated_rows",
        "dataset_source_rows",
        "dataset_snapshot_members",
    ] {
        let query = format!("SELECT fields_json FROM {table} LIMIT 1");
        assert_eq!(
            sqlx::query_scalar::<_, String>(&query)
                .fetch_one(&mut connection)
                .await
                .expect("backfilled field value"),
            "{}"
        );
    }
    let construction: Option<String> =
        sqlx::query_scalar("SELECT construction_json FROM generated_rows WHERE id = ?")
            .bind(row_id)
            .fetch_one(&mut connection)
            .await
            .expect("construction column");
    assert!(construction.is_none());
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}
