//! Production iteration handoff and Agent journal, using only synthetic local
//! records. Reserving this fixture root is not a production executor bypass.
use super::*;
use encoder_optimization_core::{agent::AgentCallReservation, ports::OptimizationAgentStore};
use project_workspace_core::{
    DatasetPurpose, OptimizationAgentSettings, ProjectOptimizationEvent,
    ProjectOptimizationPreparation, ProjectOptimizationRun, ProviderAuthentication,
    ProviderCatalog, ProviderConfiguration, ProviderKind, ProviderLimits, ProviderRole,
    optimization_iteration::{IterationDevelopmentEvidence, ProjectOptimizationIteration},
};
use project_workspace_local::{
    dataset_versions, import_dataset, inspect_dataset, optimization_agent::ProjectAgentStore,
    optimization_iterations, optimization_launch, optimization_runs, optimization_setup,
    record_provider_catalog,
};
use sqlx::{Connection, SqliteConnection};

fn identity(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}

#[tokio::test]
async fn iteration_binding_is_replayable_exact_and_required_before_agent_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (folder, original, source_run, _) = fixture(root).await;
    let version: project_workspace_core::ProjectBenchmarkVersion = serde_json::from_value(
        run(
            root,
            &["benchmark", "project", "adopt-run", &source_run.to_string()],
        )["version"]
            .clone(),
    )
    .unwrap();
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
            maximum_requests: 8,
            maximum_input_tokens: 1000,
            maximum_output_tokens: 1000,
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
    record_provider_catalog(&folder, providers.clone(), None)
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
    let scope = optimization_launch::preview_agentic(
        &folder,
        setup.id,
        OptimizationAgentSettings::quick_test(),
    )
    .await
    .unwrap()
    .scope;
    let request = optimization_launch::LaunchRequest {
        id: Uuid::new_v4(),
        scope,
    };
    // The production fixed executor must still reject settings it cannot honor.
    assert!(
        optimization_runs::start(&folder, request.clone(), "fixture")
            .await
            .is_err()
    );
    let launch = optimization_launch::authorize(&folder, request, "fixture")
        .await
        .unwrap();
    let reserved = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, Utc::now()).unwrap();
    let event =
        ProjectOptimizationEvent::reserved(Uuid::new_v4(), &reserved, reserved.created_at).unwrap();
    let url = format!("sqlite://{}", folder.join("project.sqlite").display());
    let mut database = SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("INSERT INTO project_optimization_runs(id,project_id,launch_id,launch_fingerprint,setup_id,fingerprint,metadata_json,created_at) VALUES (?,?,?,?,?,?,?,?)")
        .bind(reserved.id.to_string()).bind(reserved.project_id.to_string()).bind(&reserved.launch.id)
        .bind(&reserved.launch.fingerprint).bind(&reserved.setup.id).bind(&reserved.fingerprint)
        .bind(serde_json::to_string(&reserved).unwrap()).bind(reserved.created_at.to_rfc3339())
        .execute(&mut database).await.unwrap();
    sqlx::query("INSERT INTO project_optimization_events(id,run_id,sequence,previous_event_fingerprint,kind,fingerprint,metadata_json,created_at) VALUES (?,?,1,NULL,'reserved',?,?,?)")
        .bind(event.id.to_string()).bind(reserved.id.to_string()).bind(&event.fingerprint)
        .bind(serde_json::to_string(&event).unwrap()).bind(event.created_at.to_rfc3339())
        .execute(&mut database).await.unwrap();
    database.close().await.unwrap();
    let run_id = reserved.id.to_string();
    assert_eq!(
        run(
            root,
            &["optimization-run", "project", "iterations", &run_id]
        ),
        json!([])
    );
    assert!(
        ProjectAgentStore::open(&folder, reserved.id, Uuid::new_v4())
            .await
            .is_err()
    );

    let binding = workspace.scientific_binding.as_ref().unwrap();
    let preparing = optimization_runs::begin_preparation(&folder, reserved.id)
        .await
        .unwrap();
    let preparation = ProjectOptimizationPreparation::create(
        &reserved,
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
        reserved.id,
        &preparing.head_fingerprint,
        preparation.clone(),
    )
    .await
    .unwrap();
    let scientific_path = folder.join("runs/scientific.sqlite");
    let before = fs::read(&scientific_path).unwrap();
    let first = run(
        root,
        &["optimization-run", "project", "bind-iteration", &run_id],
    );
    let iteration: ProjectOptimizationIteration =
        serde_json::from_value(first["iteration"].clone()).unwrap();
    let replayed = run(
        root,
        &["optimization-run", "project", "bind-iteration", &run_id],
    );
    assert_eq!(first["iteration"], replayed["iteration"]);
    assert_eq!(fs::read(&scientific_path).unwrap(), before);
    assert_eq!(iteration.dataset, dataset.reference());
    assert_eq!(iteration.scope.maximum_row_changes, 8);
    assert_eq!(iteration.scope.maximum_turns, 4);
    assert_eq!(iteration.provider_catalog.id, providers.id.to_string());
    assert_eq!(
        iteration.comparison_baseline_revision,
        setup.inputs.baseline_revision
    );
    assert_eq!(iteration.development.reports.len(), 1);
    assert!(!first.to_string().contains("99999.125"));
    assert!(!first.to_string().contains("holdout"));
    assert!(!first.to_string().contains("training example"));

    // The scientific source is checked by domain constructors as well as the CLI.
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        scientific_path.display()
    ))
    .await
    .unwrap();
    let protocol = store
        .get_protocol(version.source.protocol.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    store.pool().close().await;
    let mut wrong_definition = version.definition.clone();
    wrong_definition.evaluation_configuration_fingerprint = fp('a');
    wrong_definition.fingerprint = wrong_definition.reproduce_fingerprint().unwrap();
    let alternate =
        IterationDevelopmentEvidence::baseline(&original, &protocol, &wrong_definition).unwrap();
    let changed_benchmark = ProjectOptimizationIteration::first(
        &reserved,
        &launch,
        &setup,
        &preparation,
        alternate,
        Utc::now(),
    )
    .unwrap();
    assert!(changed_benchmark.validate_benchmark(&version).is_err());
    let mut missing_suite = version.definition.clone();
    let mut extra = missing_suite
        .suites
        .iter()
        .find(|s| s.role == EvidenceRole::Development)
        .unwrap()
        .clone();
    extra.key = "another-development-suite".into();
    missing_suite.suites.push(extra);
    missing_suite.suites.sort_by(|a, b| a.key.cmp(&b.key));
    missing_suite.fingerprint = missing_suite.reproduce_fingerprint().unwrap();
    assert!(IterationDevelopmentEvidence::baseline(&original, &protocol, &missing_suite).is_err());
    for field in [
        "dataset",
        "baseline",
        "provider",
        "iteration",
        "evidence",
        "turns",
    ] {
        let mut changed = iteration.clone();
        match field {
            "dataset" => changed.dataset.id = Uuid::new_v4(),
            "baseline" => changed.comparison_baseline_revision.id = Uuid::new_v4().to_string(),
            "provider" => changed.provider_catalog.id = Uuid::new_v4().to_string(),
            "iteration" => changed.scope.iteration = 2,
            "evidence" => changed.scope.development_evidence_fingerprint = fp('f'),
            "turns" => changed.scope.maximum_turns += 1,
            _ => unreachable!(),
        }
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(
            changed
                .validate_first(&reserved, &launch, &setup, &preparation)
                .is_err(),
            "{field}"
        );
    }
    let agent = ProjectAgentStore::open(&folder, reserved.id, Uuid::new_v4())
        .await
        .unwrap();
    assert!(
        agent
            .history(iteration.scope.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let mut substituted = iteration.scope.clone();
    substituted.development_evidence_fingerprint = fp('f');
    assert!(agent.history(substituted).await.is_err());
    let call = AgentCallReservation {
        id: Uuid::new_v4(),
        scope_fingerprint: iteration.scope.fingerprint().unwrap(),
        sequence: 1,
        request_fingerprint: fp('b'),
        input_token_ceiling: 100,
        output_token_ceiling: 100,
        cost_ceiling_microusd: 0,
    };
    agent.reserve(iteration.scope.clone(), call).await.unwrap();
    // Reopening does not lose scope custody or mistake an unfinished call for no work.
    let reopened = ProjectAgentStore::open(&folder, reserved.id, Uuid::new_v4())
        .await
        .unwrap();
    assert!(
        reopened
            .recover_interrupted(&iteration.scope)
            .await
            .is_err()
    );
    assert_eq!(
        run(
            root,
            &["optimization-run", "project", "iterations", &run_id]
        ),
        json!([iteration])
    );

    let mut database = SqliteConnection::connect(&url).await.unwrap();
    for sql in [
        "UPDATE project_optimization_iterations SET fingerprint='changed'",
        "DELETE FROM project_optimization_iterations",
    ] {
        assert!(sqlx::query(sql).execute(&mut database).await.is_err());
    }
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut database)
        .await
        .unwrap();
    assert!(violations.is_empty());
    sqlx::query("DROP TRIGGER immutable_project_optimization_iterations_update")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE project_optimization_iterations SET scope_fingerprint=?")
        .bind(fp('f'))
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();
    assert!(
        optimization_iterations::list(&folder, reserved.id)
            .await
            .is_err()
    );
    assert!(
        ProjectAgentStore::open(&folder, reserved.id, Uuid::new_v4())
            .await
            .is_err()
    );
}
