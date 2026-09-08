use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args(["--output", "json", "workspace"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn local_onboarding_import_and_portable_reopen_are_real_cli_operations() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    training_transformer::fixture::write_tiny_bert_bundle(&root.join("checkpoint")).unwrap();
    let model = run(root, &["inspect-model", "checkpoint"]);
    let fingerprint = model["fingerprint"].as_str().unwrap();
    let first = run(
        root,
        &[
            "create",
            "project-one",
            "--name",
            "Encoder one",
            "--model",
            "checkpoint",
            "--expected-fingerprint",
            fingerprint,
        ],
    );
    let second = run(
        root,
        &[
            "create",
            "project-two",
            "--name",
            "Encoder two",
            "--model",
            "checkpoint",
            "--expected-fingerprint",
            fingerprint,
        ],
    );
    assert_ne!(first["manifest"]["id"], second["manifest"]["id"]);
    assert_eq!(first["datasets"], serde_json::json!([]));
    assert_eq!(
        first["modelCatalog"]["artifacts"][0]["fingerprint"],
        first["manifest"]["baseline"]["fingerprint"]
    );
    let upgraded = run(root, &["upgrade", "project-one"]);
    assert_eq!(
        upgraded["modelCatalog"]["activeBaselineRevisionId"],
        first["modelCatalog"]["activeBaselineRevisionId"]
    );
    fs::write(root.join("local.jsonl"), "{\"text\":\"offline fixture\"}\n").unwrap();
    let data = run(
        root,
        &["inspect-dataset", "local.jsonl", "--purpose", "training"],
    );
    let imported = run(
        root,
        &[
            "import-dataset",
            "project-one",
            "--source",
            "local.jsonl",
            "--name",
            "Local rows",
            "--purpose",
            "training",
            "--expected-fingerprint",
            data["artifact"]["fingerprint"].as_str().unwrap(),
        ],
    );
    assert_eq!(imported["datasets"][0]["rows"], 1);
    assert_eq!(
        run(root, &["open", "project-two"])["datasets"],
        serde_json::json!([])
    );
    let readiness = run(root, &["readiness", "project-one"]);
    assert_eq!(readiness["report"]["runnable"], false);
    assert_eq!(readiness["report"]["overall"], "unavailable");
    let checks = readiness["report"]["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|check| { check["key"] == "workspace.integrity" && check["state"] == "ready" })
    );
    assert!(checks.iter().any(|check| {
        check["key"] == "scientific.binding"
            && check["state"] == "action_required"
            && check["nextAction"]["key"] == "bind-scientific-runtime"
    }));
    assert!(checks.iter().any(|check| {
        check["key"] == "optimization.preview" && check["state"] == "action_required"
    }));
    fs::rename(root.join("project-one"), root.join("moved")).unwrap();
    fs::remove_dir_all(root.join("checkpoint")).unwrap();
    fs::remove_file(root.join("local.jsonl")).unwrap();
    let reopened = run(root, &["verify", "moved"]);
    assert_eq!(reopened["manifest"]["id"], first["manifest"]["id"]);
    assert_eq!(reopened["verified"], true);
    assert!(
        !root.join("data").exists(),
        "Workspace commands must not initialize the global synthetic-data store."
    );
    let failure = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args(["workspace", "open", "."])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(!root.join("project.sqlite").exists());
}
