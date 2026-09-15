use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0045_preserves_exposures_and_allows_dataset_architecture() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("architecture-exposure-upgrade.db");
    let options = SqliteConnectOptions::from_str(&format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    ))
    .expect("database URL")
    .create_if_missing(true)
    .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .expect("connect");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let full = Migrator::new(migrations.as_path())
        .await
        .expect("load migrations");
    let first_forty_four = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(44).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_four
        .run(&mut connection)
        .await
        .expect("pre-0045 schema");

    let dataset_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let cohort_id = Uuid::new_v4();
    let role_id = Uuid::new_v4();
    let legacy_exposure_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy', 'classify', '[\"a\",\"b\"]', '[]', ?)",
    )
    .bind(dataset_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy dataset");
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, member_count, \
          fingerprint, created_at) VALUES (?, ?, 'legacy snapshot', NULL, '{}', 1, ?, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset_id)
    .bind("sha256:legacy-snapshot")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy snapshot");
    sqlx::query(
        "INSERT INTO workflow_evaluation_cohorts \
         (id, snapshot_id, split, origin, name, fingerprint, artifact_json, created_at) \
         VALUES (?, ?, 'test', 'internal_snapshot', 'legacy cohort', ?, '{}', ?)",
    )
    .bind(cohort_id)
    .bind(snapshot_id)
    .bind("sha256:legacy-cohort")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy cohort");
    sqlx::query(
        "INSERT INTO workflow_cohort_role_decisions \
         (id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
          artifact_json, created_at) \
         VALUES (?, ?, 0, 'development', 'active', NULL, ?, '{}', ?)",
    )
    .bind(role_id)
    .bind(cohort_id)
    .bind("sha256:legacy-role")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy role");
    insert_exposure(
        &mut connection,
        legacy_exposure_id,
        cohort_id,
        role_id,
        None,
        "optimization",
        now,
    )
    .await
    .expect("legacy exposure");

    full.run(&mut connection).await.expect("0045 upgrade");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_evidence_exposures WHERE id = ? AND purpose = 'optimization'",
        )
        .bind(legacy_exposure_id)
        .fetch_one(&mut connection)
        .await
        .expect("legacy exposure count"),
        1
    );
    insert_exposure(
        &mut connection,
        Uuid::new_v4(),
        cohort_id,
        role_id,
        None,
        "dataset_architecture",
        now,
    )
    .await
    .expect("dataset architecture exposure");
    assert!(
        insert_exposure(
            &mut connection,
            Uuid::new_v4(),
            cohort_id,
            role_id,
            None,
            "unbounded_agent",
            now,
        )
        .await
        .is_err(),
        "the rebuilt purpose check must remain closed to unknown values"
    );
    assert!(
        insert_exposure(
            &mut connection,
            Uuid::new_v4(),
            cohort_id,
            role_id,
            Some(Uuid::new_v4()),
            "dataset_architecture",
            now,
        )
        .await
        .is_err(),
        "workflow exposure references must resolve after the rebuild"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}

async fn insert_exposure(
    connection: &mut SqliteConnection,
    id: Uuid,
    cohort_id: Uuid,
    role_id: Uuid,
    workflow_run_id: Option<Uuid>,
    purpose: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workflow_evidence_exposures \
         (id, cohort_id, role_decision_id, evaluation_run_id, workflow_run_id, \
          workflow_iteration, purpose, disclosure, adaptation_eligible, requires_retirement, \
          fingerprint, artifact_json, created_at) \
         VALUES (?, ?, ?, NULL, ?, NULL, ?, 'slices', 1, 0, ?, '{}', ?)",
    )
    .bind(id)
    .bind(cohort_id)
    .bind(role_id)
    .bind(workflow_run_id)
    .bind(purpose)
    .bind(format!("sha256:{id}"))
    .bind(created_at)
    .execute(connection)
    .await
    .map(|_| ())
}
