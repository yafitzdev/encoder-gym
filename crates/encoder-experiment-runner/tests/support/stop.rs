use super::*;
use std::sync::atomic::AtomicBool;

struct Interruptible {
    inner: FakeRankingBackend,
    mode: AtomicUsize,
    stopped: AtomicBool,
    trains: AtomicUsize,
}

impl EncoderTaskBackend for Interruptible {
    fn identity(&self) -> BackendIdentity {
        self.inner.identity()
    }
    fn stop_requested(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>> {
        self.inner.inspect(project)
    }
    fn train(
        &self,
        project: ExternalProjectSnapshot,
        candidate: TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainOutput, EncoderTaskAdapterError>> {
        Box::pin(async move {
            self.trains.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                self.stopped.store(true, Ordering::SeqCst);
                return Err(EncoderTaskAdapterError::Failure(
                    "training interrupted".into(),
                ));
            }
            let result = self.inner.train(project, candidate).await;
            if self.mode.load(Ordering::SeqCst) == 2 {
                self.stopped.store(true, Ordering::SeqCst);
            }
            result
        })
    }
    fn evaluate(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite: String,
        seconds: u64,
    ) -> BoxFuture<'_, Result<EvaluationReport, EncoderTaskAdapterError>> {
        Box::pin(async move {
            if self.mode.load(Ordering::SeqCst) == 3 && model.key.starts_with("candidate") {
                self.stopped.store(true, Ordering::SeqCst);
                return Err(EncoderTaskAdapterError::Failure(
                    "evaluation interrupted".into(),
                ));
            }
            self.inner
                .evaluate(project, model, contract, suite, seconds)
                .await
        })
    }
}

#[tokio::test]
async fn stop_preserves_scientific_work_instead_of_failing_the_candidate() {
    for mode in [1, 2, 3] {
        let store = SqliteExperimentStore::connect("sqlite::memory:")
            .await
            .unwrap();
        let backend = Interruptible {
            inner: FakeRankingBackend::new(),
            mode: AtomicUsize::new(mode),
            stopped: AtomicBool::new(false),
            trains: AtomicUsize::new(0),
        };
        let runner = ExperimentRunner::new(&store, &backend);
        let project = runner
            .register_project(project(&backend.inner))
            .await
            .unwrap();
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
                vec![candidate.clone()],
            )
            .await
            .unwrap();
        let run = runner.create_run(protocol.id).await.unwrap();
        assert!(matches!(
            runner.run_development(run.run_id).await,
            Err(encoder_experiment_runner::ExperimentRunnerError::Stopped)
        ));
        let paused = runner.status(run.run_id).await.unwrap();
        use encoder_experiment_core::journal::{CandidateExecutionState, ExperimentEventKind};
        assert_eq!(
            paused.candidates[&candidate.id].state,
            if mode == 1 {
                CandidateExecutionState::Training
            } else {
                CandidateExecutionState::Trained
            }
        );
        let events = store.load_events(run.run_id).await.unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, ExperimentEventKind::CandidateFailed { .. }))
        );
        backend.mode.store(0, Ordering::SeqCst);
        backend.stopped.store(false, Ordering::SeqCst);
        let resumed = runner.run_development(run.run_id).await.unwrap();
        assert_eq!(
            resumed.candidates[&candidate.id].state,
            CandidateExecutionState::DevelopmentCompleted
        );
        assert_eq!(
            backend.trains.load(Ordering::SeqCst),
            if mode == 1 { 2 } else { 1 }
        );
        let final_events = store.load_events(run.run_id).await.unwrap();
        assert_eq!(&final_events[..events.len()], &events);
        assert_eq!(
            backend
                .inner
                .selected_candidate_sealed_calls
                .load(Ordering::SeqCst),
            0
        );
    }
}
