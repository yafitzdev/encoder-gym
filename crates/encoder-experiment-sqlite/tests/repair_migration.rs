use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn repair_migration_preserves_campaign_history_and_adds_append_only_evidence_tables() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("repair-upgrade.db");
    let options = SqliteConnectOptions::from_str(&format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    ))
    .unwrap()
    .create_if_missing(true)
    .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let full = Migrator::new(migrations.as_path()).await.unwrap();
    let previous = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(2).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    previous.run(&mut connection).await.unwrap();
    let project_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO encoder_experiment_projects \
         (id, fingerprint, source_fingerprint, artifact_json, created_at) \
         VALUES (?, ?, ?, '{}', '2026-09-02T00:00:00Z')",
    )
    .bind(project_id)
    .bind("sha256:legacy-project")
    .bind("sha256:legacy-source")
    .execute(&mut connection)
    .await
    .unwrap();
    full.run(&mut connection).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM encoder_experiment_projects")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
    for table in [
        "encoder_repair_observation_sets",
        "encoder_repair_diagnoses",
        "encoder_repair_diagnosis_observation_sets",
        "encoder_repair_proposals",
        "encoder_repair_proposal_reviews",
        "encoder_repair_proposal_applications",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(exists, 1, "missing table {table}");
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .unwrap()
            .is_empty()
    );
}
