use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0050_preserves_workflows_and_fences_child_execution_links() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("workflow-child-execution-upgrade.db");
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
    let first_forty_nine = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(49).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_nine
        .run(&mut connection)
        .await
        .expect("pre-0050 schema");

    let run_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO workflow_runs \
         (id, definition_id, state, current_stage, iteration, latest_attempt_id, \
          latest_attempt_fingerprint, cancel_requested, artifact_json, created_at, updated_at) \
         VALUES (?, ?, 'running', 'generation', 0, ?, 'sha256:attempt', 0, '{}', ?, ?)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(attempt_id)
    .bind(now)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy run");
    sqlx::query(
        "INSERT INTO workflow_stage_attempts \
         (id, workflow_run_id, sequence, iteration, stage, attempt, state, predecessor_id, \
          predecessor_fingerprint, retryable, artifact_json, fingerprint, started_at, finished_at) \
         VALUES (?, ?, 0, 0, 'generation', 1, 'running', NULL, NULL, 0, '{}', \
          'sha256:attempt', ?, NULL)",
    )
    .bind(attempt_id)
    .bind(run_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy attempt");

    full.run(&mut connection).await.expect("0050 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workflow_runs")
            .fetch_one(&mut connection)
            .await
            .expect("run count"),
        1
    );

    let link_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_child_executions \
         (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
          logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, 'generation', 1, 'generation_job', 'primary', ?, '{}', ?, ?)",
    )
    .bind(link_id)
    .bind(run_id)
    .bind(attempt_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", Uuid::new_v4()))
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("first child link");

    assert!(
        sqlx::query("UPDATE workflow_child_executions SET logical_key = 'changed' WHERE id = ?")
            .bind(link_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "child links must be immutable"
    );
    assert!(
        sqlx::query(
            "INSERT INTO workflow_child_executions \
             (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
              logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, ?, 'generation', 3, 'generation_job', 'replacement', ?, '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(run_id)
        .bind(attempt_id)
        .bind(Uuid::new_v4())
        .bind(format!("sha256:{}", Uuid::new_v4()))
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "child ordinals cannot skip"
    );
    assert!(
        sqlx::query(
            "INSERT INTO workflow_child_executions \
             (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
              logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, ?, 'generation', 2, 'training_run', 'wrong-kind', ?, '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(run_id)
        .bind(attempt_id)
        .bind(Uuid::new_v4())
        .bind(format!("sha256:{}", Uuid::new_v4()))
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "a child kind cannot cross stage ownership"
    );
}
