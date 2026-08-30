use std::path::Path;

use serde_json::Value;

#[allow(dead_code)]
mod support;

use support::run_json;

#[test]
fn deterministic_construction_is_previewable_and_never_contacts_the_configured_provider() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("hybrid.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let config_path = directory.path().join("project.toml");
    std::fs::write(&config_path, config()).expect("write config");

    let preview = run_json(
        &database_url,
        [
            "config",
            "construction-preview",
            path(&config_path),
            "--label",
            "billing",
            "--count",
            "2",
        ],
    );
    assert_eq!(preview["provider_required"], false);
    assert_eq!(preview["llm_field_count"], 0);
    assert_eq!(preview["deterministic_field_count"], 3);
    assert_eq!(
        preview["completed_rows"][0]["text"],
        "support ticket T-0100"
    );

    let initialized = run_json(&database_url, ["config", "init", path(&config_path)]);
    let plan_id = string_at(&initialized, "/generation_plan/id");
    let dataset_id = string_at(&initialized, "/dataset/id");
    let generated = run_json(
        &database_url,
        ["generate", &plan_id, "--config", path(&config_path)],
    );
    assert_eq!(generated["state"], "completed");
    assert_eq!(generated["accepted_rows"], 2);
    let job_id = string_at(&generated, "/id");
    let attempts = run_json(&database_url, ["job", "attempts", &job_id]);
    assert_eq!(attempts[0]["kind"], "deterministic_construction");
    assert_eq!(attempts[0]["backend_metadata"]["provider_called"], false);

    let rows = run_json(
        &database_url,
        ["rows", "--job-id", &job_id, "--limit", "10"],
    );
    assert_eq!(rows[0]["fields"]["ticket_id"], "T-0100");
    assert_eq!(rows[1]["fields"]["origin"], "rules");
    assert_eq!(
        rows[0]["construction"]["fields"]["text"]["source"],
        "deterministic"
    );

    let next = run_json(
        &database_url,
        ["job", "prompt", &job_id, "--requested-count", "1"],
    );
    assert_eq!(next["provider_required"], false);
    assert!(next["request"].is_null());
    assert_eq!(next["start_index"], 2);
    assert_eq!(next["completed_rows"][0]["fields"]["ticket_id"], "T-0102");

    let export_path = directory.path().join("accepted.jsonl");
    let exported = run_json(
        &database_url,
        [
            "export",
            &dataset_id,
            "--format",
            "jsonl",
            "--file",
            path(&export_path),
        ],
    );
    assert_eq!(exported["row_count"], 2);
    let first: Value = serde_json::from_str(
        std::fs::read_to_string(export_path)
            .expect("read export")
            .lines()
            .next()
            .expect("first row"),
    )
    .expect("JSONL row");
    assert_eq!(first["fields"]["origin"], "rules");
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}

fn config() -> &'static str {
    r#"version = 1

[dataset]
name = "deterministic-support"
task = "Construct deterministic support fixtures."
labels = ["billing"]

[generation]
target_per_cell = 2
batch_size = 2
max_retries = 0
max_attempt_multiplier = 1
backend = "openai-compatible"
base_url = "http://127.0.0.1:9/v1"
model = "must-not-be-called"

[generation.construction]
seed = 9

[[generation.construction.fields]]
name = "ticket_id"
value_type = "string"
[generation.construction.fields.recipe]
type = "sequence"
prefix = "T-"
start = 100
step = 1
width = 4

[[generation.construction.fields]]
name = "origin"
value_type = "string"
[generation.construction.fields.recipe]
type = "fixed"
value = "rules"

[[generation.construction.fields]]
name = "text"
value_type = "string"
[generation.construction.fields.recipe]
type = "template"
template = "support ticket ${ticket_id}"
"#
}
