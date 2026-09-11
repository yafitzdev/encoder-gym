//! Identity-only setup through actual CLI processes; no native work or providers.
#[path = "fixtures/benchmark_support.rs"]
mod benchmark_support;
use benchmark_support::{complete_rejected_candidate, fixture, register_fixture_model};
use chrono::Utc;
use encoder_experiment_sqlite::SqliteExperimentStore;
use project_workspace_core::{
    BaselineRevision, ProviderAuthentication, ProviderCatalog, ProviderConfiguration, ProviderKind,
    ProviderLimits, ProviderRole,
};
use project_workspace_local::{
    dataset_versions, import_dataset, inspect_dataset, open_workspace, record_provider_catalog,
};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use uuid::Uuid;

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
                "start",
                "--file",
                "launch.json",
            ],
        )["run"],
        started["run"],
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
        json!([authorized["authorization"], orphan["authorization"]])
    );
    assert_eq!(
        run(root, &["optimization-run", "project", "list"]),
        json!([started["run"]])
    );
    let activity = run(root, &["activity", "project", "list"]);
    let text = activity.to_string();
    assert!(text.contains("optimization.launch"));
    assert!(text.contains("optimization.start"));
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
