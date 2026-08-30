use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0039_preserves_legacy_duplicates_and_guards_future_inserts() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("generation-execution-upgrade.db");
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
    let first_thirty_eight = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(38).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_thirty_eight
        .run(&mut connection)
        .await
        .expect("pre-0039 schema");

    let dataset_id = Uuid::new_v4();
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
    for source_ref in ["legacy-1", "legacy-2"] {
        insert_source_row(
            &mut connection,
            dataset_id,
            source_ref,
            "same normalized text",
            now,
        )
        .await
        .expect("legacy duplicate");
    }

    full.run(&mut connection).await.expect("0039 upgrade");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dataset_source_rows WHERE dataset_id = ?",
        )
        .bind(dataset_id)
        .fetch_one(&mut connection)
        .await
        .expect("source row count"),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dataset_normalized_text_claims WHERE dataset_id = ?",
        )
        .bind(dataset_id)
        .fetch_one(&mut connection)
        .await
        .expect("claim count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generation_execution_specs")
            .fetch_one(&mut connection)
            .await
            .expect("execution spec count"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generation_request_attempts")
            .fetch_one(&mut connection)
            .await
            .expect("attempt count"),
        0
    );

    let duplicate = insert_source_row(
        &mut connection,
        dataset_id,
        "future-duplicate",
        "same normalized text",
        now,
    )
    .await;
    assert!(duplicate.is_err(), "a future duplicate must be rejected");
    insert_source_row(
        &mut connection,
        dataset_id,
        "future-unique",
        "different normalized text",
        now,
    )
    .await
    .expect("new unique source row");
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}

async fn insert_source_row(
    connection: &mut SqliteConnection,
    dataset_id: Uuid,
    source_ref: &str,
    normalized_text: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, \
          label, dimensions_json, provenance_json, created_at) \
         VALUES (?, ?, 'imported', ?, 'label=a', 'text', ?, 'a', '{}', '{}', ?)",
    )
    .bind(Uuid::new_v4())
    .bind(dataset_id)
    .bind(source_ref)
    .bind(normalized_text)
    .bind(created_at)
    .execute(connection)
    .await?;
    Ok(())
}
