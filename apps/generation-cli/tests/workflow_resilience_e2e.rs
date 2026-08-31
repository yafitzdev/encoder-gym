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

use dataset_quality_core::policy::{EvaluatorEgressPolicy, QualityPreset};
use serde_json::Value;
use workflow_core::workflow::{WorkflowQualityAuthenticity, WorkflowQualityGateRequest};

use support::{
    run_json,
    workflow_fixture::{GenerationMode, WorkflowFixture},
};

#[test]
fn quality_gated_workflow_pauses_for_the_exact_latest_manifest_and_trains_qualified_data() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-workflow.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let manifest_path = manifest_path.to_str().expect("UTF-8 manifest path");
    let prepared = run_json(
        fixture.database_url(),
        ["project", "prepare", manifest_path],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");

    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    assert_eq!(paused["run"]["state"], "awaiting_user");
    assert_eq!(
        paused["latest_attempt"]["stage"], "curation_review",
        "quality-gated workflow did not reach its review pause: {paused}"
    );
    assert_eq!(
        paused["latest_attempt"]["state"], "awaiting_user",
        "quality-gated workflow did not persist its review pause: {paused}"
    );
    assert_eq!(artifact_count(&paused, "quality_audit_plan"), 1);
    assert_eq!(artifact_count(&paused, "quality_audit_run"), 1);
    assert_eq!(artifact_count(&paused, "quality_report"), 1);
    let initial_proposal_id = latest_artifact_id(&paused, "curation_proposal");
    let audit_run_id = latest_artifact_id(&paused, "quality_audit_run");

    let repeated = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(repeated["attempt_count"], paused["attempt_count"]);
    assert_eq!(
        repeated["latest_attempt"]["id"],
        paused["latest_attempt"]["id"]
    );
    assert_eq!(
        latest_artifact_id(&repeated, "curation_proposal"),
        initial_proposal_id
    );

    // Approve the displayed proposal, then append newer row reviews. Resume
    // must materialize a successor and refuse the now-stale manifest.
    let stale_approval = run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &initial_proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve before the later append-only row reviews",
        ],
    );
    let stale_manifest_id = string_at(&stale_approval, "/manifest/id");
    let assessments = run_json(
        fixture.database_url(),
        ["quality", "assessments", &audit_run_id, "--limit", "100"],
    );
    let assessments = assessments.as_array().expect("quality assessments");
    assert!(!assessments.is_empty());
    for assessment in assessments {
        let assessment_id = string_at(assessment, "/id");
        run_json(
            fixture.database_url(),
            [
                "quality",
                "row-review",
                &assessment_id,
                "--include",
                "--reviewer",
                "workflow-test",
                "--reason",
                "include this row in the workflow training snapshot",
            ],
        );
    }

    let refreshed_pause = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(refreshed_pause["run"]["state"], "awaiting_user");
    assert_eq!(
        refreshed_pause["latest_attempt"]["stage"],
        "curation_review"
    );
    assert_eq!(artifact_count(&refreshed_pause, "snapshot"), 0);
    let successor_proposal_id = latest_artifact_id(&refreshed_pause, "curation_proposal");
    assert_ne!(successor_proposal_id, initial_proposal_id);
    assert_eq!(
        latest_artifact_id(&refreshed_pause, "curation_proposal"),
        successor_proposal_id
    );
    assert!(
        !refreshed_pause["attempts"]
            .as_array()
            .expect("attempts")
            .iter()
            .flat_map(|attempt| attempt["artifacts"].as_array().expect("artifacts"))
            .any(|artifact| artifact["kind"] == "quality_manifest"
                && artifact["artifact_id"] == stale_manifest_id),
        "a stale approved manifest must not enter workflow history"
    );

    let approved = run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &successor_proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve the exact latest reviewed proposal",
        ],
    );
    let manifest_id = string_at(&approved, "/manifest/id");
    let resumed = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert!(matches!(
        resumed["run"]["state"].as_str(),
        Some("awaiting_approval" | "development_complete")
    ));
    assert_eq!(artifact_count(&resumed, "quality_manifest"), 1);
    assert_eq!(artifact_count(&resumed, "curation_application"), 1);
    assert_eq!(artifact_count(&resumed, "snapshot"), 1);
    assert_eq!(artifact_count(&resumed, "training_run"), 1);
    let snapshot_id = latest_artifact_id(&resumed, "snapshot");
    let shown = run_json(fixture.database_url(), ["snapshot", "show", &snapshot_id]);
    assert_eq!(shown["qualified"], true);
    assert_eq!(shown["manifest_id"], manifest_id);
    let qualified_member_ids = run_json(
        fixture.database_url(),
        ["snapshot", "members", &snapshot_id],
    )
    .as_array()
    .expect("qualified snapshot members")
    .iter()
    .map(|member| string_at(member, "/source_row_id"))
    .collect::<std::collections::BTreeSet<_>>();
    for evidence_snapshot_id in [fixture.development_snapshot_id, fixture.sealed_snapshot_id] {
        let evidence_member_ids = run_json(
            fixture.database_url(),
            ["snapshot", "members", &evidence_snapshot_id.to_string()],
        )
        .as_array()
        .expect("evaluation snapshot members")
        .iter()
        .map(|member| string_at(member, "/source_row_id"))
        .collect::<std::collections::BTreeSet<_>>();
        assert!(
            qualified_member_ids.is_disjoint(&evidence_member_ids),
            "qualified training data must be disjoint from every evaluation cohort"
        );
    }
    let provenance = run_json(
        fixture.database_url(),
        ["provenance", "workflow-run", &workflow_run_id],
    );
    let provenance = serde_json::to_string(&provenance).expect("workflow provenance JSON");
    for kind in [
        "quality_audit_plan",
        "quality_audit_run",
        "dataset_quality_report",
        "curation_proposal",
        "curation_manifest_review",
        "approved_curation_manifest",
        "curation_application",
    ] {
        assert!(
            provenance.contains(kind),
            "quality-gated workflow provenance omitted {kind}: {provenance}"
        );
    }
}

