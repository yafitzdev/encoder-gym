#[allow(dead_code)]
mod support;

use std::path::PathBuf;

use serde_json::Value;
use tempfile::TempDir;

use support::run_json;

#[test]
fn full_offline_pi_research_review_binding_and_generation_handoff() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sidecar = root.join("adapters/research-agent-pi/dist/main.js");
    if !sidecar.is_file() {
        eprintln!(
            "skipping process-level Pi acceptance because {} is not built; run npm run build in adapters/research-agent-pi",
            sidecar.display()
        );
        return;
    }
    let temporary = tempfile::tempdir().expect("temporary directory");
    let database_url = sqlite_url(&temporary, "research-cli.db");
    let dataset = run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "support-intents",
            "--task",
            "Classify a customer support message by its primary intent.",
            "--label",
            "billing",
            "--label",
            "fraud",
            "--dimension",
            "difficulty=easy,hard",
        ],
    );
    let dataset_id = text(&dataset, "id");
    let brief = path(&root, "examples/research/support-authenticity-brief.json");
    let script = path(&root, "examples/research/support-scripted-turns.json");
    let corpus = path(&root, "examples/research/support-corpus.json");
    let sidecar = sidecar.to_string_lossy().into_owned();
    let research = run_json(
        &database_url,
        [
            "research",
            "start",
            &brief,
            "--script",
            &script,
            "--corpus",
            &corpus,
            "--pi-sidecar",
            &sidecar,
        ],
    );
    assert_eq!(research["run"]["state"], "awaiting_review");
    assert_eq!(research["run"]["usage"]["model_turns"], 8);
    let run_id = text(&research["run"], "id");
    let profile_id = text(&research["profile"], "id");
    let profile_fingerprint = text(&research["profile"], "fingerprint");
    let before = run_json(&database_url, ["research", "status", &run_id]);
    for call in before["tool_calls"].as_array().expect("tool calls") {
        if call["kind"] == "fetch_page" {
            assert!(call["response"].get("content").is_none());
        }
    }
    let rows_before_generation = run_json(&database_url, ["rows", "--dataset-id", &dataset_id]);
    assert_eq!(rows_before_generation.as_array().map(Vec::len), Some(0));

    let review = run_json(
        &database_url,
        [
            "research",
            "review",
            &profile_id,
            "--approve",
            "--reason",
            "offline acceptance",
        ],
    );
    assert_eq!(review["decision"], "approve");
    let binding = run_json(
        &database_url,
        ["research", "bind", &dataset_id, &profile_id],
    );
    assert_eq!(binding["profile_fingerprint"], profile_fingerprint);
    let context = run_json(&database_url, ["research", "context", &dataset_id]);
    assert_eq!(context["profile_id"], profile_id);

    let plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let plan_id = text(&plan, "id");
    let job = run_json(
        &database_url,
        [
            "generate",
            &plan_id,
            "--backend",
            "fake",
            "--batch-size",
            "4",
        ],
    );
    assert_eq!(job["state"], "completed");
    let job_id = text(&job, "id");
    let execution = run_json(&database_url, ["job", "execution", &job_id]);
    assert!(execution["authenticity_context_fingerprint"].is_string());
    let prompt = run_json(
        &database_url,
        [
            "job",
            "prompt",
            &job_id,
            "--cell-index",
            "0",
            "--requested-count",
            "1",
        ],
    );
    let user_prompt = prompt["request"]["user_prompt"]
        .as_str()
        .expect("prompt text");
    assert!(user_prompt.contains(&profile_fingerprint));
    assert!(!user_prompt.contains("charged twice??"));

    let after = run_json(&database_url, ["research", "status", &run_id]);
    assert_eq!(before["tool_call_count"], after["tool_call_count"]);
    assert_eq!(before["evidence_count"], after["evidence_count"]);
    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
}

fn text(value: &Value, field: &str) -> String {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} was not a string in {value}"))
        .to_owned()
}

fn path(root: &std::path::Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn sqlite_url(directory: &TempDir, name: &str) -> String {
    let path = directory.path().join(name);
    format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    )
}
