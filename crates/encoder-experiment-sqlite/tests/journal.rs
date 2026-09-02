use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    journal::{ExperimentEventKind, first_event, replay_experiment},
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    ports::ExperimentStore,
    protocol::ExperimentProtocol,
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde_json::json;
use uuid::Uuid;

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn time(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
}

fn fixture() -> (ExternalProjectSnapshot, ExperimentProtocol) {
    let project = ExternalProjectSnapshot::create(
        "nomos",
        EncoderTaskKind::RetrievalRanking,
        "experiment-revision",
        digest('a'),
        BackendIdentity::new("nomos", "nomos-ranking-v1", digest('b')).unwrap(),
        vec![
            ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('c')).unwrap(),
            ExternalArtifactIdentity::new("development", EvidenceRole::Development, 1, digest('d'))
                .unwrap(),
            ExternalArtifactIdentity::new("sealed", EvidenceRole::SealedAcceptance, 1, digest('e'))
                .unwrap(),
        ],
        ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, digest('f')).unwrap(),
        json!({"adapter_protocol":"nomos-ranking-v1"}),
        time(0),
    )
    .unwrap();
    let contract = MetricContract::create(
        vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
        "mrr",
        vec![
            MetricGate::new(
                "mrr",
                EvidenceRole::Development,
                MetricGateCondition::MinimumImprovement { value: 0.001 },
            )
            .unwrap(),
            MetricGate::new(
                "mrr",
                EvidenceRole::SealedAcceptance,
                MetricGateCondition::MinimumImprovement { value: 0.001 },
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let report = |role, value, second| {
        EvaluationReport::create(
            &project,
            project.baseline_model.clone(),
            role,
            if role == EvidenceRole::Development {
                "development"
            } else {
                "sealed"
            },
            if role == EvidenceRole::Development {
                digest('1')
            } else {
                digest('2')
            },
            &contract,
            BTreeMap::from([("mrr".into(), value)]),
            100,
            time(second),
        )
        .unwrap()
    };
    let candidate = TrainingCandidate::create(
        &project,
        1,
        60,
        BTreeMap::from([("loss".into(), ParameterValue::Text("triplet".into()))]),
    )
    .unwrap();
    let baseline_development = report(EvidenceRole::Development, 0.8, 1);
    let baseline_sealed = report(EvidenceRole::SealedAcceptance, 0.79, 2);
    let protocol = ExperimentProtocol::create(
        &project,
        contract,
        baseline_development,
        baseline_sealed,
        OptimizationBudget {
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
        },
        60,
        "development",
        "sealed",
        vec![candidate],
        time(3),
    )
    .unwrap();
    (project, protocol)
}

#[tokio::test]
async fn immutable_artifacts_and_compare_and_append_journal_round_trip() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let (project, protocol) = fixture();
    store.create_project(project.clone()).await.unwrap();
    store.create_protocol(protocol.clone()).await.unwrap();

    let run_id = Uuid::new_v4();
    let first = first_event(&protocol, run_id, time(4)).unwrap();
    store.create_run(first).await.unwrap();
    let events = store.load_events(run_id).await.unwrap();
    let view = replay_experiment(&project, &protocol, &events).unwrap();
    let event = view
        .next_event(
            &protocol,
            ExperimentEventKind::CandidateTrainingStarted {
                candidate_id: protocol.candidates[0].id,
            },
            time(5),
        )
        .unwrap();
    let conflicting = view
        .next_event(
            &protocol,
            ExperimentEventKind::CandidateTrainingStarted {
                candidate_id: protocol.candidates[0].id,
            },
            time(5),
        )
        .unwrap();

    store.append_event(event).await.unwrap();
    assert!(store.append_event(conflicting).await.is_err());
    let reloaded_project = store.get_project(project.id).await.unwrap().unwrap();
    let reloaded_protocol = store.get_protocol(protocol.id).await.unwrap().unwrap();
    let reloaded_events = store.load_events(run_id).await.unwrap();
    let reloaded =
        replay_experiment(&reloaded_project, &reloaded_protocol, &reloaded_events).unwrap();
    assert_eq!(reloaded.last_sequence, 2);
    assert_eq!(
        store
            .find_project_by_fingerprint(project.fingerprint)
            .await
            .unwrap()
            .unwrap()
            .id,
        project.id
    );
}

#[tokio::test]
async fn persisted_json_tampering_fails_closed() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let (project, _) = fixture();
    store.create_project(project.clone()).await.unwrap();
    let mut value = serde_json::to_value(&project).unwrap();
    value["source_revision"] = json!("tampered");
    sqlx::query("UPDATE encoder_experiment_projects SET artifact_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&value).unwrap())
        .bind(project.id)
        .execute(store.pool())
        .await
        .unwrap();

    assert!(store.get_project(project.id).await.is_err());
}
