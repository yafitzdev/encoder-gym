#![cfg(feature = "test-fixtures")]

#[path = "support/production_fixture.rs"]
mod production_fixture;

use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};
use workflow_core::ports::BenchmarkGenerationStore;

struct Fixture {
    directory: tempfile::TempDir,
    database_url: String,
    manifest: PathBuf,
}

impl Fixture {
    fn new(outcome: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            directory
                .path()
                .join("optimization.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let manifest = directory.path().join("optimize.toml");
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let seed = production_fixture::seed(&database_url).await;
            let generation = seed.fixture.store.get_benchmark_generation(seed.fixture.generation_id).await.unwrap().unwrap();
            std::fs::write(directory.path().join("fake-backend.json"), serde_json::to_vec(&json!({
                "project": seed.fixture.project,
                "generation": generation,
                "outcome": outcome,
            })).unwrap()).unwrap();
            std::fs::write(&manifest, toml::to_string(&json!({
                "schema_version": 1,
                "name": "offline bounded optimization",
                "adapter": "nomos",
                "project_source_revision": seed.fixture.project.source_revision,
                "training_snapshot_id": seed.training_snapshot.id,
                "training_snapshot_fingerprint": seed.training_snapshot.fingerprint,
                "training_snapshot_specification_fingerprint": seed.training_snapshot.specification_fingerprint,
                "benchmark_generation_id": generation.id,
                "benchmark_generation_fingerprint": generation.fingerprint,
                "approval_mode": "explicit_sealed_use",
                "selection_policy": "maximize_worst_suite_then_mean",
                "final_decision_policy": "strict_metric_contract",
            })).unwrap()).unwrap();
            seed.fixture.store.pool().close().await;
        });
        Self {
            directory,
            database_url,
            manifest,
        }
    }

    fn command(&self, real: bool, args: &[&str]) -> Command {
        let mut command = Command::new(if real {
            env!("CARGO_BIN_EXE_synth")
        } else {
            env!("CARGO_BIN_EXE_synth-optimize-fixture")
        });
        command
            .current_dir(self.directory.path())
            .args([
                "--database-url",
                &self.database_url,
                "--output",
                "json",
                "encoder",
                "optimize",
            ])
            .args(args)
            .arg("--workspace")
            .arg(self.directory.path());
        command
    }

    fn run(&self, real: bool, args: &[&str]) -> Output {
        self.command(real, args).output().unwrap()
    }

    fn json(&self, real: bool, args: &[&str]) -> Value {
        let result = self.run(real, args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&result.stdout)))
    }

    fn start(&self) -> Value {
        self.json(
            true,
            &["start", "--manifest", self.manifest.to_str().unwrap()],
        )
    }

    fn status(&self, id: &str) -> Value {
        self.json(true, &["status", id])
    }
    fn resume(&self, id: &str) -> Value {
        self.json(false, &["resume", id])
    }
    fn report(&self, id: &str) -> Value {
        self.json(true, &["report", id])
    }

    fn advance_to_boundary(&self, id: &str) -> Value {
        for _ in 0..12 {
            let state = self.status(id);
            if state["next_command"] != "resume" {
                return state;
            }
            self.resume(id);
        }
        panic!("finite optimization did not reach a boundary");
    }

    fn calls(&self) -> Vec<Value> {
        let file = self.directory.path().join("backend-calls.jsonl");
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn selected_sealed_calls(&self) -> usize {
        self.calls()
            .iter()
            .filter(|value| {
                value["operation"] == "evaluate"
                    && value["baseline"] == false
                    && value["role"] == "sealed_acceptance"
            })
            .count()
    }

    fn sql(&self, statement: &str) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let pool = sqlx::SqlitePool::connect(&self.database_url).await.unwrap();
            sqlx::query(statement).execute(&pool).await.unwrap();
            pool.close().await;
        });
    }

    fn fault(&self, table: &str, kind: Option<&str>) {
        let condition = kind
            .map(|kind| format!("WHEN json_extract(NEW.artifact_json, '$.event.kind') = '{kind}'"))
            .unwrap_or_default();
        self.sql(&format!("CREATE TRIGGER injected_interruption BEFORE INSERT ON {table} {condition} BEGIN SELECT RAISE(ABORT, 'injected child-to-parent interruption'); END"));
    }

    fn clear_fault(&self) {
        self.sql("DROP TRIGGER injected_interruption");
    }

    fn retry_stage(&self, id: &str, table: &str) -> Value {
        self.fault(table, None);
        assert!(!self.run(false, &["resume", id]).status.success());
        let calls = self.calls();
        self.clear_fault();
        let resumed = self.resume(id);
        assert_eq!(
            self.calls(),
            calls,
            "linking a completed child must not repeat backend work"
        );
        resumed
    }
}

