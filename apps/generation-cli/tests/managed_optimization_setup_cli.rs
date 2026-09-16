//! Identity-only setup through actual CLI processes; no native work or providers.
#[path = "fixtures/benchmark_support.rs"]
mod benchmark_support;
use benchmark_support::{complete_rejected_candidate, fixture, register_fixture_model};
use chrono::Utc;
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalArtifactIdentity},
    ports::ExperimentStore,
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_sqlite::SqliteExperimentStore;
use project_workspace_core::{
    BaselineRevision, BoundIdentity, OptimizationLaunchAuthorization, OptimizationSetup,
    ProjectOptimizationExperiment, ProjectOptimizationFinalResult,
    ProjectOptimizationFinalResultKind, ProjectOptimizationMaterialization,
    ProjectOptimizationOutcome, ProjectOptimizationOutcomeKind, ProjectOptimizationPreparation,
    ProviderAuthentication, ProviderCatalog, ProviderConfiguration, ProviderKind, ProviderLimits,
    ProviderRole,
};
use project_workspace_local::{
    dataset_versions, import_dataset, inspect_dataset, open_workspace, optimization_runs,
    record_provider_catalog,
};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use uuid::Uuid;

#[tokio::test]
async fn shared_benchmark_is_verified_against_the_current_adapter_without_sealed_execution() {
    let temp = tempfile::tempdir().unwrap();
    let (folder, project, first, _) = fixture(temp.path()).await;
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let protocol = store
        .get_protocol(store.load_events(first).await.unwrap()[0].protocol_id)
        .await
        .unwrap()
        .unwrap();
    let definition = NomosBackend::recorded_benchmark(&project, &protocol).unwrap();
    NomosBackend::verify_benchmark_definition(&project, &definition).unwrap();
    let mut changed = definition;
    changed.backend.configuration_fingerprint = format!("sha256:{}", "f".repeat(64));
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert!(NomosBackend::verify_benchmark_definition(&project, &changed).is_err());
    store.pool().close().await;
}

