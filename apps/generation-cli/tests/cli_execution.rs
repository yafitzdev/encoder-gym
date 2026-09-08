pub mod support;

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use support::{run, run_json};

#[path = "support/prediction_failure.rs"]
mod prediction_failure;

fn project(directory: &Path) -> (String, Value) {
    let database_url = format!(
        "sqlite://{}",
        directory
            .join("execution.db")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let config = directory.join("project.toml");
    std::fs::write(
        &config,
        r#"version = 1
[dataset]
name = "execution-contract"
task = "Classify fixture text"
labels = ["a", "b"]
[generation]
target_per_cell = 3
backend = "fake"
"#,
    )
    .unwrap();
    let initialized = run_json(&database_url, ["config", "init", config.to_str().unwrap()]);
    (database_url, initialized)
}

#[test]
fn failed_generation_is_nonzero_and_keeps_one_inspectable_json_result() {
    let directory = tempfile::tempdir().unwrap();
    let (database_url, initialized) = project(directory.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = [0; 8192];
                    assert!(stream.read(&mut bytes).unwrap() > 0);
                    stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "generation never called the local fixture"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture server failed: {error}"),
            }
        }
    });
    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            &endpoint,
            "--model",
            "local-fixture",
        ],
    );
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args([
            "--database-url",
            &database_url,
            "--output",
            "json",
            "generate",
            initialized["generation_plan"]["id"].as_str().unwrap(),
            "--backend",
            "openai-compatible",
            "--max-retries",
            "0",
            "--api-key-env",
            "SYNTH_TEST_EXECUTION_KEY",
        ])
        .env("SYNTH_TEST_EXECUTION_KEY", "local-fixture-only")
        .env("RUST_LOG", "error")
        .output()
        .unwrap();
    server.join().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["state"], "failed");
    assert_eq!(result["accepted_rows"], 0);
    assert_eq!(result["failed_requests"], 1);
    assert!(
        !output.status.success(),
        "failed execution must not report process success"
    );
    let id = result["id"].as_str().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains(id));
    let historical = run_json(&database_url, ["job", "status", id]);
    assert_eq!(historical, result, "inspection remains a successful read");
}

#[test]
fn checkpoint_write_failure_is_nonzero_with_durable_training_facts() {
    let directory = tempfile::tempdir().unwrap();
    let (database_url, initialized) = project(directory.path());
    run_json(
        &database_url,
        [
            "generate",
            initialized["generation_plan"]["id"].as_str().unwrap(),
        ],
    );
    let snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            initialized["dataset"]["id"].as_str().unwrap(),
            "--name",
            "training",
            "--train-ratio",
            "1",
            "--validation-ratio",
            "0",
            "--test-ratio",
            "0",
        ],
    );
    let blocked = directory.path().join("artifact-root-is-a-file");
    std::fs::write(&blocked, "fixture").unwrap();
    let output = run(
        &database_url,
        [
            "training",
            "run",
            snapshot["snapshot"]["id"].as_str().unwrap(),
            "--epochs",
            "1",
            "--artifact-root",
            blocked.to_str().unwrap(),
        ],
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run"]["state"], "failed");
    assert_eq!(result["checkpoints"], serde_json::json!([]));
    assert!(
        !output.status.success(),
        "failed checkpoint publication is an execution failure"
    );
    let historical = run_json(
        &database_url,
        ["training", "status", result["run"]["id"].as_str().unwrap()],
    );
    assert_eq!(historical, result["run"]);
}

#[test]
fn diagnostic_logging_never_enters_json_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let (database_url, _) = project(directory.path());
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args([
            "--database-url",
            &database_url,
            "--output",
            "json",
            "dataset",
            "list",
        ])
        .env("RUST_LOG", "sqlx::query=debug")
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("stdout is one JSON value even with dependency diagnostics enabled");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert!(
        !output.stderr.is_empty(),
        "diagnostics must still be available on stderr"
    );
}

#[test]
fn failed_prediction_is_nonzero_and_preserves_the_failed_evaluation() {
    let directory = tempfile::tempdir().unwrap();
    let (database_url, initialized) = project(directory.path());
    run_json(
        &database_url,
        [
            "generate",
            initialized["generation_plan"]["id"].as_str().unwrap(),
        ],
    );
    let snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            initialized["dataset"]["id"].as_str().unwrap(),
            "--name",
            "prediction-failure",
            "--train-ratio",
            "1",
            "--validation-ratio",
            "0",
            "--test-ratio",
            "0",
        ],
    );
    let snapshot_id = snapshot["snapshot"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let checkpoint_id =
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(prediction_failure::checkpoint(
                &database_url,
                snapshot_id,
                directory.path(),
            ));
    let output = run(
        &database_url,
        [
            "evaluation",
            "run",
            &checkpoint_id.to_string(),
            "--split",
            "train",
        ],
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run"]["state"], "failed");
    assert_eq!(result["run"]["processed_examples"], 0);
    assert!(
        result["run"]["error_message"]
            .as_str()
            .unwrap()
            .contains("finite")
    );
    assert!(
        !output.status.success(),
        "prediction failure must propagate through the CLI"
    );
    assert_eq!(
        run_json(
            &database_url,
            [
                "evaluation",
                "status",
                result["run"]["id"].as_str().unwrap()
            ]
        ),
        result["run"]
    );
}
