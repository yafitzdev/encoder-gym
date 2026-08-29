use std::process::{Command, Output};

use chrono::Utc;
use dataset_core::domain::{SplitConfiguration, SplitRatios};
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

#[test]
fn sealed_cohort_exposure_is_governed_from_the_cli() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("governance-cli.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let snapshot_id = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(create_snapshot_fixture(&database_url));

    let created = run_json(
        &database_url,
        [
            "cohort",
            "create",
            &snapshot_id.to_string(),
            "--name",
            "release-holdout",
            "--split",
            "test",
            "--role",
            "sealed-acceptance",
            "--reason",
            "fresh holdout",
        ],
    );
    let cohort_id = created["cohort"]["id"]
        .as_str()
        .expect("cohort ID")
        .to_owned();

    run_json(
        &database_url,
        [
            "exposure",
            "record",
            &cohort_id,
            "--purpose",
            "acceptance",
            "--disclosure",
            "aggregate",
        ],
    );
    run_json(
        &database_url,
        [
            "exposure",
            "record",
            &cohort_id,
            "--purpose",
            "acceptance",
            "--disclosure",
            "aggregate",
        ],
    );
    assert_eq!(
        run_json(&database_url, ["exposure", "risk", &cohort_id])["risk"],
        "elevated"
    );

    let rejected = run(
        &database_url,
        [
            "exposure",
            "record",
            &cohort_id,
            "--purpose",
            "manual-inspection",
            "--disclosure",
            "row-content",
        ],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--retirement-reason is required"));

    run_json(
        &database_url,
        [
            "exposure",
            "record",
            &cohort_id,
            "--purpose",
            "manual-inspection",
            "--disclosure",
            "row-content",
            "--retirement-reason",
            "rows were inspected",
        ],
    );
    let shown = run_json(&database_url, ["cohort", "show", &cohort_id]);
    assert_eq!(shown["current_role"]["disposition"], "retired");
    assert_eq!(
        run_json(&database_url, ["cohort", "history", &cohort_id])
            .as_array()
            .expect("history")
            .len(),
        2
    );
}

async fn create_snapshot_fixture(database_url: &str) -> Uuid {
    let store = SqliteStore::connect(database_url)
        .await
        .expect("database connects");
    let dataset = DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into(), "fraud".into()],
        Vec::new(),
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let snapshot_id = Uuid::new_v4();
    let split = SplitConfiguration::new(SplitRatios::new(0.8, 0.1, 0.1).expect("ratios"), 42);
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, member_count, \
          fingerprint, created_at) VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset.id)
    .bind("governance CLI fixture")
    .bind(serde_json::to_string(&split).expect("split JSON"))
    .bind(1_i64)
    .bind("sha256:governance-cli-fixture")
    .bind(Utc::now())
    .execute(store.pool())
    .await
    .expect("snapshot fixture persisted");
    snapshot_id
}

fn run_json<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> Value {
    let output = run(database_url, arguments);
    assert!(
        output.status.success(),
        "CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON stdout")
}

fn run<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_synth"))
        .args(["--database-url", database_url, "--output", "json"])
        .args(arguments)
        .output()
        .expect("CLI starts")
}
