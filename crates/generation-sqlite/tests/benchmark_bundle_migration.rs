use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0046_preserves_legacy_workflows_and_adds_bundle_authority() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("benchmark-bundle-upgrade.db");
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
    let first_forty_five = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(45).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_five
        .run(&mut connection)
        .await
        .expect("pre-0046 schema");

    let dataset_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let configuration_id = Uuid::new_v4();
    let report_id = Uuid::new_v4();
    let development_suite_id = Uuid::new_v4();
    let workflow_definition_id = Uuid::new_v4();
    let preparation_id = Uuid::new_v4();
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
        "INSERT INTO generation_plans (id, dataset_id, cells_json, created_at) \
         VALUES (?, ?, '[]', ?)",
    )
    .bind(plan_id)
    .bind(dataset_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy plan");
    sqlx::query(
        "INSERT INTO project_configurations \
         (id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, created_at) \
         VALUES (?, ?, ?, ?, '{}', ?)",
    )
    .bind(configuration_id)
    .bind("sha256:legacy-configuration")
    .bind(dataset_id)
    .bind(plan_id)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy configuration");
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
    .expect("legacy report");
    sqlx::query(
        "INSERT INTO workflow_benchmark_suites \
         (id, name, kind, contamination_report_id, contamination_override_fingerprint, \
          artifact_json, fingerprint, created_at) \
         VALUES (?, 'development', 'development', ?, NULL, '{}', ?, ?)",
    )
    .bind(development_suite_id)
    .bind(report_id)
    .bind("sha256:legacy-suite")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy suite");
    sqlx::query(
        "INSERT INTO workflow_definitions \
         (id, name, dataset_id, project_configuration_id, development_suite_id, \
          sealed_suite_id, artifact_json, fingerprint, created_at) \
         VALUES (?, 'legacy workflow', ?, ?, ?, NULL, '{}', ?, ?)",
    )
    .bind(workflow_definition_id)
    .bind(dataset_id)
    .bind(configuration_id)
    .bind(development_suite_id)
    .bind("sha256:legacy-workflow")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy workflow");
    sqlx::query(
        "INSERT INTO project_preparations \
         (id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
          development_suite_id, sealed_suite_id, workflow_definition_id, artifact_json, \
          fingerprint, created_at) \
         VALUES (?, 'legacy preparation', ?, ?, ?, ?, NULL, ?, '{}', ?, ?)",
    )
    .bind(preparation_id)
    .bind("sha256:legacy-manifest")
    .bind(dataset_id)
    .bind(configuration_id)
    .bind(development_suite_id)
    .bind(workflow_definition_id)
    .bind("sha256:legacy-preparation")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy preparation");

    full.run(&mut connection).await.expect("0046 upgrade");

    assert_eq!(
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT benchmark_bundle_id FROM workflow_definitions WHERE id = ?",
        )
        .bind(workflow_definition_id)
        .fetch_one(&mut connection)
        .await
        .expect("legacy workflow bundle"),
        None
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT benchmark_bundle_id FROM project_preparations WHERE id = ?",
        )
        .bind(preparation_id)
        .fetch_one(&mut connection)
        .await
        .expect("legacy preparation bundle"),
        None
    );

    let bundle_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflow_benchmark_bundles \
         (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, NULL, ?, '{}', ?, ?)",
    )
    .bind(bundle_id)
    .bind(development_suite_id)
    .bind(report_id)
    .bind("sha256:bundle")
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("benchmark bundle");
    sqlx::query("UPDATE workflow_definitions SET benchmark_bundle_id = ? WHERE id = ?")
        .bind(bundle_id)
        .bind(workflow_definition_id)
        .execute(&mut connection)
        .await
        .expect("bind workflow");
    sqlx::query("UPDATE project_preparations SET benchmark_bundle_id = ? WHERE id = ?")
        .bind(bundle_id)
        .bind(preparation_id)
        .execute(&mut connection)
        .await
        .expect("bind preparation");

    assert!(
        sqlx::query(
            "INSERT INTO workflow_benchmark_bundles \
             (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
              fingerprint, created_at) VALUES (?, ?, ?, ?, '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(development_suite_id)
        .bind(development_suite_id)
        .bind(report_id)
        .bind("sha256:duplicate-suite-bundle")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "development and sealed suite identities must be distinct"
    );
    assert!(
        sqlx::query(
            "INSERT INTO workflow_benchmark_bundles \
             (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
              fingerprint, created_at) VALUES (?, ?, NULL, ?, '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(report_id)
        .bind("sha256:orphan-bundle")
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "bundle suite references must resolve"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut connection)
            .await
            .expect("foreign key check")
            .is_empty()
    );
}
