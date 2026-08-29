use std::{collections::BTreeSet, sync::Arc, time::Instant};

use chrono::Utc;
use thiserror::Error;
use training_core::ports::Predictor;
use uuid::Uuid;

use crate::{
    domain::{EvaluationDomainError, EvaluationPrediction, EvaluationRun, EvaluationRunState},
    metrics::EvaluationMetricsAccumulator,
    ports::{
        EvaluationExampleSource, EvaluationExampleSourceError, EvaluationStore,
        EvaluationStoreError, PredictionQuery,
    },
    validation::{PredictionValidationError, validate_prediction},
};

const METRIC_PAGE_SIZE: u32 = 1_000;

pub struct EvaluationRunner {
    store: Arc<dyn EvaluationStore>,
    source: Arc<dyn EvaluationExampleSource>,
    predictor: Arc<dyn Predictor>,
}

impl EvaluationRunner {
    pub fn new(
        store: Arc<dyn EvaluationStore>,
        source: Arc<dyn EvaluationExampleSource>,
        predictor: Arc<dyn Predictor>,
    ) -> Self {
        Self {
            store,
            source,
            predictor,
        }
    }

    /// Runs or resumes an evaluation from its last atomically committed batch.
    pub async fn run(
        &self,
        run_id: Uuid,
        labels: Vec<String>,
    ) -> Result<EvaluationRun, EvaluationRunnerError> {
        validate_labels(&labels)?;
        let mut run = self
            .store
            .get_evaluation_run(run_id)
            .await?
            .ok_or(EvaluationRunnerError::RunNotFound(run_id))?;
        run.protocol.validate_for_labels(&labels)?;
        if run.source_identity.labels.is_empty() {
            run.source_identity.labels = labels.clone();
        } else if run.source_identity.labels != labels {
            return Err(EvaluationRunnerError::SourceLabels);
        }
        if self.predictor.labels() != labels.as_slice() {
            return Err(EvaluationRunnerError::PredictorLabels);
        }
        if !matches!(
            run.state,
            EvaluationRunState::Queued | EvaluationRunState::Running
        ) {
            return Err(EvaluationRunnerError::RunNotRunnable(run.state));
        }

        let source_count = self
            .source
            .count_examples(run.snapshot_id, run.protocol.split)
            .await?;
        if source_count == 0 {
            return Err(EvaluationDomainError::EmptyEvaluationSet.into());
        }
        if run.total_examples == 0 && run.processed_examples == 0 {
            run.total_examples = source_count;
        } else if run.total_examples != source_count {
            return Err(EvaluationRunnerError::CohortSizeChanged {
                expected: run.total_examples,
                actual: source_count,
            });
        }
        let persisted_count = self.store.count_predictions(run.id).await?;
        if persisted_count != run.processed_examples {
            return Err(EvaluationRunnerError::ProgressMismatch {
                recorded: run.processed_examples,
                persisted: persisted_count,
            });
        }
        if run.cancel_requested {
            return self.cancel_run(run).await;
        }
        if run.state == EvaluationRunState::Queued {
            run.transition(EvaluationRunState::Running)?;
            self.store.save_evaluation_run(&run).await?;
        }

        let mut started = Instant::now();
        while run.processed_examples < run.total_examples {
            let current = self
                .store
                .get_evaluation_run(run.id)
                .await?
                .ok_or(EvaluationRunnerError::RunNotFound(run.id))?;
            if current.cancel_requested {
                run.cancel_requested = true;
                run.elapsed_milliseconds = run
                    .elapsed_milliseconds
                    .saturating_add(elapsed_milliseconds(started));
                return self.cancel_run(run).await;
            }

            run.current_batch = run.completed_batches + 1;
            run.updated_at = Utc::now();
            self.store.save_evaluation_run(&run).await?;

            let remaining = run.total_examples - run.processed_examples;
            let limit = remaining.min(run.protocol.batch_size as u64) as u32;
            let examples = self
                .source
                .query_examples(
                    run.snapshot_id,
                    run.protocol.split,
                    limit,
                    run.processed_examples,
                )
                .await?;
            if examples.len() != limit as usize {
                return self
                    .fail_run(
                        run,
                        format!(
                            "evaluation source returned {} examples for a requested batch of {limit}",
                            examples.len()
                        ),
                        started,
                    )
                    .await;
            }
            if examples
                .iter()
                .any(|example| !labels.contains(&example.expected_label))
            {
                return self
                    .fail_run(
                        run,
                        "evaluation source contains an unknown label".into(),
                        started,
                    )
                    .await;
            }
            let texts = examples
                .iter()
                .map(|example| example.text.clone())
                .collect::<Vec<_>>();
            let batch_predictions = match self.predictor.predict_batch(&texts) {
                Ok(predictions) if predictions.len() == examples.len() => predictions,
                Ok(predictions) => {
                    return self
                        .fail_run(
                            run,
                            format!(
                                "predictor returned {} rows for a batch of {}",
                                predictions.len(),
                                examples.len()
                            ),
                            started,
                        )
                        .await;
                }
                Err(error) => return self.fail_run(run, error.to_string(), started).await,
            };

            let mut predictions = Vec::with_capacity(examples.len());
            for (example, prediction) in examples.into_iter().zip(batch_predictions) {
                if let Err(error) = validate_prediction(&labels, &prediction) {
                    return self.fail_run(run, error.to_string(), started).await;
                }
                predictions.push(EvaluationPrediction {
                    id: Uuid::new_v4(),
                    evaluation_run_id: run.id,
                    snapshot_member_id: example.snapshot_member_id,
                    source_row_id: example.source_row_id,
                    text: example.text,
                    expected_label: example.expected_label,
                    predicted_label: prediction.label,
                    confidence: prediction.confidence,
                    probabilities: prediction.probabilities,
                    dimensions: example.dimensions,
                    created_at: Utc::now(),
                });
            }
            run.processed_examples += predictions.len() as u64;
            run.example_count = run.processed_examples;
            run.completed_batches += 1;
            run.correct_predictions += predictions
                .iter()
                .filter(|prediction| prediction.expected_label == prediction.predicted_label)
                .count() as u64;
            run.elapsed_milliseconds = run
                .elapsed_milliseconds
                .saturating_add(elapsed_milliseconds(started));
            self.store
                .commit_evaluation_batch(&run, &predictions)
                .await?;
            started = Instant::now();
        }

        let persisted_count = self.store.count_predictions(run.id).await?;
        if persisted_count != run.total_examples {
            return self
                .fail_run(
                    run,
                    format!(
                        "evaluation completed inference but persisted {persisted_count} of {} predictions",
                        source_count
                    ),
                    started,
                )
                .await;
        }
        let mut accumulator = EvaluationMetricsAccumulator::new(&labels, &run.protocol);
        let mut offset = 0_u32;
        loop {
            let page = self
                .store
                .query_predictions(PredictionQuery::page(run.id, METRIC_PAGE_SIZE, offset))
                .await?;
            let page_len = page.len();
            for prediction in &page {
                accumulator.add(prediction);
            }
            if page_len < METRIC_PAGE_SIZE as usize {
                break;
            }
            offset = offset
                .checked_add(METRIC_PAGE_SIZE)
                .ok_or(EvaluationRunnerError::PredictionOffsetOverflow)?;
        }
        run.metrics = Some(accumulator.finish());
        run.example_count = persisted_count;
        run.elapsed_milliseconds = run
            .elapsed_milliseconds
            .saturating_add(elapsed_milliseconds(started));
        run.transition(EvaluationRunState::Completed)?;
        self.store.save_evaluation_run(&run).await?;
        self.store
            .get_evaluation_run(run.id)
            .await?
            .ok_or(EvaluationRunnerError::RunNotFound(run.id))
    }

