pub mod support;

use std::path::{Path, PathBuf};

use project_config::{GenerationBackendKind, TrainingBackendKind};
use project_preparation::BootstrapManifest;
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use training_transformer::fixture::write_tiny_bert_bundle;

use support::{run, run_json};

#[test]
fn local_sources_bootstrap_idempotently_and_run_the_complete_offline_workflow() {
    let fixture = PilotFixture::new();
    let manifest = fixture.manifest();

    let preview = run_json(
        fixture.database_url(),
        ["project", "bootstrap-preview", manifest],
    );
    assert_eq!(preview["eligible"], true);
    assert_eq!(preview["preparation"]["initial_target_rows"], 16);
    assert_eq!(preview["sources"][0]["accepted_rows"], 16);
    assert_eq!(preview["sources"][1]["accepted_rows"], 10);
    let initial_fingerprint = string_at(&preview, "/bootstrap_fingerprint");
    assert_eq!(
        run_json(fixture.database_url(), ["project", "bootstrap-list"]),
        serde_json::json!([]),
        "preview must not persist bootstrap state"
    );
    assert_eq!(
        run_json(fixture.database_url(), ["dataset", "list"]),
        serde_json::json!([]),
        "preview must not persist source datasets"
    );

    let created = run_json(fixture.database_url(), ["project", "bootstrap", manifest]);
    assert_eq!(created["created"], true);
    assert_eq!(
        created["bootstrap"]["bootstrap_fingerprint"],
        initial_fingerprint
    );
    let bootstrap_id = string_at(&created, "/bootstrap/id");
    let definition_id = string_at(&created, "/preparation/workflow_definition_id");
    let development_snapshot = string_at(&created, "/bootstrap/sources/0/snapshot_id");
    let sealed_snapshot = string_at(&created, "/bootstrap/sources/1/snapshot_id");
    assert_eq!(
        created["next_command"],
        format!("synth workflow start {definition_id}")
    );

    let repeated = run_json(fixture.database_url(), ["project", "bootstrap", manifest]);
    assert_eq!(repeated["created"], false);
    assert_eq!(repeated["bootstrap"]["id"], bootstrap_id);
    assert_eq!(
        run_json(
            fixture.database_url(),
            ["project", "bootstrap-show", &bootstrap_id],
        )["bootstrap"]["id"],
        bootstrap_id
    );
    assert_eq!(
        run_json(fixture.database_url(), ["dataset", "list"])
            .as_array()
            .expect("datasets")
            .len(),
        3,
        "two benchmark datasets and one generated training dataset"
    );
    assert_eq!(
        run_json(fixture.database_url(), ["workflow", "list"]),
        serde_json::json!([]),
        "bootstrap prepares a definition but must not start a workflow"
    );
    assert_all_test_with_imported_provenance(fixture.database_url(), &development_snapshot, 16);
    assert_all_test_with_imported_provenance(fixture.database_url(), &sealed_snapshot, 10);
    let provenance = run_json(
        fixture.database_url(),
        ["provenance", "project-bootstrap", &bootstrap_id],
    );
    let provenance = serde_json::to_string(&provenance).expect("provenance JSON");
    for kind in [
        "project_bootstrap",
        "project_preparation",
        "snapshot",
        "dataset_import",
    ] {
        assert!(provenance.contains(kind), "missing {kind}: {provenance}");
    }

    let started = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let run_id = string_at(&started, "/run/id");
    let developed = if started["run"]["state"] == "awaiting_approval" {
        run_json(fixture.database_url(), ["workflow", "approve", &run_id])
    } else {
        started
    };
    assert_eq!(developed["run"]["state"], "development_complete");
    let finalized = run_json(fixture.database_url(), ["workflow", "finalize", &run_id]);
    assert_eq!(finalized["run"]["current_stage"], "sealed_evaluation");
    let promoted = run_json(fixture.database_url(), ["workflow", "promote", &run_id]);
    assert_eq!(promoted["run"]["state"], "completed");
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );

    fixture.append_development_row();
    let changed = run_json(
        fixture.database_url(),
        ["project", "bootstrap-preview", manifest],
    );
    assert_ne!(changed["bootstrap_fingerprint"], initial_fingerprint);
    assert_eq!(changed["sources"][0]["accepted_rows"], 17);
    let changed_created = run_json(fixture.database_url(), ["project", "bootstrap", manifest]);
    assert_eq!(changed_created["created"], true);
    assert_ne!(changed_created["bootstrap"]["id"], bootstrap_id);
}

