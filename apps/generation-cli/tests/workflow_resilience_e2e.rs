pub mod support;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::Value;

use support::{
    run_json,
    workflow_fixture::{GenerationMode, WorkflowFixture},
};

#[test]
fn interrupted_workflow_resumes_once_and_then_stays_idempotent() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let initialized = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id, "--initialize-only"],
    );
    let run_id = string_at(&initialized, "/run/id");
    assert_eq!(initialized["attempt"]["stage"], "initial_allocation");

    let recovery = run_json(fixture.database_url(), ["recovery", "list"]);
    assert!(
        recovery
            .as_array()
            .expect("recovery list")
            .iter()
            .any(|record| record["workflow_kind"] == "encoder_workflow"
                && record["workflow_id"] == run_id)
    );

    let resumed = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert!(matches!(
        resumed["run"]["state"].as_str(),
        Some("awaiting_approval" | "development_complete")
    ));
    assert_eq!(artifact_count(&resumed, "initial_allocation"), 1);
    assert_eq!(artifact_count(&resumed, "generation_plan"), 1);

    let attempt_count = resumed["attempt_count"].clone();
    let latest_attempt_id = resumed["latest_attempt"]["id"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(repeated["latest_attempt"]["id"], latest_attempt_id);
    assert_eq!(artifact_count(&repeated, "initial_allocation"), 1);
    assert_eq!(artifact_count(&repeated, "generation_plan"), 1);

    let unresolved = run_json(fixture.database_url(), ["recovery", "list"]);
    assert!(
        !unresolved
            .as_array()
            .expect("recovery list")
            .iter()
            .any(|record| record["workflow_kind"] == "encoder_workflow"
                && record["workflow_id"] == run_id)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
fn cancellation_is_persisted_before_work_starts_and_resume_is_safe() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let initialized = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id, "--initialize-only"],
    );
    let run_id = string_at(&initialized, "/run/id");

    let requested = run_json(fixture.database_url(), ["workflow", "cancel", &run_id]);
    assert_eq!(requested["cancel_requested"], true);
    let cancelled = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["latest_attempt"]["state"], "cancelled");
    assert_eq!(artifact_count(&cancelled, "generation_job"), 0);

    let attempt_count = cancelled["attempt_count"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["run"]["state"], "cancelled");
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
fn failing_backend_retries_are_bounded_and_append_only() {
    let server = AlwaysFailingServer::start();
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible {
        base_url: server.base_url(),
        model: "always-fails".into(),
    });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let dataset_id = string_at(&prepared, "/preparation/dataset_id");

    let first = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let run_id = string_at(&first, "/run/id");
    assert_failed_generation_attempt(&first, 1, true);

    let second = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_failed_generation_attempt(&second, 2, false);
    let mut failed_attempts = generation_failures(&second);
    failed_attempts.sort_by_key(|attempt| attempt["attempt"].as_u64());
    assert_eq!(failed_attempts.len(), 2);
    let retry_start = second["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .find(|attempt| {
            attempt["stage"] == "generation"
                && attempt["attempt"] == 2
                && attempt["state"] == "running"
        })
        .expect("persisted retry start");
    assert_eq!(retry_start["predecessor_id"], failed_attempts[0]["id"]);
    assert_eq!(failed_attempts[1]["predecessor_id"], retry_start["id"]);

    let attempt_count = second["attempt_count"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(generation_failures(&repeated).len(), 2);

    let rows = run_json(
        fixture.database_url(),
        ["rows", "--dataset-id", &dataset_id, "--status", "accepted"],
    );
    assert_eq!(rows, serde_json::json!([]));
    let jobs = run_json(
        fixture.database_url(),
        ["job", "list", "--dataset-id", &dataset_id],
    );
    assert_eq!(jobs.as_array().expect("jobs").len(), 2);
    assert!(
        jobs.as_array()
            .expect("jobs")
            .iter()
            .all(|job| job["state"] == "failed")
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
#[ignore = "requires an explicitly configured live endpoint and may incur provider cost"]
fn live_openai_compatible_smoke_is_explicitly_opt_in() {
    let base_url = std::env::var("SYNTH_E2E_OPENAI_BASE_URL")
        .expect("set SYNTH_E2E_OPENAI_BASE_URL before running the ignored smoke test");
    let model = std::env::var("SYNTH_E2E_OPENAI_MODEL")
        .expect("set SYNTH_E2E_OPENAI_MODEL before running the ignored smoke test");
    assert!(
        std::env::var_os("SYNTH_OPENAI_API_KEY").is_some(),
        "set SYNTH_OPENAI_API_KEY before running the ignored smoke test"
    );
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible { base_url, model });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let result = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    assert_ne!(result["run"]["state"], "failed");
    assert!(artifact_count(&result, "generation_job") > 0);
}

fn assert_failed_generation_attempt(status: &Value, attempt: u64, retryable: bool) {
    assert_eq!(
        status["run"]["state"],
        if retryable { "awaiting_user" } else { "failed" }
    );
    assert_eq!(status["latest_attempt"]["stage"], "generation");
    assert_eq!(status["latest_attempt"]["state"], "failed");
    assert_eq!(status["latest_attempt"]["attempt"], attempt);
    assert_eq!(status["latest_attempt"]["retryable"], retryable);
}

fn generation_failures(status: &Value) -> Vec<&Value> {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .filter(|attempt| attempt["stage"] == "generation" && attempt["state"] == "failed")
        .collect()
}

fn artifact_count(status: &Value, kind: &str) -> usize {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .filter(|artifact| artifact["kind"] == kind)
        .count()
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

struct AlwaysFailingServer {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AlwaysFailingServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server binds");
        let address = listener.local_addr().expect("mock server address");
        listener
            .set_nonblocking(true)
            .expect("mock server becomes nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => fail_request(&mut stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock server accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }
}

impl Drop for AlwaysFailingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock server stops");
        }
    }
}

fn fail_request(stream: &mut TcpStream) {
    let mut request = [0_u8; 8192];
    let _ = stream.read(&mut request);
    stream
        .write_all(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .expect("mock response writes");
}
