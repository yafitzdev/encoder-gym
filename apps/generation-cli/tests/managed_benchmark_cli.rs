//! Actual CLI processes against synthetic scientific records; no native execution.
use chrono::Utc;
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    journal::first_event,
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    ports::ExperimentStore,
    protocol::ExperimentProtocol,
};
use encoder_experiment_sqlite::{SCHEMA_ID, SqliteExperimentStore, schema_fingerprint};
use project_workspace_core::{
    AdapterBinding, BoundIdentity, RuntimeBinding, RuntimeKind, ScientificBinding,
    ScientificStoreBinding,
};
use project_workspace_local::{create_workspace, inspect_model, record_scientific_binding};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
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
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
async fn fixture(root: &Path) -> (std::path::PathBuf, ExternalProjectSnapshot, Uuid, Uuid) {
    let model = root.join("checkpoint");
    training_transformer::fixture::write_tiny_bert_bundle(&model).unwrap();
    let preview = inspect_model(&model).unwrap();
    let folder = root.join("project");
    let workspace = create_workspace(
        &folder,
        "Offline benchmark fixture",
        &model,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let config = json!({"adapter_protocol":"nomos-ranking-v3","source_reference":{},"baseline_evidence":{},"training_inputs":["train.jsonl"],
        "reference_models":{"reference":{"path":"reference","format":"sentence-transformers","bytes":1,"fingerprint":fp('1'),"provenance":{}}},
        "agent_evaluation":{"backend":"onnx","chat_model":{"path":"missing-chat","format":"onnxruntime-genai","bytes":1,"fingerprint":fp('2'),"source":{}},"selector_strategy":"multiview","candidate_strategy":"multiview","nomos_top_k":2,"max_attempts":1,
            "development":{"suite":"development","sessions":5,"pairing":"cycle","condition":"nomos"},"sealed":{"suite":"promotion","sessions":5,"pairing":"cycle","condition":"nomos"}},
        "suites":{"development":{"path":"development.jsonl","role":"development","fingerprint":fp('3'),"retrieval_fingerprint":fp('4'),"agent_fingerprint":fp('5')},
            "holdout":{"path":"holdout.jsonl","role":"sealed_acceptance","fingerprint":fp('6'),"retrieval_fingerprint":fp('7'),"agent_fingerprint":fp('8')}}});
    let project = ExternalProjectSnapshot::create(
        "Offline benchmark fixture",
        EncoderTaskKind::RetrievalRanking,
        "fixture-revision",
        fp('a'),
        BackendIdentity::new("nomos", "nomos-ranking-v3", fp('b')).unwrap(),
        [
            ("train.jsonl", EvidenceRole::Training),
            ("development.jsonl", EvidenceRole::Development),
            ("holdout.jsonl", EvidenceRole::SealedAcceptance),
        ]
        .into_iter()
        .map(|(key, role)| ExternalArtifactIdentity::new(key, role, 1, fp('c')).unwrap())
        .collect(),
        ModelArtifactIdentity::new(
            "baseline",
            "sentence-transformers",
            preview.bytes,
            preview.fingerprint,
        )
        .unwrap(),
        config,
        Utc::now(),
    )
    .unwrap();
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    store.create_project(project.clone()).await.unwrap();
    let first = create_run(&store, &project, 0.0, 1).await;
    let changed = create_run(&store, &project, 0.1, 2).await;
    store.pool().close().await;
    let binding = ScientificBinding::new(
        Uuid::new_v4(),
        workspace.manifest.id,
        workspace.model_catalog.unwrap().active_baseline_revision_id,
        None,
        AdapterBinding {
            key: "nomos".into(),
            protocol: "nomos-ranking-v3".into(),
            configuration_fingerprint: fp('b'),
        },
        RuntimeBinding {
            kind: RuntimeKind::Managed,
            location: "runs/missing-runtime".into(),
            executable: None,
            project_snapshot: BoundIdentity {
                id: project.id.to_string(),
                fingerprint: project.fingerprint.clone(),
            },
        },
        ScientificStoreBinding {
            database_path: "runs/scientific.sqlite".into(),
            schema: BoundIdentity {
                id: SCHEMA_ID.into(),
                fingerprint: schema_fingerprint(),
            },
            snapshot_fingerprint: None,
            snapshot_bytes: None,
        },
        "fixture",
        "Offline recorded benchmark",
        Utc::now(),
    )
    .unwrap();
    record_scientific_binding(&folder, binding, None)
        .await
        .unwrap();
    (folder, project, first, changed)
}
async fn create_run(
    store: &SqliteExperimentStore,
    project: &ExternalProjectSnapshot,
    tolerance: f64,
    seed: i64,
) -> Uuid {
    let protocol = protocol_for(project, tolerance, seed);
    store.create_protocol(protocol.clone()).await.unwrap();
    let id = Uuid::new_v4();
    store
        .create_run(first_event(&protocol, id, Utc::now()).unwrap())
        .await
        .unwrap();
    id
}

fn protocol_for(
    project: &ExternalProjectSnapshot,
    tolerance: f64,
    seed: i64,
) -> ExperimentProtocol {
    let contract = MetricContract::create(
        vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
        "mrr",
        [EvidenceRole::Development, EvidenceRole::SealedAcceptance]
            .into_iter()
            .map(|role| {
                MetricGate::new(
                    "mrr",
                    role,
                    MetricGateCondition::MaximumRegression { value: tolerance },
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let report = |role, key, digest, score| {
        EvaluationReport::create(
            project,
            project.baseline_model.clone(),
            role,
            key,
            fp(digest),
            &contract,
            BTreeMap::from([("mrr".into(), score)]),
            5,
            Utc::now(),
        )
        .unwrap()
    };
    ExperimentProtocol::create(
        project,
        contract.clone(),
        report(EvidenceRole::Development, "development", '3', 0.7),
        report(EvidenceRole::SealedAcceptance, "holdout", '6', 99999.125),
        OptimizationBudget {
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
        },
        60,
        "development",
        "holdout",
        vec![
            TrainingCandidate::create(
                project,
                1,
                60,
                BTreeMap::from([("seed".into(), ParameterValue::Integer(seed))]),
            )
            .unwrap(),
        ],
        Utc::now(),
    )
    .unwrap()
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
    let adopted = run(
        root,
        &["benchmark", "project", "adopt-run", &first.to_string()],
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
