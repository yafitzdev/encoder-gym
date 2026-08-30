#[allow(dead_code)]
mod support;

use std::path::PathBuf;

use serde_json::Value;
use tempfile::TempDir;

use support::run_json;

#[test]
fn full_offline_pi_architecture_review_application_and_generation_handoff() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sidecar = root.join("adapters/research-agent-pi/dist/main.js");
    if !sidecar.is_file() {
        eprintln!(
            "skipping process-level Pi acceptance because {} is not built",
            sidecar.display()
        );
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let database_url = sqlite_url(&temporary, "architect-cli.db");
    run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "support-architect",
            "--task",
            "Classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
            "--dimension",
            "style=clean,messy",
        ],
    );
    let brief = path(&root, "examples/architect/support-architect-brief.json");
    let script = path(
        &root,
        "examples/architect/support-architect-scripted-turns.json",
    );
    let sidecar = sidecar.to_string_lossy().into_owned();
    let outcome = run_json(
        &database_url,
        [
            "architect",
            "start",
            &brief,
            "--script",
            &script,
            "--pi-sidecar",
            &sidecar,
        ],
    );
    assert_eq!(outcome["run"]["state"], "awaiting_review");
    assert_eq!(outcome["run"]["usage"]["allocation_previews"], 1);
    let run_id = text(&outcome["run"], "id");
    let proposal_id = text(&outcome["proposal"], "id");
    let before = run_json(&database_url, ["architect", "status", &run_id]);
    let review = run_json(
        &database_url,
        [
            "architect",
            "review",
            &proposal_id,
            "--approve",
            "--reason",
            "offline acceptance",
        ],
    );
    assert_eq!(review["decision"], "approve");
    let applied = run_json(&database_url, ["architect", "apply", &proposal_id]);
    let plan_id = text(&applied["plan"], "id");
    let context_fingerprint = text(&applied["strategy_context"], "fingerprint");
    let context = run_json(&database_url, ["architect", "context", &plan_id]);
    assert_eq!(context["fingerprint"], context_fingerprint);

    let job = run_json(
        &database_url,
        [
            "generate",
            &plan_id,
            "--backend",
            "fake",
            "--batch-size",
            "5",
        ],
    );
    assert_eq!(job["state"], "completed");
    assert_eq!(job["accepted_rows"], 20);
    let job_id = text(&job, "id");
    let execution = run_json(&database_url, ["job", "execution", &job_id]);
    assert_eq!(
        execution["strategy_context_fingerprint"],
        context_fingerprint
    );
    let clean = run_json(
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
    let messy = run_json(
        &database_url,
        [
            "job",
            "prompt",
            &job_id,
            "--cell-index",
            "1",
            "--requested-count",
            "1",
        ],
    );
    let instruction = "Use indirect cues that remain consistent with the trusted target label.";
    assert!(
        !clean["request"]["user_prompt"]
            .as_str()
            .unwrap()
            .contains(instruction)
    );
    assert!(
        messy["request"]["user_prompt"]
            .as_str()
            .unwrap()
            .contains(instruction)
    );

    let after = run_json(&database_url, ["architect", "status", &run_id]);
    assert_eq!(before["tool_calls"], after["tool_calls"]);
    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
}

fn text(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap().to_owned()
}

fn path(root: &std::path::Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn sqlite_url(directory: &TempDir, name: &str) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        directory
            .path()
            .join(name)
            .to_string_lossy()
            .replace('\\', "/")
    )
}
