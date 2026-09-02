use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use chrono::Utc;
use encoder_experiment_core::{
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

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

#[derive(Clone)]
struct FakeRankingBackend {
    identity: BackendIdentity,
    selected_candidate_sealed_calls: Arc<AtomicUsize>,
}

impl FakeRankingBackend {
    fn new() -> Self {
        Self {
            identity: BackendIdentity::new("fake-ranking", "v1", digest('b')).unwrap(),
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
        let sealed_calls = self.selected_candidate_sealed_calls.clone();
        Box::pin(async move {
            let role = if suite_key == "development" {
                EvidenceRole::Development
            } else {
                EvidenceRole::SealedAcceptance
            };
            let (mrr, recall) = match (model.key.as_str(), role) {
                ("baseline", EvidenceRole::Development) => (0.80, 0.90),
                ("baseline", EvidenceRole::SealedAcceptance) => (0.79, 0.89),
                ("candidate-1", EvidenceRole::Development) => (0.83, 0.90),
                ("candidate-2", EvidenceRole::Development) => (0.81, 0.90),
                ("candidate-1", EvidenceRole::SealedAcceptance) => {
                    sealed_calls.fetch_add(1, Ordering::SeqCst);
                    (0.81, 0.90)
                }
                _ => return Err(EncoderTaskAdapterError("unexpected fake evaluation".into())),
            };
            EvaluationReport::create(
                &project,
                model,
                role,
                suite_key,
                if role == EvidenceRole::Development {
                    digest('1')
                } else {
                    digest('2')
                },
                &contract,
                BTreeMap::from([("mrr".into(), mrr), ("recall_at_3".into(), recall)]),
                100,
                Utc::now(),
            )
            .map_err(|error| EncoderTaskAdapterError(error.to_string()))
        })
    }
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
