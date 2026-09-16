//! Actual CLI processes against synthetic scientific records; no native execution.
use chrono::Utc;
use encoder_experiment_core::{
    domain::{EvidenceRole, ModelArtifactIdentity},
    journal::first_event,
    ports::ExperimentStore,
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use project_workspace_core::BoundIdentity;
use project_workspace_local::record_scientific_binding;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use uuid::Uuid;

fn fp(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn run(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args(["--output", "json", "workspace"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "workspace {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[path = "fixtures/benchmark_support.rs"]
mod benchmark_support;
use benchmark_support::{
    complete_rejected_candidate, create_run, fixture, initial_benchmark_fixture, protocol_for,
    register_fixture_model,
};

#[path = "fixtures/optimization_result_check.rs"]
mod optimization_result_check;

#[path = "fixtures/optimization_iteration_check.rs"]
mod optimization_iteration_check;

#[path = "fixtures/optimization_agent_dataset_check.rs"]
mod optimization_agent_dataset_check;

#[tokio::test]
async fn executable_path_rebind_reuses_the_exact_existing_scientific_store() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let executable = env!("CARGO_BIN_EXE_synth-benchmark-fixture");
    let (folder, _) = initial_benchmark_fixture(root, Path::new(executable)).await;
    let before = project_workspace_local::open_workspace(&folder, true)
        .await
        .unwrap();
    let before_binding = before.scientific_binding.unwrap();
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join(&before_binding.store.database_path).display()
    ))
    .await
    .unwrap();
    let before_inventory = store.inventory().await.unwrap();
    store.pool().close().await;

    let preview = run(
        root,
        &[
            "preview-nomos-binding",
            "project",
            "--runtime",
            "runtime",
            "--python",
            executable,
        ],
    );
    assert_eq!(preview["store"]["action"], "reuse_verified_bound_store");
    assert_eq!(
        preview["store"]["databasePath"],
        before_binding.store.database_path
    );

    let rebound = run(
        root,
        &[
            "bind-nomos",
            "project",
            "--runtime",
            "runtime",
            "--python",
            executable,
            "--reason",
            "Update moved interpreter path",
        ],
    );
    assert_eq!(
        rebound["scientificBinding"]["previousBindingId"],
        before_binding.id.to_string()
    );
    assert_eq!(
        rebound["scientificBinding"]["store"]["databasePath"],
        before_binding.store.database_path
    );

    let after = project_workspace_local::open_workspace(&folder, true)
        .await
        .unwrap();
    let after_binding = after.scientific_binding.unwrap();
    assert_eq!(after_binding.previous_binding_id, Some(before_binding.id));
    assert_eq!(after_binding.store, before_binding.store);
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join(&after_binding.store.database_path).display()
    ))
    .await
    .unwrap();
    assert_eq!(store.inventory().await.unwrap(), before_inventory);
    store.pool().close().await;
}

