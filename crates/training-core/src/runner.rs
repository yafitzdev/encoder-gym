use std::sync::Arc;

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{TrainingCheckpoint, TrainingRequest, TrainingRun, TrainingRunState},
    ports::{CheckpointSink, TrainingBackend, TrainingStore, TrainingStoreError},
};

pub struct TrainingRunner {
    store: Arc<dyn TrainingStore>,
    backend: Arc<dyn TrainingBackend>,
    artifacts: Arc<dyn CheckpointSink>,
}

impl TrainingRunner {
    pub fn new(
        store: Arc<dyn TrainingStore>,
        backend: Arc<dyn TrainingBackend>,
        artifacts: Arc<dyn CheckpointSink>,
    ) -> Self {
        Self {
            store,
            backend,
            artifacts,
        }
    }

    pub async fn run(
        &self,
        run_id: Uuid,
        request: TrainingRequest,
    ) -> Result<TrainingRun, TrainingRunnerError> {
        let request = request.seal_for_backend()?;
        if request.run_id != run_id {
            return Err(TrainingRunnerError::RequestRunMismatch);
        }
        let mut run = self
            .store
            .get_training_run(run_id)
            .await?
            .ok_or(TrainingRunnerError::RunNotFound(run_id))?;
        if run.state != TrainingRunState::Queued {
            return Err(TrainingRunnerError::RunNotQueued(run.state));
        }
        if run.snapshot_id != request.snapshot_id
            || run.configuration != request.configuration
            || run.input_binding != request.input_binding
        {
            return Err(TrainingRunnerError::RequestRunMismatch);
        }
        if run.cancel_requested {
            run.transition(TrainingRunState::Cancelled)?;
            self.store.save_training_run(&run).await?;
            return Ok(run);
        }

        run.transition(TrainingRunState::Running)?;
        self.store.save_training_run(&run).await?;
        let mut session = match self.backend.start(request) {
            Ok(session) => session,
            Err(error) => return self.fail_run(run, error.to_string()).await,
        };

        while run.completed_epochs < run.configuration.epochs {
            let persisted = self
                .store
                .get_training_run(run.id)
                .await?
                .ok_or(TrainingRunnerError::RunNotFound(run.id))?;
            if persisted.cancel_requested {
                run.cancel_requested = true;
                run.transition(TrainingRunState::Cancelled)?;
                self.store.save_training_run(&run).await?;
                return Ok(run);
            }

            let metrics = match session.train_batch() {
                Ok(metrics) => metrics,
                Err(error) => return self.fail_run(run, error.to_string()).await,
            };
            let expected_epoch = run.completed_epochs + 1;
            let expected_batch = if run.current_epoch == expected_epoch {
                run.completed_batches + 1
            } else {
                1
            };
            if metrics.epoch != expected_epoch
                || metrics.batch != expected_batch
                || metrics.batch == 0
                || metrics.batches_in_epoch == 0
                || metrics.batch > metrics.batches_in_epoch
                || metrics.epoch_complete != (metrics.batch == metrics.batches_in_epoch)
            {
                return self
                    .fail_run(
                        run,
                        format!(
                            "backend returned invalid progress epoch {}/{expected_epoch}, batch \
                             {}/{expected_batch}, batches_in_epoch {}, epoch_complete {}",
                            metrics.epoch,
                            metrics.batch,
                            metrics.batches_in_epoch,
                            metrics.epoch_complete,
                        ),
                    )
                    .await;
            }
            run.current_epoch = metrics.epoch;
            run.completed_batches = metrics.batch;
            run.batches_in_epoch = metrics.batches_in_epoch;
            run.processed_examples = metrics.processed_examples;
            run.latest_training_loss = Some(metrics.training_loss);
            run.latest_validation_loss = metrics.validation_loss;
            run.latest_learning_rate = Some(metrics.learning_rate);
            run.elapsed_milliseconds = metrics.elapsed_milliseconds;
            if metrics.epoch_complete {
                run.completed_epochs = metrics.epoch;
            }
            run.updated_at = Utc::now();
            self.store.save_training_run(&run).await?;

            let is_final = run.completed_epochs == run.configuration.epochs;
            if metrics.epoch_complete
                && (is_final || metrics.epoch % run.configuration.checkpoint_every == 0)
            {
                let bytes = match session.serialize_checkpoint() {
                    Ok(bytes) => bytes,
                    Err(error) => return self.fail_run(run, error.to_string()).await,
                };
                let stored =
                    match self
                        .artifacts
                        .write(run.id, metrics.epoch, &run.model_format, &bytes)
                    {
                        Ok(stored) => stored,
                        Err(error) => return self.fail_run(run, error.to_string()).await,
                    };
                self.store
                    .create_checkpoint(&TrainingCheckpoint {
                        id: Uuid::new_v4(),
                        run_id: run.id,
                        epoch: metrics.epoch,
                        artifact_path: stored.path,
                        artifact_checksum: stored.checksum,
                        artifact_size_bytes: stored.size_bytes,
                        model_format: run.model_format.clone(),
                        training_loss: metrics.training_loss,
                        validation_loss: metrics.validation_loss,
                        is_final,
                        created_at: Utc::now(),
                    })
                    .await?;
            }
        }

        run.transition(TrainingRunState::Completed)?;
        self.store.save_training_run(&run).await?;
        Ok(run)
    }

    async fn fail_run(
        &self,
        mut run: TrainingRun,
        message: String,
    ) -> Result<TrainingRun, TrainingRunnerError> {
        run.error_message = Some(message);
        run.transition(TrainingRunState::Failed)?;
        self.store.save_training_run(&run).await?;
        Ok(run)
    }
}

#[derive(Debug, Error)]
pub enum TrainingRunnerError {
    #[error(transparent)]
    Store(#[from] TrainingStoreError),
    #[error(transparent)]
    Domain(#[from] crate::domain::TrainingDomainError),
    #[error("training run not found: {0}")]
    RunNotFound(Uuid),
    #[error("training run must be queued, but was {0:?}")]
    RunNotQueued(TrainingRunState),
    #[error("training request does not match the persisted run")]
    RequestRunMismatch,
}
