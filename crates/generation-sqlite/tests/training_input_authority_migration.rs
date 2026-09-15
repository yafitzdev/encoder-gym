use std::{borrow::Cow, path::PathBuf, str::FromStr};

use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use training_core::domain::TrainingConfiguration;
use uuid::Uuid;

type InputAuthorityProjection = (
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<Uuid>,
    Option<String>,
);

#[tokio::test]
async fn migration_0049_preserves_legacy_runs_and_fences_new_input_authority() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("training-input-authority-upgrade.db");
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
    let first_forty_eight = Migrator {
        migrations: Cow::Owned(full.migrations.iter().take(48).cloned().collect()),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    };
    first_forty_eight
        .run(&mut connection)
        .await
        .expect("pre-0049 schema");

    let legacy_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let configuration =
        serde_json::to_string(&TrainingConfiguration::default()).expect("configuration JSON");
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO training_runs \
         (id, snapshot_id, backend_name, model_format, state, configuration_json, \
          created_at, updated_at) VALUES (?, ?, 'hashing-linear', 'hashing-linear-v1', \
          'completed', ?, ?, ?)",
    )
    .bind(legacy_id)
    .bind(snapshot_id)
    .bind(&configuration)
    .bind(now)
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("legacy training run");

    full.run(&mut connection).await.expect("0049 upgrade");

    let legacy_projection: InputAuthorityProjection = sqlx::query_as(
        "SELECT input_protocol, input_population_fingerprint, input_member_count, \
         input_fingerprint, input_authority_kind, input_authority_id, \
         input_authority_fingerprint FROM training_runs WHERE id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&mut connection)
    .await
    .expect("legacy projection");
    assert_eq!(
        legacy_projection,
        (None, None, None, None, None, None, None)
    );

    assert!(
        sqlx::query(
            "INSERT INTO training_runs \
             (id, snapshot_id, backend_name, model_format, state, configuration_json, \
              input_protocol, created_at, updated_at) \
             VALUES (?, ?, 'hashing-linear', 'hashing-linear-v1', 'queued', ?, \
              'train_and_validation_v1', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(snapshot_id)
        .bind(&configuration)
        .bind(now)
        .bind(now)
        .execute(&mut connection)
        .await
        .is_err(),
        "partial bindings must be rejected"
    );
    assert!(
        sqlx::query("UPDATE training_runs SET input_protocol = 'retrofit' WHERE id = ?")
            .bind(legacy_id)
            .execute(&mut connection)
            .await
            .is_err(),
        "legacy runs cannot be retroactively blessed"
    );
}