#[tokio::test]
async fn benchmark_initialize_evaluates_the_baseline_once_and_replays_without_native_work() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let executable = Path::new(env!("CARGO_BIN_EXE_synth-benchmark-fixture"));
    let (folder, project) = initial_benchmark_fixture(root, executable).await;
    assert_eq!(run(root, &["benchmark", "project", "list"]), json!([]));

    let initialized = run(root, &["benchmark", "project", "initialize"]);
    assert_eq!(initialized["version"]["number"], 1);
    assert_eq!(
        initialized["version"]["definition"]["suites"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|suite| suite["role"] == "development")
            .count(),
        2
    );
    let public = initialized.to_string();
    assert!(!public.contains("99999.125"));
    assert!(!public.contains("NEVER_DISCLOSE_HOLDOUT"));
    let invocations = fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap();
    assert_eq!(invocations.lines().count(), 6);

    let protocol_id: Uuid = initialized["version"]["source"]["protocol"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let protocol = store.get_protocol(protocol_id).await.unwrap().unwrap();
    assert_eq!(protocol.baseline_development_reports().len(), 2);
    assert_eq!(protocol.baseline_sealed_report.metrics["mrr"], 99_999.125);
    assert!(
        store
            .experiment_run_ids_for_project(project.id)
            .await
            .unwrap()
            .is_empty()
    );
    store.pool().close().await;

    let replayed = run(root, &["benchmark", "project", "initialize"]);
    assert_eq!(replayed["version"], initialized["version"]);
    assert_eq!(
        fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap(),
        invocations
    );
    assert_eq!(
        run(root, &["benchmark", "project", "list"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let action = run(
        root,
        &[
            "activity",
            "project",
            "show",
            initialized["actionId"].as_str().unwrap(),
        ],
    );
    assert_eq!(action["operation"], "benchmark.initialize");
    assert_eq!(action["state"], "succeeded");
    assert!(!action.to_string().contains("99999.125"));
    assert!(!action.to_string().contains("NEVER_DISCLOSE_HOLDOUT"));
}

#[tokio::test]
async fn benchmark_preview_adoption_versions_replay_and_inspection_use_real_cli_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, project, first, changed) = fixture(root).await;
    let database = fs::read(folder.join("project.sqlite")).unwrap();
    let scientific = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    let preview = run(
        root,
        &["benchmark", "project", "preview-run", &first.to_string()],
    );
    assert_eq!(fs::read(folder.join("project.sqlite")).unwrap(), database);
    assert!(!preview.to_string().contains("99999.125"));
    assert!(!preview.to_string().contains("baseline_model"));
    let mismatch = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args([
            "--output",
            "json",
            "workspace",
            "benchmark",
            "project",
            "adopt-run",
            &first.to_string(),
            "--expected-definition",
            &fp('d'),
        ])
        .output()
        .unwrap();
    assert!(!mismatch.status.success());
    assert_eq!(run(root, &["benchmark", "project", "list"]), json!([]));
    let adopted = run(
        root,
        &[
            "benchmark",
            "project",
            "adopt-run",
            &first.to_string(),
            "--expected-definition",
            preview["definition"]["fingerprint"].as_str().unwrap(),
        ],
    );
    let one = &adopted["version"];
    assert_eq!(
        fs::read(folder.join("runs/scientific.sqlite")).unwrap(),
        scientific
    );
    assert_eq!(one["id"], preview["versionId"]);
    assert_eq!(one["number"], 1);
    assert_eq!(
        run(
            root,
            &["benchmark", "project", "adopt-run", &first.to_string()]
        )["version"],
        *one
    );
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let unchanged_protocol = store.load_events(first).await.unwrap()[0].protocol_id;
    store.pool().close().await;
    let writable = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let repeated = create_run(&writable, &project, 0.0, 3).await;
    writable.pool().close().await;
    assert_eq!(
        run(
            root,
            &["benchmark", "project", "adopt-run", &repeated.to_string()]
        )["version"],
        *one
    );
    assert_eq!(
        one["source"]["protocol"]["id"],
        unchanged_protocol.to_string()
    );
    let scientific_after_fixture = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    let stale = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args([
            "--output",
            "json",
            "workspace",
            "benchmark",
            "project",
            "adopt-run",
            &changed.to_string(),
        ])
        .output()
        .unwrap();
    assert!(!stale.status.success());
    let two = run(
        root,
        &[
            "benchmark",
            "project",
            "adopt-run",
            &changed.to_string(),
            "--expected-parent",
            one["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(two["version"]["number"], 2);
    assert_eq!(
        run(root, &["benchmark", "project", "list"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let action = run(
        root,
        &[
            "activity",
            "project",
            "show",
            adopted["actionId"].as_str().unwrap(),
        ],
    );
    assert_eq!(action["state"], "succeeded");
    assert!(action.to_string().contains("benchmark_version"));
    assert!(!action.to_string().contains("99999.125"));
    fs::rename(&folder, root.join("moved")).unwrap();
    let before = fs::read(root.join("moved/project.sqlite")).unwrap();
    assert_eq!(
        run(
            root,
            &["benchmark", "moved", "inspect", one["id"].as_str().unwrap()]
        ),
        *one
    );
    assert_eq!(fs::read(root.join("moved/project.sqlite")).unwrap(), before);
    assert_eq!(
        fs::read(root.join("moved/runs/scientific.sqlite")).unwrap(),
        scientific_after_fixture
    );
    assert!(!root.join("moved/runs/missing-runtime").exists());
}

#[tokio::test]
async fn native_benchmark_context_pins_agent_and_membership_but_not_training_or_baseline() {
    use encoder_experiment_nomos::NomosBackend;
    let temp = tempfile::tempdir().unwrap();
    let (_, project, _, _) = fixture(temp.path()).await;
    let initial =
        NomosBackend::recorded_benchmark(&project, &protocol_for(&project, 0.0, 1)).unwrap();
    let mut model_changed = project.clone();
    model_changed.baseline_model.fingerprint = fp('d');
    model_changed.task_configuration["training_inputs"] = json!(["other-training.jsonl"]);
    model_changed.task_configuration["baseline_evidence"] = json!({"updated":true});
    model_changed.fingerprint = model_changed.reproduce_fingerprint().unwrap();
    assert_eq!(
        initial,
        NomosBackend::recorded_benchmark(&model_changed, &protocol_for(&model_changed, 0.0, 2))
            .unwrap()
    );
    for key in ["nomos_top_k", "max_attempts"] {
        let mut changed = project.clone();
        changed.task_configuration["agent_evaluation"][key] = json!(3);
        changed.fingerprint = changed.reproduce_fingerprint().unwrap();
        assert_ne!(
            initial.fingerprint,
            NomosBackend::recorded_benchmark(&changed, &protocol_for(&changed, 0.0, 1))
                .unwrap()
                .fingerprint
        );
    }
    let mut changed = project.clone();
    changed.task_configuration["agent_evaluation"]["chat_model"]["fingerprint"] = json!(fp('d'));
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert_ne!(
        initial.fingerprint,
        NomosBackend::recorded_benchmark(&changed, &protocol_for(&changed, 0.0, 1))
            .unwrap()
            .fingerprint
    );
    let mut changed = project.clone();
    changed
        .inputs
        .iter_mut()
        .find(|v| v.role == EvidenceRole::Development)
        .unwrap()
        .fingerprint = fp('e');
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert_ne!(
        initial.fingerprint,
        NomosBackend::recorded_benchmark(&changed, &protocol_for(&changed, 0.0, 1))
            .unwrap()
            .fingerprint
    );
    let mut changed = project.clone();
    changed.task_configuration["suites"]["development"]["fingerprint"] = json!(fp('f'));
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert!(NomosBackend::recorded_benchmark(&changed, &protocol_for(&changed, 0.0, 1)).is_err());
}

#[tokio::test]
async fn benchmark_results_include_registered_rejections_and_all_owned_history_without_sealed_scores()
 {
    use project_workspace_local::{open_workspace, scientific_binding_history};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, project, first, changed) = fixture(root).await;
    let adopted = run(
        root,
        &["benchmark", "project", "adopt-run", &first.to_string()],
    );
    let version = adopted["version"]["id"].as_str().unwrap();
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let (model, receipt, configuration) =
        complete_rejected_candidate(&store, &project, first).await;
    // Reusing one frozen baseline report in another run must retain both contexts.
    let protocol_id = store.load_events(first).await.unwrap()[0].protocol_id;
    let protocol = store.get_protocol(protocol_id).await.unwrap().unwrap();
    store
        .create_run(first_event(&protocol, Uuid::new_v4(), Utc::now()).unwrap())
        .await
        .unwrap();
    let other = create_run(&store, &project, 0.0, 3).await;
    let (unregistered, _, _) = complete_rejected_candidate(&store, &project, other).await;
    store.pool().close().await;
    let workspace = register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &model,
        first,
        &receipt,
        &configuration,
        "Rejected candidate",
    )
    .await;
    let rejected_id = workspace
        .model_catalog
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .find(|m| {
            m.source_model
                .as_ref()
                .is_some_and(|s| s.id == model.id.to_string())
        })
        .unwrap()
        .id;
    // An owned model without a matching evaluation must remain in the table.
    let missing =
        ModelArtifactIdentity::new("not-evaluated", model.format.clone(), model.bytes, fp('e'))
            .unwrap();
    register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &missing,
        Uuid::new_v4(),
        &fp('d'),
        &fp('a'),
        "Not evaluated",
    )
    .await;
    let mut previous = workspace.scientific_binding.unwrap();
    let original_binding = previous.id;
    let baseline = workspace.model_catalog.as_ref().unwrap().active_model().id;
    let original_revision = previous.baseline_revision_id;
    // Two later bindings have different scientific snapshots and stores. The
    // middle binding is neither the benchmark's original source nor the active one.
    for (index, score) in [(2, 0.55), (3, 0.45)] {
        let mut another_project = project.clone();
        another_project.id = Uuid::new_v4();
        another_project.source_fingerprint =
            artifact_core::fingerprint(&json!({"source": index})).unwrap();
        another_project.fingerprint = another_project.reproduce_fingerprint().unwrap();
        let path = format!("runs/scientific-{index}.sqlite");
        let store =
            SqliteExperimentStore::connect(&format!("sqlite://{}", folder.join(&path).display()))
                .await
                .unwrap();
        store.create_project(another_project.clone()).await.unwrap();
        let mut protocol = protocol_for(&another_project, 0.0, index);
        protocol
            .baseline_development_report
            .metrics
            .insert("mrr".into(), score);
        protocol.baseline_development_report.fingerprint = protocol
            .baseline_development_report
            .reproduce_fingerprint()
            .unwrap();
        protocol.fingerprint = protocol.reproduce_fingerprint().unwrap();
        store.create_protocol(protocol.clone()).await.unwrap();
        store
            .create_run(first_event(&protocol, Uuid::new_v4(), Utc::now()).unwrap())
            .await
            .unwrap();
        // A foreign project co-located in this store cannot become project results.
        let mut foreign = another_project.clone();
        foreign.id = Uuid::new_v4();
        foreign.source_fingerprint =
            artifact_core::fingerprint(&json!({"foreign": index})).unwrap();
        foreign.fingerprint = foreign.reproduce_fingerprint().unwrap();
        store.create_project(foreign.clone()).await.unwrap();
        create_run(&store, &foreign, 0.0, 9).await;
        assert_eq!(
            store
                .experiment_run_ids_for_project(another_project.id)
                .await
                .unwrap()
                .len(),
            1
        );
        store.pool().close().await;
        let mut binding = previous.clone();
        binding.id = Uuid::new_v4();
        binding.previous_binding_id = Some(previous.id);
        binding.runtime.project_snapshot = BoundIdentity {
            id: another_project.id.to_string(),
            fingerprint: another_project.fingerprint,
        };
        binding.store.database_path = path;
        binding.created_at = Utc::now();
        binding.specification_fingerprint = binding.reproduce_specification_fingerprint().unwrap();
        binding.fingerprint = binding.reproduce_fingerprint().unwrap();
        record_scientific_binding(&folder, binding.clone(), Some(previous.id))
            .await
            .unwrap();
        previous = binding;
    }
    assert_eq!(scientific_binding_history(&folder).await.unwrap().len(), 3);
    let paths = [
        "project.sqlite",
        "runs/scientific.sqlite",
        "runs/scientific-2.sqlite",
        "runs/scientific-3.sqlite",
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|p| fs::read(folder.join(p)).unwrap())
        .collect();
    let output = run(root, &["benchmark", "project", "results", version]);
    let models = output["models"].as_array().unwrap();
    assert_eq!(models.len(), 3);
    assert_eq!(models.iter().filter(|m| m["isBaseline"] == true).count(), 1);
    let rejected = models
        .iter()
        .find(|m| m["modelId"] == rejected_id.to_string())
        .unwrap();
    assert_eq!(rejected["reports"].as_array().unwrap().len(), 1);
    assert_eq!(rejected["reports"][0]["result"]["metrics"]["mrr"], 0.6);
    let context = &rejected["reports"][0]["contexts"][0];
    assert_eq!(context["assessment"]["verdict"], "failed");
    assert_eq!(context["baselineModelId"], baseline.to_string());
    assert_eq!(context["baselineRevisionId"], original_revision.to_string());
    assert_eq!(
        context["source"]["scientificBinding"]["id"],
        original_binding.to_string()
    );
    let base = models.iter().find(|m| m["isBaseline"] == true).unwrap();
    let reports = base["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 4); // two distinct original reports, plus both later bindings
    assert!(
        reports
            .iter()
            .any(|r| r["contexts"].as_array().unwrap().len() == 2)
    );
    for score in [0.55, 0.45] {
        assert!(
            reports
                .iter()
                .any(|r| r["result"]["metrics"]["mrr"] == score)
        );
    }
    assert!(
        models
            .iter()
            .any(|m| m["name"] == "Not evaluated" && m["reports"] == json!([]))
    );
    assert!(!output.to_string().contains(&unregistered.id.to_string()));
    assert!(!output.to_string().contains(&changed.to_string()));
    assert!(!output.to_string().contains("99999.125"));
    assert!(!output.to_string().contains("NEVER_PROJECT_NATIVE_METADATA"));
    assert!(!output.to_string().contains("sealed_assessment"));
    for (path, before) in paths.iter().zip(&before) {
        assert_eq!(fs::read(folder.join(path)).unwrap(), *before);
    }
    assert_eq!(
        open_workspace(&folder, false)
            .await
            .unwrap()
            .benchmark_versions
            .len(),
        1
    );
    fs::rename(&folder, root.join("moved-results")).unwrap();
    assert_eq!(
        run(root, &["benchmark", "moved-results", "results", version]),
        output
    );
}

#[tokio::test]
async fn result_projection_retains_original_verdict_after_baseline_change_and_rejects_substitution()
{
    use encoder_experiment_nomos::NomosBackend;
    use project_workspace_core::{
        ProjectBenchmarkVersion,
        benchmark_results::{BenchmarkRunEvidence, ProjectBenchmarkResults},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, project, first, _) = fixture(root).await;
    let adopted = run(
        root,
        &["benchmark", "project", "adopt-run", &first.to_string()],
    );
    let version: ProjectBenchmarkVersion =
        serde_json::from_value(adopted["version"].clone()).unwrap();
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let (model, receipt, configuration) =
        complete_rejected_candidate(&store, &project, first).await;
    let events = store.load_events(first).await.unwrap();
    let protocol = store
        .get_protocol(events[0].protocol_id)
        .await
        .unwrap()
        .unwrap();
    store.pool().close().await;
    let workspace = register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &model,
        first,
        &receipt,
        &configuration,
        "Candidate",
    )
    .await;
    let binding = workspace.scientific_binding.unwrap();
    let catalog = workspace.model_catalog.unwrap();
    let candidate = catalog
        .artifacts
        .iter()
        .find(|m| m.source_model.is_some())
        .unwrap();
    let definition = NomosBackend::recorded_benchmark(&project, &protocol).unwrap();
    let evidence = || BenchmarkRunEvidence {
        binding: &binding,
        project: &project,
        protocol: &protocol,
        definition: &definition,
        events: &events,
    };
    let mut results = ProjectBenchmarkResults::new(version.clone(), &catalog).unwrap();
    results.include_run(&catalog, evidence()).unwrap();
    let unchanged = results.clone();
    results.include_run(&catalog, evidence()).unwrap();
    assert_eq!(results, unchanged);
    for field in ["receipt", "source"] {
        let mut altered = catalog.clone();
        let artifact = altered
            .artifacts
            .iter_mut()
            .find(|m| m.id == candidate.id)
            .unwrap();
        if field == "receipt" {
            artifact.producing_run.as_mut().unwrap().fingerprint = fp('d');
        } else {
            artifact.source_model.as_mut().unwrap().fingerprint = fp('e');
        }
        let mut projection = ProjectBenchmarkResults::new(version.clone(), &altered).unwrap();
        let before = projection.clone();
        assert!(projection.include_run(&altered, evidence()).is_err());
        assert_eq!(projection, before);
    }
    let mut changed_definition = definition.clone();
    changed_definition.evaluation_configuration_fingerprint = fp('d');
    changed_definition.fingerprint = changed_definition.reproduce_fingerprint().unwrap();
    assert!(
        results
            .include_run(
                &catalog,
                BenchmarkRunEvidence {
                    definition: &changed_definition,
                    ..evidence()
                }
            )
            .is_err()
    );
    assert_eq!(results, unchanged);
    let mut altered_events = events.clone();
    altered_events[1].fingerprint = fp('e');
    assert!(
        results
            .include_run(
                &catalog,
                BenchmarkRunEvidence {
                    events: &altered_events,
                    ..evidence()
                }
            )
            .is_err()
    );
    assert_eq!(results, unchanged);
    // A validly rehashed report cannot reuse an existing UUID with different scores.
    let mut collision = protocol.clone();
    collision.id = Uuid::new_v4();
    collision
        .baseline_development_report
        .metrics
        .insert("mrr".into(), 0.75);
    collision.baseline_development_report.fingerprint = collision
        .baseline_development_report
        .reproduce_fingerprint()
        .unwrap();
    collision.fingerprint = collision.reproduce_fingerprint().unwrap();
    let collision_events = vec![first_event(&collision, Uuid::new_v4(), Utc::now()).unwrap()];
    assert!(
        results
            .include_run(
                &catalog,
                BenchmarkRunEvidence {
                    protocol: &collision,
                    events: &collision_events,
                    ..evidence()
                }
            )
            .is_err()
    );
    assert_eq!(results, unchanged);
    // This synthetic catalog change tests projection, not promotion authority.
    // Actual promotion eligibility remains enforced by its owning workflow.
    let promoted = catalog
        .with_promotion(
            candidate.clone(),
            Uuid::new_v4(),
            "fixture-decision".into(),
            fp('a'),
            "fixture",
            "Later baseline selection",
            Utc::now(),
        )
        .unwrap();
    let mut later = ProjectBenchmarkResults::new(version, &promoted).unwrap();
    later.include_run(&promoted, evidence()).unwrap();
    let now_baseline = later.models.iter().find(|m| m.is_baseline).unwrap();
    assert_eq!(now_baseline.model_id, candidate.id);
    let original = &now_baseline.reports[0].contexts[0];
    assert_eq!(original.baseline_model_id, catalog.active_model().id);
    assert_eq!(
        original.baseline_revision_id,
        catalog.active_baseline_revision_id
    );
    assert_eq!(
        original.assessment.as_ref().unwrap().verdict,
        encoder_experiment_core::metrics::CandidateVerdict::Failed
    );
    assert!(!serde_json::to_string(&later).unwrap().contains("99999.125"));
}
