//! A recorded derived-project experiment through the production results CLI.
//! This verifies lookup/provenance, not native training or an Agent invocation.
use super::*;
use encoder_experiment_core::domain::{ExternalArtifactIdentity, ExternalProjectSnapshot};
use project_workspace_core::{
    DatasetPurpose, ProjectOptimizationExperiment, ProjectOptimizationMaterialization,
    ProjectOptimizationPreparation, ProviderAuthentication, ProviderCatalog, ProviderConfiguration,
    ProviderKind, ProviderLimits, ProviderRole,
    benchmark_results::{BenchmarkRunEvidence, OptimizationResultLineage, ProjectBenchmarkResults},
};
use project_workspace_local::{
    dataset_versions, import_dataset, inspect_dataset, open_workspace, optimization_launch,
    optimization_runs, optimization_setup, record_provider_catalog,
};

fn identity(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}

#[tokio::test]
async fn derived_optimization_results_follow_receipts_and_keep_original_baseline_context() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, original, source_run, _) = fixture(root).await;
    let version = run(
        root,
        &["benchmark", "project", "adopt-run", &source_run.to_string()],
    )["version"]
        .clone();
    let version: project_workspace_core::ProjectBenchmarkVersion =
        serde_json::from_value(version).unwrap();
    let source = root.join("selected.jsonl");
    fs::write(
        &source,
        "{\"text\":\"training example\",\"label\":\"route\"}\n",
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
    let provider = |role| ProviderConfiguration {
        role,
        kind: ProviderKind::Fake,
        endpoint: None,
        model: "deterministic".into(),
        authentication: ProviderAuthentication::None,
        secret: None,
        limits: ProviderLimits {
            maximum_requests: 2,
            maximum_input_tokens: 100,
            maximum_output_tokens: 100,
            maximum_cost_microusd: 0,
        },
    };
    let providers = ProviderCatalog::create(
        Uuid::new_v4(),
        workspace.manifest.id,
        1,
        None,
        vec![
            provider(ProviderRole::Advisor),
            provider(ProviderRole::Generation),
        ],
        "fixture",
        "offline",
        Utc::now(),
    )
    .unwrap();
    record_provider_catalog(&folder, providers, None)
        .await
        .unwrap();
    let model = workspace.model_catalog.as_ref().unwrap().active_model().id;
    let selection = optimization_setup::preview(&folder, model, dataset.id, version.id)
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
    let scope = optimization_launch::preview(&folder, setup.id)
        .await
        .unwrap()
        .scope;
    let reserved = optimization_runs::start(
        &folder,
        optimization_launch::LaunchRequest {
            id: Uuid::new_v4(),
            scope,
        },
        "fixture",
    )
    .await
    .unwrap();
    let launch = optimization_launch::list(&folder)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let binding = workspace.scientific_binding.as_ref().unwrap();
    let preparing = optimization_runs::begin_preparation(&folder, reserved.run.id)
        .await
        .unwrap();
    let preparation = ProjectOptimizationPreparation::create(
        &reserved.run,
        &launch,
        &setup,
        identity(binding.id, &binding.fingerprint),
        binding.runtime.project_snapshot.clone(),
        BoundIdentity {
            id: "nomos:nomos-ranking-v3".into(),
            fingerprint: binding.adapter.configuration_fingerprint.clone(),
        },
        1,
        vec!["development".into()],
        "holdout".into(),
        Utc::now(),
    )
    .unwrap();
    optimization_runs::finish_preparation(
        &folder,
        reserved.run.id,
        &preparing.head_fingerprint,
        preparation.clone(),
    )
    .await
    .unwrap();

    let mut inputs = original.inputs.clone();
    let training = ExternalArtifactIdentity::new(
        "selected-training.jsonl",
        EvidenceRole::Training,
        1,
        fp('d'),
    )
    .unwrap();
    *inputs
        .iter_mut()
        .find(|input| input.role == EvidenceRole::Training)
        .unwrap() = training.clone();
    let mut configuration = original.task_configuration.clone();
    configuration["training_inputs"] = json!([training.key]);
    // Native materialization re-inspects the exact baseline bytes and assigns
    // a new scientific artifact UUID. Content, not that local UUID, is shared.
    let mut derived_baseline = original.baseline_model.clone();
    derived_baseline.id = Uuid::new_v4();
    let derived = ExternalProjectSnapshot::create(
        "Derived training project",
        original.task,
        &original.source_revision,
        fp('e'),
        original.backend.clone(),
        inputs,
        derived_baseline,
        configuration,
        Utc::now(),
    )
    .unwrap();
    let materializing = optimization_runs::begin_materialization(&folder, reserved.run.id)
        .await
        .unwrap();
    let materialization = ProjectOptimizationMaterialization::create(
        &reserved.run,
        &launch,
        &preparation,
        BoundIdentity {
            id: format!("{}:{}", reserved.run.id, dataset.id),
            fingerprint: fp('d'),
        },
        training,
        identity(derived.id, &derived.fingerprint),
        Utc::now(),
    )
    .unwrap();
    optimization_runs::finish_materialization(
        &folder,
        reserved.run.id,
        &materializing.head_fingerprint,
        materialization.clone(),
    )
    .await
    .unwrap();

    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    store.create_project(derived.clone()).await.unwrap();
    let mut protocol = protocol_for(&derived, 0.0, 10);
    protocol.id = reserved.run.child_id("experiment-protocol", 1).unwrap();
    protocol.candidates[0].id = reserved.run.child_id("candidate", 1).unwrap();
    protocol.candidates[0].fingerprint = protocol.candidates[0].reproduce_fingerprint().unwrap();
    protocol.fingerprint = protocol.reproduce_fingerprint().unwrap();
    store.create_protocol(protocol.clone()).await.unwrap();
    let child = reserved.run.child_id("experiment-run", 1).unwrap();
    let first = first_event(&protocol, child, Utc::now()).unwrap();
    store.create_run(first.clone()).await.unwrap();
    let (native_model, receipt, config) =
        complete_rejected_candidate(&store, &derived, child).await;
    let workspace = register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &native_model,
        child,
        &receipt,
        &config,
        "Derived candidate",
    )
    .await;
    let catalog = workspace.model_catalog.as_ref().unwrap();
    let candidate = catalog
        .artifacts
        .iter()
        .find(|value| {
            value
                .producing_run
                .as_ref()
                .is_some_and(|value| value.id == child.to_string())
        })
        .unwrap();
    let lookup = || {
        run(
            root,
            &["benchmark", "project", "results", &version.id.to_string()],
        )
    };
    // Sharing a store and a benchmark is insufficient: an unlinked project is
    // not discovered by scanning for a matching name/model/metric definition.
    let before = lookup();
    assert!(
        before["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["modelId"] == candidate.id.to_string())
            .unwrap()["reports"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let attaching = optimization_runs::begin_experiment_attachment(&folder, reserved.run.id)
        .await
        .unwrap();
    let experiment = ProjectOptimizationExperiment::create(
        &reserved.run,
        &launch,
        &preparation,
        &materialization,
        version.source.protocol.clone(),
        identity(
            protocol.candidates[0].id,
            &protocol.candidates[0].fingerprint,
        ),
        identity(protocol.id, &protocol.fingerprint),
        identity(child, &first.fingerprint),
        Utc::now(),
    )
    .unwrap();
    optimization_runs::finish_experiment_attachment(
        &folder,
        reserved.run.id,
        &attaching.head_fingerprint,
        experiment.clone(),
    )
    .await
    .unwrap();
    let events = store.load_events(child).await.unwrap();
    // Exercise the lookup while committed fixture writes are still in WAL.
    // Keep the writer open so its close-time checkpoint cannot race this read.
    let scientific_path = folder.join("runs/scientific.sqlite");
    let wal_path = folder.join("runs/scientific.sqlite-wal");
    let before_wal_read = fs::read(&scientific_path).unwrap();
    let wal_before = fs::read(&wal_path).unwrap();
    let from_wal = lookup();
    assert!(
        fs::read(&scientific_path).unwrap() == before_wal_read,
        "Results reads changed the scientific database with a live writer"
    );
    assert!(
        fs::read(&wal_path).unwrap() == wal_before,
        "Results reads changed the scientific WAL"
    );
    // Finish fixture writes explicitly before comparing standalone DB bytes.
    // Pool shutdown may release a connection before its worker finishes closing.
    let checkpoint: (i32, i32, i32) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(checkpoint, (0, 0, 0));
    store.pool().close().await;
    let project_before = fs::read(folder.join("project.sqlite")).unwrap();
    let scientific_before = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    let after = lookup();
    assert_eq!(after, from_wal);
    let reports = &after["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["modelId"] == candidate.id.to_string())
        .unwrap()["reports"];
    assert_eq!(reports.as_array().unwrap().len(), 1);
    assert_eq!(reports[0]["result"]["metrics"]["mrr"], 0.6);
    let context = &reports[0]["contexts"][0];
    assert_eq!(context["runId"], child.to_string());
    assert_eq!(context["baselineModelId"], model.to_string());
    assert_eq!(
        context["baselineRevisionId"],
        binding.baseline_revision_id.to_string()
    );
    assert_eq!(context["assessment"]["verdict"], "failed");
    assert_eq!(
        context["source"]["projectSnapshot"]["id"],
        derived.id.to_string()
    );
    assert_eq!(
        context["source"]["scientificBinding"]["id"],
        binding.id.to_string()
    );
    assert_eq!(after, lookup());
    assert!(
        fs::read(folder.join("project.sqlite")).unwrap() == project_before,
        "Results reads changed the project database"
    );
    assert!(
        fs::read(folder.join("runs/scientific.sqlite")).unwrap() == scientific_before,
        "Results reads changed the scientific database"
    );
    assert!(!after.to_string().contains("99999.125"));
    assert!(!after.to_string().contains("NEVER_PROJECT_NATIVE_METADATA"));

    // Recompute a forged receipt's hash: structural validity must not let it
    // redirect the actual child's baseline or materialized project.
    let definition =
        encoder_experiment_nomos::NomosBackend::recorded_benchmark(&derived, &protocol).unwrap();
    for tamper in 0..4 {
        let mut prep = preparation.clone();
        let mut mat = materialization.clone();
        let mut attached = experiment.clone();
        match tamper {
            0 => prep.execution_binding.id = Uuid::new_v4().to_string(),
            1 => mat.scientific_project.id = Uuid::new_v4().to_string(),
            2 => attached.experiment_run.fingerprint = fp('0'),
            _ => attached.source_protocol.id = Uuid::new_v4().to_string(),
        }
        prep.fingerprint = prep.reproduce().unwrap();
        mat.preparation_fingerprint = prep.fingerprint.clone();
        mat.fingerprint = mat.reproduce().unwrap();
        attached.materialization_fingerprint = mat.fingerprint.clone();
        attached.scientific_project = mat.scientific_project.clone();
        attached.fingerprint = attached.reproduce().unwrap();
        let mut projection = ProjectBenchmarkResults::new(version.clone(), catalog).unwrap();
        let before = projection.clone();
        assert!(
            projection
                .include_optimization_run(
                    catalog,
                    BenchmarkRunEvidence {
                        binding,
                        project: &derived,
                        protocol: &protocol,
                        definition: &definition,
                        events: &events,
                    },
                    OptimizationResultLineage {
                        run: &reserved.run,
                        launch: &launch,
                        setup: &setup,
                        preparation: &prep,
                        materialization: &mat,
                        experiment: &attached,
                        runtime_project: &original,
                    }
                )
                .is_err()
        );
        assert_eq!(projection, before);
    }
    // A later baseline selection must not reinterpret the historical verdict.
    // This is a pure catalog fixture, not an actual promotion of a rejected model.
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
    later
        .include_optimization_run(
            &promoted,
            BenchmarkRunEvidence {
                binding,
                project: &derived,
                protocol: &protocol,
                definition: &definition,
                events: &events,
            },
            OptimizationResultLineage {
                run: &reserved.run,
                launch: &launch,
                setup: &setup,
                preparation: &preparation,
                materialization: &materialization,
                experiment: &experiment,
                runtime_project: &original,
            },
        )
        .unwrap();
    let current = later.models.iter().find(|value| value.is_baseline).unwrap();
    assert_eq!(current.model_id, candidate.id);
    assert_eq!(current.reports[0].contexts[0].baseline_model_id, model);
    assert_eq!(
        current.reports[0].contexts[0]
            .assessment
            .as_ref()
            .unwrap()
            .verdict,
        encoder_experiment_core::metrics::CandidateVerdict::Failed
    );
    assert_eq!(
        open_workspace(&folder, false).await.unwrap().manifest.id,
        workspace.manifest.id
    );
}
