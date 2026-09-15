use std::{borrow::Cow, path::PathBuf, str::FromStr};

use analysis_core::domain::AnalysisReport;
use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn migration_0017_keeps_legacy_reports_decodable_and_backfills_findings() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("upgrade.db");
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
    let first_sixteen = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(16).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_sixteen
        .run(&mut connection)
        .await
        .expect("pre-0017 schema");

    let report_id = Uuid::new_v4();
    let evaluation_run_id = Uuid::new_v4();
    let created_at = chrono::Utc::now();
    let legacy_key = "label=billing/style=messy";
    let report_json = serde_json::json!({
        "id": report_id,
        "evaluation_run_id": evaluation_run_id,
        "minimum_support": 1,
        "prediction_count": 2,
        "error_count": 1,
        "findings": [{
            "rank": 1,
            "kind": "cell",
            "key": legacy_key,
            "attributes": {"label": "billing", "style": "messy"},
            "support": 2,
            "error_count": 1,
            "error_rate": 0.5
        }],
        "errors": [],
        "created_at": created_at
    });
    sqlx::query(
        "INSERT INTO analysis_reports \
         (id, evaluation_run_id, minimum_support, prediction_count, error_count, \
          report_json, fingerprint, created_at) VALUES (?, ?, 1, 2, 1, ?, ?, ?)",
    )
    .bind(report_id)
    .bind(evaluation_run_id)
    .bind(report_json.to_string())
    .bind("sha256:legacy")
    .bind(created_at)
    .execute(&mut connection)
    .await
    .expect("legacy report");

    full.run(&mut connection).await.expect("0017 upgrade");
    let normalized: (String, String) = sqlx::query_as(
        "SELECT finding_key, identity_json FROM analysis_findings \
         WHERE analysis_report_id = ?",
    )
    .bind(report_id)
    .fetch_one(&mut connection)
    .await
    .expect("backfilled finding");
    assert_eq!(normalized.0, legacy_key);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&normalized.1).expect("identity")["kind"],
        "cell"
    );
    let stored: String =
        sqlx::query_scalar("SELECT report_json FROM analysis_reports WHERE id = ?")
            .bind(report_id)
            .fetch_one(&mut connection)
            .await
            .expect("stored report");
    let decoded: AnalysisReport = serde_json::from_str(&stored).expect("legacy defaults decode");
    assert_eq!(decoded.id, report_id);
    assert!(decoded.protocol_fingerprint.is_empty());
    assert!(decoded.comparison_diagnosis.is_none());
}
