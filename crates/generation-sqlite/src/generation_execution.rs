use generation_core::{
    domain::{GeneratedRow, ValidationStatus},
    jobs::{
        GenerationAttempt, GenerationAttemptState, GenerationExecutionSpec, GenerationJob, JobState,
    },
    ports::{BoxFuture, GenerationExecutionStore, GenerationStrategyStore, StoreError},
    strategy::GenerationStrategyAssignment,
};
use research_core::profile::GenerationAuthenticityAssignment;
use semantic_catalog::GenerationSemanticAssignment;
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    JobRecord, SqliteStore, insert_generated_rows, insert_generation_job, store_error, to_json,
};

impl SqliteStore {
    /// Atomically creates a runnable job with its execution and semantic identities.
    pub async fn create_generation_execution_bundle(
        &self,
        job: &GenerationJob,
        spec: &GenerationExecutionSpec,
        semantics: &GenerationSemanticAssignment,
    ) -> Result<(), StoreError> {
        self.create_generation_execution_bundle_with_authenticity(job, spec, semantics, None)
            .await
    }

    /// Atomically creates a runnable job and pins an optional approved
    /// authenticity context beside its semantic context.
    pub async fn create_generation_execution_bundle_with_authenticity(
        &self,
        job: &GenerationJob,
        spec: &GenerationExecutionSpec,
        semantics: &GenerationSemanticAssignment,
        authenticity: Option<&GenerationAuthenticityAssignment>,
    ) -> Result<(), StoreError> {
        self.create_generation_execution_bundle_with_contexts(
            job,
            spec,
            semantics,
            authenticity,
            None,
        )
        .await
    }