fn invoke(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args(["--output", "json", "workspace"])
        .args(args)
        .output()
        .unwrap()
}
fn run(root: &Path, args: &[&str]) -> Value {
    let result = invoke(root, args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
async fn prepared(root: &Path) -> (PathBuf, String, String, String) {
    let (folder, project, first, _) = fixture(root).await;
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    let (model, receipt, config) = complete_rejected_candidate(&store, &project, first).await;
    store.pool().close().await;
    register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &model,
        first,
        &receipt,
        &config,
        "Candidate 01",
    )
    .await;
    let source = root.join("rows.jsonl");
    fs::write(
        &source,
        "{\"text\":\"SETUP_ROW_CANARY base\",\"label\":\"route\"}\n",
    )
    .unwrap();
    let preview =
        inspect_dataset(&source, project_workspace_core::DatasetPurpose::Training).unwrap();
    let workspace = import_dataset(
        &folder,
        &source,
        "Base rows",
        project_workspace_core::DatasetPurpose::Training,
        &preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    let data = dataset_versions::create_base(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Base data",
        &[workspace.datasets[0].id],
    )
    .await
    .unwrap();
    let benchmark = run(
        root,
        &["benchmark", "project", "adopt-run", &first.to_string()],
    );
    (
        folder,
        workspace
            .model_catalog
            .unwrap()
            .active_model()
            .id
            .to_string(),
        data.id.to_string(),
        benchmark["version"]["id"].as_str().unwrap().into(),
    )
}
fn preview(root: &Path, model: &str, data: &str, benchmark: &str) -> Value {
    run(
        root,
        &[
            "optimization-setup",
            "project",
            "preview",
            "--model",
            model,
            "--dataset-version",
            data,
            "--benchmark-version",
            benchmark,
        ],
    )
}
fn request(root: &Path, preview: &Value, id: Uuid) -> Value {
    let value =
        json!({"id":id,"expectedParent":preview["expectedParent"],"inputs":preview["inputs"]});
    fs::write(root.join("setup.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    value
}
fn save(root: &Path) -> Value {
    run(
        root,
        &[
            "optimization-setup",
            "project",
            "save",
            "--file",
            "setup.json",
        ],
    )
}

async fn configure_fake_providers(
    folder: &Path,
    previous: Option<&ProviderCatalog>,
) -> ProviderCatalog {
    let workspace = open_workspace(folder, false).await.unwrap();
    let provider = |role, maximum_requests| ProviderConfiguration {
        role,
        kind: ProviderKind::Fake,
        endpoint: None,
        model: format!("fixture-{}", role.key()),
        authentication: ProviderAuthentication::None,
        secret: None,
        limits: ProviderLimits {
            maximum_requests,
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 2_000,
            maximum_cost_microusd: 0,
        },
    };
    let catalog = ProviderCatalog::create(
        Uuid::new_v4(),
        workspace.manifest.id,
        previous.map_or(1, |catalog| catalog.sequence + 1),
        previous.map(|catalog| catalog.id),
        vec![
            provider(ProviderRole::Generation, 11),
            provider(ProviderRole::Advisor, 7),
        ],
        "operator",
        "configure fixture providers",
        Utc::now(),
    )
    .unwrap();
    record_provider_catalog(
        Path::new(&workspace.folder),
        catalog.clone(),
        previous.map(|value| value.id),
    )
    .await
    .unwrap();
    catalog
}

#[tokio::test]
async fn provider_connections_are_pinned_by_the_real_cli_without_credential_values() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, _, _, _) = prepared(root).await;
    let project = open_workspace(&folder, false).await.unwrap().manifest.id;
    let connection = Uuid::new_v4();
    let provider = |model| {
        json!({"kind":"openai-compatible", "endpoint":"https://provider.example.test/v1", "model":model, "authentication":"bearer", "connection_id":connection,
        "limits":{"maximumRequests":5,"maximumInputTokens":10000,"maximumOutputTokens":1000,"maximumCostMicrousd":100000}})
    };
    let settings = json!({"version":1, "generation":provider("generation-model"), "advisor":provider("agent-model")});
    fs::write(
        root.join("connections.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let saved = run(
        root,
        &[
            "providers",
            "project",
            "configure",
            "--file",
            "connections.json",
        ],
    );
    let expected = format!("{project}:connection:{connection}");
    for item in saved["catalog"]["providers"].as_array().unwrap() {
        assert_eq!(item["secret"]["id"], expected);
        assert!(item["secret"].get("environmentFallback").is_none());
    }
    let reloaded = run(root, &["providers", "project", "show"]);
    assert_eq!(saved["catalog"], reloaded["catalog"]);
    let mut invalid = settings;
    invalid["advisor"]["environment_fallback"] = "SYNTH_ADVISOR_API_KEY".into();
    fs::write(
        root.join("connections.json"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    let refused = invoke(
        root,
        &[
            "providers",
            "project",
            "configure",
            "--expected-revision-id",
            saved["catalog"]["id"].as_str().unwrap(),
            "--file",
            "connections.json",
        ],
    );
    assert!(!refused.status.success());
    assert_eq!(
        run(root, &["providers", "project", "show"])["catalog"],
        saved["catalog"]
    );
}

#[tokio::test]
async fn agent_settings_preview_and_authority_are_immutable_and_cannot_run_as_a_fixed_recipe() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, model, data, benchmark) = prepared(root).await;
    configure_fake_providers(&folder, None).await;
    let choice = preview(root, &model, &data, &benchmark);
    request(root, &choice, Uuid::new_v4());
    let setup = save(root)["setup"].clone();
    let before = fs::read(folder.join("project.sqlite")).unwrap();
    let standard = run(
        root,
        &[
            "optimization-launch",
            "project",
            "preview",
            "--setup",
            setup["id"].as_str().unwrap(),
            "--agentic",
        ],
    );
    assert_eq!(
        standard["scope"]["agentic"],
        serde_json::to_value(project_workspace_core::OptimizationAgentSettings::default()).unwrap()
    );
    assert_eq!(standard["scope"]["limits"]["maximumIterations"], 3);
    assert_eq!(standard["scope"]["limits"]["maximumDatasetRowChanges"], 192);
    let preview = run(
        root,
        &[
            "optimization-launch",
            "project",
            "preview",
            "--setup",
            setup["id"].as_str().unwrap(),
            "--quick-test",
        ],
    );
    assert_eq!(before, fs::read(folder.join("project.sqlite")).unwrap());
    assert_eq!(preview["scope"]["agentic"]["mode"], "quick_test");
    assert_eq!(preview["scope"]["agentic"]["generationConcurrency"], 1);
    assert_eq!(preview["scope"]["limits"]["maximumIterations"], 1);
    assert_eq!(preview["scope"]["limits"]["maximumTrainingSeconds"], 120);
    assert_eq!(preview["scope"]["finalEvaluation"], "development_only");
    assert_eq!(preview["scope"]["limits"]["maximumFinalEvaluations"], 0);
    let request = json!({"id":Uuid::new_v4(), "scope":preview["scope"]});
    fs::write(
        root.join("agentic-launch.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let saved = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "agentic-launch.json",
        ],
    );
    assert_eq!(saved["authorization"]["scope"], preview["scope"]);
    let retry = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "agentic-launch.json",
        ],
    );
    assert_eq!(retry["authorization"], saved["authorization"]);
    let reserved = run(
        root,
        &[
            "optimization-run",
            "project",
            "start",
            "--file",
            "agentic-launch.json",
        ],
    );
    assert_eq!(reserved["run"]["state"], "queued");
    let rejected = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "materialize",
            reserved["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("fixed-recipe executor cannot honor")
    );
    assert_eq!(
        run(root, &["optimization-run", "project", "list"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let settings = project_workspace_core::OptimizationAgentSettings {
        maximum_iterations: 2,
        generation_concurrency: 4,
        objective: "Improve routing without generic regression.".into(),
        ..Default::default()
    };
    fs::write(
        root.join("agent-settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let custom = run(
        root,
        &[
            "optimization-launch",
            "project",
            "preview",
            "--setup",
            setup["id"].as_str().unwrap(),
            "--settings-file",
            "agent-settings.json",
        ],
    );
    assert_eq!(custom["scope"]["agentic"]["generationConcurrency"], 4);
    assert_eq!(custom["scope"]["limits"]["maximumModels"], 2);
    assert_eq!(
        custom["scope"]["limits"]["maximumDevelopmentEvaluations"],
        2
    );
    let mut invalid = serde_json::to_value(settings).unwrap();
    invalid["generationConcurrency"] = 0.into();
    fs::write(
        root.join("agent-settings.json"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    assert!(
        !invoke(
            root,
            &[
                "optimization-launch",
                "project",
                "preview",
                "--setup",
                setup["id"].as_str().unwrap(),
                "--settings-file",
                "agent-settings.json"
            ]
        )
        .status
        .success()
    );
    let history = run(root, &["optimization-launch", "project", "list"]);
    assert_eq!(history, json!([saved["authorization"]]));
}

#[tokio::test]
async fn one_click_authority_pins_exact_inputs_and_provider_revisions_without_secret_or_row_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, model, data, benchmark) = prepared(root).await;
    let providers = configure_fake_providers(&folder, None).await;
    let choice = preview(root, &model, &data, &benchmark);
    request(root, &choice, Uuid::new_v4());
    let setup = save(root)["setup"].clone();

    // Reproduce a project before launch/run tables. Reads stay byte-for-byte read-only.
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("DROP TABLE project_optimization_events")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("DROP TABLE project_optimization_runs")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("DROP TABLE optimization_launch_authorizations")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version IN (10,12)")
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();
    let before = fs::read(folder.join("project.sqlite")).unwrap();
    assert_eq!(
        run(root, &["optimization-launch", "project", "list"]),
        json!([])
    );
    assert_eq!(before, fs::read(folder.join("project.sqlite")).unwrap());
    assert_eq!(
        run(root, &["optimization-run", "project", "list"]),
        json!([])
    );
    assert_eq!(before, fs::read(folder.join("project.sqlite")).unwrap());
    let launch = run(
        root,
        &[
            "optimization-launch",
            "project",
            "preview",
            "--setup",
            setup["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(before, fs::read(folder.join("project.sqlite")).unwrap());
    assert_eq!(launch["modelName"], "Offline benchmark fixture baseline");
    assert_eq!(launch["datasetRows"], 1);
    assert_eq!(launch["benchmarkNumber"], 1);
    assert_eq!(launch["scope"]["setup"]["id"], setup["id"]);
    assert_eq!(
        launch["scope"]["providerCatalog"]["id"],
        providers.id.to_string()
    );
    assert_eq!(launch["scope"]["limits"]["maximumIterations"], 3);
    assert_eq!(launch["scope"]["limits"]["maximumModels"], 3);
    assert_eq!(launch["scope"]["limits"]["maximumFinalEvaluations"], 1);
    assert_eq!(launch["scope"]["generation"]["maximumRequests"], 11);
    assert_eq!(launch["scope"]["advisor"]["maximumRequests"], 7);
    assert_eq!(
        launch["scope"]["finalEvaluation"],
        "selected_candidate_once"
    );
    assert!(!launch.to_string().contains("SETUP_ROW_CANARY"));
    assert!(!launch.to_string().contains("apiKey"));

    let request = json!({"id":Uuid::new_v4(),"scope":launch["scope"]});
    fs::write(
        root.join("launch.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let authorized = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "launch.json",
        ],
    );
    let retried = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "launch.json",
        ],
    );
    assert_eq!(authorized["authorization"], retried["authorization"]);
    assert_eq!(
        run(root, &["optimization-launch", "project", "list"]),
        json!([authorized["authorization"]])
    );
    let started = run(
        root,
        &[
            "optimization-run",
            "project",
            "start",
            "--file",
            "launch.json",
        ],
    );
    let retried_run = run(
        root,
        &[
            "optimization-run",
            "project",
            "start",
            "--file",
            "launch.json",
        ],
    );
    assert_eq!(started["run"], retried_run["run"]);
    assert_eq!(started["run"]["state"], "queued");
    assert_eq!(started["run"]["run"]["launch"]["id"], request["id"]);
    assert_eq!(started["run"]["run"]["setup"]["id"], setup["id"]);
    assert_eq!(started["run"]["lastSequence"], 1);
    assert_eq!(
        run(root, &["optimization-run", "project", "list"]),
        json!([started["run"]])
    );
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "show",
                started["run"]["run"]["id"].as_str().unwrap(),
            ],
        ),
        started["run"]
    );
    assert!(!started.to_string().contains("SETUP_ROW_CANARY"));
    assert!(!started.to_string().contains("apiKey"));

    // Preparation records an honest failed attempt, then retries against the
    // exact same inputs after the native runtime is repaired.
    let failed_prepare = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "prepare",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(
        !failed_prepare.status.success(),
        "{}",
        String::from_utf8_lossy(&failed_prepare.stderr)
    );
    let failed = run(
        root,
        &[
            "optimization-run",
            "project",
            "show",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(
        failed["state"],
        "preparation_failed",
        "{}",
        String::from_utf8_lossy(&failed_prepare.stderr)
    );
    assert_eq!(failed["attempt"], 1);
    assert_eq!(failed["failureCode"], "input_verification_failed");
    // The fixture intentionally has no native runtime. Complete the second
    // attempt through the persistence boundary with a typed verified receipt;
    // the actual CLI must then replay it without trying to execute anything.
    let retry = optimization_runs::begin_preparation(
        &folder,
        started["run"]["run"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    let typed_setup: OptimizationSetup = serde_json::from_value(setup.clone()).unwrap();
    let typed_launch: OptimizationLaunchAuthorization =
        serde_json::from_value(authorized["authorization"].clone()).unwrap();
    let workspace = open_workspace(&folder, false).await.unwrap();
    let binding = workspace.scientific_binding.unwrap();
    let receipt = ProjectOptimizationPreparation::create(
        &retry.run,
        &typed_launch,
        &typed_setup,
        BoundIdentity {
            id: binding.id.to_string(),
            fingerprint: binding.fingerprint.clone(),
        },
        binding.runtime.project_snapshot,
        BoundIdentity {
            id: "nomos:nomos-ranking-v3".into(),
            fingerprint: binding.adapter.configuration_fingerprint,
        },
        1,
        vec!["development".into()],
        "holdout".into(),
        Utc::now(),
    )
    .unwrap();
    let mut foreign_receipt = receipt.clone();
    foreign_receipt.execution_binding = BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: format!("sha256:{}", "9".repeat(64)),
    };
    foreign_receipt.fingerprint = foreign_receipt.reproduce().unwrap();
    assert!(
        optimization_runs::finish_preparation(
            &folder,
            retry.run.id,
            &retry.head_fingerprint,
            foreign_receipt,
        )
        .await
        .is_err()
    );
    let recorded = optimization_runs::finish_preparation(
        &folder,
        retry.run.id,
        &retry.head_fingerprint,
        receipt,
    )
    .await
    .unwrap();
    assert_eq!(
        recorded.state,
        project_workspace_core::ProjectOptimizationRunState::Ready
    );
    let prepared = run(
        root,
        &[
            "optimization-run",
            "project",
            "prepare",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(prepared["run"]["state"], "ready");
    assert_eq!(prepared["run"]["attempt"], 2);
    assert_eq!(prepared["run"]["preparation"]["datasetRows"], 1);
    assert_eq!(
        prepared["run"]["preparation"]["model"]["id"],
        setup["inputs"]["model"]["id"]
    );
    assert_eq!(prepared["run"]["preparation"]["benchmark"]["id"], benchmark);
    assert_eq!(
        prepared["run"]["preparation"]["providerCatalog"]["id"],
        providers.id.to_string()
    );
    assert_eq!(
        prepared["run"]["preparation"]["developmentSuites"],
        json!(["development"])
    );
    assert_eq!(prepared["run"]["preparation"]["finalSuite"], "holdout");
    let prepared_retry = run(
        root,
        &[
            "optimization-run",
            "project",
            "prepare",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(prepared_retry["run"], prepared["run"]);
    assert!(!prepared.to_string().contains("SETUP_ROW_CANARY"));
    assert!(!prepared.to_string().contains("apiKey"));

    // Materialization is its own retryable stage. The intentionally absent
    // fixture runtime fails honestly; a typed receipt completes the recovered
    // attempt, and the real CLI then replays it without touching native work.
    let failed_materialization = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "materialize",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!failed_materialization.status.success());
    let failed_materialization_view = run(
        root,
        &[
            "optimization-run",
            "project",
            "show",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(
        failed_materialization_view["state"],
        "materialization_failed"
    );
    assert_eq!(failed_materialization_view["materializationAttempt"], 1);
    assert_eq!(
        failed_materialization_view["failureCode"],
        "dataset_materialization_failed"
    );
    let materializing = optimization_runs::begin_materialization(
        &folder,
        started["run"]["run"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    let preparation = materializing.preparation.as_ref().unwrap();
    let materialization = ProjectOptimizationMaterialization::create(
        &materializing.run,
        &typed_launch,
        preparation,
        BoundIdentity {
            id: format!("{}:{}", materializing.run.id, data),
            fingerprint: format!("sha256:{}", "7".repeat(64)),
        },
        ExternalArtifactIdentity::new(
            format!("runs/project/{data}/dataset.jsonl"),
            EvidenceRole::Training,
            42,
            format!("sha256:{}", "8".repeat(64)),
        )
        .unwrap(),
        BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: format!("sha256:{}", "9".repeat(64)),
        },
        Utc::now(),
    )
    .unwrap();
    let materialized = optimization_runs::finish_materialization(
        &folder,
        materializing.run.id,
        &materializing.head_fingerprint,
        materialization,
    )
    .await
    .unwrap();
    assert_eq!(
        materialized.state,
        project_workspace_core::ProjectOptimizationRunState::Materialized
    );
    let materialized = run(
        root,
        &[
            "optimization-run",
            "project",
            "materialize",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(materialized["run"]["state"], "materialized");
    assert_eq!(materialized["run"]["materializationAttempt"], 2);
    assert_eq!(
        materialized["run"]["preparation"],
        prepared["run"]["preparation"]
    );
    assert!(!materialized.to_string().contains("SETUP_ROW_CANARY"));

    // The fixture has no native materialization on disk, so the actual CLI
    // records an honest attachment failure. A typed child receipt proves the
    // recovery transition and exact idempotent CLI replay without execution.
    let failed_attachment = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "attach",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!failed_attachment.status.success());
    let failed_attachment_view = run(
        root,
        &[
            "optimization-run",
            "project",
            "show",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(
        failed_attachment_view["state"],
        "experiment_attachment_failed"
    );
    let attaching = optimization_runs::begin_experiment_attachment(
        &folder,
        started["run"]["run"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    let attached_receipt = ProjectOptimizationExperiment::create(
        &attaching.run,
        &typed_launch,
        attaching.preparation.as_ref().unwrap(),
        attaching.materialization.as_ref().unwrap(),
        BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        },
        BoundIdentity {
            id: attaching.run.child_id("candidate", 1).unwrap().to_string(),
            fingerprint: format!("sha256:{}", "b".repeat(64)),
        },
        BoundIdentity {
            id: attaching
                .run
                .child_id("experiment-protocol", 1)
                .unwrap()
                .to_string(),
            fingerprint: format!("sha256:{}", "c".repeat(64)),
        },
        BoundIdentity {
            id: attaching
                .run
                .child_id("experiment-run", 1)
                .unwrap()
                .to_string(),
            fingerprint: format!("sha256:{}", "d".repeat(64)),
        },
        Utc::now(),
    )
    .unwrap();
    let attached = optimization_runs::finish_experiment_attachment(
        &folder,
        attaching.run.id,
        &attaching.head_fingerprint,
        attached_receipt.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        attached.state,
        project_workspace_core::ProjectOptimizationRunState::ReadyToRun
    );
    let attached = run(
        root,
        &[
            "optimization-run",
            "project",
            "attach",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(attached["run"]["state"], "ready_to_run");
    assert_eq!(attached["run"]["experimentAttempt"], 2);
    assert!(!attached.to_string().contains("SETUP_ROW_CANARY"));

    // Execution is also an honest, recoverable stage. This fixture has no
    // scientific child matching the injected attachment, so the real command
    // fails before native work. A typed outcome completes the recovered root,
    // and an exact CLI retry is then a read-only replay.
    let failed_execution = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "execute",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!failed_execution.status.success());
    let failed_execution_view = run(
        root,
        &[
            "optimization-run",
            "project",
            "show",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(failed_execution_view["state"], "execution_failed");
    assert_eq!(failed_execution_view["executionAttempt"], 1);
    assert_eq!(
        failed_execution_view["failureCode"],
        "candidate_execution_failed"
    );
    let executing = optimization_runs::begin_execution(
        &folder,
        started["run"]["run"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    let outcome = ProjectOptimizationOutcome::create(
        &executing.run,
        &typed_launch,
        executing.preparation.as_ref().unwrap(),
        executing.materialization.as_ref().unwrap(),
        executing.experiment.as_ref().unwrap(),
        BoundIdentity {
            id: attached_receipt.experiment_run.id,
            fingerprint: format!("sha256:{}", "e".repeat(64)),
        },
        ProjectOptimizationOutcomeKind::CandidateReady,
        Some(BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: format!("sha256:{}", "f".repeat(64)),
        }),
        Utc::now(),
    )
    .unwrap();
    let completed = optimization_runs::finish_execution(
        &folder,
        executing.run.id,
        &executing.head_fingerprint,
        outcome.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        completed.state,
        project_workspace_core::ProjectOptimizationRunState::ReadyForFinalEvaluation
    );
    let completed = run(
        root,
        &[
            "optimization-run",
            "project",
            "execute",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(completed["run"]["state"], "ready_for_final_evaluation");
    assert_eq!(completed["run"]["executionAttempt"], 2);
    assert_eq!(
        completed["run"]["outcome"],
        serde_json::to_value(outcome).unwrap()
    );
    assert!(!completed.to_string().contains("SETUP_ROW_CANARY"));

    // Candidate custody is a separate retryable operation. This injected
    // root has no corresponding scientific child or checkpoint, so it must
    // fail without inventing a Models entry or dataset link.
    let before_registration = open_workspace(&folder, false).await.unwrap();
    let failed_registration = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "register",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!failed_registration.status.success());
    let after_registration = open_workspace(&folder, false).await.unwrap();
    assert_eq!(
        before_registration.model_catalog,
        after_registration.model_catalog
    );
    assert_eq!(
        before_registration.model_dataset_links,
        after_registration.model_dataset_links
    );

    let failed_final = invoke(
        root,
        &[
            "optimization-run",
            "project",
            "finalize",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert!(!failed_final.status.success());
    let failed_final_view = run(
        root,
        &[
            "optimization-run",
            "project",
            "show",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(failed_final_view["state"], "final_evaluation_failed");
    assert_eq!(failed_final_view["finalAttempt"], 1);
    let evaluating = optimization_runs::begin_final_evaluation(
        &folder,
        started["run"]["run"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    let final_result = ProjectOptimizationFinalResult::create(
        &evaluating.run,
        &typed_launch,
        evaluating.preparation.as_ref().unwrap(),
        evaluating.materialization.as_ref().unwrap(),
        evaluating.experiment.as_ref().unwrap(),
        evaluating.outcome.as_ref().unwrap(),
        BoundIdentity {
            id: evaluating
                .experiment
                .as_ref()
                .unwrap()
                .experiment_run
                .id
                .clone(),
            fingerprint: format!("sha256:{}", "1".repeat(64)),
        },
        ProjectOptimizationFinalResultKind::CandidateRejected,
        BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: format!("sha256:{}", "2".repeat(64)),
        },
        Utc::now(),
    )
    .unwrap();
    optimization_runs::finish_final_evaluation(
        &folder,
        evaluating.run.id,
        &evaluating.head_fingerprint,
        final_result.clone(),
    )
    .await
    .unwrap();
    let finalized = run(
        root,
        &[
            "optimization-run",
            "project",
            "finalize",
            started["run"]["run"]["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(finalized["run"]["state"], "candidate_rejected");
    assert_eq!(finalized["run"]["finalAttempt"], 2);
    assert_eq!(
        finalized["run"]["finalResult"],
        serde_json::to_value(final_result).unwrap()
    );
    assert!(!finalized.to_string().contains("SETUP_ROW_CANARY"));
    let cancel_request = json!({"id":Uuid::new_v4(),"scope":launch["scope"]});
    fs::write(
        root.join("cancel-launch.json"),
        serde_json::to_vec(&cancel_request).unwrap(),
    )
    .unwrap();
    let cancel_authorized = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "cancel-launch.json",
        ],
    );
    let cancel_started = run(
        root,
        &[
            "optimization-run",
            "project",
            "start",
            "--file",
            "cancel-launch.json",
        ],
    );
    let cancel_id: Uuid = cancel_started["run"]["run"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let checked_files = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = checked_files.clone();
    let in_flight = project_workspace_local::progress::with_file_progress(
        std::sync::Arc::new(move |name, _, _| captured.lock().unwrap().push(name.to_owned())),
        optimization_runs::begin_preparation(&folder, cancel_id),
    )
    .await
    .unwrap();
    assert!(
        checked_files
            .lock()
            .unwrap()
            .iter()
            .all(|name| name == "encoder-gym.json"),
        "journal reservation must not rescan model and dataset contents; execution owns those checks"
    );
    let cancelled = run(
        root,
        &[
            "optimization-run",
            "project",
            "cancel",
            &cancel_id.to_string(),
        ],
    );
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["run"]["failureCode"], "user_requested");
    assert_eq!(cancelled["run"]["lastSequence"], 3);
    assert!(
        optimization_runs::fail_preparation(
            &folder,
            cancel_id,
            &in_flight.head_fingerprint,
            "input_verification_failed",
        )
        .await
        .is_err(),
        "a stale stage result cannot overwrite cancellation"
    );
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "cancel",
                &cancel_id.to_string(),
            ],
        )["run"],
        cancelled["run"],
        "cancellation is idempotent"
    );
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "show",
                &cancel_id.to_string(),
            ],
        ),
        cancelled["run"],
        "cancelled state survives a separate process read"
    );
    let orphan_request = json!({"id":Uuid::new_v4(),"scope":launch["scope"]});
    fs::write(
        root.join("orphan-launch.json"),
        serde_json::to_vec(&orphan_request).unwrap(),
    )
    .unwrap();
    let orphan = run(
        root,
        &[
            "optimization-launch",
            "project",
            "authorize",
            "--file",
            "orphan-launch.json",
        ],
    );

    // A provider revision made after preview invalidates a new authorization.
    let updated = configure_fake_providers(&folder, Some(&providers)).await;
    assert_ne!(updated.fingerprint, providers.fingerprint);
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "providers",
                started["run"]["run"]["id"].as_str().unwrap()
            ]
        ),
        serde_json::to_value(&providers).unwrap(),
        "execution resolves the original provider revision after defaults change"
    );
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "start",
                "--file",
                "launch.json",
            ],
        )["run"],
        finalized["run"],
        "an exact retry returns its already-reserved run after later settings change"
    );
    assert!(
        !invoke(
            root,
            &[
                "optimization-run",
                "project",
                "start",
                "--file",
                "orphan-launch.json",
            ],
        )
        .status
        .success(),
        "an old authorization cannot become a new run after provider settings change"
    );
    let mut stale = request;
    stale["id"] = Uuid::new_v4().to_string().into();
    fs::write(
        root.join("launch.json"),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    assert!(
        !invoke(
            root,
            &[
                "optimization-launch",
                "project",
                "authorize",
                "--file",
                "launch.json"
            ]
        )
        .status
        .success()
    );
    let history = run(root, &["optimization-launch", "project", "list"]);
    assert_eq!(
        history,
        json!([
            authorized["authorization"],
            cancel_authorized["authorization"],
            orphan["authorization"]
        ])
    );
    assert_eq!(
        run(root, &["optimization-run", "project", "list"]),
        json!([finalized["run"], cancelled["run"]])
    );
    let activity = run(root, &["activity", "project", "list"]);
    let text = activity.to_string();
    assert!(text.contains("optimization.launch"));
    assert!(text.contains("optimization.start"));
    assert!(text.contains("optimization.prepare"));
    assert!(text.contains("optimization.materialize"));
    assert!(text.contains("optimization.attach_experiment"));
    assert!(text.contains("optimization.execute"));
    assert!(text.contains("optimization.register_candidate"));
    assert!(text.contains("optimization.final_evaluation"));
    assert!(text.contains("optimization.cancel"));
    assert!(text.contains(started["run"]["run"]["id"].as_str().unwrap()));
    assert!(!text.contains("SETUP_ROW_CANARY"));
    assert!(!text.contains("apiKey"));
    assert!(!text.contains("fixture-generation"));

    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE optimization_launch_authorizations SET scope_fingerprint='changed'")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_launch_authorizations")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE project_optimization_runs SET launch_fingerprint='changed'")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM project_optimization_events")
            .execute(&mut database)
            .await
            .is_err()
    );
    database.close().await.unwrap();
}

#[tokio::test]
async fn selections_replay_without_reactivation_and_survive_folder_movement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, model, data, benchmark) = prepared(root).await;
    // Reproduce a version-8 project: read-only preview must not upgrade it.
    let mut old_db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("DROP TABLE optimization_setups")
        .execute(&mut old_db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=9")
        .execute(&mut old_db)
        .await
        .unwrap();
    old_db.close().await.unwrap();
    let custody = run(root, &["open", "project"]);
    let scientific = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    let manifest = fs::read(folder.join("encoder-gym.json")).unwrap();
    let before = fs::read(folder.join("project.sqlite")).unwrap();
    let choice = preview(root, &model, &data, &benchmark);
    assert_eq!(before, fs::read(folder.join("project.sqlite")).unwrap());
    assert_eq!(choice["datasetRows"], 1);
    assert!(choice["expectedParent"].is_null());
    assert!(!choice.to_string().contains("SETUP_ROW_CANARY"));
    assert!(!choice.to_string().contains("99999.125"));
    let original = request(root, &choice, Uuid::new_v4());
    let first = save(root);
    assert_eq!(first["setup"]["inputs"], choice["inputs"]);
    assert_eq!(save(root)["setup"], first["setup"]);
    let variant = dataset_versions::fork(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Alternate data",
        data.parse().unwrap(),
    )
    .await
    .unwrap();
    let choice = preview(root, &model, &variant.id.to_string(), &benchmark);
    assert_eq!(choice["expectedParent"], first["setup"]["id"]);
    request(root, &choice, Uuid::new_v4());
    let second = save(root);
    assert_eq!(second["setup"]["number"], 2);
    fs::write(
        root.join("setup.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    assert_eq!(save(root)["setup"], first["setup"]);
    let history = run(root, &["optimization-setup", "project", "list"]);
    assert_eq!(history, json!([first["setup"], second["setup"]]));
    let actions = run(root, &["activity", "project", "list"]);
    assert!(!actions.to_string().contains("SETUP_ROW_CANARY"));
    assert!(!actions.to_string().contains("99999.125"));
    assert!(Uuid::parse_str(first["actionId"].as_str().unwrap()).is_ok());
    assert_eq!(
        scientific,
        fs::read(folder.join("runs/scientific.sqlite")).unwrap()
    );
    assert_eq!(manifest, fs::read(folder.join("encoder-gym.json")).unwrap());
    let after = run(root, &["open", "project"]);
    for field in [
        "modelCatalog",
        "datasets",
        "benchmarkVersions",
        "modelDatasetLinks",
    ] {
        assert_eq!(
            after[field], custody[field],
            "setup must not change {field}"
        );
    }
    let moved = root.join("moved");
    fs::rename(&folder, &moved).unwrap();
    assert_eq!(run(root, &["optimization-setup", "moved", "list"]), history);
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        moved.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE optimization_setups SET number=99")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_setups")
            .execute(&mut db)
            .await
            .is_err()
    );
    sqlx::query("DROP TRIGGER immutable_optimization_setups_update")
        .execute(&mut db)
        .await
        .unwrap();
    sqlx::query("UPDATE optimization_setups SET metadata_json=json_set(metadata_json, '$.inputs.model.fingerprint', ?) WHERE number=2")
        .bind(format!("sha256:{}", "e".repeat(64))).execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    assert!(
        !invoke(root, &["optimization-setup", "moved", "list"])
            .status
            .success()
    );
}

#[tokio::test]
async fn setup_rejects_foreign_stale_substituted_and_changed_source_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, model, data, benchmark) = prepared(root).await;
    let choice = preview(root, &model, &data, &benchmark);
    let valid = request(root, &choice, Uuid::new_v4());
    let workspace = open_workspace(&folder, false).await.unwrap();
    let catalog = workspace.model_catalog.as_ref().unwrap();
    let original_data = dataset_versions::inspect(&folder, data.parse().unwrap())
        .await
        .unwrap();
    let branch = dataset_versions::list(&folder)
        .await
        .unwrap()
        .remove(0)
        .dataset;
    let mut test_row = original_data.members[0].clone();
    test_row.split = dataset_core::domain::SnapshotSplit::Test;
    let test_data = dataset_core::versions::DatasetVersion::initial(
        Uuid::new_v4(),
        &branch,
        vec![test_row],
        Utc::now(),
    )
    .unwrap();
    assert!(
        project_workspace_core::OptimizationInputs::bind(
            workspace.manifest.id,
            model.parse().unwrap(),
            catalog,
            &test_data,
            &workspace.benchmark_versions[0]
        )
        .is_err()
    );
    let mut empty_data = original_data.clone();
    empty_data.members.clear();
    assert!(
        project_workspace_core::OptimizationInputs::bind(
            workspace.manifest.id,
            model.parse().unwrap(),
            catalog,
            &empty_data,
            &workspace.benchmark_versions[0]
        )
        .is_err()
    );
    let candidate = catalog
        .artifacts
        .iter()
        .find(|m| m.id.to_string() != model)
        .unwrap()
        .id
        .to_string();
    assert!(
        !invoke(
            root,
            &[
                "optimization-setup",
                "project",
                "preview",
                "--model",
                &candidate,
                "--dataset-version",
                &data,
                "--benchmark-version",
                &benchmark
            ]
        )
        .status
        .success()
    );
    for field in [
        "projectId",
        "model",
        "baselineRevision",
        "benchmark",
        "dataset",
    ] {
        let mut bad = valid.clone();
        match field {
            "projectId" => bad["inputs"][field] = Uuid::new_v4().to_string().into(),
            "dataset" => bad["inputs"][field]["id"] = Uuid::new_v4().to_string().into(),
            _ => bad["inputs"][field]["fingerprint"] = format!("sha256:{}", "d".repeat(64)).into(),
        }
        fs::write(root.join("setup.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(
            !invoke(
                root,
                &[
                    "optimization-setup",
                    "project",
                    "save",
                    "--file",
                    "setup.json"
                ]
            )
            .status
            .success(),
            "{field}"
        );
    }
    fs::write(root.join("setup.json"), serde_json::to_vec(&valid).unwrap()).unwrap();
    let source = folder.join(&workspace.datasets[0].artifact.path);
    let original = fs::read_to_string(&source).unwrap();
    fs::write(&source, original.replace("base", "nope")).unwrap();
    assert!(
        !invoke(
            root,
            &[
                "optimization-setup",
                "project",
                "save",
                "--file",
                "setup.json"
            ]
        )
        .status
        .success()
    );
    fs::write(&source, original).unwrap();
    let first = save(root);
    let variant = dataset_versions::fork(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Variant",
        data.parse().unwrap(),
    )
    .await
    .unwrap();
    let next = preview(root, &model, &variant.id.to_string(), &benchmark);
    let mut stale = request(root, &next, Uuid::new_v4());
    stale["expectedParent"] = Value::Null;
    fs::write(root.join("setup.json"), serde_json::to_vec(&stale).unwrap()).unwrap();
    assert!(
        !invoke(
            root,
            &[
                "optimization-setup",
                "project",
                "save",
                "--file",
                "setup.json"
            ]
        )
        .status
        .success()
    );
    request(root, &next, Uuid::new_v4());
    // A new baseline revision invalidates a preview even when model bytes match.
    let old = catalog.active_revision();
    let restored = BaselineRevision::restoration(
        Uuid::new_v4(),
        workspace.manifest.id,
        2,
        old.model_artifact_id,
        old.id,
        old.id,
        "fixture",
        "Restore original baseline",
        Utc::now(),
    )
    .unwrap();
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("INSERT INTO baseline_revisions VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(restored.id.to_string())
        .bind(restored.project_id.to_string())
        .bind(2_i64)
        .bind(restored.model_artifact_id.to_string())
        .bind(old.id.to_string())
        .bind("restoration")
        .bind(&restored.fingerprint)
        .bind(serde_json::to_string(&restored).unwrap())
        .execute(&mut db)
        .await
        .unwrap();
    sqlx::query("UPDATE model_catalog_state SET active_baseline_revision_id=? WHERE singleton=1")
        .bind(restored.id.to_string())
        .execute(&mut db)
        .await
        .unwrap();
    db.close().await.unwrap();
    assert!(
        !invoke(
            root,
            &[
                "optimization-setup",
                "project",
                "save",
                "--file",
                "setup.json"
            ]
        )
        .status
        .success()
    );
    assert_eq!(
        run(root, &["optimization-setup", "project", "list"]),
        json!([first["setup"]])
    );
}