#[test]
fn rejected_benchmark_rows_block_bootstrap_without_partial_artifacts() {
    let fixture = PilotFixture::new();
    fixture.corrupt_sealed_label();
    let output = run(
        fixture.database_url(),
        ["project", "bootstrap", fixture.manifest()],
    );
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(error.contains("rejected 1 of 10 rows"), "{error}");
    assert_eq!(
        run_json(fixture.database_url(), ["project", "bootstrap-list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["dataset", "list"]),
        serde_json::json!([])
    );
}

#[test]
fn bootstrapped_project_runs_the_real_local_bert_adapter() {
    let fixture = PilotFixture::new();
    let bundle_path = fixture.directory().join("tiny-bert");
    write_tiny_bert_bundle(&bundle_path).expect("tiny BERT bundle");
    let encoder = run_json(
        fixture.database_url(),
        [
            "encoder",
            "register",
            "--name",
            "pilot-tiny-bert",
            bundle_path.to_str().expect("UTF-8 bundle path"),
        ],
    );
    let encoder_id = string_at(&encoder, "/id");
    fixture.configure_bert(&encoder_id);
    let created = run_json(
        fixture.database_url(),
        ["project", "bootstrap", fixture.manifest()],
    );
    let definition_id = string_at(&created, "/preparation/workflow_definition_id");
    let started = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let run_id = string_at(&started, "/run/id");
    let developed = if started["run"]["state"] == "awaiting_approval" {
        run_json(fixture.database_url(), ["workflow", "approve", &run_id])
    } else {
        started
    };
    assert_eq!(developed["run"]["state"], "development_complete");
    let artifact_kinds = developed["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .map(|artifact| artifact["kind"].as_str().expect("artifact kind"))
        .collect::<Vec<_>>();
    assert!(artifact_kinds.contains(&"training_run"));
    assert!(artifact_kinds.contains(&"checkpoint"));
    run_json(fixture.database_url(), ["workflow", "finalize", &run_id]);
    assert_eq!(
        run_json(fixture.database_url(), ["workflow", "promote", &run_id])["run"]["state"],
        "completed"
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
#[ignore = "requires an explicitly configured live endpoint and may incur provider cost"]
fn bootstrapped_pilot_runs_bounded_openai_compatible_generation() {
    let base_url = std::env::var("SYNTH_E2E_OPENAI_BASE_URL")
        .expect("set SYNTH_E2E_OPENAI_BASE_URL before running the ignored smoke test");
    let model = std::env::var("SYNTH_E2E_OPENAI_MODEL")
        .expect("set SYNTH_E2E_OPENAI_MODEL before running the ignored smoke test");
    assert!(
        std::env::var_os("SYNTH_OPENAI_API_KEY").is_some(),
        "set SYNTH_OPENAI_API_KEY before running the ignored smoke test"
    );
    let fixture = PilotFixture::new();
    fixture.configure_openai(&base_url, &model);
    let created = run_json(
        fixture.database_url(),
        ["project", "bootstrap", fixture.manifest()],
    );
    let definition_id = string_at(&created, "/preparation/workflow_definition_id");
    let status = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );

    for kind in [
        "generation_job",
        "training_run",
        "checkpoint",
        "evaluation_run",
    ] {
        assert!(
            workflow_artifact_count(&status, kind) > 0,
            "live bootstrapped workflow is missing {kind}: {}",
            live_workflow_summary(&status)
        );
    }
    let jobs = status["generation"]
        .as_array()
        .expect("generation status")
        .iter()
        .flat_map(|generation| generation["jobs"].as_array().expect("generation jobs"));
    assert!(jobs.into_iter().all(|job| job["state"] == "completed"));
}

#[test]
fn late_persistence_failure_rolls_back_every_bootstrap_artifact() {
    let fixture = PilotFixture::new();
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let store = SqliteStore::connect(fixture.database_url())
                .await
                .expect("database connects");
            sqlx::query(
                "CREATE TRIGGER reject_test_bootstrap BEFORE INSERT ON project_bootstraps \
                 BEGIN SELECT RAISE(ABORT, 'injected late bootstrap failure'); END",
            )
            .execute(store.pool())
            .await
            .expect("failure trigger installs");
        });
    let output = run(
        fixture.database_url(),
        ["project", "bootstrap", fixture.manifest()],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("injected late bootstrap failure"));
    assert_eq!(
        run_json(fixture.database_url(), ["project", "bootstrap-list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["dataset", "list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["snapshot", "list"]),
        serde_json::json!([])
    );
}

#[test]
fn doctor_detects_tampered_bootstrap_source_facts() {
    let fixture = PilotFixture::new();
    let created = run_json(
        fixture.database_url(),
        ["project", "bootstrap", fixture.manifest()],
    );
    let bootstrap_id = string_at(&created, "/bootstrap/id");
    let bootstrap_uuid = uuid::Uuid::parse_str(&bootstrap_id).expect("bootstrap UUID");
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let store = SqliteStore::connect(fixture.database_url())
                .await
                .expect("database connects");
            let artifact: String =
                sqlx::query_scalar("SELECT artifact_json FROM project_bootstraps WHERE id = ?")
                    .bind(bootstrap_uuid)
                    .fetch_one(store.pool())
                    .await
                    .expect("bootstrap JSON loads");
            let mut artifact: Value = serde_json::from_str(&artifact).expect("bootstrap JSON");
            artifact["sources"][0]["accepted_rows"] = serde_json::json!(999);
            sqlx::query("UPDATE project_bootstraps SET artifact_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&artifact).expect("bootstrap JSON serializes"))
                .bind(bootstrap_uuid)
                .execute(store.pool())
                .await
                .expect("bootstrap JSON tampers");
        });
    let output = run(fixture.database_url(), ["doctor"]);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(report["healthy"], false);
    assert!(
        report["checks"]
            .as_array()
            .expect("doctor checks")
            .iter()
            .any(|check| {
                check["name"] == "project_bootstrap_facts" && check["status"] == "fail"
            })
    );
}

fn assert_all_test_with_imported_provenance(database_url: &str, snapshot_id: &str, count: usize) {
    let members = run_json(database_url, ["snapshot", "members", snapshot_id]);
    let members = members.as_array().expect("snapshot members");
    assert_eq!(members.len(), count);
    assert!(members.iter().all(|member| member["split"] == "test"));
    assert!(
        members
            .iter()
            .all(|member| member["source_provenance"]["kind"] == "imported")
    );
}

fn workflow_artifact_count(status: &Value, kind: &str) -> usize {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .filter(|artifact| artifact["kind"] == kind)
        .count()
}

fn live_workflow_summary(status: &Value) -> String {
    let state = status["run"]["state"].as_str().unwrap_or("unknown");
    let stage = status["latest_attempt"]["stage"]
        .as_str()
        .unwrap_or("unknown");
    let reason = status["latest_attempt"]["reason"]
        .as_str()
        .unwrap_or("no stage reason");
    let job_error = status["generation"]
        .as_array()
        .and_then(|generations| generations.last())
        .and_then(|generation| generation["jobs"].as_array())
        .and_then(|jobs| jobs.last())
        .and_then(|job| job["error_message"].as_str())
        .unwrap_or("no generation job error");
    format!("state={state}, stage={stage}, reason={reason}, job_error={job_error}")
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

struct PilotFixture {
    _directory: tempfile::TempDir,
    database_url: String,
    manifest_path: PathBuf,
    development_path: PathBuf,
    sealed_path: PathBuf,
}

impl PilotFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("pilot directory");
        let source_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/pilot-support");
        let development_path = directory.path().join("development.jsonl");
        let sealed_path = directory.path().join("sealed.csv");
        std::fs::copy(source_root.join("development.jsonl"), &development_path)
            .expect("development copied");
        std::fs::copy(source_root.join("sealed.csv"), &sealed_path).expect("sealed copied");
        let manifest_source = std::fs::read_to_string(source_root.join("project-bootstrap.toml"))
            .expect("pilot manifest source");
        let mut manifest = BootstrapManifest::parse_toml(&manifest_source).expect("pilot manifest");
        manifest.project.training.artifact_root = directory.path().join("artifacts");
        let manifest_path = directory.path().join("project-bootstrap.toml");
        std::fs::write(
            &manifest_path,
            toml::to_string_pretty(&manifest).expect("manifest serializes"),
        )
        .expect("manifest writes");
        let database = directory.path().join("pilot.db");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            database.to_string_lossy().replace('\\', "/")
        );
        Self {
            _directory: directory,
            database_url,
            manifest_path,
            development_path,
            sealed_path,
        }
    }

    fn database_url(&self) -> &str {
        &self.database_url
    }

    fn directory(&self) -> &Path {
        self._directory.path()
    }

    fn manifest(&self) -> &str {
        self.manifest_path.to_str().expect("UTF-8 manifest path")
    }

    fn append_development_row(&self) {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.development_path)
            .expect("development opens");
        writeln!(
            file,
            r#"{{"text":"A second invoice appeared after changing plans.","label":"billing","dimensions":{{"difficulty":"hard","writing_style":"clean"}}}}"#
        )
        .expect("development row appends");
    }

    fn configure_bert(&self, encoder_id: &str) {
        let source = std::fs::read_to_string(&self.manifest_path).expect("manifest reads");
        let mut manifest = BootstrapManifest::parse_toml(&source).expect("manifest parses");
        manifest.project.training.backend = TrainingBackendKind::BertCpu;
        manifest.project.training.base_model_id =
            Some(uuid::Uuid::parse_str(encoder_id).expect("encoder UUID"));
        manifest.project.training.epochs = 1;
        manifest.project.training.learning_rate = 0.01;
        manifest
            .project
            .training
            .transformer
            .maximum_sequence_length = 8;
        manifest.project.training.transformer.batch_size = 4;
        manifest.project.training.transformer.weight_decay = 0.0;
        manifest.project.training.transformer.warmup_ratio = 0.0;
        std::fs::write(
            &self.manifest_path,
            toml::to_string_pretty(&manifest).expect("manifest serializes"),
        )
        .expect("BERT manifest writes");
    }

    fn configure_openai(&self, base_url: &str, model: &str) {
        let source = std::fs::read_to_string(&self.manifest_path).expect("manifest reads");
        let mut manifest = BootstrapManifest::parse_toml(&source).expect("manifest parses");
        manifest.project.generation.backend = GenerationBackendKind::OpenaiCompatible;
        manifest.project.generation.base_url = Some(base_url.into());
        manifest.project.generation.model = model.into();
        manifest.project.generation.temperature = Some(0.2);
        manifest.project.generation.max_tokens = Some(256);
        manifest.project.generation.batch_size = 1;
        manifest.project.generation.max_retries = 0;
        manifest.project.generation.max_attempt_multiplier = 1;
        manifest.workflow.total_rows = 5;
        manifest.workflow.reserved_rows = 1;
        manifest.workflow.budget.maximum_initial_rows = 5;
        manifest.workflow.budget.maximum_cumulative_rows = 6;
        manifest.workflow.budget.maximum_generation_attempts = 4;
        manifest.workflow.budget.maximum_generation_requests = 4;
        std::fs::write(
            &self.manifest_path,
            toml::to_string_pretty(&manifest).expect("manifest serializes"),
        )
        .expect("OpenAI manifest writes");
    }

    fn corrupt_sealed_label(&self) {
        let source = std::fs::read_to_string(&self.sealed_path).expect("sealed reads");
        std::fs::write(
            &self.sealed_path,
            source.replacen(",billing,easy,clean", ",unknown,easy,clean", 1),
        )
        .expect("sealed corrupts");
    }
}
