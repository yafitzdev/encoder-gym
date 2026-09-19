//! Real CLI, real saved native diagnostics and dataset adapters; deterministic
//! Pi wire responses and one loopback generator. No trainer/provider overrides
//! exist in the production command.
use super::*;
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    DatasetPurpose, OptimizationAgentSettings, ProjectOptimizationRun, ProviderAuthentication,
    ProviderCatalog, ProviderConfiguration, ProviderKind, ProviderLimits, ProviderRole,
};
use project_workspace_local::{
    dataset_versions, import_dataset, inspect_dataset, optimization_launch, optimization_runs,
    optimization_setup, record_provider_catalog,
};
use sqlx::{Connection, SqliteConnection};
use std::io::{Read, Write};
#[path = "optimization_cases_check.rs"]
mod cases_check;
#[path = "optimization_agent_crash_check.rs"]
mod crash_check;
#[path = "optimization_agent_final_check.rs"]
mod final_check;
#[path = "optimization_agent_loop_check.rs"]
mod loop_check;
#[path = "optimization_agent_stop_check.rs"]
mod stop_check;
#[path = "optimization_training_time_check.rs"]
mod training_time_check;

fn native_row(id: &str, question: &str) -> Value {
    let tool = |id| json!({"tool_id":id,"tool_family":"search","description":"Find evidence","capabilities":["search"],"input_modalities":["text"],"output_modalities":["text"],"evidence_roles":["primary"],"side_effect_class":"none","argument_schema":{}});
    json!({"schema_version":"decision-state.v2","decision_state_id":id,"question":question,"evaluation_partition":"train","accepted":true,"task_kind":"route","previous_candidate_ids":[],"legal_candidate_ids":["a","b"],"label":{"acceptable_tools":["a"],"hard_negative_tools":["b"]},"tool_registry":{"registry_id":"r","registry_fingerprint":"sha256:registry","tools":[tool("a"),tool("b")]}})
}

async fn install_trigger(database: &Path, statement: &str) {
    let mut connection = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    sqlx::query(statement)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
}

async fn drop_trigger(database: &Path, name: &str) {
    let mut connection = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    sqlx::query(&format!("DROP TRIGGER {name}"))
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
}

fn assert_injected(output: std::process::Output, message: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn telemetry(output: &std::process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| line.strip_prefix("ENCODER_GYM_PROGRESS "))
        .filter_map(|json| serde_json::from_str::<Value>(json).ok())
        .filter(|event| event.get("training").is_some())
        .collect()
}

#[tokio::test]
async fn cli_agent_inspects_generates_qualifies_and_replays_exact_dataset_without_training() {
    scenario(false, None).await;
}

#[tokio::test]
async fn cli_v3_repairs_reviews_and_publishes_only_native_semantic_admissions() {
    scenario(false, Some("v3_prepare")).await;
}

#[tokio::test]
async fn cli_v3_wrong_label_review_is_durable_and_never_published() {
    scenario(false, Some("v3_semantic_reject")).await;
}

#[tokio::test]
async fn cli_agent_preflight_blocks_irreparable_starting_dataset_before_provider_dispatch() {
    scenario(true, Some("dirty_preflight")).await;
}

#[tokio::test]
async fn cli_agent_edits_qualified_data_trains_exact_sample_evaluates_and_replays() {
    scenario(true, None).await;
}

#[tokio::test]
async fn cli_agent_loop_uses_previous_result_and_recovers_completion_without_repeating_work() {
    scenario(true, Some("two_iterations")).await;
}

#[tokio::test]
async fn cli_agent_loop_no_change_ends_without_another_training_or_generation() {
    scenario(true, Some("no_change")).await;
}

#[tokio::test]
async fn cli_agent_loop_stops_at_cumulative_row_budget_before_another_call() {
    scenario(true, Some("row_limit")).await;
}

#[tokio::test]
async fn cli_agent_run_request_ceiling_stops_before_another_call_and_cannot_retry() {
    scenario(true, Some("agent_limit")).await;
}

#[tokio::test]
async fn cli_generation_run_token_ceiling_stops_before_dispatch_and_cannot_retry() {
    scenario(true, Some("generation_limit")).await;
}

#[tokio::test]
async fn cli_canary_rejection_prevents_remaining_batches_publication_and_training() {
    scenario(true, Some("canary_rejected")).await;
}

#[tokio::test]
async fn cli_agent_loop_keeps_best_eligible_dataset_but_inspects_latest_result() {
    scenario(true, Some("eligible")).await;
}

#[tokio::test]
async fn cli_agent_final_rejection_cannot_promote_a_development_winner() {
    scenario(true, Some("eligible_rejected")).await;
}

#[tokio::test]
async fn cli_agent_loop_can_finish_first_analysis_without_edits_or_training() {
    scenario(true, Some("no_change_first")).await;
}

#[tokio::test]
async fn cli_agent_stop_and_explicit_resume_preserve_completed_work_and_unknown_call_charge() {
    scenario(true, Some("stop_resume")).await;
}

#[tokio::test]
async fn cli_agent_training_stop_and_resume_preserve_cumulative_time() {
    scenario(true, Some("training_stop")).await;
}

#[tokio::test]
async fn cli_agent_training_lost_accounting_reuses_output_without_refunding_unknown_time() {
    scenario(true, Some("training_settlement")).await;
}

#[tokio::test]
async fn cli_agent_training_deadline_exhausts_budget_without_repeating_native_work() {
    scenario(true, Some("training_timeout")).await;
}

#[cfg(windows)]
#[tokio::test]
async fn cli_agent_recovers_abrupt_death_at_each_artifact_boundary_without_repeating_work() {
    scenario(true, Some("crash_boundaries")).await;
}