// Owns only this test's child, so panic paths cannot leave a fixture blocked.
struct RunningFixture(Option<Child>);
impl Drop for RunningFixture {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn automatic_drive_holds_one_worker_lease_and_observes_cancellation_between_stages() {
    let fixture = Fixture::new("paused_preparation");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    let mut process = RunningFixture(Some(
        fixture
            .command(false, &["drive", id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(40);
    while !fixture.directory.path().join("preparation-ready").exists() {
        assert!(
            process.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "fixture stopped before its preparation barrier"
        );
        assert!(
            Instant::now() < deadline,
            "fixture did not reach its preparation barrier"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let running = fixture.status(id);
    assert_eq!(running["worker"]["state"], "running");
    assert_eq!(running["stopped_reason"], "execution_running");
    assert_eq!(
        running["automatic_execution_authorized_by"],
        "local-operator"
    );
    for command in ["drive", "resume"] {
        let duplicate = fixture.run(true, &[command, id]);
        assert!(!duplicate.status.success());
        assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already running"));
    }
    let cancelled = fixture.json(true, &["cancel", id, "--reason", "stop after preparation"]);
    let mut input = process.0.as_mut().unwrap().stdin.take().unwrap();
    writeln!(input, "continue").unwrap();
    drop(input);
    let output = process.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stopped: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stopped["state"], "cancelled");
    assert_eq!(stopped["head_fingerprint"], cancelled["head_fingerprint"]);
    assert_eq!(stopped["artifacts"], start["artifacts"]);
    assert_eq!(
        fixture.calls().len(),
        3,
        "only the already-running protocol preparation may finish"
    );
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| call["operation"] == "evaluate" && call["baseline"] == true)
    );
    assert_eq!(fixture.status(id)["worker"]["state"], "idle");
    assert_eq!(
        fixture.json(true, &["drive", id])["head_fingerprint"],
        stopped["head_fingerprint"]
    );
}

#[test]
fn automatic_drive_finishes_routine_work_and_preserves_terminal_identity() {
    let fixture = Fixture::new("development_rejection");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    let finished = fixture.json(false, &["drive", id, "--authorized-by", "operator"]);
    assert_eq!(finished["state"], "completed");
    assert_eq!(finished["decision"], "retain_baseline");
    assert_eq!(finished["automatic_execution_authorized_by"], "operator");
    assert_eq!(finished["artifacts"], start["artifacts"]);
    assert_eq!(fixture.calls().len(), 6);
    assert_eq!(fixture.selected_sealed_calls(), 0);
    let calls = fixture.calls();
    let repeated = fixture.json(true, &["drive", id, "--authorized-by", "operator"]);
    assert_eq!(repeated["head_fingerprint"], finished["head_fingerprint"]);
    assert_eq!(fixture.calls(), calls);
}

#[test]
fn automatic_drive_stops_for_separate_holdout_approval() {
    let fixture = Fixture::new("promote");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    let paused = fixture.json(false, &["drive", id, "--authorized-by", "operator"]);
    assert_eq!(paused["human_authorization_required"], true);
    assert_eq!(paused["next_command"], "authorize-sealed");
    assert_eq!(fixture.calls().len(), 6);
    assert_eq!(fixture.selected_sealed_calls(), 0);
    assert_eq!(
        fixture.json(true, &["drive", id, "--authorized-by", "operator"])["head_fingerprint"],
        paused["head_fingerprint"]
    );
    assert!(
        !fixture
            .run(
                true,
                &["drive", id, "--authorized-by", "different-operator"]
            )
            .status
            .success()
    );
    fixture.json(
        false,
        &["authorize-sealed", id, "--authorized-by", "operator"],
    );
    let finished = fixture.json(false, &["drive", id, "--authorized-by", "operator"]);
    assert_eq!(finished["state"], "completed");
    assert_eq!(finished["decision"], "promote_candidate");
    assert_eq!(fixture.selected_sealed_calls(), 1);
    assert_eq!(fixture.calls().len(), 7);
}

#[test]
fn automatic_drive_recovery_does_not_duplicate_completed_children() {
    for (table, kind) in [
        (
            "encoder_production_optimization_events",
            "campaign_attached",
        ),
        ("encoder_production_campaign_events", "iteration_prepared"),
        ("encoder_production_campaign_events", "run_started"),
        (
            "encoder_production_campaign_events",
            "development_completed",
        ),
        ("encoder_production_campaign_events", "iteration_finalized"),
        ("encoder_production_optimization_events", "completed"),
    ] {
        let fixture = Fixture::new("development_rejection");
        let start = fixture.start();
        let id = start["run_id"].as_str().unwrap();
        fixture.fault(table, Some(kind));
        let interrupted = fixture.run(false, &["drive", id]);
        assert!(
            !interrupted.status.success(),
            "fault was not exercised: {table}/{kind}"
        );
        assert!(
            String::from_utf8_lossy(&interrupted.stderr)
                .contains("injected child-to-parent interruption")
        );
        assert_eq!(
            fixture.status(id)["automatic_execution_authorized_by"],
            "local-operator"
        );
        let calls = fixture.calls();
        assert!(!fixture.run(false, &["drive", id]).status.success());
        assert_eq!(
            fixture.calls(),
            calls,
            "an unresolved write failure must not repeat completed backend work"
        );
        fixture.clear_fault();
        let completed = fixture.json(false, &["drive", id]);
        assert_eq!(completed["state"], "completed");
        assert_eq!(completed["artifacts"], start["artifacts"]);
        assert_eq!(fixture.calls().len(), 6);
        assert_eq!(fixture.selected_sealed_calls(), 0);
        assert_eq!(
            completed["last_sequence"], 4,
            "creation, one authorization, campaign link, completion"
        );
    }
}

#[test]
fn automatic_drive_rejects_bad_authority_and_replays_cancelled_runs_without_backend() {
    let fixture = Fixture::new("promote");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    for actor in ["", " operator", "a\nb", &"x".repeat(121)] {
        assert!(
            !fixture
                .run(true, &["drive", id, "--authorized-by", actor])
                .status
                .success()
        );
        assert_eq!(
            fixture.status(id)["head_fingerprint"],
            start["head_fingerprint"]
        );
    }
    fixture.fault(
        "encoder_production_optimization_events",
        Some("automatic_execution_authorized"),
    );
    assert!(!fixture.run(false, &["drive", id]).status.success());
    assert_eq!(
        fixture.status(id)["head_fingerprint"],
        start["head_fingerprint"]
    );
    fixture.clear_fault();
    let cancelled = fixture.json(true, &["cancel", id, "--reason", "stop before work"]);
    let repeated = fixture.json(true, &["drive", id]);
    assert_eq!(repeated["head_fingerprint"], cancelled["head_fingerprint"]);
    assert_eq!(repeated["automatic_execution_authorized_by"], Value::Null);
    assert!(fixture.calls().is_empty());
}

#[test]
fn automatic_drive_recovers_sealed_result_and_finishes_training_failure() {
    let fixture = Fixture::new("sealed_rejection");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    fixture.json(false, &["drive", id]);
    fixture.json(false, &["authorize-sealed", id]);
    fixture.fault("encoder_experiment_events", Some("finalized"));
    assert!(!fixture.run(false, &["drive", id]).status.success());
    assert_eq!(fixture.status(id)["experiment_state"], "sealed_evaluated");
    assert_eq!(fixture.selected_sealed_calls(), 1);
    let calls = fixture.calls();
    fixture.clear_fault();
    let finished = fixture.json(false, &["drive", id]);
    assert_eq!(finished["decision"], "retain_baseline");
    assert_eq!(fixture.calls(), calls);

    let fixture = Fixture::new("training_failure");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    let finished = fixture.json(false, &["drive", id]);
    assert_eq!(finished["state"], "completed");
    assert_eq!(finished["decision"], "retain_baseline");
    assert_eq!(fixture.calls().len(), 4);
    assert_eq!(fixture.selected_sealed_calls(), 0);
    assert_eq!(
        fixture.report(id)["budget_and_recovery"]["observed_training_seconds"],
        0
    );
}

#[test]
fn new_non_adopted_optimization_rejects_development_and_replays_terminal_commands() {
    let fixture = Fixture::new("development_rejection");
    let preview = fixture.json(
        true,
        &["preview", "--manifest", fixture.manifest.to_str().unwrap()],
    );
    assert_eq!(preview["persisted"], false);
    assert_eq!(preview["adopts_existing_experiment"], false);
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    assert_eq!(fixture.start()["run_id"], id);
    let completed = fixture.advance_to_boundary(id);
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["decision"], "retain_baseline");
    assert_eq!(fixture.selected_sealed_calls(), 0);
    let report = fixture.report(id);
    assert_eq!(
        report["budget_and_recovery"]["adopted_previously_proven_run"],
        false
    );
    assert_eq!(
        report["budget_and_recovery"]["observed_development_evaluations"],
        2
    );
    assert_eq!(report["training_data_change"]["total_rows"], 13);
    assert_eq!(fixture.calls().len(), 6);
    let calls = fixture.calls();
    assert_eq!(
        fixture.json(true, &["resume", id])["head_fingerprint"],
        completed["head_fingerprint"]
    );
    assert_eq!(
        fixture.start()["head_fingerprint"],
        completed["head_fingerprint"]
    );
    assert_eq!(fixture.calls(), calls);
}

#[test]
fn cancellation_reports_are_available_before_any_protocol_or_backend_exists() {
    let fixture = Fixture::new("promote");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    let planned = fixture.report(id);
    assert_eq!(planned["state"], "planned");
    let cancelled = fixture.json(
        true,
        &[
            "cancel",
            id,
            "--reason",
            "operator stopped before execution",
        ],
    );
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(fixture.report(id)["state"], "cancelled");
    assert_eq!(
        fixture.json(true, &["resume", id])["head_fingerprint"],
        cancelled["head_fingerprint"]
    );
    assert!(fixture.calls().is_empty());
}

#[test]
fn cancellation_after_protocol_preparation_prevents_training_and_preserves_artifacts() {
    let fixture = Fixture::new("promote");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    fixture.resume(id);
    let prepared = fixture.resume(id);
    assert_eq!(prepared["campaign_state"], "ready_to_start");
    let calls = fixture.calls();
    assert_eq!(calls.len(), 3);
    let cancelled = fixture.json(true, &["cancel", id, "--reason", "stop before training"]);
    assert_eq!(cancelled["state"], "cancelled");
    let repeated = fixture.json(true, &["resume", id]);
    assert_eq!(repeated["head_fingerprint"], cancelled["head_fingerprint"]);
    assert_eq!(repeated["artifacts"], start["artifacts"]);
    let report = fixture.report(id);
    assert_eq!(report["state"], "cancelled");
    assert_eq!(
        report["budget_and_recovery"]["observed_training_seconds"],
        0
    );
    assert_eq!(fixture.calls(), calls);
    assert_eq!(fixture.selected_sealed_calls(), 0);
}

#[test]
fn full_parent_recovers_durable_children_and_uses_exactly_one_authorized_sealed_result() {
    let fixture = Fixture::new("promote");
    let start = fixture.start();
    let id = start["run_id"].as_str().unwrap();
    assert!(
        !fixture
            .run(
                false,
                &["authorize-sealed", id, "--authorized-by", "operator"]
            )
            .status
            .success()
    );
    let campaign = "encoder_production_campaign_events";
    let optimization = "encoder_production_optimization_events";
    let attached = fixture.retry_stage(id, optimization);
    assert_eq!(attached["campaign_state"], "ready_to_prepare");
    let prepared = fixture.retry_stage(id, campaign);
    assert_eq!(prepared["campaign_state"], "ready_to_start");
    assert_eq!(fixture.calls().len(), 3);
    let running = fixture.retry_stage(id, campaign);
    assert_eq!(running["campaign_state"], "running_development");
    let awaiting = fixture.retry_stage(id, campaign);
    assert_eq!(awaiting["campaign_state"], "awaiting_sealed_authorization");
    assert_eq!(fixture.calls().len(), 6);
    let calls = fixture.calls();
    assert_eq!(
        fixture.resume(id)["campaign_state"],
        "awaiting_sealed_authorization"
    );
    assert_eq!(fixture.calls(), calls);
    assert_eq!(fixture.selected_sealed_calls(), 0);

    fixture.fault(campaign, None);
    assert!(
        !fixture
            .run(
                false,
                &["authorize-sealed", id, "--authorized-by", "operator"]
            )
            .status
            .success()
    );
    fixture.clear_fault();
    assert!(
        !fixture
            .run(
                false,
                &[
                    "authorize-sealed",
                    id,
                    "--authorized-by",
                    "different-operator"
                ]
            )
            .status
            .success()
    );
    let authorized = fixture.json(
        false,
        &["authorize-sealed", id, "--authorized-by", "operator"],
    );
    assert_eq!(authorized["campaign_state"], "sealed_authorized");
    let repeated = fixture.json(
        false,
        &["authorize-sealed", id, "--authorized-by", "operator"],
    );
    assert_eq!(authorized, repeated);
    assert_eq!(fixture.selected_sealed_calls(), 0);

    fixture.fault("encoder_experiment_events", Some("finalized"));
    assert!(!fixture.run(false, &["resume", id]).status.success());
    assert_eq!(fixture.status(id)["experiment_state"], "sealed_evaluated");
    fixture.clear_fault();
    let finalized = fixture.resume(id);
    assert_eq!(finalized["campaign_state"], "renewal_required");
    assert_eq!(fixture.selected_sealed_calls(), 1);
    fixture.resume(id);
    let completed = fixture.retry_stage(id, optimization);
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["decision"], "promote_candidate");
    assert_eq!(completed["artifacts"], start["artifacts"]);
    let report = fixture.report(id);
    assert_eq!(report["decision"], "promote_candidate");
    assert_eq!(report["sealed_evidence"]["generation_state"], "exhausted");
    assert_eq!(report["sealed_evidence"]["candidate_exposures"], 1);
    assert_eq!(
        report["budget_and_recovery"]["observed_training_seconds"],
        1
    );
    assert_eq!(
        report["budget_and_recovery"]["observed_development_evaluations"],
        2
    );
    assert_eq!(
        report["budget_and_recovery"]["observed_sealed_evaluations"],
        1
    );
    assert!(
        !report["known_evidence_limits"]
            .to_string()
            .contains("failed development")
    );
    assert_eq!(fixture.calls().len(), 7);
    let provenance = fixture.json(false, &["provenance", id]);
    assert_eq!(fixture.json(false, &["provenance", id]), provenance);
}

#[test]
fn sealed_rejection_and_training_failure_retain_baseline_with_honest_usage() {
    for outcome in ["sealed_rejection", "training_failure"] {
        let fixture = Fixture::new(outcome);
        let start = fixture.start();
        let id = start["run_id"].as_str().unwrap();
        let mut state = fixture.advance_to_boundary(id);
        if state["next_command"] == "authorize-sealed" {
            fixture.json(
                false,
                &["authorize-sealed", id, "--authorized-by", "operator"],
            );
            state = fixture.advance_to_boundary(id);
        }
        assert_eq!(state["state"], "completed");
        assert_eq!(state["decision"], "retain_baseline");
        let report = fixture.report(id);
        if outcome == "sealed_rejection" {
            assert_eq!(fixture.selected_sealed_calls(), 1);
            assert!(
                !report["known_evidence_limits"]
                    .to_string()
                    .contains("failed development")
            );
        } else {
            assert_eq!(fixture.selected_sealed_calls(), 0);
            assert_eq!(report["budget_and_recovery"]["candidate_failure_events"], 1);
            assert_eq!(
                report["budget_and_recovery"]["observed_development_evaluations"],
                0
            );
        }
    }
}