    /// Atomically creates a runnable job with every immutable prompt input.
    pub async fn create_generation_execution_bundle_with_contexts(
        &self,
        job: &GenerationJob,
        spec: &GenerationExecutionSpec,
        semantics: &GenerationSemanticAssignment,
        authenticity: Option<&GenerationAuthenticityAssignment>,
        strategy: Option<&GenerationStrategyAssignment>,
    ) -> Result<(), StoreError> {
        validate_execution(job, spec)?;
        if semantics.job_id != job.id
            || semantics.context.fingerprint != spec.semantic_context_fingerprint
            || semantics
                .context
                .reproduce_fingerprint()
                .map_err(store_error)?
                != semantics.context.fingerprint
            || semantics.reproduce_fingerprint().map_err(store_error)? != semantics.fingerprint
        {
            return Err(StoreError(
                "generation semantic assignment and execution specification are inconsistent"
                    .into(),
            ));
        }
        match (authenticity, &spec.authenticity_context_fingerprint) {
            (None, None) => {}
            (Some(assignment), Some(expected))
                if assignment.job_id == job.id
                    && assignment.context.dataset_id == job.dataset_id
                    && assignment.context.fingerprint == *expected
                    && assignment
                        .context
                        .reproduce_fingerprint()
                        .map_err(store_error)?
                        == assignment.context.fingerprint
                    && assignment.reproduce_fingerprint().map_err(store_error)?
                        == assignment.fingerprint => {}
            _ => {
                return Err(StoreError(
                    "generation authenticity assignment and execution specification are inconsistent"
                        .into(),
                ));
            }
        }
        match (strategy, &spec.strategy_context_fingerprint) {
            (None, None) => {}
            (Some(assignment), Some(expected))
                if assignment.job_id == job.id
                    && assignment.context.dataset_id == job.dataset_id
                    && assignment.context.plan_id == job.plan_id
                    && assignment.context.fingerprint == *expected
                    && assignment
                        .context
                        .reproduce_fingerprint()
                        .map_err(store_error)?
                        == assignment.context.fingerprint
                    && assignment.reproduce_fingerprint().map_err(store_error)?
                        == assignment.fingerprint => {}
            _ => {
                return Err(StoreError(
                    "generation strategy assignment and execution specification are inconsistent"
                        .into(),
                ));
            }
        }
        let mut transaction = self.pool().begin().await.map_err(store_error)?;
        insert_generation_job(&mut transaction, job).await?;
        insert_execution_spec(&mut transaction, spec).await?;
        sqlx::query(
            "INSERT INTO generation_job_semantics \
             (job_id, context_fingerprint, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(semantics.job_id)
        .bind(&semantics.context.fingerprint)
        .bind(to_json(semantics)?)
        .bind(&semantics.fingerprint)
        .bind(semantics.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        if let Some(assignment) = authenticity {
            sqlx::query(
                "INSERT INTO generation_job_authenticity (job_id, dataset_id, profile_id, \
                 binding_id, context_fingerprint, assignment_fingerprint, assignment_json, \
                 created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(assignment.job_id)
            .bind(assignment.context.dataset_id)
            .bind(assignment.context.profile_id)
            .bind(assignment.context.binding_id)
            .bind(&assignment.context.fingerprint)
            .bind(&assignment.fingerprint)
            .bind(to_json(assignment)?)
            .bind(assignment.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        }
        if let Some(assignment) = strategy {
            sqlx::query(
                "INSERT INTO generation_job_strategies (job_id, context_id, \
                 context_fingerprint, assignment_fingerprint, assignment_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(assignment.job_id)
            .bind(assignment.context.id)
            .bind(&assignment.context.fingerprint)
            .bind(&assignment.fingerprint)
            .bind(to_json(assignment)?)
            .bind(assignment.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        }
        transaction.commit().await.map_err(store_error)?;
        Ok(())
    }

    pub async fn generation_strategy_assignment(
        &self,
        job_id: Uuid,
    ) -> Result<Option<GenerationStrategyAssignment>, StoreError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT assignment_json FROM generation_job_strategies WHERE job_id = ?",
        )
        .bind(job_id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let assignment: GenerationStrategyAssignment =
            serde_json::from_str(&value).map_err(store_error)?;
        if assignment.reproduce_fingerprint().map_err(store_error)? != assignment.fingerprint
            || assignment
                .context
                .reproduce_fingerprint()
                .map_err(store_error)?
                != assignment.context.fingerprint
        {
            return Err(StoreError(
                "generation strategy assignment fingerprint mismatch".into(),
            ));
        }
        Ok(Some(assignment))
    }
}

impl GenerationStrategyStore for SqliteStore {
    fn save_generation_strategy(
        &self,
        assignment: &GenerationStrategyAssignment,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let assignment = assignment.clone();
        Box::pin(async move {
            if assignment.reproduce_fingerprint().map_err(store_error)? != assignment.fingerprint
                || assignment
                    .context
                    .reproduce_fingerprint()
                    .map_err(store_error)?
                    != assignment.context.fingerprint
            {
                return Err(StoreError(
                    "generation strategy assignment fingerprint mismatch".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO generation_job_strategies (job_id, context_id, \
                 context_fingerprint, assignment_fingerprint, assignment_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(assignment.job_id)
            .bind(assignment.context.id)
            .bind(&assignment.context.fingerprint)
            .bind(&assignment.fingerprint)
            .bind(to_json(&assignment)?)
            .bind(assignment.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_generation_strategy(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationStrategyAssignment>, StoreError>> {
        Box::pin(async move { self.generation_strategy_assignment(job_id).await })
    }
}

impl GenerationExecutionStore for SqliteStore {
    fn create_generation_execution(
        &self,
        job: &GenerationJob,
        spec: &GenerationExecutionSpec,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let job = job.clone();
        let spec = spec.clone();
        Box::pin(async move {
            validate_execution(&job, &spec)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_generation_job(&mut transaction, &job).await?;
            insert_execution_spec(&mut transaction, &spec).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_generation_execution_spec(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationExecutionSpec>, StoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ExecutionSpecRow>(
                "SELECT job_id, dataset_id, plan_id, backend_name, backend_model, fingerprint, \
                 artifact_json, created_at FROM generation_execution_specs WHERE job_id = ?",
            )
            .bind(job_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(ExecutionSpecRow::into_domain)
            .transpose()
        })
    }

    fn next_generation_attempt_sequence(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            let maximum = sqlx::query_scalar::<_, Option<i64>>(
                "SELECT MAX(sequence) FROM generation_request_attempts WHERE job_id = ?",
            )
            .bind(job_id)
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?
            .unwrap_or(0);
            u64::try_from(maximum)
                .map_err(store_error)?
                .checked_add(1)
                .ok_or_else(|| StoreError("generation attempt sequence overflowed".into()))
        })
    }

    fn start_generation_attempt(
        &self,
        attempt: &GenerationAttempt,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let attempt = attempt.clone();
        Box::pin(async move {
            validate_started_attempt(&attempt)?;
            sqlx::query(
                "INSERT INTO generation_request_attempts \
                 (id, job_id, sequence, cell_key, retry_index, requested_count, state, \
                  request_fingerprint, outcome_fingerprint, artifact_json, started_at, finished_at) \
                 VALUES (?, ?, ?, ?, ?, ?, 'started', ?, NULL, ?, ?, NULL)",
            )
            .bind(attempt.id)
            .bind(attempt.job_id)
            .bind(as_i64(attempt.sequence)?)
            .bind(attempt.cell.key())
            .bind(i64::from(attempt.retry_index))
            .bind(i64::from(attempt.requested_count))
            .bind(&attempt.request_fingerprint)
            .bind(to_json(&attempt)?)
            .bind(attempt.started_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn finish_generation_attempt(
        &self,
        attempt: &GenerationAttempt,
        rows: &[GeneratedRow],
    ) -> BoxFuture<'_, Result<GenerationJob, StoreError>> {
        let attempt = attempt.clone();
        let rows = rows.to_vec();
        Box::pin(async move {
            validate_terminal_attempt(&attempt, &rows)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let persisted = sqlx::query_as::<_, AttemptRow>(
                "SELECT id, job_id, sequence, cell_key, retry_index, requested_count, state, \
                 request_fingerprint, outcome_fingerprint, artifact_json, started_at, finished_at \
                 FROM generation_request_attempts WHERE id = ?",
            )
            .bind(attempt.id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(store_error)?
            .ok_or_else(|| StoreError(format!("generation attempt not found: {}", attempt.id)))?;
            let started = persisted.into_domain()?;
            validate_same_request(&started, &attempt)?;
            insert_generated_rows(&mut transaction, &rows).await?;
            let result = sqlx::query(
                "UPDATE generation_request_attempts SET state = ?, outcome_fingerprint = ?, \
                 artifact_json = ?, finished_at = ? WHERE id = ? AND state = 'started'",
            )
            .bind(attempt_state(attempt.state))
            .bind(&attempt.outcome_fingerprint)
            .bind(to_json(&attempt)?)
            .bind(attempt.finished_at)
            .bind(attempt.id)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(StoreError(format!(
                    "generation attempt {} is no longer open",
                    attempt.id
                )));
            }
            reconcile_counters(&mut transaction, attempt.job_id).await?;
            let job = load_job(&mut transaction, attempt.job_id).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(job)
        })
    }

    fn list_generation_attempts(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<GenerationAttempt>, StoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AttemptRow>(
                "SELECT id, job_id, sequence, cell_key, retry_index, requested_count, state, \
                 request_fingerprint, outcome_fingerprint, artifact_json, started_at, finished_at \
                 FROM generation_request_attempts WHERE job_id = ? ORDER BY sequence",
            )
            .bind(job_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(AttemptRow::into_domain)
            .collect()
        })
    }

    fn interrupt_open_generation_attempts(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<GenerationAttempt>, StoreError>> {
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let records = sqlx::query_as::<_, AttemptRow>(
                "SELECT id, job_id, sequence, cell_key, retry_index, requested_count, state, \
                 request_fingerprint, outcome_fingerprint, artifact_json, started_at, finished_at \
                 FROM generation_request_attempts WHERE job_id = ? AND state = 'started' \
                 ORDER BY sequence",
            )
            .bind(job_id)
            .fetch_all(&mut *transaction)
            .await
            .map_err(store_error)?;
            let mut interrupted = Vec::with_capacity(records.len());
            for record in records {
                let mut attempt = record.into_domain()?;
                attempt.interrupt().map_err(store_error)?;
                sqlx::query(
                    "UPDATE generation_request_attempts SET state = 'interrupted', \
                     outcome_fingerprint = ?, artifact_json = ?, finished_at = ? \
                     WHERE id = ? AND state = 'started'",
                )
                .bind(&attempt.outcome_fingerprint)
                .bind(to_json(&attempt)?)
                .bind(attempt.finished_at)
                .bind(attempt.id)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                interrupted.push(attempt);
            }
            reconcile_counters(&mut transaction, job_id).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(interrupted)
        })
    }

    fn reconcile_generation_job(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<GenerationJob, StoreError>> {
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            reconcile_counters(&mut transaction, job_id).await?;
            let job = load_job(&mut transaction, job_id).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(job)
        })
    }
}

async fn insert_execution_spec(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    spec: &GenerationExecutionSpec,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO generation_execution_specs \
         (job_id, dataset_id, plan_id, backend_name, backend_model, fingerprint, \
          artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(spec.job_id)
    .bind(spec.dataset_id)
    .bind(spec.plan_id)
    .bind(&spec.backend.name)
    .bind(&spec.backend.model)
    .bind(&spec.fingerprint)
    .bind(to_json(spec)?)
    .bind(spec.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct ExecutionSpecRow {
    job_id: Uuid,
    dataset_id: Uuid,
    plan_id: Uuid,
    backend_name: String,
    backend_model: String,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ExecutionSpecRow {
    fn into_domain(self) -> Result<GenerationExecutionSpec, StoreError> {
        let spec: GenerationExecutionSpec =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if spec.job_id != self.job_id
            || spec.dataset_id != self.dataset_id
            || spec.plan_id != self.plan_id
            || spec.backend.name != self.backend_name
            || spec.backend.model != self.backend_model
            || spec.fingerprint != self.fingerprint
            || spec.created_at != self.created_at
            || spec.reproduce_fingerprint().map_err(store_error)? != spec.fingerprint
        {
            return Err(StoreError(
                "persisted generation execution specification failed integrity checks".into(),
            ));
        }
        Ok(spec)
    }
}

#[derive(Debug, FromRow)]
struct AttemptRow {
    id: Uuid,
    job_id: Uuid,
    sequence: i64,
    cell_key: String,
    retry_index: i64,
    requested_count: i64,
    state: String,
    request_fingerprint: String,
    outcome_fingerprint: Option<String>,
    artifact_json: String,
    started_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AttemptRow {
    fn into_domain(self) -> Result<GenerationAttempt, StoreError> {
        let attempt: GenerationAttempt =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if attempt.id != self.id
            || attempt.job_id != self.job_id
            || attempt.sequence != as_u64(self.sequence)?
            || attempt.cell.key() != self.cell_key
            || attempt.retry_index != u32::try_from(self.retry_index).map_err(store_error)?
            || attempt.requested_count
                != u32::try_from(self.requested_count).map_err(store_error)?
            || attempt_state(attempt.state) != self.state
            || attempt.request_fingerprint != self.request_fingerprint
            || attempt.outcome_fingerprint != self.outcome_fingerprint
            || attempt.started_at != self.started_at
            || attempt.finished_at != self.finished_at
        {
            return Err(StoreError(
                "persisted generation attempt normalized fields differ from its artifact".into(),
            ));
        }
        if attempt.state.is_terminal() {
            if attempt
                .reproduce_outcome_fingerprint()
                .map_err(store_error)?
                != attempt
                    .outcome_fingerprint
                    .clone()
                    .ok_or_else(|| StoreError("terminal attempt has no fingerprint".into()))?
            {
                return Err(StoreError(
                    "persisted generation attempt fingerprint does not reproduce".into(),
                ));
            }
        } else if attempt.outcome_fingerprint.is_some() || attempt.finished_at.is_some() {
            return Err(StoreError(
                "started generation attempt unexpectedly has a terminal outcome".into(),
            ));
        }
        Ok(attempt)
    }
}

fn validate_execution(
    job: &GenerationJob,
    spec: &GenerationExecutionSpec,
) -> Result<(), StoreError> {
    if job.state != JobState::Queued
        || job.id != spec.job_id
        || job.dataset_id != spec.dataset_id
        || job.plan_id != spec.plan_id
        || job.backend_name != spec.backend.name
        || job.backend_model != spec.backend.model
        || job.requested_rows
            != spec
                .initial_needs
                .iter()
                .map(|need| u64::from(need.remaining_count))
                .sum::<u64>()
        || spec.reproduce_fingerprint().map_err(store_error)? != spec.fingerprint
    {
        return Err(StoreError(
            "generation job and execution specification are inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_started_attempt(attempt: &GenerationAttempt) -> Result<(), StoreError> {
    if attempt.state != GenerationAttemptState::Started
        || attempt.outcome_fingerprint.is_some()
        || attempt.finished_at.is_some()
    {
        return Err(StoreError(
            "only a pristine started generation attempt can be persisted".into(),
        ));
    }
    Ok(())
}

fn validate_terminal_attempt(
    attempt: &GenerationAttempt,
    rows: &[GeneratedRow],
) -> Result<(), StoreError> {
    if !attempt.state.is_terminal()
        || attempt
            .reproduce_outcome_fingerprint()
            .map_err(store_error)?
            != attempt
                .outcome_fingerprint
                .clone()
                .ok_or_else(|| StoreError("terminal attempt has no fingerprint".into()))?
    {
        return Err(StoreError(
            "generation attempt does not have a valid terminal outcome".into(),
        ));
    }
    let accepted = rows
        .iter()
        .filter(|row| row.validation_status == ValidationStatus::Accepted)
        .count();
    let rejected = rows.len().saturating_sub(accepted);
    if rows.len() != usize::try_from(attempt.persisted_rows).map_err(store_error)?
        || accepted != usize::try_from(attempt.accepted_rows).map_err(store_error)?
        || rejected != usize::try_from(attempt.rejected_rows).map_err(store_error)?
        || rows.iter().any(|row| {
            row.generation_job_id != attempt.job_id || row.cell_key != attempt.cell.key()
        })
    {
        return Err(StoreError(
            "generation attempt outcome and persisted rows are inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_same_request(
    started: &GenerationAttempt,
    terminal: &GenerationAttempt,
) -> Result<(), StoreError> {
    if started.state != GenerationAttemptState::Started
        || started.id != terminal.id
        || started.job_id != terminal.job_id
        || started.sequence != terminal.sequence
        || started.cell != terminal.cell
        || started.requested_count != terminal.requested_count
        || started.retry_index != terminal.retry_index
        || started.request_fingerprint != terminal.request_fingerprint
        || started.started_at != terminal.started_at
    {
        return Err(StoreError(
            "terminal generation attempt changed its immutable request identity".into(),
        ));
    }
    Ok(())
}

async fn reconcile_counters(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job_id: Uuid,
) -> Result<(), StoreError> {
    let result = sqlx::query(
        "UPDATE generation_jobs SET \
         generated_rows = (SELECT COUNT(*) FROM generated_rows WHERE generation_job_id = ?), \
         accepted_rows = (SELECT COUNT(*) FROM generated_rows \
                          WHERE generation_job_id = ? AND validation_status = 'accepted'), \
         rejected_rows = (SELECT COUNT(*) FROM generated_rows \
                          WHERE generation_job_id = ? AND validation_status = 'rejected'), \
         failed_requests = (SELECT COUNT(*) FROM generation_request_attempts \
                            WHERE job_id = ? AND state IN ('failed', 'interrupted')), \
         updated_at = ? WHERE id = ?",
    )
    .bind(job_id)
    .bind(job_id)
    .bind(job_id)
    .bind(job_id)
    .bind(chrono::Utc::now())
    .bind(job_id)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    if result.rows_affected() != 1 {
        return Err(StoreError(format!("generation job not found: {job_id}")));
    }
    Ok(())
}

async fn load_job(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job_id: Uuid,
) -> Result<GenerationJob, StoreError> {
    sqlx::query_as::<_, JobRecord>(
        "SELECT id, dataset_id, plan_id, backend_name, backend_model, state, requested_rows, \
         generated_rows, accepted_rows, rejected_rows, failed_requests, cancel_requested, \
         error_message, created_at, updated_at FROM generation_jobs WHERE id = ?",
    )
    .bind(job_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(store_error)?
    .ok_or_else(|| StoreError(format!("generation job not found: {job_id}")))?
    .into_domain()
}

fn attempt_state(state: GenerationAttemptState) -> &'static str {
    match state {
        GenerationAttemptState::Started => "started",
        GenerationAttemptState::Succeeded => "succeeded",
        GenerationAttemptState::Failed => "failed",
        GenerationAttemptState::Interrupted => "interrupted",
    }
}

fn as_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(store_error)
}

fn as_u64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(store_error)
}
