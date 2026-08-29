use std::{path::Path, process::Command};

use serde_json::Value;
use training_transformer::fixture::write_tiny_bert_bundle;

#[test]
fn tiny_transformer_cli_trains_continues_evaluates_and_traces_offline() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("transformer.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let bundle_path = directory.path().join("tiny-bert");
    write_tiny_bert_bundle(&bundle_path).expect("tiny bundle");
    let csv_path = directory.path().join("examples.csv");
    std::fs::write(&csv_path, examples_csv()).expect("dataset CSV");
    let artifact_root = directory.path().join("artifacts");

    let dataset = run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "tiny-transformer",
            "--task",
            "Classify support text",
            "--label",
            "billing",
            "--label",
            "account",
        ],
    );
    let dataset_id = string_at(&dataset, "/id");
    let imported = run_json(
        &database_url,
        [
            "dataset",
            "import",
            &dataset_id,
            "--input",
            path(&csv_path),
            "--format",
            "csv",
        ],
    );
    assert_eq!(imported["accepted_rows"], 20);
    let snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            "tiny-snapshot",
            "--train-ratio",
            "0.6",
            "--validation-ratio",
            "0.2",
            "--test-ratio",
            "0.2",
            "--seed",
            "7",
        ],
    );
    let snapshot_id = string_at(&snapshot, "/snapshot/id");

    let encoder = run_json(
        &database_url,
        [
            "encoder",
            "register",
            "--name",
            "tiny-bert",
            path(&bundle_path),
        ],
    );
    let encoder_id = string_at(&encoder, "/id");
    assert!(string_at(&encoder, "/fingerprint").starts_with("sha256:"));
    let verified = run_json(&database_url, ["encoder", "verify", &encoder_id]);
    assert_eq!(verified["verified"], true);

    let training = run_json(
        &database_url,
        [
            "training",
            "run",
            &snapshot_id,
            "--backend",
            "bert-cpu",
            "--encoder-id",
            &encoder_id,
            "--epochs",
            "2",
            "--learning-rate",
            "0.01",
            "--checkpoint-every",
            "1",
            "--maximum-sequence-length",
            "8",
            "--batch-size",
            "4",
            "--warmup-ratio",
            "0",
            "--weight-decay",
            "0",
            "--artifact-root",
            path(&artifact_root),
        ],
    );
    assert_eq!(training["run"]["state"], "completed");
    assert_eq!(training["run"]["base_model_id"], encoder_id);
    assert!(training["run"]["processed_examples"].as_u64().unwrap() > 0);
    let first_checkpoint = final_checkpoint(&training);

    let prediction = run_json(
        &database_url,
        [
            "training",
            "predict",
            &first_checkpoint,
            "--text",
            "billing invoice payment",
        ],
    );
    assert!(matches!(
        prediction["label"].as_str(),
        Some("billing" | "account")
    ));

    let continued = run_json(
        &database_url,
        [
            "training",
            "continue",
            &first_checkpoint,
            "--epochs",
            "1",
            "--artifact-root",
            path(&artifact_root),
        ],
    );
    assert_eq!(continued["run"]["state"], "completed");
    assert_eq!(continued["run"]["parent_checkpoint_id"], first_checkpoint);
    let continued_checkpoint = final_checkpoint(&continued);

    let evaluation = run_json(
        &database_url,
        [
            "evaluation",
            "run",
            &continued_checkpoint,
            "--split",
            "test",
        ],
    );
    assert_eq!(evaluation["run"]["state"], "completed");
    assert_eq!(evaluation["predictions"].as_array().unwrap().len(), 4);
    let evaluation_id = string_at(&evaluation, "/run/id");
    let trace = run_json(
        &database_url,
        ["provenance", "evaluation-run", &evaluation_id],
    );
    let trace = serde_json::to_string(&trace).expect("trace JSON");
    for kind in [
        "evaluation_run",
        "checkpoint",
        "training_run",
        "base_model",
        "snapshot",
    ] {
        assert!(trace.contains(kind), "trace is missing {kind}");
    }

    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
}

fn final_checkpoint(training: &Value) -> String {
    training["checkpoints"]
        .as_array()
        .expect("checkpoint array")
        .iter()
        .find(|checkpoint| checkpoint["is_final"] == true)
        .and_then(|checkpoint| checkpoint["id"].as_str())
        .expect("final checkpoint")
        .to_owned()
}

fn run_json<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args(["--database-url", database_url, "--output", "json"])
        .args(arguments)
        .output()
        .expect("CLI starts");
    assert!(
        output.status.success(),
        "CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not one JSON value: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
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

fn examples_csv() -> String {
    let mut csv = String::from("text,label\n");
    for index in 0..10 {
        csv.push_str(&format!(
            "billing invoice payment charged {index},billing\n"
        ));
        csv.push_str(&format!("account login password access {index},account\n"));
    }
    csv
}