#[test]
fn same_name_unqualified_snapshot_cannot_satisfy_a_workflow_quality_gate() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-collision.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let manifest_path = manifest_path.to_str().expect("UTF-8 manifest path");
    let prepared = run_json(
        fixture.database_url(),
        ["project", "prepare", manifest_path],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let dataset_id = string_at(&prepared, "/preparation/dataset_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    let proposal_id = latest_artifact_id(&paused, "curation_proposal");
    run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve the exact displayed proposal",
        ],
    );

    let colliding_name = format!("baseline-workflow-{workflow_run_id}-0");
    run_json(
        fixture.database_url(),
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            &colliding_name,
            "--train-ratio",
            "0.8",
            "--validation-ratio",
            "0.0",
            "--test-ratio",
            "0.2",
            "--seed",
            "42",
        ],
    );
    let rejected = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(rejected["run"]["state"], "awaiting_user");
    assert_eq!(rejected["latest_attempt"]["stage"], "snapshot");
    assert_eq!(rejected["latest_attempt"]["state"], "failed");
    assert!(
        rejected["latest_attempt"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("not qualified by the exact approved manifest"))
    );
    assert_eq!(artifact_count(&rejected, "training_run"), 0);
}

#[test]
fn quality_gated_workflow_can_be_cancelled_while_waiting_for_manifest_approval() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-cancellation.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let prepared = run_json(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest_path.to_str().expect("UTF-8 manifest path"),
        ],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    assert_eq!(paused["latest_attempt"]["stage"], "curation_review");
    assert_eq!(paused["latest_attempt"]["state"], "awaiting_user");

    let requested = run_json(
        fixture.database_url(),
        ["workflow", "cancel", &workflow_run_id],
    );
    assert_eq!(requested["cancel_requested"], true);
    let cancelled = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["latest_attempt"]["state"], "cancelled");
    assert_eq!(artifact_count(&cancelled, "snapshot"), 0);
    assert_eq!(artifact_count(&cancelled, "training_run"), 0);
}

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
    assert!(
        artifact_count(&result, "generation_job") > 0,
        "live workflow produced no generation job: {}",
        live_failure_summary(&result)
    );
    assert_ne!(
        result["run"]["state"],
        "failed",
        "live workflow failed: {}",
        live_failure_summary(&result)
    );
}

fn live_failure_summary(status: &Value) -> String {
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

fn latest_artifact_id(status: &Value, kind: &str) -> String {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .rev()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .find(|artifact| artifact["kind"] == kind)
        .and_then(|artifact| artifact["artifact_id"].as_str())
        .unwrap_or_else(|| panic!("workflow has no {kind} artifact: {status}"))
        .to_owned()
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
