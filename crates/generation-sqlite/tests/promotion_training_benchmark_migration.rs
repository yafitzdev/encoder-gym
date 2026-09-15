use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    ports::PromotionStore,
    promotion::{ModelPromotion, PromotionState},
};

#[tokio::test]
async fn migration_0048_preserves_legacy_promotions_without_fabricating_clearance() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("promotion-clearance-upgrade.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let options = SqliteConnectOptions::from_str(&url)
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
    let first_forty_seven = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(47).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_seven
        .run(&mut connection)
        .await
        .expect("pre-0048 schema");

    let now = chrono::Utc::now();
    let mut promotion = ModelPromotion {
        id: Uuid::new_v4(),
        workflow_run_id: Uuid::new_v4(),
        checkpoint_id: Uuid::new_v4(),
        checkpoint_fingerprint: "sha256:checkpoint".into(),
        training_snapshot_id: Uuid::new_v4(),
        training_snapshot_fingerprint: "sha256:snapshot".into(),
        training_benchmark_check_id: None,
        training_benchmark_check_fingerprint: None,
        development_assessment_id: Uuid::new_v4(),
        development_assessment_fingerprint: "sha256:development".into(),
        sealed_assessment_id: Uuid::new_v4(),
        sealed_assessment_fingerprint: "sha256:sealed".into(),
        development_suite_id: Uuid::new_v4(),
        development_suite_fingerprint: "sha256:development-suite".into(),
        sealed_suite_id: Uuid::new_v4(),
        sealed_suite_fingerprint: "sha256:sealed-suite".into(),
        policy_fingerprint: "sha256:policy".into(),
        state: PromotionState::Promoted,
        reason: "sealed acceptance contract passed".into(),
        created_at: now,
        fingerprint: String::new(),
    };
    promotion.fingerprint = promotion
        .reproduce_fingerprint()
        .expect("legacy promotion fingerprint");
    sqlx::query(
        "INSERT INTO model_promotions \
         (id, workflow_run_id, checkpoint_id, training_snapshot_id, development_assessment_id, \
          sealed_assessment_id, state, fingerprint, artifact_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'promoted', ?, ?, ?)",
    )
    .bind(promotion.id)
    .bind(promotion.workflow_run_id)
    .bind(promotion.checkpoint_id)
    .bind(promotion.training_snapshot_id)
    .bind(promotion.development_assessment_id)
    .bind(promotion.sealed_assessment_id)
    .bind(&promotion.fingerprint)
    .bind(serde_json::to_string(&promotion).expect("promotion JSON"))
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy promotion row");

    full.run(&mut connection)
        .await
        .expect("0048 and 0049 upgrade");
    let projection: (Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT training_benchmark_check_id, training_benchmark_check_fingerprint \
         FROM model_promotions WHERE id = ?",
    )
    .bind(promotion.id)
    .fetch_one(&mut connection)
    .await
    .expect("legacy projection");
    assert_eq!(projection, (None, None));
    assert!(
        sqlx::query(
            "UPDATE model_promotions SET training_benchmark_check_id = ?, \
             training_benchmark_check_fingerprint = 'sha256:retrofit' WHERE id = ?",
        )
        .bind(Uuid::new_v4())
        .bind(promotion.id)
        .execute(&mut connection)
        .await
        .is_err(),
        "a legacy promotion cannot be retroactively blessed"
    );
    assert!(
        sqlx::query(
            "INSERT INTO model_promotions \
             (id, workflow_run_id, checkpoint_id, training_snapshot_id, \
              development_assessment_id, sealed_assessment_id, state, fingerprint, \
              artifact_json, created_at) \
             SELECT ?, ?, checkpoint_id, training_snapshot_id, development_assessment_id, \
              sealed_assessment_id, state, fingerprint, artifact_json, created_at \
             FROM model_promotions WHERE id = ?",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(promotion.id)
        .execute(&mut connection)
        .await
        .is_err(),
        "post-migration promotions cannot masquerade as legacy rows"
    );
    drop(connection);

    let store = SqliteStore::connect(&url).await.expect("store reconnects");
    assert_eq!(
        store
            .get_promotion(promotion.id)
            .await
            .expect("legacy promotion read"),
        Some(promotion)
    );
}
