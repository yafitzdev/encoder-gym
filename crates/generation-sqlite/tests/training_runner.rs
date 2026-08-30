use std::{
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};

use dataset_core::{
    domain::{SnapshotSplit, SplitConfiguration, SplitRatios},
    ports::{AcceptedRowSource, SnapshotStore},
    splitting::build_snapshot,
};
use generation_core::{
    domain::{DatasetDefinition, GenerationParameters},
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy},
    planning::equal_target_plan,
    ports::{DatasetStore, PlanStore},
    validation::ValidationPipeline,
};
use generation_test_support::{FakeGenerationBackend, persist_test_generation_execution};
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{
        BatchMetrics, TrainingConfiguration, TrainingExample, TrainingRequest, TrainingRun,
        TrainingRunState,
    },
    ports::{
        CheckpointSink, PredictorLoader, TrainingBackend, TrainingBackendError, TrainingSession,
        TrainingStore,
    },
    runner::TrainingRunner,
};
use training_linear::{HashingLinearBackend, HashingLinearPredictorLoader, LocalCheckpointStore};

struct BlockingBackend {
    started: mpsc::Sender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl TrainingBackend for BlockingBackend {
    fn name(&self) -> &str {
        "blocking-test"
    }

    fn model_format(&self) -> &str {
        "blocking-test-v1"
    }

    fn start(
        &self,
        _request: TrainingRequest,
    ) -> Result<Box<dyn TrainingSession>, TrainingBackendError> {
        Ok(Box::new(BlockingSession {
            started: Some(self.started.clone()),
            release: self.release.clone(),
            step: 0,
        }))
    }
}

struct BlockingSession {
    started: Option<mpsc::Sender<()>>,
    release: Arc<(Mutex<bool>, Condvar)>,
    step: u32,
}

impl TrainingSession for BlockingSession {
    fn train_batch(&mut self) -> Result<BatchMetrics, TrainingBackendError> {
        self.step += 1;
        if self.step == 2 {
            if let Some(started) = self.started.take() {
                started
                    .send(())
                    .map_err(|error| TrainingBackendError::Training(error.to_string()))?;
                let (lock, condition) = &*self.release;
                let mut released = lock
                    .lock()
                    .map_err(|error| TrainingBackendError::Training(error.to_string()))?;
                while !*released {
                    released = condition
                        .wait(released)
                        .map_err(|error| TrainingBackendError::Training(error.to_string()))?;
                }
            }
        }
        let (epoch, batch, batches_in_epoch, epoch_complete) = match self.step {
            1 => (1, 1, 1, true),
            2 => (2, 1, 2, false),
            _ => (2, 2, 2, true),
        };
        Ok(BatchMetrics {
            epoch,
            batch,
            batches_in_epoch,
            processed_examples: u64::from(self.step),
            training_loss: 1.0 / f64::from(self.step),
            validation_loss: None,
            learning_rate: 0.1,
            elapsed_milliseconds: 0,
            epoch_complete,
        })
    }

