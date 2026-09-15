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
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
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

#[tokio::test]
async fn migration_0057_preserves_children_and_allows_one_supervisor_across_stage_attempts() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory
        .path()
        .join("workflow-supervisor-child-upgrade.db");
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
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let full = Migrator::new(migrations.as_path())
        .await
        .expect("load migrations");
    let first_fifty_six = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(56).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_fifty_six
        .run(&mut connection)
        .await
        .expect("pre-0057 schema");

    let run_id = Uuid::new_v4();
    let first_attempt_id = Uuid::new_v4();
    let generation_job_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO workflow_runs \
         (id, definition_id, state, current_stage, iteration, latest_attempt_id, \
          latest_attempt_fingerprint, cancel_requested, artifact_json, created_at, updated_at) \
         VALUES (?, ?, 'running', 'generation', 0, ?, 'sha256:first-attempt', 0, '{}', ?, ?)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(first_attempt_id)
    .bind(now)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("workflow run");
    sqlx::query(
        "INSERT INTO workflow_stage_attempts \
         (id, workflow_run_id, sequence, iteration, stage, attempt, state, predecessor_id, \
          predecessor_fingerprint, retryable, artifact_json, fingerprint, started_at, finished_at) \
         VALUES (?, ?, 0, 0, 'generation', 1, 'running', NULL, NULL, 0, '{}', \
          'sha256:first-attempt', ?, NULL)",
    )
    .bind(first_attempt_id)
    .bind(run_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("first workflow attempt");
    sqlx::query(
        "INSERT INTO workflow_child_executions \
         (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
          logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, 'generation', 1, 'generation_job', 'legacy-primary', ?, '{}', ?, ?)",
    )
    .bind(Uuid::new_v4())
    .bind(run_id)
    .bind(first_attempt_id)
    .bind(generation_job_id)
    .bind(format!("sha256:{}", Uuid::new_v4()))
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy child");

    full.run(&mut connection).await.expect("0057 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workflow_child_executions")
            .fetch_one(&mut connection)
            .await
            .expect("preserved child count"),
        1
    );

    let supervisor_run_id = Uuid::new_v4();
    let first_supervisor_link_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_child_executions \
         (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
          logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, 'generation', 2, 'generation_supervisor_run', \
          'quality-supervision', ?, '{}', ?, ?)",
    )
    .bind(first_supervisor_link_id)
    .bind(run_id)
    .bind(first_attempt_id)
    .bind(supervisor_run_id)
    .bind(format!("sha256:{}", Uuid::new_v4()))
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("first supervisor child link");

    let second_attempt_id = Uuid::new_v4();
    sqlx::query("UPDATE workflow_stage_attempts SET state = 'awaiting_user' WHERE id = ?")
        .bind(first_attempt_id)
        .execute(&mut connection)
        .await
        .expect("close first attempt");
    sqlx::query(
        "INSERT INTO workflow_stage_attempts \
         (id, workflow_run_id, sequence, iteration, stage, attempt, state, predecessor_id, \
          predecessor_fingerprint, retryable, artifact_json, fingerprint, started_at, finished_at) \
         VALUES (?, ?, 1, 0, 'generation', 2, 'running', ?, 'sha256:first-attempt', 0, '{}', \
          'sha256:second-attempt', ?, NULL)",
    )
    .bind(second_attempt_id)
    .bind(run_id)
    .bind(first_attempt_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("second workflow attempt");
    sqlx::query(
        "UPDATE workflow_runs SET latest_attempt_id = ?, latest_attempt_fingerprint = \
         'sha256:second-attempt' WHERE id = ?",
    )
    .bind(second_attempt_id)
    .bind(run_id)
    .execute(&mut connection)
    .await
    .expect("advance workflow projection");
    let second_supervisor_link_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_child_executions \
         (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
          logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, 'generation', 1, 'generation_supervisor_run', \
          'quality-supervision', ?, '{}', ?, ?)",
    )
    .bind(second_supervisor_link_id)
    .bind(run_id)
    .bind(second_attempt_id)
    .bind(supervisor_run_id)
    .bind(format!("sha256:{}", Uuid::new_v4()))
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("same supervisor linked by resumed attempt");

    assert!(
        sqlx::query(
            "INSERT INTO workflow_child_executions \
             (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
              logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, ?, 'generation', 2, 'generation_supervisor_run', \
              'different-authority', ?, '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(run_id)
        .bind(second_attempt_id)
        .bind(supervisor_run_id)
        .bind(format!("sha256:{}", Uuid::new_v4()))
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "a supervisor child cannot cross logical workflow authority"
    );
    assert!(
        sqlx::query("UPDATE workflow_child_executions SET logical_key = 'changed' WHERE id = ?")
            .bind(second_supervisor_link_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "supervisor child links must remain immutable"
    );
}
