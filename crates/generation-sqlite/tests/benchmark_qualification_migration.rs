use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0051_preserves_bundles_and_guards_immutable_qualifications() {
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
