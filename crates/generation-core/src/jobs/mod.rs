//! Local generation-job lifecycle and execution orchestration.

mod execution;

pub use execution::{
    GenerationAttempt, GenerationAttemptFailureKind, GenerationAttemptState,
    GenerationBackendIdentity, GenerationExecutionError, GenerationExecutionPolicy,
    GenerationExecutionSpec, PromptTemplateIdentity,
};

use std::{sync::Arc, time::Duration};

use artifact_core::FingerprintError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    deduplication::{NormalizedTextDeduplicator, normalize_text},
    domain::{GeneratedCandidate, GeneratedRow, GenerationParameters, ValidationStatus},
    ports::{GenerationBackend, GenerationBackendError, GenerationStore, StoreError},
    prompting::PromptBuilder,
    validation::{ValidationContext, ValidationPipeline},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationJob {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub plan_id: Uuid,
    pub backend_name: String,
    pub backend_model: String,
    pub state: JobState,
    pub requested_rows: u64,
    pub generated_rows: u64,
    pub accepted_rows: u64,
    pub rejected_rows: u64,
    pub failed_requests: u64,
    pub cancel_requested: bool,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GenerationJob {
    pub fn queued(
        dataset_id: Uuid,
        plan_id: Uuid,
        backend_name: impl Into<String>,
        backend_model: impl Into<String>,
        requested_rows: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            dataset_id,
            plan_id,
            backend_name: backend_name.into(),
            backend_model: backend_model.into(),
            state: JobState::Queued,
            requested_rows,
            generated_rows: 0,
            accepted_rows: 0,
            rejected_rows: 0,
            failed_requests: 0,
            cancel_requested: false,
            error_message: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn remaining_rows(&self) -> u64 {
        self.requested_rows.saturating_sub(self.accepted_rows)
    }

    pub fn transition(&mut self, next: JobState) -> Result<(), JobTransitionError> {
        let allowed = matches!(
            (self.state, next),
            (
                JobState::Queued,
                JobState::Running | JobState::Cancelled | JobState::Failed
            ) | (
                JobState::Running,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        );
        if !allowed {
            return Err(JobTransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        self.updated_at = Utc::now();
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("invalid generation job transition from {from:?} to {to:?}")]
pub struct JobTransitionError {
    pub from: JobState,
    pub to: JobState,
}

#[derive(Debug, Clone)]
pub struct JobRunnerPolicy {
    pub batch_size: u32,
    pub max_request_retries: u32,
    pub max_attempt_multiplier: u32,
    pub retry_delay: Duration,
}

impl From<&JobRunnerPolicy> for GenerationExecutionPolicy {
    fn from(value: &JobRunnerPolicy) -> Self {
        Self {
            batch_size: value.batch_size,
            max_request_retries: value.max_request_retries,
            max_attempt_multiplier: value.max_attempt_multiplier,
            retry_delay_milliseconds: value.retry_delay.as_millis().try_into().unwrap_or(u64::MAX),
        }
    }
}

impl Default for JobRunnerPolicy {
    fn default() -> Self {
        Self {
            batch_size: 20,
            max_request_retries: 3,
            max_attempt_multiplier: 3,
            retry_delay: Duration::from_millis(500),
        }
    }
}

pub struct JobRunner {
    store: Arc<dyn GenerationStore>,
    backend: Arc<dyn GenerationBackend>,
    policy: JobRunnerPolicy,
    validation: ValidationPipeline,
    prompt_builder: PromptBuilder,
}

impl JobRunner {
    pub fn new(
        store: Arc<dyn GenerationStore>,
        backend: Arc<dyn GenerationBackend>,
        policy: JobRunnerPolicy,
        validation: ValidationPipeline,
    ) -> Self {
        Self {
            store,
            backend,
            policy,
            validation,
            prompt_builder: PromptBuilder::default(),
        }
    }

    pub fn with_semantic_context(
        mut self,
        context: semantic_catalog::ResolvedSemanticContext,
    ) -> Self {
        self.prompt_builder = PromptBuilder::with_semantics(context);
        self
    }

    pub async fn run(
        &self,
        job_id: Uuid,
        parameters: GenerationParameters,
    ) -> Result<GenerationJob, JobRunnerError> {
        let outcome = self.run_inner(job_id, parameters).await;
        if let Err(error) = &outcome {
            self.fail_running_job(job_id, error).await;
        }
        outcome
    }

    async fn run_inner(
        &self,
        job_id: Uuid,
        parameters: GenerationParameters,
    ) -> Result<GenerationJob, JobRunnerError> {
        let mut job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(JobRunnerError::JobNotFound(job_id))?;
        let execution = self
            .store
            .get_generation_execution_spec(job_id)
            .await?
            .ok_or(JobRunnerError::ExecutionSpecNotFound(job_id))?;
        self.validate_execution(&job, &execution, &parameters)?;
        if job.state != JobState::Queued {
            return Err(JobRunnerError::JobNotQueued(job.state));
        }
        if job.cancel_requested {
            job.transition(JobState::Cancelled)?;
            self.store.save_job(&job).await?;
            return Ok(job);
        }

        let dataset = self
            .store
            .get_dataset(job.dataset_id)
            .await?
            .ok_or(JobRunnerError::DatasetNotFound(job.dataset_id))?;
        let plan = self
            .store
            .get_plan(job.plan_id)
            .await?
            .ok_or(JobRunnerError::PlanNotFound(job.plan_id))?;
        if plan.dataset_id != dataset.id {
            return Err(JobRunnerError::PlanDatasetMismatch);
        }

        job.transition(JobState::Running)?;
        self.store.save_job(&job).await?;

        let existing = self.store.accepted_normalized_texts(dataset.id).await?;
        let mut deduplicator = NormalizedTextDeduplicator::from_normalized(existing);
        let historical_attempts = self.store.list_generation_attempts(job.id).await?;
        let mut next_attempt_sequence = self.store.next_generation_attempt_sequence(job.id).await?;

        for planned in &plan.cells {
            let counts = self.store.dataset_cell_counts(dataset.id).await?;
            let initial_accepted = counts
                .get(&planned.cell.key())
                .map_or(0, |counts| counts.accepted);
            let mut remaining = planned.target_count.saturating_sub(initial_accepted);
            let initial_required = execution
                .initial_needs
                .iter()
                .find(|need| need.planned.cell == planned.cell)
                .map_or(0, |need| need.remaining_count);
            let max_attempted_rows = initial_required
                .saturating_mul(self.policy.max_attempt_multiplier.max(1))
                .max(initial_required);
            let mut attempted_rows = historical_attempts
                .iter()
                .filter(|attempt| attempt.cell == planned.cell)
                .map(|attempt| attempt.requested_count)
                .fold(0_u32, u32::saturating_add);

            while remaining > 0 && attempted_rows < max_attempted_rows {
                if self.refresh_cancellation(&mut job).await? {
                    return Ok(job);
                }
                let requested_count = remaining
                    .min(self.policy.batch_size.max(1))
                    .min(max_attempted_rows - attempted_rows);
                let request = self.prompt_builder.build(
                    &dataset,
                    planned.cell.clone(),
                    requested_count,
                    parameters.clone(),
                    &[],
                );

                let (result, mut attempt) = match self
                    .generate_with_retries(
                        &mut job,
                        request,
                        &mut next_attempt_sequence,
                        &mut attempted_rows,
                        max_attempted_rows,
                    )
                    .await?
                {
                    Some(result) => result,
                    None => return Ok(job),
                };
                let provider_returned_rows = result.rows.len().try_into().unwrap_or(u32::MAX);
                let metadata = json!({
                    "backend": result.backend_metadata.clone(),
                    "usage": result.usage.clone(),
                    "backend_errors": result.errors.clone(),
                    "generation_attempt_id": attempt.id,
                    "request_fingerprint": attempt.request_fingerprint.clone(),
                    "semantic_context": self.prompt_builder.semantic_context(),
                });
                let rows = result
                    .rows
                    .into_iter()
                    .take(requested_count as usize)
                    .map(|candidate| {
                        self.build_row(
                            &job,
                            &dataset,
                            planned,
                            candidate,
                            &mut deduplicator,
                            metadata.clone(),
                        )
                    })
                    .collect::<Vec<_>>();

                let accepted = rows
                    .iter()
                    .filter(|row| row.validation_status == ValidationStatus::Accepted)
                    .count() as u64;
                let rejected = rows.len() as u64 - accepted;
                attempt.succeed(
                    provider_returned_rows,
                    rows.len().try_into().unwrap_or(u32::MAX),
                    accepted.try_into().unwrap_or(u32::MAX),
                    rejected.try_into().unwrap_or(u32::MAX),
                    result.usage,
                    result.backend_metadata,
                    result.errors,
                )?;
                job = self
                    .store
                    .finish_generation_attempt(&attempt, &rows)
                    .await?;
                remaining = remaining.saturating_sub(u32::try_from(accepted).unwrap_or(u32::MAX));
            }
        }

        let accepted = self
            .store
            .dataset_cell_counts(dataset.id)
            .await?
            .into_iter()
            .map(|(key, counts)| (key, counts.accepted))
            .collect();
        let needs = crate::planning::calculate_generation_needs(&plan, &accepted);
        job = self.store.reconcile_generation_job(job.id).await?;
        if needs.iter().all(|need| need.remaining_count == 0) {
            job.transition(JobState::Completed)?;
        } else {
            job.error_message = Some(
                "generation stopped after the configured attempt limit with incomplete coverage"
                    .into(),
            );
            job.transition(JobState::Failed)?;
        }
        self.store.save_job(&job).await?;
        Ok(job)
    }

    async fn fail_running_job(&self, job_id: Uuid, error: &JobRunnerError) {
        let _ = self.store.interrupt_open_generation_attempts(job_id).await;
        let Ok(Some(mut job)) = self.store.get_job(job_id).await else {
            return;
        };
        if job.state != JobState::Running {
            return;
        }
        job.error_message = Some(format!("generation runner aborted: {error}"));
        if job.transition(JobState::Failed).is_ok() {
            let _ = self.store.save_job(&job).await;
        }
    }

    async fn refresh_cancellation(&self, job: &mut GenerationJob) -> Result<bool, JobRunnerError> {
        let persisted = self
            .store
            .get_job(job.id)
            .await?
            .ok_or(JobRunnerError::JobNotFound(job.id))?;
        if persisted.cancel_requested {
            job.cancel_requested = true;
            job.transition(JobState::Cancelled)?;
            self.store.save_job(job).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn generate_with_retries(
        &self,
        job: &mut GenerationJob,
        request: crate::domain::GenerationRequest,
        next_attempt_sequence: &mut u64,
        attempted_rows: &mut u32,
        max_attempted_rows: u32,
    ) -> Result<Option<(crate::domain::GenerationResult, GenerationAttempt)>, JobRunnerError> {
        let total_attempts = self.policy.max_request_retries.saturating_add(1);
        for retry_index in 0..total_attempts {
            if attempted_rows.saturating_add(request.requested_count) > max_attempted_rows {
                job.error_message = Some(
                    "generation stopped before retrying because the persisted attempt budget was exhausted"
                        .into(),
                );
                job.transition(JobState::Failed)?;
                self.store.save_job(job).await?;
                return Ok(None);
            }
            *attempted_rows = attempted_rows.saturating_add(request.requested_count);
            let mut attempt =
                GenerationAttempt::start(job.id, *next_attempt_sequence, retry_index, &request)?;
            *next_attempt_sequence = next_attempt_sequence.checked_add(1).ok_or(
                GenerationExecutionError::InvalidAttempt("attempt sequence overflowed".into()),
            )?;
            self.store.start_generation_attempt(&attempt).await?;
            match self.backend.generate(request.clone()).await {
                Ok(result) => return Ok(Some((result, attempt))),
                Err(error) => {
                    let retryable = error.is_retryable();
                    attempt.fail(backend_error_kind(&error), error.to_string(), retryable)?;
                    *job = self.store.finish_generation_attempt(&attempt, &[]).await?;
                    if !retryable || retry_index + 1 == total_attempts {
                        job.error_message = Some(error.to_string());
                        job.transition(JobState::Failed)?;
                        self.store.save_job(job).await?;
                        return Ok(None);
                    }
                    tokio::time::sleep(retry_delay(&self.policy, retry_index, &error)).await;
                }
            }
        }
        unreachable!("retry loop always returns")
    }

    fn validate_execution(
        &self,
        job: &GenerationJob,
        execution: &GenerationExecutionSpec,
        parameters: &GenerationParameters,
    ) -> Result<(), JobRunnerError> {
        let semantic_fingerprint = self
            .prompt_builder
            .semantic_context()
            .map_or("none", |context| context.fingerprint.as_str());
        if execution.reproduce_fingerprint()? != execution.fingerprint
            || execution.job_id != job.id
            || execution.dataset_id != job.dataset_id
            || execution.plan_id != job.plan_id
            || execution.backend.name != self.backend.name()
            || execution.backend.model != self.backend.model()
            || &execution.parameters != parameters
            || execution.policy != GenerationExecutionPolicy::from(&self.policy)
            || execution.prompt_template != PromptBuilder::template_identity()?
            || execution.semantic_context_fingerprint != semantic_fingerprint
        {
            return Err(JobRunnerError::ExecutionSpecMismatch(job.id));
        }
        Ok(())
    }

    fn build_row(
        &self,
        job: &GenerationJob,
        dataset: &crate::domain::DatasetDefinition,
        planned: &crate::domain::PlannedCell,
        candidate: GeneratedCandidate,
        deduplicator: &mut NormalizedTextDeduplicator,
        generation_metadata: serde_json::Value,
    ) -> GeneratedRow {
        let result = self.validation.validate(
            &ValidationContext {
                dataset,
                target: &planned.cell,
            },
            &candidate,
        );
        let mut errors = result
            .issues
            .into_iter()
            .map(|issue| format!("{}: {}", issue.code, issue.message))
            .collect::<Vec<_>>();
        if result.is_valid && !deduplicator.check_and_record(&candidate.text) {
            errors.push("duplicate_text: normalized text already exists".into());
        }
        let validation_status = if errors.is_empty() {
            ValidationStatus::Accepted
        } else {
            ValidationStatus::Rejected
        };

        GeneratedRow {
            id: Uuid::new_v4(),
            dataset_id: dataset.id,
            plan_id: job.plan_id,
            generation_job_id: job.id,
            cell_key: planned.cell.key(),
            normalized_text: normalize_text(&candidate.text),
            text: candidate.text,
            label: candidate.label,
            dimensions: candidate.dimensions,
            generator_backend: self.backend.name().to_owned(),
            generator_model: self.backend.model().to_owned(),
            created_at: Utc::now(),
            validation_status,
            validation_errors: errors,
            generation_metadata,
        }
    }
}

#[derive(Debug, Error)]
pub enum JobRunnerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Transition(#[from] JobTransitionError),
    #[error(transparent)]
    Execution(#[from] GenerationExecutionError),
    #[error(transparent)]
    Fingerprint(#[from] FingerprintError),
    #[error("generation job not found: {0}")]
    JobNotFound(Uuid),
    #[error("generation execution specification not found: {0}")]
    ExecutionSpecNotFound(Uuid),
    #[error("generation execution specification does not match job runtime: {0}")]
    ExecutionSpecMismatch(Uuid),
    #[error("dataset not found: {0}")]
    DatasetNotFound(Uuid),
    #[error("generation plan not found: {0}")]
    PlanNotFound(Uuid),
    #[error("generation job must be queued, but was {0:?}")]
    JobNotQueued(JobState),
    #[error("generation plan and dataset do not match")]
    PlanDatasetMismatch,
}

const fn backend_error_kind(error: &GenerationBackendError) -> GenerationAttemptFailureKind {
    match error {
        GenerationBackendError::Configuration(_) => GenerationAttemptFailureKind::Configuration,
        GenerationBackendError::Request(_)
        | GenerationBackendError::Rejected(_)
        | GenerationBackendError::RateLimited { .. } => GenerationAttemptFailureKind::Request,
        GenerationBackendError::InvalidResponse(_) => GenerationAttemptFailureKind::InvalidResponse,
    }
}

fn retry_delay(
    policy: &JobRunnerPolicy,
    retry_index: u32,
    error: &GenerationBackendError,
) -> Duration {
    let exponential = policy
        .retry_delay
        .saturating_mul(2_u32.saturating_pow(retry_index.min(16)))
        .min(Duration::from_secs(60));
    error
        .retry_after_milliseconds()
        .map(Duration::from_millis)
        .map_or(exponential, |provider| provider.max(exponential))
}

#[cfg(test)]
mod tests {
    use super::{GenerationJob, JobState};
    use uuid::Uuid;

    #[test]
    fn lifecycle_rejects_terminal_to_running_transition() {
        let mut job = GenerationJob::queued(Uuid::new_v4(), Uuid::new_v4(), "fake", "fake-v1", 10);
        job.transition(JobState::Running).expect("valid transition");
        job.transition(JobState::Completed)
            .expect("valid transition");
        assert!(job.transition(JobState::Running).is_err());
        assert!(job.state.is_terminal());
    }
}