    async fn cancel_run(
        &self,
        mut run: EvaluationRun,
    ) -> Result<EvaluationRun, EvaluationRunnerError> {
        run.transition(EvaluationRunState::Cancelled)?;
        self.store.save_evaluation_run(&run).await?;
        Ok(run)
    }

    async fn fail_run(
        &self,
        mut run: EvaluationRun,
        message: String,
        started: Instant,
    ) -> Result<EvaluationRun, EvaluationRunnerError> {
        run.error_message = Some(message);
        run.elapsed_milliseconds = run
            .elapsed_milliseconds
            .saturating_add(elapsed_milliseconds(started));
        run.transition(EvaluationRunState::Failed)?;
        self.store.save_evaluation_run(&run).await?;
        Ok(run)
    }
}

fn validate_labels(labels: &[String]) -> Result<(), EvaluationDomainError> {
    let unique = labels
        .iter()
        .filter(|label| !label.trim().is_empty())
        .collect::<BTreeSet<_>>();
    if unique.len() < 2 || unique.len() != labels.len() {
        Err(EvaluationDomainError::Labels)
    } else {
        Ok(())
    }
}

fn elapsed_milliseconds(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Error)]
pub enum EvaluationRunnerError {
    #[error(transparent)]
    Store(#[from] EvaluationStoreError),
    #[error(transparent)]
    Source(#[from] EvaluationExampleSourceError),
    #[error(transparent)]
    Domain(#[from] EvaluationDomainError),
    #[error(transparent)]
    Prediction(#[from] PredictionValidationError),
    #[error("evaluation run not found: {0}")]
    RunNotFound(Uuid),
    #[error("evaluation run is not runnable: {0:?}")]
    RunNotRunnable(EvaluationRunState),
    #[error("predictor labels must exactly match the evaluation label order")]
    PredictorLabels,
    #[error("persisted source identity labels do not match the requested labels")]
    SourceLabels,
    #[error("evaluation cohort size changed: expected {expected}, found {actual}")]
    CohortSizeChanged { expected: u64, actual: u64 },
    #[error("evaluation progress mismatch: run records {recorded}, database contains {persisted}")]
    ProgressMismatch { recorded: u64, persisted: u64 },
    #[error("prediction pagination offset overflowed")]
    PredictionOffsetOverflow,
}
