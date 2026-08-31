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
use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition},
    ports::DatasetStore,
};
use project_preparation::PreparedProject;
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use tempfile::TempDir;
use uuid::Uuid;
use workflow_core::workflow::WorkflowDefinition;

use support::{run, run_json};

#[test]
fn manifest_previews_prepares_idempotently_and_yields_a_startable_workflow() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("project-preparation-cli.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let snapshot_id = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(create_snapshot_fixture(&database_url));
    let manifest_path = directory.path().join("project-preparation.toml");
    std::fs::write(
        &manifest_path,
        include_str!("../../../examples/project-preparation.toml").replace(
            "00000000-0000-0000-0000-000000000000",
            &snapshot_id.to_string(),
        ),
    )
    .expect("manifest written");
    let manifest = manifest_path.to_str().expect("UTF-8 path");

    let preview = run_json(&database_url, ["project", "preview", manifest]);
    assert_eq!(preview["eligible"], true);
    assert_eq!(preview["requested_total_rows"], 1_000);
    assert_eq!(preview["initial_target_rows"], 900);
    assert_eq!(preview["cells"].as_array().expect("cells").len(), 48);
    assert_eq!(
        preview["cells"]
            .as_array()
            .expect("cells")
            .iter()
            .map(|cell| cell["target"].as_u64().expect("target"))
            .sum::<u64>(),
        900
    );
    assert_eq!(
        run_json(&database_url, ["project", "list"]),
        serde_json::json!([]),
        "preview must not persist preparation artifacts"
    );

    let created = run_json(&database_url, ["project", "prepare", manifest]);
    assert_eq!(created["created"], true);
    let preparation_id = created["preparation"]["id"]
        .as_str()
        .expect("preparation ID");
    let definition_id = created["preparation"]["workflow_definition_id"]
        .as_str()
        .expect("definition ID");
    assert_eq!(
        created["next_command"],
        format!("synth workflow start {definition_id}")
    );
    let repeated = run_json(&database_url, ["project", "prepare", manifest]);
    assert_eq!(repeated["created"], false);
    assert_eq!(repeated["preparation"]["id"], preparation_id);
    assert_eq!(
        run_json(&database_url, ["project", "list"])
            .as_array()
            .expect("preparations")
            .len(),
        1
    );
    let shown = run_json(&database_url, ["project", "show", preparation_id]);
    assert_eq!(shown["workflow_definition"]["id"], definition_id);
    assert_eq!(
        run_json(&database_url, ["workflow", "list"]),
        serde_json::json!([]),
        "preparation must not start execution"
    );

    let started = run_json(
        &database_url,
        ["workflow", "start", definition_id, "--initialize-only"],
    );
    assert_eq!(started["run"]["definition_id"], definition_id);
    assert_eq!(started["attempt"]["stage"], "initial_allocation");
}

#[test]
fn doctor_verifies_bundle_authority_and_retains_legacy_bindings_for_inspection() {
    let fixture = PreparedDoctorFixture::new();
    assert_doctor_bundle_check(&fixture.database_url, true, "pass");

    fixture.mutate(|store| {
        Box::pin(async move {
            let artifact_json: String =
                sqlx::query_scalar("SELECT artifact_json FROM workflow_definitions WHERE id = ?")
                    .bind(fixture.definition_id)
                    .fetch_one(store.pool())
                    .await
                    .expect("definition artifact");
            let mut definition: WorkflowDefinition =
                serde_json::from_str(&artifact_json).expect("definition JSON");
            definition.benchmark_bundle = None;
            definition.fingerprint = definition
                .reproduce_fingerprint()
                .expect("legacy definition fingerprint");
            sqlx::query(
                "UPDATE workflow_definitions SET benchmark_bundle_id = NULL, artifact_json = ?, \
                 fingerprint = ? WHERE id = ?",
            )
            .bind(serde_json::to_string(&definition).expect("definition JSON"))
            .bind(&definition.fingerprint)
            .bind(definition.id)
            .execute(store.pool())
            .await
            .expect("legacy definition persisted");

            let artifact_json: String =
                sqlx::query_scalar("SELECT artifact_json FROM project_preparations WHERE id = ?")
                    .bind(fixture.preparation_id)
                    .fetch_one(store.pool())
                    .await
                    .expect("preparation artifact");
            let mut preparation: PreparedProject =
                serde_json::from_str(&artifact_json).expect("preparation JSON");
            preparation.benchmark_bundle_id = None;
            preparation.benchmark_bundle_fingerprint = None;
            preparation.fingerprint = preparation
                .reproduce_fingerprint()
                .expect("legacy preparation fingerprint");
            sqlx::query(
                "UPDATE project_preparations SET benchmark_bundle_id = NULL, artifact_json = ?, \
                 fingerprint = ? WHERE id = ?",
            )
            .bind(serde_json::to_string(&preparation).expect("preparation JSON"))
            .bind(&preparation.fingerprint)
            .bind(preparation.id)
            .execute(store.pool())
            .await
            .expect("legacy preparation persisted");
        })
    });

    let report = assert_doctor_bundle_check(&fixture.database_url, true, "pass");
    assert!(
        doctor_check(&report, "benchmark_bundle_facts")["message"]
            .as_str()
            .expect("check message")
            .contains("2 legacy NULL binding(s) retained for inspection and marked non-executable")
    );
}

