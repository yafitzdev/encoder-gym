pub mod support;

use std::collections::BTreeMap;

use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceProvenance, SplitConfiguration,
        SplitRatios,
    },
    ports::SnapshotStore,
    splitting::reproduce_snapshot_fingerprint,
};
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use support::{run, run_json};

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

    let sealed_contamination = run_json(
        &database_url,
        ["contamination", "check", "--cohort", &cohort_id],
    );
    assert_eq!(sealed_contamination["status"], "clean");
    let benchmark_path = directory.path().join("sealed-benchmark.json");
    std::fs::write(
        &benchmark_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "contamination_report_id": sealed_contamination["id"],
            "name": "release gate",
            "kind": "sealed_acceptance",
            "task": "classify support requests",
            "labels": ["billing", "fraud"],
            "required_model_formats": [],
            "cohorts": [{
                "cohort_id": cohort_id,
                "protocol": evaluation_core::domain::EvaluationProtocol::default(),
                "disclosure": "aggregate",
                "adaptation_eligible": false
            }],
            "contract": {
                "metric_requirements": [{
                    "target": {"kind": "overall"},
                    "metric": "accuracy",
                    "minimum": 0.8,
                    "minimum_support": 1
                }]
            }
        }))
        .expect("benchmark JSON"),
    )
    .expect("benchmark definition written");
    let benchmark = run_json(
        &database_url,
        [
            "benchmark",
            "create",
            "--definition",
            benchmark_path.to_str().expect("UTF-8 path"),
        ],
    );
    let benchmark_id = benchmark["id"].as_str().expect("benchmark ID");
    assert_eq!(benchmark["kind"], "sealed_acceptance");
    assert_eq!(
        run_json(&database_url, ["benchmark", "validate", benchmark_id])["valid"],
        true
    );
    let unauthorized = run(
        &database_url,
        [
            "benchmark",
            "assess",
            benchmark_id,
            "--run",
            &format!("{cohort_id}={}", Uuid::new_v4()),
        ],
    );
    assert!(!unauthorized.status.success());
    assert!(
        String::from_utf8_lossy(&unauthorized.stderr)
            .contains("sealed assessment requires --authorize-sealed")
    );

    let diagnostic = run_json(
        &database_url,
        [
            "cohort",
            "create",
            &snapshot_id.to_string(),
            "--name",
            "diagnostic-copy",
            "--split",
            "test",
            "--role",
            "diagnostic",
            "--reason",
            "contamination fixture",
        ],
    );
    let diagnostic_id = diagnostic["cohort"]["id"].as_str().expect("diagnostic ID");
    let report = run_json(
        &database_url,
        [
            "contamination",
            "check",
            "--cohort",
            &cohort_id,
            diagnostic_id,
        ],
    );
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["counts"]["source_row"], 1);
    let report_id = report["id"].as_str().expect("report ID");
    run_json(
        &database_url,
        [
            "contamination",
            "override",
            report_id,
            "--reason",
            "intentional process-test overlap",
            "--approved-by",
            "test-operator",
        ],
    );
    assert_eq!(
        run_json(&database_url, ["contamination", "show", report_id])["eligible"],
        true
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
        run_json(&database_url, ["benchmark", "validate", benchmark_id])["valid"],
        false
    );
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
    let split = SplitConfiguration::new(SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"), 42);
    let source_row_id = Uuid::new_v4();
    let member_id = Uuid::new_v4();
    let created_at = Utc::now();
    let provenance = SourceProvenance::Generated {
        generation_job_id: Uuid::nil(),
        backend: "fake".into(),
        model: "fake-v1".into(),
        construction_plan_fingerprint: None,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) \
         VALUES (?, ?, 'generated', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(source_row_id)
    .bind(dataset.id)
    .bind(source_row_id.to_string())
    .bind(r#"{"label":"billing","dimensions":{}}"#)
    .bind("charged twice")
    .bind("charged twice")
    .bind("billing")
    .bind("{}")
    .bind(serde_json::to_string(&provenance).expect("provenance JSON"))
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("source fixture persisted");
    let mut snapshot = DatasetSnapshot {
        id: snapshot_id,
        source_dataset_id: dataset.id,
        name: "governance CLI fixture".into(),
        description: None,
        split_configuration: split,
        member_count: 1,
        fingerprint: String::new(),
        created_at,
    };
    let member = SnapshotMember {
        id: member_id,
        snapshot_id,
        source_row_id,
        split: SnapshotSplit::Test,
        text: "charged twice".into(),
        label: "billing".into(),
        dimensions: BTreeMap::new(),
        fields: BTreeMap::new(),
        source_provenance: provenance,
        source_created_at: created_at,
    };
    snapshot.fingerprint = reproduce_snapshot_fingerprint(&snapshot, std::slice::from_ref(&member))
        .expect("snapshot fingerprint");
    store
        .create_snapshot(&snapshot, std::slice::from_ref(&member))
        .await
        .expect("snapshot fixture persisted");
    snapshot_id
}