    fn serialize_checkpoint(&self) -> Result<Vec<u8>, TrainingBackendError> {
        Ok(format!("{{\"step\":{}}}", self.step).into_bytes())
    }
}

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("training.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

#[tokio::test]
async fn generated_rows_train_into_durable_loadable_checkpoints() {
    let (directory, store) = store().await;
    let dataset = DatasetDefinition::new(
        "support",
        "classify support messages",
        vec!["billing".into(), "account".into()],
        vec![],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("persist dataset");
    let plan = equal_target_plan(&dataset, 10).expect("plan");
    store.create_plan(&plan).await.expect("persist plan");
    let generation_job = GenerationJob::queued(
        dataset.id,
        plan.id,
        "fake",
        "deterministic-v1",
        plan.total_target_count(),
    );
    let generation_policy = JobRunnerPolicy {
        batch_size: 5,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &generation_job, &plan, &generation_policy)
        .await
        .expect("execution");
    let generation_runner = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        generation_policy,
        ValidationPipeline::standard(None),
    );
    generation_runner
        .run(generation_job.id, GenerationParameters::default())
        .await
        .expect("generate");

    let source_rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("source rows");
    let (snapshot, members) = build_snapshot(
        dataset.id,
        "baseline",
        None,
        SplitConfiguration::new(SplitRatios::new(0.8, 0.1, 0.1).expect("ratios"), 7),
        source_rows,
    )
    .expect("snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("persist snapshot");

    let configuration = TrainingConfiguration {
        feature_dimension: 128,
        epochs: 5,
        learning_rate: 0.2,
        l2: 0.0,
        checkpoint_every: 2,
        seed: 11,
    };
    let backend = HashingLinearBackend;
    let run = TrainingRun::queued(
        snapshot.id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
    )
    .expect("training run");
    store
        .create_training_run(&run)
        .await
        .expect("persist training run");
    let to_example = |member: &dataset_core::domain::SnapshotMember| TrainingExample {
        snapshot_member_id: member.id,
        text: member.text.clone(),
        label: member.label.clone(),
    };
    let request = TrainingRequest::in_memory(
        run.id,
        snapshot.id,
        dataset.labels.clone(),
        members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Train)
            .map(to_example)
            .collect(),
        members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Validation)
            .map(to_example)
            .collect(),
        configuration,
    );
    let artifacts = Arc::new(LocalCheckpointStore::new(
        directory.path().join("artifacts"),
    ));
    let runner = TrainingRunner::new(
        Arc::new(store.clone()),
        Arc::new(HashingLinearBackend),
        artifacts.clone(),
    );
    let completed = runner.run(run.id, request).await.expect("train");

    assert_eq!(completed.state, TrainingRunState::Completed);
    assert_eq!(completed.completed_epochs, 5);
    let checkpoints = store.list_checkpoints(run.id).await.expect("checkpoints");
    assert_eq!(
        checkpoints
            .iter()
            .map(|checkpoint| checkpoint.epoch)
            .collect::<Vec<_>>(),
        vec![2, 4, 5]
    );
    assert!(checkpoints.last().expect("final checkpoint").is_final);
    let final_checkpoint = checkpoints.last().expect("final checkpoint");
    let bytes = artifacts
        .read(&final_checkpoint.artifact_path)
        .expect("artifact reads");
    let predictor = HashingLinearPredictorLoader
        .load(&bytes)
        .expect("artifact loads");
    assert!(
        dataset
            .labels
            .contains(&predictor.predict("billing payment").expect("predict").label)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_is_observed_between_batches_and_keeps_completed_checkpoints() {
    let (directory, store) = store().await;
    let (dataset, snapshot, members) = generated_snapshot(&store).await;
    let configuration = TrainingConfiguration {
        feature_dimension: 64,
        epochs: 2,
        learning_rate: 0.1,
        l2: 0.0,
        checkpoint_every: 1,
        seed: 1,
    };
    let (started_sender, started_receiver) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let backend = Arc::new(BlockingBackend {
        started: started_sender,
        release: release.clone(),
    });
    let run = TrainingRun::queued(
        snapshot.id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
    )
    .expect("run");
    store.create_training_run(&run).await.expect("persist run");
    let request = TrainingRequest::in_memory(
        run.id,
        snapshot.id,
        dataset.labels,
        members
            .iter()
            .map(|member| TrainingExample {
                snapshot_member_id: member.id,
                text: member.text.clone(),
                label: member.label.clone(),
            })
            .collect(),
        vec![],
        configuration,
    );
    let runner = TrainingRunner::new(
        Arc::new(store.clone()),
        backend,
        Arc::new(LocalCheckpointStore::new(
            directory.path().join("cancel-artifacts"),
        )),
    );
    let run_id = run.id;
    let task = tokio::spawn(async move { runner.run(run_id, request).await });
    tokio::task::spawn_blocking(move || started_receiver.recv())
        .await
        .expect("wait task joins")
        .expect("first epoch started");
    assert!(
        store
            .request_training_cancellation(run_id)
            .await
            .expect("request cancellation")
    );
    {
        let (lock, condition) = &*release;
        *lock.lock().expect("release lock") = true;
        condition.notify_one();
    }
    let cancelled = task
        .await
        .expect("runner task joins")
        .expect("runner returns");

    assert_eq!(cancelled.state, TrainingRunState::Cancelled);
    assert_eq!(cancelled.completed_epochs, 1);
    assert_eq!(cancelled.current_epoch, 2);
    assert_eq!(cancelled.completed_batches, 1);
    assert!(cancelled.cancel_requested);
    assert_eq!(
        store
            .list_checkpoints(run_id)
            .await
            .expect("checkpoints")
            .len(),
        1
    );
}

async fn generated_snapshot(
    store: &SqliteStore,
) -> (
    DatasetDefinition,
    dataset_core::domain::DatasetSnapshot,
    Vec<dataset_core::domain::SnapshotMember>,
) {
    let dataset = DatasetDefinition::new(
        "support",
        "classify",
        vec!["billing".into(), "account".into()],
        vec![],
    )
    .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");
    let plan = equal_target_plan(&dataset, 2).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let job = GenerationJob::queued(
        dataset.id,
        plan.id,
        "fake",
        "deterministic-v1",
        plan.total_target_count(),
    );
    let generation_policy = JobRunnerPolicy {
        batch_size: 2,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(store, &job, &plan, &generation_policy)
        .await
        .expect("execution");
    JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        generation_policy,
        ValidationPipeline::standard(None),
    )
    .run(job.id, GenerationParameters::default())
    .await
    .expect("generate");
    let rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("source rows");
    let (snapshot, members) = build_snapshot(
        dataset.id,
        "snapshot",
        None,
        SplitConfiguration::new(SplitRatios::new(1.0, 0.0, 0.0).expect("ratios"), 1),
        rows,
    )
    .expect("snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("persist snapshot");
    (dataset, snapshot, members)
}
