use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use chrono::Utc;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition,
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    journal::{ExperimentRunState, FinalDecision},
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    ports::{
        AdapterInspection, BoxFuture, EncoderTaskAdapterError, EncoderTaskBackend, TrainOutput,
    },
};
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde_json::json;

async fn development_selected_run(
    store: &SqliteExperimentStore,
    backend: &FakeRankingBackend,
) -> uuid::Uuid {
    let runner = ExperimentRunner::new(store, backend);
    let project = runner.register_project(project(backend)).await.unwrap();
    let candidate = TrainingCandidate::create(
        &project,
        1,
        60,
        BTreeMap::from([("loss".into(), ParameterValue::Text("triplet".into()))]),
    )
    .unwrap();
    let protocol = runner
        .prepare_protocol(
            project.id,
            contract(),
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
        )
        .await
        .unwrap();
    let run = runner.create_run(protocol.id).await.unwrap();
    let selected = runner.run_development(run.run_id).await.unwrap();
    assert_eq!(
        selected.state,
        ExperimentRunState::AwaitingSealedAuthorization
    );
    run.run_id
}

#[tokio::test]
async fn sealed_authorization_retries_preserve_the_original_actor_and_journal() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let run_id = development_selected_run(&store, &backend).await;
    let runner = ExperimentRunner::new(&store, &backend);
    let authorized = runner
        .authorize_sealed(run_id, "original-operator")
        .await
        .unwrap();
    let repeated = runner
        .authorize_sealed(run_id, "original-operator")
        .await
        .unwrap();
    assert_eq!(authorized, repeated);
    assert!(
        runner
            .authorize_sealed(run_id, "different-operator")
            .await
            .is_err()
    );
    assert_eq!(runner.status(run_id).await.unwrap(), authorized);
    assert_eq!(
        backend
            .selected_candidate_sealed_calls
            .load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn sealed_finalization_recovers_without_evaluating_again() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let run_id = development_selected_run(&store, &backend).await;
    let runner = ExperimentRunner::new(&store, &backend);
    runner.authorize_sealed(run_id, "operator").await.unwrap();
    sqlx::query(
        "CREATE TRIGGER interrupt_finalization BEFORE INSERT ON encoder_experiment_events \
         WHEN json_extract(NEW.artifact_json, '$.event.kind') = 'finalized' \
         BEGIN SELECT RAISE(ABORT, 'injected interruption after sealed evidence'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();
    assert!(runner.run_sealed(run_id).await.is_err());
    let interrupted = runner.status(run_id).await.unwrap();
    assert_eq!(interrupted.state, ExperimentRunState::SealedEvaluated);
    assert!(interrupted.sealed_report.is_some());
    assert_eq!(
        backend
            .selected_candidate_sealed_calls
            .load(Ordering::SeqCst),
        1
    );
    sqlx::query("DROP TRIGGER interrupt_finalization")
        .execute(store.pool())
        .await
        .unwrap();
    let resumed = ExperimentRunner::new(&store, &backend)
        .run_sealed(run_id)
        .await
        .unwrap();
    assert_eq!(resumed.state, ExperimentRunState::Completed);
    assert_eq!(
        resumed.final_decision,
        Some(FinalDecision::PromoteCandidate)
    );
    assert_eq!(resumed.sealed_report, interrupted.sealed_report);
    assert_eq!(runner.run_sealed(run_id).await.unwrap(), resumed);
    assert_eq!(
        backend
            .selected_candidate_sealed_calls
            .load(Ordering::SeqCst),
        1
    );
}

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

#[derive(Clone)]
struct FakeRankingBackend {
    identity: BackendIdentity,
    evaluation_calls: Arc<AtomicUsize>,
    selected_candidate_sealed_calls: Arc<AtomicUsize>,
}

impl FakeRankingBackend {
    fn new() -> Self {
        Self {
            identity: BackendIdentity::new("fake-ranking", "v1", digest('b')).unwrap(),
            evaluation_calls: Arc::new(AtomicUsize::new(0)),
            selected_candidate_sealed_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl EncoderTaskBackend for FakeRankingBackend {
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }

    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>> {
        Box::pin(async move {
            project
                .validate_integrity()
                .map_err(|error| EncoderTaskAdapterError(error.to_string()))?;
            Ok(AdapterInspection {
                source_fingerprint: project.source_fingerprint,
                verified_artifact_keys: project.inputs.into_iter().map(|input| input.key).collect(),
                metadata: json!({"fake":true}),
            })
        })
    }

    fn train(
        &self,
        _project: ExternalProjectSnapshot,
        candidate: TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainOutput, EncoderTaskAdapterError>> {
        Box::pin(async move {
            let character = if candidate.sequence == 1 { '7' } else { '8' };
            Ok(TrainOutput {
                model: ModelArtifactIdentity::new(
                    format!("candidate-{}", candidate.sequence),
                    "fake-ranking",
                    10,
                    digest(character),
                )
                .unwrap(),
                duration_seconds: 1,
                metadata: json!({"sequence":candidate.sequence}),
            })
        })
    }

    fn evaluate(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
        _maximum_seconds: u64,
    ) -> BoxFuture<'_, Result<EvaluationReport, EncoderTaskAdapterError>> {
        let evaluation_calls = self.evaluation_calls.clone();
        let sealed_calls = self.selected_candidate_sealed_calls.clone();
        Box::pin(async move {
            evaluation_calls.fetch_add(1, Ordering::SeqCst);
            let role = if suite_key.starts_with("development") {
                EvidenceRole::Development
            } else {
                EvidenceRole::SealedAcceptance
            };
            let (mrr, recall) = match (model.key.as_str(), suite_key.as_str(), role) {
                ("baseline", _, EvidenceRole::Development) => (0.80, 0.90),
                ("baseline", _, EvidenceRole::SealedAcceptance) => (0.79, 0.89),
                ("candidate-1", "development_b", EvidenceRole::Development) => (0.79, 0.90),
                ("candidate-1", _, EvidenceRole::Development) => (0.83, 0.90),
                ("candidate-2", "development_b", EvidenceRole::Development) => (0.82, 0.90),
                ("candidate-2", _, EvidenceRole::Development) => (0.81, 0.90),
                ("candidate-1", _, EvidenceRole::SealedAcceptance) => {
                    sealed_calls.fetch_add(1, Ordering::SeqCst);
                    (0.81, 0.90)
                }
                _ => return Err(EncoderTaskAdapterError("unexpected fake evaluation".into())),
            };
            let suite_fingerprint = match suite_key.as_str() {
                "development_b" => digest('3'),
                _ if role == EvidenceRole::Development => digest('1'),
                _ => digest('2'),
            };
            EvaluationReport::create(
                &project,
                model,
                role,
                suite_key,
                suite_fingerprint,
                &contract,
                BTreeMap::from([("mrr".into(), mrr), ("recall_at_3".into(), recall)]),
                100,
                Utc::now(),
            )
            .map_err(|error| EncoderTaskAdapterError(error.to_string()))
        })
    }
}

#[tokio::test]
async fn multi_suite_runner_rejects_a_candidate_that_fails_any_suite() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let runner = ExperimentRunner::new(&store, &backend);
    let project = runner.register_project(project(&backend)).await.unwrap();
    let candidates = (1..=2)
        .map(|sequence| {
            TrainingCandidate::create(
                &project,
                sequence,
                60,
                BTreeMap::from([(
                    "learning_rate".into(),
                    ParameterValue::Number(0.000_003 * f64::from(sequence)),
                )]),
            )
            .unwrap()
        })
        .collect();
    let protocol = runner
        .prepare_multi_protocol(
            project.id,
            contract(),
            OptimizationBudget {
                maximum_candidates: 2,
                maximum_training_seconds: 120,
                maximum_development_evaluations: 4,
                maximum_sealed_evaluations: 1,
            },
            60,
            vec!["development_b".into(), "development_a".into()],
            "sealed",
            candidates,
        )
        .await
        .unwrap();
    assert_eq!(
        protocol.development_suite_keys(),
        vec!["development_a", "development_b"]
    );
    let run = runner.create_run(protocol.id).await.unwrap();
    let development = runner.run_development(run.run_id).await.unwrap();
    assert_eq!(
        development.state,
        ExperimentRunState::AwaitingSealedAuthorization
    );
    assert_eq!(
        development.selected_candidate_id,
        Some(protocol.candidates[1].id)
    );
    assert_eq!(
        development.candidates[&protocol.candidates[0].id]
            .development_assessments
            .len(),
        2
    );
}

fn project(backend: &FakeRankingBackend) -> ExternalProjectSnapshot {
    ExternalProjectSnapshot::create(
        "production pilot",
        EncoderTaskKind::RetrievalRanking,
        "revision",
        digest('a'),
        backend.identity(),
        vec![
            ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('c')).unwrap(),
            ExternalArtifactIdentity::new("development", EvidenceRole::Development, 1, digest('d'))
                .unwrap(),
            ExternalArtifactIdentity::new("sealed", EvidenceRole::SealedAcceptance, 1, digest('e'))
                .unwrap(),
        ],
        ModelArtifactIdentity::new("baseline", "fake-ranking", 1, digest('f')).unwrap(),
        json!({"suites":["development","sealed"]}),
        Utc::now(),
    )
    .unwrap()
}

fn contract() -> MetricContract {
    MetricContract::create(
        vec![
            MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap(),
            MetricDefinition::new("recall_at_3", MetricDirection::HigherIsBetter).unwrap(),
        ],
        "mrr",
        vec![
            MetricGate::new(
                "mrr",
                EvidenceRole::Development,
                MetricGateCondition::MinimumImprovement { value: 0.001 },
            )
            .unwrap(),
            MetricGate::new(
                "recall_at_3",
                EvidenceRole::Development,
                MetricGateCondition::MaximumRegression { value: 0.0 },
            )
            .unwrap(),
            MetricGate::new(
                "mrr",
                EvidenceRole::SealedAcceptance,
                MetricGateCondition::MinimumImprovement { value: 0.001 },
            )
            .unwrap(),
            MetricGate::new(
                "recall_at_3",
                EvidenceRole::SealedAcceptance,
                MetricGateCondition::MaximumRegression { value: 0.0 },
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test]
async fn durable_runner_selects_on_development_and_uses_one_candidate_sealed_report() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let runner = ExperimentRunner::new(&store, &backend);
    let project = runner.register_project(project(&backend)).await.unwrap();
    let candidates = (1..=2)
        .map(|sequence| {
            TrainingCandidate::create(
                &project,
                sequence,
                60,
                BTreeMap::from([
                    ("loss".into(), ParameterValue::Text("triplet".into())),
                    (
                        "learning_rate".into(),
                        ParameterValue::Number(0.000_003 * f64::from(sequence)),
                    ),
                ]),
            )
            .unwrap()
        })
        .collect();
    let protocol = runner
        .prepare_protocol(
            project.id,
            contract(),
            OptimizationBudget {
                maximum_candidates: 2,
                maximum_training_seconds: 120,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            "development",
            "sealed",
            candidates,
        )
        .await
        .unwrap();
    let run = runner.create_run(protocol.id).await.unwrap();
    let development = runner.run_development(run.run_id).await.unwrap();

    assert_eq!(
        development.state,
        ExperimentRunState::AwaitingSealedAuthorization
    );
    assert_eq!(
        development.selected_candidate_id,
        Some(protocol.candidates[0].id)
    );
    assert_eq!(
        backend
            .selected_candidate_sealed_calls
            .load(Ordering::SeqCst),
        0
    );

    runner
        .authorize_sealed(run.run_id, "local-operator")
        .await
        .unwrap();
    let completed = runner.run_sealed(run.run_id).await.unwrap();
    assert_eq!(completed.state, ExperimentRunState::Completed);
    assert_eq!(
        completed.final_decision,
        Some(FinalDecision::PromoteCandidate)
    );
    assert_eq!(
        backend
            .selected_candidate_sealed_calls
            .load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn reserved_protocol_and_run_identities_are_recovered_idempotently() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let runner = ExperimentRunner::new(&store, &backend);
    let project = runner.register_project(project(&backend)).await.unwrap();
    let candidate = TrainingCandidate::create(
        &project,
        1,
        60,
        BTreeMap::from([("learning_rate".into(), ParameterValue::Number(0.000_003))]),
    )
    .unwrap();
    let protocol_id = uuid::Uuid::new_v4();
    let run_id = uuid::Uuid::new_v4();
    let request = || {
        (
            protocol_id,
            project.id,
            contract(),
            OptimizationBudget {
                maximum_candidates: 1,
                maximum_training_seconds: 60,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            vec!["development_a".into(), "development_b".into()],
            "sealed",
            vec![candidate.clone()],
        )
    };
    let first_request = request();
    let protocol = runner
        .prepare_multi_protocol_identified(
            first_request.0,
            first_request.1,
            first_request.2,
            first_request.3,
            first_request.4,
            first_request.5,
            first_request.6,
            first_request.7,
        )
        .await
        .unwrap();
    let second_request = request();
    let recovered = runner
        .prepare_multi_protocol_identified(
            second_request.0,
            second_request.1,
            second_request.2,
            second_request.3,
            second_request.4,
            second_request.5,
            second_request.6,
            second_request.7,
        )
        .await
        .unwrap();
    assert_eq!(protocol, recovered);

    let first = runner
        .create_run_identified(protocol_id, run_id)
        .await
        .unwrap();
    let recovered = runner
        .create_run_identified(protocol_id, run_id)
        .await
        .unwrap();
    assert_eq!(first, recovered);
    assert_eq!(first.run_id, run_id);
}

#[tokio::test]
async fn shared_benchmark_protocol_reuses_baseline_reports_without_evaluation() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let backend = FakeRankingBackend::new();
    let runner = ExperimentRunner::new(&store, &backend);
    let source = runner.register_project(project(&backend)).await.unwrap();
    let source_candidate = TrainingCandidate::create(
        &source,
        1,
        60,
        BTreeMap::from([("learning_rate".into(), ParameterValue::Number(0.000_003))]),
    )
    .unwrap();
    let source_protocol = runner
        .prepare_multi_protocol(
            source.id,
            contract(),
            OptimizationBudget {
                maximum_candidates: 1,
                maximum_training_seconds: 60,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            vec!["development_b".into(), "development_a".into()],
            "sealed",
            vec![source_candidate],
        )
        .await
        .unwrap();
    let benchmark =
        BenchmarkDefinition::from_protocol(&source, &source_protocol, digest('9')).unwrap();
    let target = ExternalProjectSnapshot::create(
        "production pilot with selected training data",
        source.task,
        source.source_revision.clone(),
        digest('8'),
        source.backend.clone(),
        source
            .inputs
            .iter()
            .cloned()
            .map(|mut input| {
                if input.role == EvidenceRole::Training {
                    input.fingerprint = digest('7');
                }
                input
            })
            .collect(),
        ModelArtifactIdentity::new(
            "materialized/baseline",
            source.baseline_model.format.clone(),
            source.baseline_model.bytes,
            source.baseline_model.fingerprint.clone(),
        )
        .unwrap(),
        json!({"suites":["development_a","development_b","sealed"],"training":"selected"}),
        Utc::now(),
    )
    .unwrap();
    let target = runner.register_project(target).await.unwrap();
    let candidate = TrainingCandidate::create(
        &target,
        1,
        60,
        BTreeMap::from([("learning_rate".into(), ParameterValue::Number(0.000_003))]),
    )
    .unwrap();
    let protocol_id = uuid::Uuid::new_v4();
    let evaluations_before = backend.evaluation_calls.load(Ordering::SeqCst);
    let prepared = runner
        .prepare_multi_protocol_from_benchmark_identified(
            protocol_id,
            target.id,
            source_protocol.id,
            benchmark.clone(),
            OptimizationBudget {
                maximum_candidates: 1,
                maximum_training_seconds: 60,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            vec![candidate],
        )
        .await
        .unwrap();
    assert_eq!(
        backend.evaluation_calls.load(Ordering::SeqCst),
        evaluations_before
    );
    assert!(
        prepared
            .baseline_development_reports()
            .iter()
            .all(|report| report.reference.is_some() && report.model == target.baseline_model)
    );
    assert!(
        prepared.baseline_sealed_report.reference.is_some()
            && prepared.baseline_sealed_report.model == target.baseline_model
    );

    let recovered = runner
        .prepare_multi_protocol_from_benchmark_identified(
            protocol_id,
            target.id,
            source_protocol.id,
            benchmark,
            prepared.budget.clone(),
            60,
            prepared.candidates.clone(),
        )
        .await
        .unwrap();
    assert_eq!(prepared, recovered);
    assert_eq!(
        backend.evaluation_calls.load(Ordering::SeqCst),
        evaluations_before
    );

    let run = runner.create_run(prepared.id).await.unwrap();
    let completed = runner.run_development(run.run_id).await.unwrap();
    assert_eq!(completed.state, ExperimentRunState::Completed);
    assert_eq!(
        completed.final_decision,
        Some(FinalDecision::RetainBaseline)
    );
    assert_eq!(
        backend.evaluation_calls.load(Ordering::SeqCst),
        evaluations_before + 2
    );
}
