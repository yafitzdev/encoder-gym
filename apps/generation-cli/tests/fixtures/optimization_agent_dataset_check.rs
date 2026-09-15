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
#[path = "optimization_agent_loop_check.rs"]
mod loop_check;

fn native_row(id: &str, question: &str) -> Value {
    let tool = |id| json!({"tool_id":id,"tool_family":"search","description":"Find evidence","capabilities":["search"],"input_modalities":["text"],"output_modalities":["text"],"evidence_roles":["primary"],"side_effect_class":"none","argument_schema":{}});
    json!({"schema_version":"decision-state.v2","decision_state_id":id,"question":question,"evaluation_partition":"train","accepted":true,"task_kind":"route","previous_candidate_ids":[],"legal_candidate_ids":["a","b"],"label":{"acceptable_tools":["a"],"hard_negative_tools":["b"]},"tool_registry":{"registry_id":"r","registry_fingerprint":"sha256:registry","tools":[tool("a"),tool("b")]}})
}

#[tokio::test]
async fn cli_agent_inspects_generates_qualifies_and_replays_exact_dataset_without_training() {
    scenario(false, None).await;
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
async fn cli_agent_loop_keeps_best_eligible_dataset_but_inspects_latest_result() {
    scenario(true, Some("eligible")).await;
}

#[tokio::test]
async fn cli_agent_loop_can_finish_first_analysis_without_edits_or_training() {
    scenario(true, Some("no_change_first")).await;
}

async fn scenario(complete: bool, loop_mode: Option<&str>) {
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
        saved["inputs"][suite["path"].as_str().unwrap()]["disagreements"] = json!((0..count).map(|index| json!({
            "decision_state_id":format!("dev-failure-{index}"), "question":"Search for an exact reference", "task_kind":"route", "expected_rank":2,
            "expected_capabilities":["search"], "predicted_capabilities":["write"]})).collect::<Vec<_>>());
        fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    }
    if loop_mode == Some("eligible") {
        // Only the test-native executable interprets this fixture setting.
        fs::write(
            root.join("runtime/runs/fixture-eligible-candidates"),
            "two candidates",
        )
        .unwrap();
    }
    let source = root.join("selected.jsonl");
    fs::write(
        &source,
        format!(
            "{}\n{}\n",
            native_row("old", "Search something"),
            native_row("keep", "Retain this useful example")
        ),
    )
    .unwrap();
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
    let generator = (loop_mode != Some("no_change_first")).then(|| std::thread::spawn(move || {
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
            maximum_requests: 8,
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
        settings = OptimizationAgentSettings::default();
        settings.maximum_iterations = if mode == "two_iterations" { 2 } else { 3 };
        settings.maximum_row_changes = if mode == "row_limit" { 2 } else { 8 };
        settings.training.maximum_seconds_per_iteration = 120;
    }
    settings.training.device = project_workspace_core::OptimizationDevice::Cpu;
    settings.training.maximum_training_rows = Some(1);
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
    let invoke_iteration = || {
        Command::new(env!("CARGO_BIN_EXE_synth"))
            .current_dir(root)
            .env("AGENT_FIXTURE_CALLS", &calls)
            .env("AGENT_FIXTURE_LOOP", loop_mode.unwrap_or(""))
            .args([
                "--output",
                "json",
                "workspace",
                "optimization-run",
                "project",
                if loop_mode.is_some() {
                    "drive-agent"
                } else if complete {
                    "complete-iteration"
                } else {
                    "prepare-candidate"
                },
                &reserved.id.to_string(),
                "--pi-sidecar",
            ])
            .arg(&sidecar)
            .output()
            .unwrap()
    };
    let execute = || {
        let output = invoke_iteration();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let before_native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    if complete && loop_mode.is_none() {
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
    if loop_mode.is_some() {
        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER interrupt_loop_completion BEFORE INSERT ON optimization_iteration_completions BEGIN SELECT RAISE(ABORT, 'injected loop completion interruption'); END")
            .execute(&mut db).await.unwrap();
        let interrupted = invoke_iteration();
        assert!(!interrupted.status.success());
        assert!(
            String::from_utf8_lossy(&interrupted.stderr)
                .contains("injected loop completion interruption"),
            "{}",
            String::from_utf8_lossy(&interrupted.stderr)
        );
        assert_eq!(
            fs::read_to_string(&calls).unwrap().lines().count(),
            if loop_mode == Some("no_change_first") {
                2
            } else {
                3
            }
        );
        sqlx::query("DROP TRIGGER interrupt_loop_completion")
            .execute(&mut db)
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    let first = execute();
    if let Some(generator) = generator {
        generator.join().unwrap();
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
        assert_eq!(added, "encoder_gym.qualify_training\n");
    }
    let published: project_workspace_local::optimization_dataset::OptimizationDatasetPublication =
        serde_json::from_value(first["datasetStep"]["publication"].clone()).unwrap();
    assert_eq!(published.parent, dataset.reference());
    assert_eq!(published.removed.len(), 1);
    assert_eq!(published.generated.len(), 1);
    let rows = dataset_versions::materialization_rows(&folder, published.version.id)
        .await
        .unwrap();
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
    assert!(log.contains("Replace the ambiguous search example"));
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
