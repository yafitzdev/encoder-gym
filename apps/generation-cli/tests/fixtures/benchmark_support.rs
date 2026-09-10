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
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

fn fp(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
pub async fn fixture(root: &Path) -> (std::path::PathBuf, ExternalProjectSnapshot, Uuid, Uuid) {
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
            preview.format,
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
pub async fn create_run(
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

pub fn protocol_for(
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

pub async fn complete_rejected_candidate(
    store: &SqliteExperimentStore,
    project: &ExternalProjectSnapshot,
    run_id: Uuid,
) -> (ModelArtifactIdentity, String, String) {
    use encoder_experiment_core::{
        journal::{ExperimentEventKind as Kind, FinalDecision, replay_experiment},
        metrics::assess_candidate,
        ports::TrainOutput,
    };
    let events = store.load_events(run_id).await.unwrap();
    let protocol = store
        .get_protocol(events[0].protocol_id)
        .await
        .unwrap()
        .unwrap();
    let candidate_id = protocol.candidates[0].id;
    let model = ModelArtifactIdentity::new(
        "candidate",
        project.baseline_model.format.clone(),
        project.baseline_model.bytes,
        fp('f'),
    )
    .unwrap();
    let report = EvaluationReport::create(
        project,
        model.clone(),
        EvidenceRole::Development,
        "development",
        fp('3'),
        &protocol.metric_contract,
        BTreeMap::from([("mrr".into(), 0.6)]),
        5,
        Utc::now(),
    )
    .unwrap();
    let assessment = assess_candidate(
        project,
        &protocol.metric_contract,
        &protocol.baseline_development_report,
        &report,
        Utc::now(),
    )
    .unwrap();
    let mut receipt = String::new();
    for event in [
        Kind::CandidateTrainingStarted { candidate_id },
        Kind::CandidateTrainingCompleted {
            candidate_id,
            output: TrainOutput {
                model: model.clone(),
                duration_seconds: 1,
                metadata: json!({"secret_native_payload":"NEVER_PROJECT_NATIVE_METADATA"}),
            },
        },
        Kind::CandidateDevelopmentCompleted {
            candidate_id,
            report,
            assessment,
        },
        Kind::DevelopmentSelected { candidate_id: None },
        Kind::Finalized {
            decision: FinalDecision::RetainBaseline,
        },
    ] {
        let current = store.load_events(run_id).await.unwrap();
        let view = replay_experiment(project, &protocol, &current).unwrap();
        let event = view.next_event(&protocol, event, Utc::now()).unwrap();
        if matches!(event.event, Kind::CandidateTrainingCompleted { .. }) {
            receipt = event.fingerprint.clone();
        }
        store.append_event(event).await.unwrap();
    }
    (model, receipt, protocol.candidates[0].fingerprint.clone())
}

pub async fn register_fixture_model(
    folder: &Path,
    source: &Path,
    model: &ModelArtifactIdentity,
    run_id: Uuid,
    receipt: &str,
    configuration: &str,
    name: &str,
) -> project_workspace_local::ManagedWorkspace {
    use project_workspace_local::{
        CompletedModelRegistration, open_workspace, register_completed_model,
    };
    let workspace = open_workspace(folder, false).await.unwrap();
    register_completed_model(
        folder,
        source,
        CompletedModelRegistration {
            parent_model_id: workspace.model_catalog.as_ref().unwrap().active_model().id,
            name: name.into(),
            source_model: BoundIdentity {
                id: model.id.to_string(),
                fingerprint: model.fingerprint.clone(),
            },
            source_model_format: model.format.clone(),
            source_model_bytes: model.bytes,
            producing_run: BoundIdentity {
                id: run_id.to_string(),
                fingerprint: receipt.into(),
            },
            training_snapshot: BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: fp('a'),
            },
            trainer: BoundIdentity {
                id: "nomos:nomos-ranking-v3".into(),
                fingerprint: fp('b'),
            },
            effective_configuration_fingerprint: configuration.into(),
            source_revision: "fixture-revision".into(),
        },
    )
    .await
    .unwrap()
}
