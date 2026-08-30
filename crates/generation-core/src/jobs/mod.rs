//! Local generation-job lifecycle and execution orchestration.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    deduplication::{NormalizedTextDeduplicator, normalize_text},
    domain::{GeneratedCandidate, GeneratedRow, GenerationParameters, ValidationStatus},
    ports::{GenerationBackend, GenerationStore, StoreError},
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
        let mut job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(JobRunnerError::JobNotFound(job_id))?;
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

        for planned in &plan.cells {
            let counts = self.store.dataset_cell_counts(dataset.id).await?;
            let initial_accepted = counts
                .get(&planned.cell.key())
                .map_or(0, |counts| counts.accepted);
            let mut remaining = planned.target_count.saturating_sub(initial_accepted);
            let max_attempted_rows = remaining
                .saturating_mul(self.policy.max_attempt_multiplier.max(1))
                .max(remaining);
            let mut attempted_rows = 0_u32;

            while remaining > 0 && attempted_rows < max_attempted_rows {
                if self.refresh_cancellation(&mut job).await? {
                    return Ok(job);
                }
                let requested_count = remaining
                    .min(self.policy.batch_size.max(1))
                    .min(max_attempted_rows - attempted_rows);
                attempted_rows = attempted_rows.saturating_add(requested_count);
                let request = self.prompt_builder.build(
                    &dataset,
                    planned.cell.clone(),
                    requested_count,
                    parameters.clone(),
                    &[],
                );

                let result = match self.generate_with_retries(&mut job, request).await? {
                    Some(result) => result,
                    None => return Ok(job),
                };
                let metadata = json!({
                    "backend": result.backend_metadata,
                    "usage": result.usage,
                    "backend_errors": result.errors,
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
                self.store.insert_rows(&rows).await?;
                job.generated_rows = job.generated_rows.saturating_add(rows.len() as u64);
                job.accepted_rows = job.accepted_rows.saturating_add(accepted);
                job.rejected_rows = job.rejected_rows.saturating_add(rejected);
                job.updated_at = Utc::now();
                self.store.save_job(&job).await?;
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
    ) -> Result<Option<crate::domain::GenerationResult>, JobRunnerError> {
        let total_attempts = self.policy.max_request_retries.saturating_add(1);
        for attempt in 0..total_attempts {
            match self.backend.generate(request.clone()).await {
                Ok(result) => return Ok(Some(result)),
                Err(error) => {
                    job.failed_requests = job.failed_requests.saturating_add(1);
                    job.updated_at = Utc::now();
                    self.store.save_job(job).await?;
                    if attempt + 1 == total_attempts {
                        job.error_message = Some(error.to_string());
                        job.transition(JobState::Failed)?;
                        self.store.save_job(job).await?;
                        return Ok(None);
                    }
                    tokio::time::sleep(self.policy.retry_delay).await;
                }
            }
        }
        unreachable!("retry loop always returns")
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
    #[error("generation job not found: {0}")]
    JobNotFound(Uuid),
    #[error("dataset not found: {0}")]
    DatasetNotFound(Uuid),
    #[error("generation plan not found: {0}")]
    PlanNotFound(Uuid),
    #[error("generation job must be queued, but was {0:?}")]
    JobNotQueued(JobState),
    #[error("generation plan and dataset do not match")]
    PlanDatasetMismatch,
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
