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
    ports::{EncoderTaskBackend, ExperimentStore},
    protocol::ExperimentProtocol,
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_sqlite::{SCHEMA_ID, SqliteExperimentStore, schema_fingerprint};
use project_workspace_core::{
    AdapterBinding, BoundIdentity, RuntimeBinding, RuntimeKind, ScientificBinding,
    ScientificStoreBinding,
};
use project_workspace_local::{create_workspace, inspect_model, record_scientific_binding};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
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

/// A tiny, fully valid Nomos runtime for exercising the production benchmark
/// initialization path. The supplied executable impersonates only the two
/// native evaluation modules and never trains or contacts a provider.
#[allow(dead_code)]
pub async fn initial_benchmark_fixture(
    root: &Path,
    executable: &Path,
) -> (PathBuf, ExternalProjectSnapshot) {
    let runtime = root.join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(
        runtime.join("ENCODER_GYM_EXPERIMENT.md"),
        "# Offline benchmark initialization fixture\n",
    )
    .unwrap();
    fs::write(
        runtime.join(".gitignore"),
        "artifacts/\nruns/\nnative-invocations.log\n",
    )
    .unwrap();
    for relative in [
        "tools/collect_encoder_gym_development_observations.py",
        "tools/evaluate_dense_router.py",
        "fitz_tool/dense_router.py",
        "fitz_tool/embedding_backend.py",
        "fitz_tool/onnx_encoder.py",
        "tools/generate_encoder_gym_repair_delta_v1.py",
        "fitz_tool/encoder_gym_repair_delta_v1.py",
        "fitz_tool/generic_contracts.py",
        "fitz_tool/router_v2.py",
        "fitz_tool/scaling_matrix_v1.py",
    ] {
        let path = runtime.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "# deterministic offline fixture\n").unwrap();
    }

    let baseline = runtime.join("baseline");
    training_transformer::fixture::write_tiny_bert_bundle(&baseline).unwrap();
    for (directory, file) in [
        ("onnx", "encoder.onnx"),
        ("reference", "model.bin"),
        ("chat", "model.onnx"),
    ] {
        let path = runtime.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(file), format!("offline {directory} fixture\n")).unwrap();
    }
    for (path, content) in [
        ("train.jsonl", "{\"text\":\"training\"}\n"),
        ("development.jsonl", "{\"query\":\"development\"}\n"),
        ("regression.jsonl", "{\"query\":\"regression\"}\n"),
        ("holdout.jsonl", "{\"query\":\"NEVER_DISCLOSE_HOLDOUT\"}\n"),
    ] {
        fs::write(runtime.join(path), content).unwrap();
    }

    let (baseline_bytes, baseline_hash) = tree_pin(&baseline);
    let (_, onnx_hash) = tree_pin(&runtime.join("onnx"));
    let (reference_bytes, reference_hash) = tree_pin(&runtime.join("reference"));
    let (chat_bytes, chat_hash) = tree_pin(&runtime.join("chat"));
    let dataset = |path: &str, role: &str| {
        let (bytes, sha256) = file_pin(&runtime.join(path));
        json!({"path":path,"role":role,"bytes":bytes,"sha256":sha256})
    };
    let manifest = json!({
        "schema_version":4,
        "experiment":"Offline initial benchmark fixture",
        "tree_hash_algorithm":"sha256-ordinal-path-size-content-sha256-v1",
        "source":{"repository":"offline-fixture","access":"read_only_reference","commit":"fixture-source","snapshot_date":"2026-09-11"},
        "isolation":{"git_remote_allowed":false,"hard_links_allowed":false,"symlinks_allowed":false,"outputs_must_remain_below_experiment_root":true},
        "baseline":{
            "pytorch_path":"baseline","pytorch_tree_sha256":baseline_hash,
            "onnx_path":"onnx","onnx_tree_sha256":onnx_hash,
            "weak_agent_raw_completed":{"completed":1,"total":1},
            "weak_agent_complete_coprocessor_completed":{"completed":1,"total":1}
        },
        "reference_models":[{"key":"reference","path":"reference","format":"sentence-transformers","bytes":reference_bytes,"tree_sha256":reference_hash,"provenance":{}}],
        "agent_evaluation":{
            "backend":"onnx","chat_model_path":"chat","chat_model_format":"onnxruntime-genai","chat_model_bytes":chat_bytes,"chat_model_tree_sha256":chat_hash,"source":{},
            "selector_strategy":"multiview","candidate_strategy":"multiview","nomos_top_k":2,"max_attempts":1,
            "development":{"suite":"development","sessions":5,"pairing":"cycle","condition":"nomos"},
            "sealed":{"suite":"promotion","sessions":7,"pairing":"cycle","condition":"nomos"}
        },
        "datasets":[
            dataset("train.jsonl", "training"),
            dataset("development.jsonl", "development_holdout"),
            dataset("regression.jsonl", "development_holdout"),
            dataset("holdout.jsonl", "sealed_holdout")
        ],
        "evaluation_suites":[
            {"key":"development","dataset_path":"development.jsonl","role":"development"},
            {"key":"regression","dataset_path":"regression.jsonl","role":"development"},
            {"key":"holdout","dataset_path":"holdout.jsonl","role":"sealed_acceptance"}
        ],
        "evaluation_runs_tree_sha256":"8".repeat(64)
    });
    fs::write(
        runtime.join("encoder-gym-experiment.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    git(&runtime, &["init", "--quiet"]);
    git(&runtime, &["config", "user.name", "Encoder Gym Fixture"]);
    git(
        &runtime,
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(&runtime, &["add", "."]);
    git(&runtime, &["commit", "--quiet", "-m", "fixture"]);

    let backend = NomosBackend::open(&runtime, executable.to_path_buf()).unwrap();
    let project = backend.project_snapshot().unwrap();
    let preview = inspect_model(&baseline).unwrap();
    assert_eq!(preview.bytes, baseline_bytes);
    let folder = root.join("project");
    let workspace = create_workspace(
        &folder,
        "Initial benchmark fixture",
        &baseline,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await
    .unwrap();
    store.create_project(project.clone()).await.unwrap();
    store.pool().close().await;
    let identity = backend.identity();
    let binding = ScientificBinding::new(
        Uuid::new_v4(),
        workspace.manifest.id,
        workspace.model_catalog.unwrap().active_baseline_revision_id,
        None,
        AdapterBinding {
            key: identity.name,
            protocol: identity.protocol_version,
            configuration_fingerprint: identity.configuration_fingerprint,
        },
        RuntimeBinding {
            kind: RuntimeKind::ExternalIsolated,
            location: runtime.to_string_lossy().into_owned(),
            executable: Some(executable.to_string_lossy().into_owned()),
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
        project.source_revision.clone(),
        "Offline initial benchmark runtime",
        Utc::now(),
    )
    .unwrap();
    record_scientific_binding(&folder, binding, None)
        .await
        .unwrap();
    (folder, project)
}

#[allow(dead_code)]
fn file_pin(path: &Path) -> (u64, String) {
    let bytes = fs::read(path).unwrap();
    (bytes.len() as u64, format!("{:x}", Sha256::digest(bytes)))
}

#[allow(dead_code)]
fn tree_pin(path: &Path) -> (u64, String) {
    fn collect(path: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(&entry.path(), files);
            } else {
                files.push(entry.path());
            }
        }
    }
    let mut files = Vec::new();
    collect(path, &mut files);
    files.sort_by_cached_key(|file| {
        file.strip_prefix(path)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    });
    let mut total = 0_u64;
    let entries = files
        .into_iter()
        .map(|file| {
            let relative = file
                .strip_prefix(path)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let (bytes, sha256) = file_pin(&file);
            total += bytes;
            format!("{relative}\t{bytes}\t{sha256}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    (total, format!("{:x}", Sha256::digest(entries.as_bytes())))
}

#[allow(dead_code)]
fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