#[test]
fn doctor_rejects_benchmark_bundle_artifact_and_join_tampering() {
    let artifact_fixture = PreparedDoctorFixture::new();
    artifact_fixture.mutate(|store| {
        Box::pin(async move {
            let json: String = sqlx::query_scalar(
                "SELECT artifact_json FROM workflow_benchmark_bundles WHERE id = ?",
            )
            .bind(artifact_fixture.bundle_id)
            .fetch_one(store.pool())
            .await
            .expect("bundle artifact");
            let mut artifact: Value = serde_json::from_str(&json).expect("bundle JSON");
            artifact["development_suite_fingerprint"] = Value::String("sha256:tampered".into());
            sqlx::query("UPDATE workflow_benchmark_bundles SET artifact_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&artifact).expect("bundle JSON"))
                .bind(artifact_fixture.bundle_id)
                .execute(store.pool())
                .await
                .expect("bundle tampered");
        })
    });
    assert_doctor_bundle_check(&artifact_fixture.database_url, false, "fail");

    let join_fixture = PreparedDoctorFixture::new();
    join_fixture.mutate(|store| {
        Box::pin(async move {
            let foreign_report_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO workflow_contamination_reports \
                 (id, status, cohort_ids_json, artifact_json, fingerprint, created_at) \
                 SELECT ?, status, cohort_ids_json, artifact_json, ?, created_at \
                 FROM workflow_contamination_reports WHERE id = ?",
            )
            .bind(foreign_report_id)
            .bind(format!("sha256:foreign-{foreign_report_id}"))
            .bind(join_fixture.contamination_report_id)
            .execute(store.pool())
            .await
            .expect("foreign report inserted");
            sqlx::query(
                "UPDATE workflow_benchmark_bundles SET contamination_report_id = ? WHERE id = ?",
            )
            .bind(foreign_report_id)
            .bind(join_fixture.bundle_id)
            .execute(store.pool())
            .await
            .expect("bundle join tampered");
        })
    });
    assert_doctor_bundle_check(&join_fixture.database_url, false, "fail");
}

#[test]
fn doctor_rejects_self_consistent_workflow_and_preparation_binding_tampering() {
    let workflow_fixture = PreparedDoctorFixture::new();
    workflow_fixture.mutate(|store| {
        Box::pin(async move {
            let json: String =
                sqlx::query_scalar("SELECT artifact_json FROM workflow_definitions WHERE id = ?")
                    .bind(workflow_fixture.definition_id)
                    .fetch_one(store.pool())
                    .await
                    .expect("definition artifact");
            let mut definition: WorkflowDefinition =
                serde_json::from_str(&json).expect("definition JSON");
            let binding = definition
                .benchmark_bundle
                .as_mut()
                .expect("benchmark binding");
            binding.contamination_report_fingerprint = "sha256:tampered-report".into();
            binding.bundle_fingerprint = binding
                .reproduce_bundle_fingerprint()
                .expect("tampered binding fingerprint");
            definition.fingerprint = definition
                .reproduce_fingerprint()
                .expect("tampered definition fingerprint");
            sqlx::query(
                "UPDATE workflow_definitions SET artifact_json = ?, fingerprint = ? WHERE id = ?",
            )
            .bind(serde_json::to_string(&definition).expect("definition JSON"))
            .bind(&definition.fingerprint)
            .bind(definition.id)
            .execute(store.pool())
            .await
            .expect("definition binding tampered");
        })
    });
    assert_doctor_bundle_check(&workflow_fixture.database_url, false, "fail");

    let preparation_fixture = PreparedDoctorFixture::new();
    preparation_fixture.mutate(|store| {
        Box::pin(async move {
            let json: String =
                sqlx::query_scalar("SELECT artifact_json FROM project_preparations WHERE id = ?")
                    .bind(preparation_fixture.preparation_id)
                    .fetch_one(store.pool())
                    .await
                    .expect("preparation artifact");
            let mut preparation: PreparedProject =
                serde_json::from_str(&json).expect("preparation JSON");
            preparation.benchmark_bundle_fingerprint = Some("sha256:tampered-bundle".into());
            preparation.fingerprint = preparation
                .reproduce_fingerprint()
                .expect("tampered preparation fingerprint");
            sqlx::query(
                "UPDATE project_preparations SET artifact_json = ?, fingerprint = ? WHERE id = ?",
            )
            .bind(serde_json::to_string(&preparation).expect("preparation JSON"))
            .bind(&preparation.fingerprint)
            .bind(preparation.id)
            .execute(store.pool())
            .await
            .expect("preparation binding tampered");
        })
    });
    assert_doctor_bundle_check(&preparation_fixture.database_url, false, "fail");
}