#[cfg(windows)]
#[tokio::test]
async fn cli_agent_crash_stops_native_descendants_and_retains_unknown_training_charge() {
    scenario(true, Some("training_crash")).await;
}

async fn scenario(complete: bool, loop_mode: Option<&str>) {
    let final_rejected = loop_mode == Some("eligible_rejected");
    let loop_mode = if final_rejected {
        Some("eligible")
    } else {
        loop_mode
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, project) = initial_benchmark_fixture(
        root,
        Path::new(env!("CARGO_BIN_EXE_synth-benchmark-fixture")),
    )
    .await;
    let benchmark: project_workspace_core::ProjectBenchmarkVersion = serde_json::from_value(
        run(root, &["benchmark", "project", "initialize"])["version"].clone(),
    )
    .unwrap();
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let protocol = store
        .get_protocol(benchmark.source.protocol.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    store.pool().close().await;
    // Saved predictions, not a re-evaluation. The native reader verifies model,
    // suite and metrics and projects only permitted disagreement fields.
    for report in protocol.baseline_development_reports() {
        let suite = &project.task_configuration["suites"][&report.suite_key];
        let path = root
            .join("runtime/runs/encoder-gym-evaluations/by-content")
            .join(&report.model.fingerprint[7..])
            .join("retrieval")
            .join(&suite["retrieval_fingerprint"].as_str().unwrap()[7..])
            .join(format!("{}.json", report.suite_key));
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        saved["model"] = report.model.key.clone().into();
        let count = if loop_mode == Some("eligible") { 50 } else { 1 };
        if count == 50 {
            saved["inputs"][suite["path"].as_str().unwrap()]["metrics"]["states"] = 50.into();
        }
        saved["inputs"][suite["path"].as_str().unwrap()]["disagreements"] = json!((0..count).map(|index| json!({
            "decision_state_id":format!("dev-failure-{index}"), "question":"Search for an exact reference", "task_kind":"route", "expected_rank":2,
            "expected_capabilities":["search"], "predicted_capabilities":["write"]})).collect::<Vec<_>>());
        fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    }
    if loop_mode == Some("eligible") {
        if final_rejected {
            fs::write(
                root.join("runtime/runs/fixture-final-rejected"),
                "reject final",
            )
            .unwrap();
        }
        // Only the test-native executable interprets this fixture setting.
        fs::write(
            root.join("runtime/runs/fixture-eligible-candidates"),
            "two candidates",
        )
        .unwrap();
    }
    let source = root.join("selected.jsonl");
    let source_rows = if loop_mode == Some("dirty_preflight") {
        fs::write(root.join("runtime/runs/fixture-clearance-duplicates"), "9").unwrap();
        (0..10)
            .map(|index| native_row(&format!("duplicate-{index}"), "Search something"))
            .chain(std::iter::once(native_row(
                "keep",
                "Retain this useful example",
            )))
            .map(|row| format!("{row}\n"))
            .collect::<String>()
    } else {
        format!(
            "{}\n{}\n",
            native_row("old", "Search something"),
            native_row("keep", "Retain this useful example")
        )
    };
    fs::write(&source, source_rows).unwrap();
    let preview = inspect_dataset(&source, DatasetPurpose::Training).unwrap();
    let workspace = import_dataset(
        &folder,
        &source,
        "Selected data",
        DatasetPurpose::Training,
        &preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    let dataset = dataset_versions::create_base(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Selected data",
        &[workspace.datasets[0].id],
    )
    .await
    .unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let generator = (!matches!(loop_mode, Some("no_change_first" | "agent_limit" | "generation_limit" | "dirty_preflight"))).then(|| std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Generator was never invoked"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        let request: Value = loop {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let count: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + count {
                    break serde_json::from_slice(&bytes[end + 4..end + 4 + count]).unwrap();
                }
            }
        };
        assert_eq!(request["model"], "pinned-generator");
        assert!(!request.to_string().contains("NEVER_DISCLOSE_HOLDOUT"));
        let body = json!({"choices":[{"message":{"content":"{\"rows\":[{\"question\":\"Search the exact technical reference\"}]}"}}],"usage":{"prompt_tokens":120,"completion_tokens":40,"total_tokens":160}}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        request
    }));
    let provider = |role, model: &str| ProviderConfiguration {
        role,
        kind: ProviderKind::OpenaiCompatible,
        endpoint: Some(endpoint.clone()),
        model: model.into(),
        authentication: ProviderAuthentication::None,
        secret: None,
        limits: ProviderLimits {
            maximum_requests: 16,
            maximum_input_tokens: 1_000_000,
            maximum_output_tokens: 100_000,
            maximum_cost_microusd: 0,
        },
    };
    let providers = ProviderCatalog::create(
        Uuid::new_v4(),
        workspace.manifest.id,
        1,
        None,
        vec![
            provider(ProviderRole::Advisor, "pinned-agent"),
            provider(ProviderRole::Generation, "pinned-generator"),
        ],
        "fixture",
        "offline",
        Utc::now(),
    )
    .unwrap();
    record_provider_catalog(&folder, providers.clone(), None)
        .await
        .unwrap();
    let selection = optimization_setup::preview(
        &folder,
        workspace.model_catalog.as_ref().unwrap().active_model().id,
        dataset.id,
        benchmark.id,
    )
    .await
    .unwrap();
    let setup = optimization_setup::save(
        &folder,
        optimization_setup::SetupRequest {
            id: Uuid::new_v4(),
            expected_parent: None,
            inputs: selection.inputs,
        },
    )
    .await
    .unwrap();
    let mut settings = OptimizationAgentSettings::quick_test();
    if let Some(mode) = loop_mode {
        settings = if matches!(mode, "v3_prepare" | "v3_semantic_reject") {
            let mut value = OptimizationAgentSettings::quick_test();
            value.analysis_protocol = 3;
            value
        } else {
            OptimizationAgentSettings::default()
        };
        settings.maximum_iterations = if matches!(mode, "v3_prepare" | "v3_semantic_reject") {
            1
        } else if mode == "two_iterations" {
            2
        } else if matches!(
            mode,
            "stop_resume"
                | "training_stop"
                | "training_settlement"
                | "training_timeout"
                | "training_crash"
                | "crash_boundaries"
        ) {
            1
        } else {
            3
        };
        settings.maximum_row_changes = if mode == "row_limit" { 2 } else { 8 };
        if mode == "canary_rejected" {
            settings.maximum_row_changes = 20;
        }
        settings.training.maximum_seconds_per_iteration = 120;
        if mode == "training_timeout" {
            settings.training.maximum_seconds_per_iteration = 1;
        }
    }
    settings.training.device = project_workspace_core::OptimizationDevice::Cpu;
    settings.training.maximum_training_rows = Some(1);
    let mut run_limits = ProviderLimits {
        maximum_requests: 12,
        maximum_input_tokens: 900_000,
        maximum_output_tokens: 90_000,
        maximum_cost_microusd: 0,
    };
    let mut generation_limits = run_limits.clone();
    if loop_mode == Some("agent_limit") {
        run_limits.maximum_requests = 1;
    }
    if loop_mode == Some("generation_limit") {
        generation_limits.maximum_input_tokens = 1;
    }
    settings.provider_limits = Some(project_workspace_core::OptimizationProviderLimits {
        advisor: run_limits,
        generation: generation_limits,
    });
    if loop_mode == Some("eligible") {
        settings.training.maximum_training_rows = None;
    }
    let scope = optimization_launch::preview_agentic(&folder, setup.id, settings)
        .await
        .unwrap()
        .scope;
    // Reserve through the normal production command. The coordinator performs
    // real preparation; no SQL-seeded run or fabricated input receipt.
    let request = optimization_launch::LaunchRequest {
        id: Uuid::new_v4(),
        scope,
    };
    fs::write(
        root.join("launch.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let reserved_result = run(
        root,
        &[
            "optimization-run",
            "project",
            "start",
            "--file",
            "launch.json",
        ],
    );
    let reserved: ProjectOptimizationRun =
        serde_json::from_value(reserved_result["run"]["run"].clone()).unwrap();
    assert!(
        optimization_runs::show(&folder, reserved.id)
            .await
            .unwrap()
            .preparation
            .is_none()
    );
    // A changed default must not redirect this already-authorized run.
    let replacement = ProviderCatalog::create(
        Uuid::new_v4(),
        workspace.manifest.id,
        2,
        Some(providers.id),
        vec![
            provider(ProviderRole::Advisor, "wrong-agent"),
            provider(ProviderRole::Generation, "wrong-generator"),
        ],
        "fixture",
        "change defaults",
        Utc::now(),
    )
    .unwrap();
    record_provider_catalog(&folder, replacement, Some(providers.id))
        .await
        .unwrap();
    let calls = root.join("agent-calls.jsonl");
    let sidecar =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/optimization_agent_sidecar.mjs");
    let invocation_folder = folder.canonicalize().unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synth"));
        command
            .current_dir(root)
            .env("AGENT_FIXTURE_CALLS", &calls)
            .env("AGENT_FIXTURE_LOOP", loop_mode.unwrap_or(""))
            .args([
                "--output",
                "json",
                "workspace",
                "optimization-run",
                invocation_folder.to_str().unwrap(),
                if loop_mode.is_some()
                    && !matches!(loop_mode, Some("v3_prepare" | "v3_semantic_reject"))
                {
                    "drive-agent"
                } else if complete {
                    "complete-iteration"
                } else {
                    "prepare-candidate"
                },
                &reserved.id.to_string(),
                "--pi-sidecar",
            ])
            .arg(&sidecar);
        command
    };
    let invoke_iteration = || command().output().unwrap();
    let execute = || {
        let output = invoke_iteration();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if complete && loop_mode.is_none() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                telemetry(&output).is_empty(),
                "Completed replay must not fabricate a new training observation"
            );
            let bookkeeping: Vec<Value> = stderr
                .lines()
                .filter_map(|line| line.strip_prefix("ENCODER_GYM_PROGRESS "))
                .filter_map(|json| serde_json::from_str::<Value>(json).ok())
                .filter(|event| event["phase"] == "finalizing_iteration")
                .collect();
            assert_eq!(
                bookkeeping.len(),
                2,
                "candidate/result custody must report progress"
            );
            assert!(bookkeeping.iter().all(|event| event["iteration"] == 1
                && event["runStage"] == "evaluating"
                && event["narrative"]["origin"] == "system"));
        }
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let before_native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    if loop_mode == Some("canary_rejected") {
        for _ in 0..2 {
            let rejected = invoke_iteration();
            assert!(!rejected.status.success());
            assert!(
                String::from_utf8_lossy(&rejected.stderr)
                    .contains("Generation canary rejected 8 of 8"),
                "{}",
                String::from_utf8_lossy(&rejected.stderr)
            );
            let history = run(
                root,
                &[
                    "optimization-run",
                    "project",
                    "history",
                    &reserved.id.to_string(),
                ],
            );
            let plan = &history["iterations"][0]["repairPlan"];
            assert_eq!(plan["canary"]["status"], "rejected");
            assert_eq!(plan["canary"]["rejected"].as_array().unwrap().len(), 8);
            assert_eq!(plan["generation"][0]["attempts"], 1);
            assert_eq!(plan["generation"][0]["unresolved"], 9);
            assert!(plan["publication"].is_null());
            assert!(history["iterations"][0]["experimentRunId"].is_null());
        }
        generator.unwrap().join().unwrap();
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 3);
        let publication =
            project_workspace_local::optimization_dataset::publish(&folder, reserved.id, 1)
                .await
                .unwrap_err();
        assert!(publication.to_string().contains("passed generation canary"));
        let native = fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap();
        assert!(!native[before_native.len()..].contains("tools.train_dense"));
        return;
    }
    if loop_mode == Some("dirty_preflight") {
        let rejected = invoke_iteration();
        assert!(!rejected.status.success());
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            error.contains("at least 9 row changes")
                && error.contains("only 8 are authorized")
                && error.contains("no provider was called"),
            "{error}"
        );
        assert!(!calls.exists(), "Agent provider must not be dispatched");
        let after_native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&after_native[before_native.len()..]),
            "encoder_gym.qualify_training\n"
        );
        assert!(generator.is_none());
        return;
    }
    if loop_mode == Some("crash_boundaries") {
        crash_check::exercise(root, &folder, reserved.id, &calls, command).await;
        generator.unwrap().join().unwrap();
        return;
    }
    if matches!(loop_mode, Some("agent_limit" | "generation_limit")) {
        let exhausted = invoke_iteration();
        assert!(!exhausted.status.success());
        let expected = if loop_mode == Some("agent_limit") {
            "Agent request budget exhausted"
        } else {
            "Generation token or spend budget exhausted"
        };
        assert!(
            String::from_utf8_lossy(&exhausted.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&exhausted.stderr)
        );
        let stopped = optimization_runs::show(&folder, reserved.id).await.unwrap();
        assert_eq!(
            stopped.state,
            project_workspace_core::ProjectOptimizationRunState::AgentBudgetExhausted
        );
        assert_eq!(stopped.agent_execution.as_ref().unwrap().attempts, 1);
        let calls_before = fs::read(&calls).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&calls_before).lines().count(),
            if loop_mode == Some("agent_limit") {
                1
            } else {
                3
            }
        );
        let native_before = fs::read(root.join("runtime/native-invocations.log")).unwrap();
        assert!(
            project_workspace_local::optimization_training_time::history(&folder, reserved.id)
                .await
                .unwrap()
                .is_empty()
        );
        let retry = invoke_iteration();
        assert!(!retry.status.success());
        assert!(String::from_utf8_lossy(&retry.stderr).contains("Optimization budget exhausted"));
        assert_eq!(fs::read(&calls).unwrap(), calls_before);
        assert_eq!(
            fs::read(root.join("runtime/native-invocations.log")).unwrap(),
            native_before
        );
        assert_eq!(
            optimization_runs::show(&folder, reserved.id).await.unwrap(),
            stopped
        );
        assert!(generator.is_none());
        return;
    }
    #[cfg(windows)]
    if loop_mode == Some("training_crash") {
        training_time_check::crash(root, &folder, reserved.id, &calls, command).await;
        generator.unwrap().join().unwrap();
        return;
    }
    if loop_mode == Some("training_timeout") {
        training_time_check::timeout(root, &folder, reserved.id, &calls, command).await;
        generator.unwrap().join().unwrap();
        return;
    }
    if matches!(loop_mode, Some("training_stop" | "training_settlement")) {
        training_time_check::exercise(
            root,
            &folder,
            reserved.id,
            &calls,
            command,
            loop_mode == Some("training_stop"),
        )
        .await;
        generator.unwrap().join().unwrap();
        return;
    }
    if loop_mode == Some("stop_resume") {
        stop_check::exercise(root, &folder, reserved.id, &calls, command).await;
        generator.unwrap().join().unwrap();
        return;
    }
    if complete && loop_mode.is_none() {
        let project_database = folder.join("project.sqlite");
        let scientific_database = folder.join("runs/scientific.sqlite");

        install_trigger(
            &project_database,
            "CREATE TRIGGER interrupt_dataset_publication BEFORE INSERT ON optimization_dataset_publications BEGIN SELECT RAISE(ABORT, 'injected dataset publication interruption'); END",
        )
        .await;
        assert_injected(
            invoke_iteration(),
            "injected dataset publication interruption",
        );
        let pending_plan = run(
            root,
            &[
                "optimization-run",
                "project",
                "history",
                &reserved.id.to_string(),
            ],
        );
        let pending_plan = &pending_plan["iterations"][0]["repairPlan"];
        assert_eq!(pending_plan["generation"][0]["admitted"], 1);
        assert!(
            pending_plan["publication"].is_null(),
            "Admission alone must not claim published changes"
        );
        drop_trigger(&project_database, "interrupt_dataset_publication").await;

        install_trigger(
            &project_database,
            "CREATE TRIGGER interrupt_dataset_qualification BEFORE INSERT ON project_activity_events WHEN NEW.operation='optimization.qualification' AND NEW.state='succeeded' BEGIN SELECT RAISE(ABORT, 'injected dataset qualification interruption'); END",
        )
        .await;
        assert_injected(
            invoke_iteration(),
            "injected dataset qualification interruption",
        );
        drop_trigger(&project_database, "interrupt_dataset_qualification").await;

        install_trigger(
            &scientific_database,
            "CREATE TRIGGER interrupt_training_receipt BEFORE INSERT ON encoder_experiment_events WHEN json_extract(NEW.artifact_json,'$.event.kind')='candidate_training_completed' BEGIN SELECT RAISE(ABORT, 'injected training receipt interruption'); END",
        )
        .await;
        let trained = invoke_iteration();
        let observed = telemetry(&trained);
        assert!(observed.iter().any(|event| event["phase"] == "training"
            && event["completed"] == 2
            && event["training"]["elapsedSeconds"] == 2));
        assert!(
            observed
                .iter()
                .any(|event| event["phase"] == "saving_checkpoint"
                    && event["training"]["finalLoss"] == 0.25)
        );
        assert!(observed.iter().all(|event| event["iteration"] == 1));
        assert_injected(trained, "injected training receipt interruption");
        drop_trigger(&scientific_database, "interrupt_training_receipt").await;

        install_trigger(
            &scientific_database,
            "CREATE TRIGGER interrupt_first_development_report BEFORE INSERT ON encoder_experiment_events WHEN json_extract(NEW.artifact_json,'$.event.kind')='candidate_development_suite_completed' AND (SELECT COUNT(*) FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed')=0 BEGIN SELECT RAISE(ABORT, 'injected first development report interruption'); END",
        )
        .await;
        install_trigger(
            &project_database,
            "CREATE TRIGGER interrupt_candidate_registration BEFORE INSERT ON model_artifacts WHEN NEW.origin='trained' BEGIN SELECT RAISE(ABORT, 'injected candidate registration interruption'); END",
        )
        .await;
        let recovered = invoke_iteration();
        let observed = telemetry(&recovered);
        assert!(
            observed
                .iter()
                .any(|event| event["phase"] == "saving_checkpoint"
                    && event["training"]["finalLoss"] == 0.25)
        );
        assert!(
            observed.iter().all(|event| event["phase"] != "training"),
            "Checkpoint reuse must not replay optimizer steps"
        );
        assert_injected(recovered, "injected candidate registration interruption");
        drop_trigger(&project_database, "interrupt_candidate_registration").await;
        assert_injected(
            invoke_iteration(),
            "injected first development report interruption",
        );
        // A trained model and its exact data link remain visible even while
        // evaluation cannot persist its first result. No successful retry is
        // required to make the completed checkpoint an ordinary project model.
        let interrupted = project_workspace_local::open_workspace(&folder, false)
            .await
            .unwrap();
        let trained = interrupted
            .model_catalog
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .filter(|model| model.producing_run.is_some())
            .collect::<Vec<_>>();
        assert_eq!(trained.len(), 1);
        assert!(
            interrupted
                .model_dataset_links
                .iter()
                .any(|link| link.model_id == trained[0].id)
        );
        drop_trigger(&scientific_database, "interrupt_first_development_report").await;

        install_trigger(
            &scientific_database,
            "CREATE TRIGGER interrupt_second_development_report BEFORE INSERT ON encoder_experiment_events WHEN json_extract(NEW.artifact_json,'$.event.kind')='candidate_development_suite_completed' AND (SELECT COUNT(*) FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed')=1 BEGIN SELECT RAISE(ABORT, 'injected second development report interruption'); END",
        )
        .await;
        assert_injected(
            invoke_iteration(),
            "injected second development report interruption",
        );
        drop_trigger(&scientific_database, "interrupt_second_development_report").await;

        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER interrupt_iteration_result BEFORE INSERT ON optimization_iteration_results BEGIN SELECT RAISE(ABORT, 'injected iteration result interruption'); END")
            .execute(&mut db).await.unwrap();
        db.close().await.unwrap();
        let interrupted = invoke_iteration();
        assert!(!interrupted.status.success());
        assert!(
            String::from_utf8_lossy(&interrupted.stderr)
                .contains("injected iteration result interruption"),
            "{}",
            String::from_utf8_lossy(&interrupted.stderr)
        );
        // Ordinary report views survive interruption before the iteration result
        // is saved, without running a new evaluation.
        let projected = run(
            root,
            &["benchmark", "project", "results", &benchmark.id.to_string()],
        );
        let projected_candidate = projected["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["isBaseline"] == false)
            .unwrap();
        assert_eq!(projected_candidate["reports"].as_array().unwrap().len(), 2);
        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        sqlx::query("DROP TRIGGER interrupt_iteration_result")
            .execute(&mut db)
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    if loop_mode.is_some() && !matches!(loop_mode, Some("v3_prepare" | "v3_semantic_reject")) {
        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER interrupt_loop_completion BEFORE INSERT ON optimization_iteration_completions BEGIN SELECT RAISE(ABORT, 'injected loop completion interruption'); END")
            .execute(&mut db).await.unwrap();
        if loop_mode == Some("two_iterations") {
            // The real CLI exits without closing its execution attempt. Its
            // successor must acquire the lease before recording interruption.
            sqlx::query("CREATE TRIGGER interrupt_execution_failure BEFORE INSERT ON optimization_agent_execution_events WHEN NEW.kind='failed' BEGIN SELECT RAISE(ABORT, 'injected execution failure interruption'); END")
                .execute(&mut db).await.unwrap();
        }
        let interrupted = invoke_iteration();
        assert!(!interrupted.status.success());
        assert!(
            String::from_utf8_lossy(&interrupted.stderr)
                .contains("injected loop completion interruption"),
            "{}",
            String::from_utf8_lossy(&interrupted.stderr)
        );
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 3);
        let failed = optimization_runs::show(&folder, reserved.id).await.unwrap();
        let expected = if loop_mode == Some("two_iterations") {
            project_workspace_core::ProjectOptimizationRunState::AgentRunning
        } else {
            project_workspace_core::ProjectOptimizationRunState::AgentFailed
        };
        assert_eq!(failed.state, expected);
        assert_eq!(failed.agent_execution.as_ref().unwrap().attempts, 1);
        if loop_mode == Some("two_iterations") {
            sqlx::query("DROP TRIGGER interrupt_execution_failure")
                .execute(&mut db)
                .await
                .unwrap();
        }
        sqlx::query("DROP TRIGGER interrupt_loop_completion")
            .execute(&mut db)
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    if loop_mode == Some("no_change_first") {
        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER interrupt_execution_completion BEFORE INSERT ON optimization_agent_execution_events WHEN NEW.kind='completed' BEGIN SELECT RAISE(ABORT, 'injected root completion interruption'); END")
            .execute(&mut db).await.unwrap();
        let interrupted = invoke_iteration();
        assert!(!interrupted.status.success());
        assert!(
            String::from_utf8_lossy(&interrupted.stderr)
                .contains("injected root completion interruption")
        );
        let pending = optimization_runs::show(&folder, reserved.id).await.unwrap();
        assert_eq!(
            pending.state,
            project_workspace_core::ProjectOptimizationRunState::AgentRunning
        );
        assert_eq!(pending.agent_execution.as_ref().unwrap().attempts, 2);
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 3);
        let completions =
            project_workspace_local::optimization_completions::list(&folder, reserved.id)
                .await
                .unwrap();
        assert_eq!(completions.len(), 1);
        assert!(completions[0].end.is_some());
        sqlx::query("DROP TRIGGER interrupt_execution_completion")
            .execute(&mut db)
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    if loop_mode == Some("v3_semantic_reject") {
        let rejected = invoke_iteration();
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("Every generated row was rejected"),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        generator.unwrap().join().unwrap();
        let reviews =
            project_workspace_local::optimization_native_review::ProjectNativeReviewStore::open(
                &folder,
                reserved.id,
            )
            .await
            .unwrap();
        let admissions =
            dataset_quality_core::ports::NativeReviewStore::admissions(&reviews, reserved.id, 1)
                .await
                .unwrap();
        assert_eq!(admissions.len(), 1);
        assert_eq!(
            admissions[0].admission.decision,
            dataset_quality_core::native_assessment::NativeAdmissionDecision::LabelMismatch
        );
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 6);
        assert!(
            project_workspace_local::optimization_dataset::publication(&folder, reserved.id, 1)
                .await
                .is_err()
        );
        return;
    }
    let first = execute();
    if let Some(generator) = generator {
        generator.join().unwrap();
    }
    if loop_mode == Some("v3_prepare") {
        let publication: project_workspace_local::optimization_dataset::OptimizationDatasetPublication =
            serde_json::from_value(first["datasetStep"]["publication"].clone()).unwrap();
        assert_eq!(publication.generated.len(), 1);
        assert_eq!(publication.semantic_rejections, 0);
        assert_eq!(publication.cross_batch_duplicates, 0);
        assert!(publication.generated[0].source_record.is_some());
        let reviews =
            project_workspace_local::optimization_native_review::ProjectNativeReviewStore::open(
                &folder,
                reserved.id,
            )
            .await
            .unwrap();
        let admissions =
            dataset_quality_core::ports::NativeReviewStore::admissions(&reviews, reserved.id, 1)
                .await
                .unwrap();
        assert_eq!(admissions.len(), 1);
        assert!(admissions[0].admission.admitted());
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 6);
        assert!(first["datasetStep"]["qualification"].is_object());
        assert!(first["datasetStep"]["candidate"].is_null());
        return;
    }
    if let Some(mode) = loop_mode {
        loop_check::assert_loop(root, &folder, reserved.id, mode, &first, execute).await;
        assert_eq!(
            dataset_versions::inspect(&folder, dataset.id)
                .await
                .unwrap(),
            dataset
        );
        return;
    }
    assert_eq!(
        first["datasetStep"]["qualification"]["clearance"]["trainingRows"],
        2
    );
    assert_eq!(
        first["datasetStep"]["qualification"]["clearance"]["overlapRows"],
        0
    );
    assert_ne!(
        first["datasetStep"]["qualification"]["nativeDataset"]["runId"],
        reserved.id.to_string()
    );
    let after_native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let added = String::from_utf8_lossy(&after_native[before_native.len()..]);
    if complete {
        assert_eq!(
            added.lines().collect::<Vec<_>>(),
            vec![
                "encoder_gym.qualify_training",
                "encoder_gym.qualify_training",
                "tools.train_dense_triplet_router",
                "tools.evaluate_dense_router",
                "tools.evaluate_real_agent_sessions",
                "tools.evaluate_dense_router",
                "tools.evaluate_real_agent_sessions"
            ]
        );
        let result: project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult =
            serde_json::from_value(first["datasetStep"]["development"].clone()).unwrap();
        assert_eq!(result.reports.len(), 2);
        assert_eq!(result.assessments.len(), 2);
        assert!(
            !result.development_passed,
            "The fixture candidate must be retained even when rejected"
        );
        assert!(!first["datasetStep"].to_string().contains("99999.125"));
        let binding = project_workspace_local::optimization_iteration_execution::training(
            &folder,
            reserved.id,
            first["datasetStep"]["iterationId"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(binding.training_rows, 1);
        assert_ne!(binding.training_dataset, binding.qualified_dataset);
        assert_eq!(binding.resolved_device, "cpu");
        let registered: project_workspace_core::ModelArtifact =
            serde_json::from_value(first["datasetStep"]["candidate"]["model"].clone()).unwrap();
        let link: project_workspace_core::ModelDatasetLink =
            serde_json::from_value(first["datasetStep"]["candidate"]["dataset"].clone()).unwrap();
        let inventory = project_workspace_local::open_workspace(&folder, true)
            .await
            .unwrap();
        assert_eq!(
            inventory.model_catalog.as_ref().unwrap().active_model(),
            workspace.model_catalog.as_ref().unwrap().active_model()
        );
        assert_eq!(inventory.model_catalog.as_ref().unwrap().artifacts.len(), 2);
        assert!(
            inventory
                .model_catalog
                .as_ref()
                .unwrap()
                .artifacts
                .contains(&registered)
        );
        // The imported baseline has no invented training history.
        assert_eq!(inventory.model_dataset_links, vec![link.clone()]);
        assert_eq!(link.model_id, registered.id);
        assert_eq!(link.version, binding.training_dataset);
        assert_eq!(
            registered.training_snapshot.as_ref().unwrap().id,
            link.version.id.to_string()
        );
        let exact = dataset_versions::inspect(&folder, link.version.id)
            .await
            .unwrap();
        let qualified = dataset_versions::inspect(&folder, binding.qualified_dataset.id)
            .await
            .unwrap();
        assert_eq!(exact.members.len(), 1);
        assert!(qualified.members.contains(&exact.members[0]));
        // A one-row rendering may reuse an identical one-row generated import;
        // either way the trainer version keeps its qualified source identity.
        assert_eq!(link.inputs.iter().map(|input| input.rows).sum::<u64>(), 1);
        let viewed = dataset_versions::read_rows(&folder, exact.id, 0, 10)
            .await
            .unwrap();
        assert_eq!(viewed.rows.len(), 1);
        assert_eq!(result.training_binding_fingerprint, binding.fingerprint);
        assert_eq!(result.output.metadata["native_manifest"]["batch_size"], 8);
        assert_eq!(
            result.output.metadata["native_manifest"]["unique_trainable_rows"],
            1
        );
        let store = SqliteExperimentStore::connect_read_only(&format!(
            "sqlite://{}",
            folder.join("runs/scientific.sqlite").display()
        ))
        .await
        .unwrap();
        let protocol = store
            .get_protocol(binding.protocol.id.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(protocol.budget.maximum_sealed_evaluations, 0);
        assert_eq!(protocol.candidates[0].maximum_training_seconds, 120);
        assert_ne!(
            binding.candidate.id,
            reserved.child_id("candidate", 1).unwrap().to_string()
        );
        store.pool().close().await;
        assert_iteration_projection(&folder, reserved.id, &binding, &benchmark).await;
    } else {
        assert_eq!(
            added,
            "encoder_gym.qualify_training\nencoder_gym.qualify_training\n"
        );
    }
    let published: project_workspace_local::optimization_dataset::OptimizationDatasetPublication =
        serde_json::from_value(first["datasetStep"]["publication"].clone()).unwrap();
    assert_eq!(published.parent, dataset.reference());
    assert_eq!(published.removed.len(), 1);
    assert_eq!(published.generated.len(), 1);
    let database_before_plan = fs::read(folder.join("project.sqlite")).unwrap();
    let repair_history = run(
        root,
        &[
            "optimization-run",
            "project",
            "history",
            &reserved.id.to_string(),
        ],
    );
    let repair_plan = &repair_history["iterations"][0]["repairPlan"];
    assert_eq!(
        repair_plan["proposalFingerprint"],
        published.proposal_fingerprint
    );
    assert_eq!(repair_plan["inputRows"], 2);
    assert_eq!(
        repair_plan["proposal"]["removals"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(repair_plan["generation"][0]["requested"], 1);
    assert_eq!(repair_plan["generation"][0]["admitted"], 1);
    assert_eq!(repair_plan["generation"][0]["unresolved"], 0);
    assert_eq!(repair_plan["canary"]["status"], "passed");
    assert_eq!(
        repair_plan["canary"]["rows"][0]["question"],
        "Search the exact technical reference"
    );
    assert_eq!(
        repair_plan["canary"]["rows"][0].as_object().unwrap().len(),
        4
    );
    assert_eq!(repair_plan["publication"]["added"], 1);
    assert_eq!(repair_plan["publication"]["removed"], 1);
    assert_eq!(
        repair_plan["publication"]["datasetVersionId"],
        published.version.id.to_string()
    );
    assert_eq!(
        repair_plan["strategy"],
        "question_variants_preserve_context"
    );
    assert!(!repair_plan["evidence"].as_array().unwrap().is_empty());
    assert!(
        !repair_history
            .to_string()
            .contains("NEVER_DISCLOSE_HOLDOUT")
    );
    assert!(!repair_history.to_string().contains("tool_registry"));
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "history",
                &reserved.id.to_string()
            ]
        ),
        repair_history
    );
    assert_eq!(
        fs::read(folder.join("project.sqlite")).unwrap(),
        database_before_plan
    );
    let rows = dataset_versions::materialization_rows(&folder, published.version.id)
        .await
        .unwrap();
    if complete {
        let report = protocol.baseline_development_reports()[0];
        let suite = &project.task_configuration["suites"][&report.suite_key];
        let baseline_path = root
            .join("runtime/runs/encoder-gym-evaluations/by-content")
            .join(&report.model.fingerprint[7..])
            .join("retrieval")
            .join(&suite["retrieval_fingerprint"].as_str().unwrap()[7..])
            .join(format!("{}.json", report.suite_key));
        cases_check::verify(root, &folder, reserved.id, &repair_history, &baseline_path);
    }
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|r| r.value["question"] == "Search the exact technical reference")
    );
    assert!(rows.iter().any(|r| r.value["decision_state_id"] == "keep"));
    assert!(!rows.iter().any(|r| r.value["decision_state_id"] == "old"));
    let calls_before = fs::read(&calls).unwrap();
    let versions_before = dataset_versions::list(&folder).await.unwrap();
    let inventory_before = project_workspace_local::open_workspace(&folder, true)
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&calls_before).lines().count(), 3);
    // The loopback server is gone: completed generation must replay, not dispatch.
    let second = execute();
    assert_eq!(first["datasetStep"], second["datasetStep"]);
    assert_eq!(
        serde_json::to_value(dataset_versions::list(&folder).await.unwrap()).unwrap(),
        serde_json::to_value(versions_before).unwrap()
    );
    let inventory_after = project_workspace_local::open_workspace(&folder, true)
        .await
        .unwrap();
    assert_eq!(
        inventory_after.model_catalog,
        inventory_before.model_catalog
    );
    assert_eq!(
        inventory_after.model_dataset_links,
        inventory_before.model_dataset_links
    );
    assert_eq!(inventory_after.datasets, inventory_before.datasets);
    assert_eq!(fs::read(&calls).unwrap(), calls_before);
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        after_native
    );
    let activity = run(root, &["activity", "project", "list"]);
    let log = activity.to_string();
    assert!(log.contains("Shift search coverage by one row"));
    assert!(log.contains("\"origin\":\"agent\""));
    assert!(log.contains("\"origin\":\"generation\""));
    assert!(log.contains("Checking the complete candidate dataset"));
    assert!(!log.contains("NEVER_DISCLOSE_HOLDOUT"));
    assert_eq!(
        dataset_versions::inspect(&folder, dataset.id)
            .await
            .unwrap(),
        dataset
    );
    // An altered cached clearance cannot be accepted, even when it looks clean.
    let native = &first["datasetStep"]["qualification"]["nativeDataset"];
    let clearance = &first["datasetStep"]["qualification"]["clearance"];
    let receipt = root
        .join("runtime/runs/encoder-gym-project-runs")
        .join(native["runId"].as_str().unwrap())
        .join("qualification")
        .join(&clearance["requestFingerprint"].as_str().unwrap()[7..])
        .join("clearance.json");
    let mut changed = clearance.clone();
    changed["missingGroupRows"] = 0.into();
    fs::write(&receipt, serde_json::to_vec(&changed).unwrap()).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .env("AGENT_FIXTURE_CALLS", &calls)
        .args([
            "--output",
            "json",
            "workspace",
            "optimization-run",
            "project",
            if complete {
                "complete-iteration"
            } else {
                "prepare-candidate"
            },
            &reserved.id.to_string(),
            "--pi-sidecar",
        ])
        .arg(&sidecar)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("clearance does not match"));
    assert_eq!(fs::read(&calls).unwrap(), calls_before);
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        after_native
    );
}

