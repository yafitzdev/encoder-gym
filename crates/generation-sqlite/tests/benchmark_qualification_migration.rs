use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migrations_0051_and_0052_preserve_bundles_and_guard_qualification_history() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("benchmark-qualification-upgrade.db");
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
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let full = Migrator::new(migrations.as_path())
        .await
        .expect("load migrations");
    let first_fifty = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(50).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_fifty
        .run(&mut connection)
        .await
        .expect("pre-0051 schema");

    let report_id = Uuid::new_v4();
    let suite_id = Uuid::new_v4();
    let bundle_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO workflow_contamination_reports \
         (id, status, cohort_ids_json, artifact_json, fingerprint, created_at) \
         VALUES (?, 'clean', '[]', '{}', ?, ?)",
    )
    .bind(report_id)
    .bind("sha256:legacy-report")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("contamination report");
    sqlx::query(
        "INSERT INTO workflow_benchmark_suites \
         (id, name, kind, contamination_report_id, contamination_override_fingerprint, \
          artifact_json, fingerprint, created_at) \
         VALUES (?, 'development', 'development', ?, NULL, '{}', ?, ?)",
    )
    .bind(suite_id)
    .bind(report_id)
    .bind("sha256:legacy-suite")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("suite");
    sqlx::query(
        "INSERT INTO workflow_benchmark_bundles \
         (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, NULL, ?, '{}', ?, ?)",
    )
    .bind(bundle_id)
    .bind(suite_id)
    .bind(report_id)
    .bind("sha256:legacy-bundle")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy bundle");

    full.run(&mut connection).await.expect("0051 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workflow_benchmark_bundles")
            .fetch_one(&mut connection)
            .await
            .expect("bundle count"),
        1
    );

    let qualification_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualifications \
         (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, 'benchmark-readiness-v1', 'ready', ?, '{}', ?, ?)",
    )
    .bind(qualification_id)
    .bind(bundle_id)
    .bind("sha256:policy")
    .bind("sha256:qualification")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("qualification");

    assert!(
        sqlx::query(
            "INSERT INTO workflow_benchmark_qualifications \
             (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, artifact_json, \
              fingerprint, created_at) VALUES (?, ?, 'benchmark-readiness-v1', 'ready', ?, '{}', ?, ?)"
        )
        .bind(Uuid::new_v4())
        .bind(bundle_id)
        .bind("sha256:policy")
        .bind("sha256:other-qualification")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "one bundle/policy/protocol calculation must be canonical"
    );
    assert!(
        sqlx::query(
            "UPDATE workflow_benchmark_qualifications SET readiness = 'blocked' WHERE id = ?"
        )
        .bind(qualification_id)
        .execute(&mut connection)
        .await
        .is_err(),
        "qualification artifacts must be append-only"
    );
    assert!(
        sqlx::query("DELETE FROM workflow_benchmark_qualifications WHERE id = ?")
            .bind(qualification_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "qualification artifacts must not be deleted"
    );
    let review_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualification_reviews \
         (id, qualification_id, decision, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, 'approve', '{}', ?, ?)",
    )
    .bind(review_id)
    .bind(qualification_id)
    .bind("sha256:review")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("qualification review");
    assert!(
        sqlx::query(
            "INSERT INTO workflow_benchmark_qualification_reviews \
             (id, qualification_id, decision, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, 'reject', '{}', ?, ?)"
        )
        .bind(Uuid::new_v4())
        .bind(qualification_id)
        .bind("sha256:second-review")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "one explicit decision must close a qualification review"
    );
    assert!(
        sqlx::query(
            "UPDATE workflow_benchmark_qualification_reviews SET decision = 'reject' WHERE id = ?"
        )
        .bind(review_id)
        .execute(&mut connection)
        .await
        .is_err(),
        "qualification reviews must be append-only"
    );
    assert!(
        sqlx::query("DELETE FROM workflow_benchmark_qualification_reviews WHERE id = ?")
            .bind(review_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "qualification reviews must not be deleted"
    );
    assert!(
        sqlx::query(
            "INSERT INTO workflow_benchmark_qualifications \
             (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, artifact_json, \
              fingerprint, created_at) VALUES (?, ?, 'benchmark-readiness-v1', 'unknown', ?, '{}', ?, ?)"
        )
        .bind(Uuid::new_v4())
        .bind(bundle_id)
        .bind("sha256:another-policy")
        .bind("sha256:bad-readiness")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "readiness must use the closed state vocabulary"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}

#[tokio::test]
async fn migration_0053_preserves_legacy_workflows_and_guards_qualification_authority() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("workflow-qualification-upgrade.db");
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
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let full = Migrator::new(migrations.as_path())
        .await
        .expect("load migrations");
    let first_fifty_two = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(52).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_fifty_two
        .run(&mut connection)
        .await
        .expect("pre-0053 schema");

    let now = chrono::Utc::now();
    let dataset_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let configuration_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let suite_id = Uuid::new_v4();
    let bundle_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, 'legacy', 'classify', '[\"a\",\"b\"]', '[]', ?)",
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
        "INSERT INTO project_configurations \
         (id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, created_at) \
         VALUES (?, ?, ?, ?, '{}', ?)",
    )
    .bind(configuration_id)
    .bind("sha256:configuration")
    .bind(dataset_id)
    .bind(plan_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("configuration");
    sqlx::query(
        "INSERT INTO workflow_contamination_reports \
         (id, status, cohort_ids_json, artifact_json, fingerprint, created_at) \
         VALUES (?, 'clean', '[]', '{}', ?, ?)",
    )
    .bind(report_id)
    .bind("sha256:report")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("report");
    sqlx::query(
        "INSERT INTO workflow_benchmark_suites \
         (id, name, kind, contamination_report_id, contamination_override_fingerprint, \
          artifact_json, fingerprint, created_at) \
         VALUES (?, 'development', 'development', ?, NULL, '{}', ?, ?)",
    )
    .bind(suite_id)
    .bind(report_id)
    .bind("sha256:suite")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("suite");
    sqlx::query(
        "INSERT INTO workflow_benchmark_bundles \
         (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, NULL, ?, '{}', ?, ?)",
    )
    .bind(bundle_id)
    .bind(suite_id)
    .bind(report_id)
    .bind("sha256:bundle")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("bundle");
    sqlx::query(
        "INSERT INTO workflow_definitions \
         (id, name, dataset_id, project_configuration_id, development_suite_id, \
          sealed_suite_id, benchmark_bundle_id, artifact_json, fingerprint, created_at) \
         VALUES (?, 'legacy workflow', ?, ?, ?, NULL, ?, '{}', ?, ?)",
    )
    .bind(workflow_id)
    .bind(dataset_id)
    .bind(configuration_id)
    .bind(suite_id)
    .bind(bundle_id)
    .bind("sha256:workflow")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy workflow");

    let qualification_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualifications \
         (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, 'benchmark-readiness-v1', 'ready', ?, '{}', ?, ?)",
    )
    .bind(qualification_id)
    .bind(bundle_id)
    .bind("sha256:policy")
    .bind("sha256:qualification")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("qualification");
    let review_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualification_reviews \
         (id, qualification_id, decision, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, 'approve', '{}', ?, ?)",
    )
    .bind(review_id)
    .bind(qualification_id)
    .bind("sha256:review")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("review");

    full.run(&mut connection).await.expect("0053 upgrade");
    let legacy_binding: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT benchmark_qualification_id, benchmark_qualification_review_id \
         FROM workflow_definitions WHERE id = ?",
    )
    .bind(workflow_id)
    .fetch_one(&mut connection)
    .await
    .expect("legacy binding");
    assert_eq!(legacy_binding, (None, None));

    assert!(
        sqlx::query("UPDATE workflow_definitions SET benchmark_qualification_id = ? WHERE id = ?",)
            .bind(qualification_id)
            .bind(workflow_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "partial authority must be rejected"
    );
    sqlx::query(
        "UPDATE workflow_definitions SET benchmark_qualification_id = ?, \
         benchmark_qualification_review_id = ? WHERE id = ?",
    )
    .bind(qualification_id)
    .bind(review_id)
    .bind(workflow_id)
    .execute(&mut connection)
    .await
    .expect("bind approved authority");
    assert!(
        sqlx::query(
            "UPDATE workflow_definitions SET benchmark_qualification_id = NULL, \
             benchmark_qualification_review_id = NULL WHERE id = ?",
        )
        .bind(workflow_id)
        .execute(&mut connection)
        .await
        .is_err(),
        "bound authority must not be removable"
    );

    let blocked_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualifications \
         (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, 'benchmark-readiness-v1', 'blocked', ?, '{}', ?, ?)",
    )
    .bind(blocked_id)
    .bind(bundle_id)
    .bind("sha256:blocked-policy")
    .bind("sha256:blocked-qualification")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("blocked qualification");
    let blocked_review_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_qualification_reviews \
         (id, qualification_id, decision, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, 'reject', '{}', ?, ?)",
    )
    .bind(blocked_review_id)
    .bind(blocked_id)
    .bind("sha256:blocked-review")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("blocked review");
    let second_workflow_id = Uuid::new_v4();
    assert!(
        sqlx::query(
            "INSERT INTO workflow_definitions \
             (id, name, dataset_id, project_configuration_id, development_suite_id, \
              sealed_suite_id, benchmark_bundle_id, benchmark_qualification_id, \
              benchmark_qualification_review_id, artifact_json, fingerprint, created_at) \
             VALUES (?, 'blocked workflow', ?, ?, ?, NULL, ?, ?, ?, '{}', ?, ?)",
        )
        .bind(second_workflow_id)
        .bind(dataset_id)
        .bind(configuration_id)
        .bind(suite_id)
        .bind(bundle_id)
        .bind(blocked_id)
        .bind(blocked_review_id)
        .bind("sha256:blocked-workflow")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "blocked and rejected authority must not bind"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}