struct PreparedDoctorFixture {
    _directory: TempDir,
    database_url: String,
    preparation_id: Uuid,
    definition_id: Uuid,
    bundle_id: Uuid,
    contamination_report_id: Uuid,
}

impl PreparedDoctorFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = directory.path().join("doctor-benchmark-bundle.db");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            database.to_string_lossy().replace('\\', "/")
        );
        let snapshot_id = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(create_snapshot_fixture(&database_url));
        let manifest_path = directory.path().join("project-preparation.toml");
        std::fs::write(
            &manifest_path,
            include_str!("../../../examples/project-preparation.toml").replace(
                "00000000-0000-0000-0000-000000000000",
                &snapshot_id.to_string(),
            ),
        )
        .expect("manifest written");
        let created = run_json(
            &database_url,
            [
                "project",
                "prepare",
                manifest_path.to_str().expect("UTF-8 path"),
            ],
        );
        let preparation = &created["preparation"];
        let bundle_id = parse_uuid(preparation, "benchmark_bundle_id");
        let store = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(SqliteStore::connect(&database_url))
            .expect("database connects");
        let contamination_report_id =
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(async {
                    sqlx::query_scalar(
                    "SELECT contamination_report_id FROM workflow_benchmark_bundles WHERE id = ?",
                )
                .bind(bundle_id)
                .fetch_one(store.pool())
                .await
                .expect("bundle report")
                });
        Self {
            _directory: directory,
            database_url,
            preparation_id: parse_uuid(preparation, "id"),
            definition_id: parse_uuid(preparation, "workflow_definition_id"),
            bundle_id,
            contamination_report_id,
        }
    }

    fn mutate<F>(&self, mutation: F)
    where
        F: for<'a> FnOnce(
            &'a SqliteStore,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>,
    {
        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                let store = SqliteStore::connect(&self.database_url)
                    .await
                    .expect("database connects");
                mutation(&store).await;
            });
    }
}

fn parse_uuid(value: &Value, field: &str) -> Uuid {
    Uuid::parse_str(value[field].as_str().unwrap_or_else(|| panic!("{field}")))
        .unwrap_or_else(|error| panic!("{field}: {error}"))
}

fn assert_doctor_bundle_check(database_url: &str, succeeds: bool, status: &str) -> Value {
    let output = run(database_url, ["doctor"]);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "doctor stdout was not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(
        output.status.success(),
        succeeds,
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        doctor_check(&report, "benchmark_bundle_facts")["status"],
        status
    );
    report
}

fn doctor_check<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .expect("doctor checks")
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("doctor check {name}"))
}

async fn create_snapshot_fixture(database_url: &str) -> Uuid {
    let store = SqliteStore::connect(database_url)
        .await
        .expect("database connects");
    let dataset = DatasetDefinition::new(
        "support benchmark",
        "Classify customer support requests",
        vec![
            "billing".into(),
            "fraud".into(),
            "account".into(),
            "technical".into(),
        ],
        vec![
            DimensionDefinition::new(
                "difficulty",
                vec!["easy".into(), "medium".into(), "hard".into()],
            )
            .expect("dimension"),
            DimensionDefinition::new("writing_style", vec!["clean".into(), "messy".into()])
                .expect("dimension"),
            DimensionDefinition::new("ambiguity", vec!["obvious".into(), "ambiguous".into()])
                .expect("dimension"),
        ],
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
    let dimensions = BTreeMap::from([
        ("difficulty".into(), "easy".into()),
        ("writing_style".into(), "clean".into()),
        ("ambiguity".into(), "obvious".into()),
    ]);
    let provenance = SourceProvenance::Generated {
        generation_job_id: Uuid::nil(),
        backend: "fake".into(),
        model: "deterministic-v1".into(),
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
    .bind("billing/easy/clean/obvious")
    .bind("Why was I charged twice?")
    .bind("why was i charged twice?")
    .bind("billing")
    .bind(serde_json::to_string(&dimensions).expect("dimensions JSON"))
    .bind(serde_json::to_string(&provenance).expect("provenance JSON"))
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("source row persisted");
    let mut snapshot = DatasetSnapshot {
        id: snapshot_id,
        source_dataset_id: dataset.id,
        name: "project preparation CLI fixture".into(),
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
        text: "Why was I charged twice?".into(),
        label: "billing".into(),
        dimensions,
        fields: BTreeMap::new(),
        source_provenance: provenance,
        source_created_at: created_at,
    };
    snapshot.fingerprint = reproduce_snapshot_fingerprint(&snapshot, std::slice::from_ref(&member))
        .expect("snapshot fingerprint");
    store
        .create_snapshot(&snapshot, std::slice::from_ref(&member))
        .await
        .expect("snapshot persisted");
    snapshot_id
}