async fn assert_iteration_projection(
    folder: &Path,
    run_id: Uuid,
    training: &project_workspace_core::optimization_iteration_execution::IterationTrainingBinding,
    benchmark: &project_workspace_core::ProjectBenchmarkVersion,
) {
    use project_workspace_core::benchmark_results::{
        BenchmarkRunEvidence, IterationResultLineage, ProjectBenchmarkResults,
    };
    let inventory = project_workspace_local::open_workspace(folder, true)
        .await
        .unwrap();
    let catalog = inventory.model_catalog.as_ref().unwrap();
    let view = optimization_runs::show(folder, run_id).await.unwrap();
    let preparation = view.preparation.as_ref().unwrap();
    let launches = optimization_launch::list(folder).await.unwrap();
    let launch = launches
        .iter()
        .find(|item| item.id.to_string() == view.run.launch.id)
        .unwrap();
    let setups = optimization_setup::list(folder).await.unwrap();
    let setup = setups
        .iter()
        .find(|item| item.id.to_string() == view.run.setup.id)
        .unwrap();
    let iterations = project_workspace_local::optimization_iterations::list(folder, run_id)
        .await
        .unwrap();
    let iteration = &iterations[0];
    let bindings = project_workspace_local::scientific_binding_history(folder)
        .await
        .unwrap();
    let binding = bindings
        .iter()
        .find(|item| item.id.to_string() == preparation.execution_binding.id)
        .unwrap();
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let runtime_project = store
        .get_project(binding.runtime.project_snapshot.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let project = store
        .get_project(training.scientific_project.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let protocol = store
        .get_protocol(training.protocol.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let events = store.load_events(training.experiment_run_id).await.unwrap();
    let definition = NomosBackend::recorded_benchmark(&project, &protocol).unwrap();
    let project_result = |results: &mut ProjectBenchmarkResults, candidate_training: &project_workspace_core::optimization_iteration_execution::IterationTrainingBinding| {
        results.include_agent_iteration(catalog, BenchmarkRunEvidence {
            binding, project:&project, protocol:&protocol, definition:&definition, events:&events,
        }, IterationResultLineage {
            run:&view.run, launch, setup, preparation, iteration, training:candidate_training, runtime_project:&runtime_project, predecessors:&[],
        })
    };
    let mut results = ProjectBenchmarkResults::new(benchmark.clone(), catalog).unwrap();
    project_result(&mut results, training).unwrap();
    let model = results
        .models
        .iter()
        .find(|model| !model.is_baseline)
        .unwrap();
    assert_eq!(model.reports.len(), 2);
    assert!(model.reports.iter().all(|report| {
        report.contexts.iter().all(|context| {
            context.run_id == training.experiment_run_id
                && context.candidate_id.unwrap().to_string() == training.candidate.id
        })
    }));
    assert!(
        !serde_json::to_string(&results)
            .unwrap()
            .contains("99999.125")
    );
    let unchanged = results.clone();
    project_result(&mut results, training).unwrap();
    assert_eq!(results, unchanged);
    // Even consistently re-fingerprinted training receipts cannot associate
    // another project, protocol, candidate, population or experiment.
    for field in 0..5 {
        let mut changed = training.clone();
        match field {
            0 => changed.scientific_project.id = Uuid::new_v4().to_string(),
            1 => changed.protocol.id = Uuid::new_v4().to_string(),
            2 => changed.candidate.id = Uuid::new_v4().to_string(),
            3 => changed.experiment_run_id = Uuid::new_v4(),
            _ => changed.training_artifact.fingerprint = format!("sha256:{}", "b".repeat(64)),
        }
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(project_result(&mut results, &changed).is_err());
        assert_eq!(results, unchanged);
    }
    store.pool().close().await;
}
