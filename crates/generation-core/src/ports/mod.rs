//! Project-owned interfaces implemented by infrastructure adapters.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use thiserror::Error;
use uuid::Uuid;

use crate::{
    coverage::CellCounts,
    domain::{
        BackendConfiguration, DatasetDefinition, GeneratedRow, GenerationPlan, GenerationRequest,
        GenerationResult, ValidationStatus,
    },
    jobs::{GenerationAttempt, GenerationExecutionSpec, GenerationJob},
    strategy::GenerationStrategyAssignment,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GenerationBackendError {
    #[error("generation backend configuration is invalid: {0}")]
    Configuration(String),
    #[error("generation backend request failed: {0}")]
    Request(String),
    #[error("generation backend permanently rejected the request: {0}")]
    Rejected(String),
    #[error("generation backend rate limited the request: {message}")]
    RateLimited {
        message: String,
        retry_after_milliseconds: Option<u64>,
    },
    #[error("generation backend response was invalid: {0}")]
    InvalidResponse(String),
}

impl GenerationBackendError {
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Request(_) | Self::RateLimited { .. })
    }

    pub const fn retry_after_milliseconds(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_milliseconds,
                ..
            } => *retry_after_milliseconds,
            _ => None,
        }
    }
}

pub trait GenerationBackend: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("persistence operation failed: {0}")]
pub struct StoreError(pub String);

#[derive(Debug, Clone, Default)]
pub struct RowQuery {
    pub dataset_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    pub status: Option<ValidationStatus>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct DatasetQuery {
    pub name_contains: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Default)]
pub struct JobQuery {
    pub dataset_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
    pub state: Option<crate::jobs::JobState>,
    pub limit: u32,
    pub offset: u32,
}

pub trait DatasetStore: Send + Sync {
    fn create_dataset(&self, dataset: &DatasetDefinition) -> BoxFuture<'_, Result<(), StoreError>>;
    fn get_dataset(&self, id: Uuid)
    -> BoxFuture<'_, Result<Option<DatasetDefinition>, StoreError>>;
    fn list_datasets(&self) -> BoxFuture<'_, Result<Vec<DatasetDefinition>, StoreError>>;
    fn query_datasets(
        &self,
        query: DatasetQuery,
    ) -> BoxFuture<'_, Result<Vec<DatasetDefinition>, StoreError>>;
}

pub trait PlanStore: Send + Sync {
    fn create_plan(&self, plan: &GenerationPlan) -> BoxFuture<'_, Result<(), StoreError>>;
    fn get_plan(&self, id: Uuid) -> BoxFuture<'_, Result<Option<GenerationPlan>, StoreError>>;
}

pub trait JobStore: Send + Sync {
    fn create_job(&self, job: &GenerationJob) -> BoxFuture<'_, Result<(), StoreError>>;
    fn save_job(&self, job: &GenerationJob) -> BoxFuture<'_, Result<(), StoreError>>;
    fn get_job(&self, id: Uuid) -> BoxFuture<'_, Result<Option<GenerationJob>, StoreError>>;
    fn request_job_cancellation(&self, id: Uuid) -> BoxFuture<'_, Result<bool, StoreError>>;
    fn list_jobs(&self, query: JobQuery) -> BoxFuture<'_, Result<Vec<GenerationJob>, StoreError>>;
}

/// Durable facts for one reproducible generation execution and every provider call.
pub trait GenerationExecutionStore: Send + Sync {
    /// Atomically creates the queued job and its immutable execution specification.
    fn create_generation_execution(
        &self,
        job: &GenerationJob,
        spec: &GenerationExecutionSpec,
    ) -> BoxFuture<'_, Result<(), StoreError>>;

    fn get_generation_execution_spec(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationExecutionSpec>, StoreError>>;

    fn next_generation_attempt_sequence(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<u64, StoreError>>;

    /// Persists intent before the external provider call begins.
    fn start_generation_attempt(
        &self,
        attempt: &GenerationAttempt,
    ) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Atomically persists a terminal attempt, its rows, and reconciled job counters.
    fn finish_generation_attempt(
        &self,
        attempt: &GenerationAttempt,
        rows: &[GeneratedRow],
    ) -> BoxFuture<'_, Result<GenerationJob, StoreError>>;

    fn list_generation_attempts(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<GenerationAttempt>, StoreError>>;

    /// Marks any provider calls left open by a dead process as outcome-unknown.
    fn interrupt_open_generation_attempts(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<GenerationAttempt>, StoreError>>;

    /// Rebuilds user-visible counters from persisted rows and request attempts.
    fn reconcile_generation_job(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<GenerationJob, StoreError>>;
}

pub trait RowStore: Send + Sync {
    fn insert_rows(&self, rows: &[GeneratedRow]) -> BoxFuture<'_, Result<(), StoreError>>;
    fn list_rows(&self, query: RowQuery) -> BoxFuture<'_, Result<Vec<GeneratedRow>, StoreError>>;
    fn accepted_normalized_texts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>>;
    fn cell_counts(
        &self,
        plan_id: Uuid,
    ) -> BoxFuture<'_, Result<BTreeMap<String, CellCounts>, StoreError>>;
    fn dataset_cell_counts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<BTreeMap<String, CellCounts>, StoreError>>;
}

pub trait BackendConfigurationStore: Send + Sync {
    fn save_backend_configuration(
        &self,
        configuration: &BackendConfiguration,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    fn get_backend_configuration(
        &self,
        name: &str,
    ) -> BoxFuture<'_, Result<Option<BackendConfiguration>, StoreError>>;
}

/// Persistence boundary for the exact approved strategy context pinned to a
/// generation job. Kept separate so ordinary generation stores need not know
/// about optional strategy guidance.
pub trait GenerationStrategyStore: Send + Sync {
    fn save_generation_strategy(
        &self,
        assignment: &GenerationStrategyAssignment,
    ) -> BoxFuture<'_, Result<(), StoreError>>;

    fn get_generation_strategy(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationStrategyAssignment>, StoreError>>;
}

pub trait GenerationStore:
    DatasetStore + PlanStore + JobStore + RowStore + GenerationExecutionStore
{
}

impl<T> GenerationStore for T where
    T: DatasetStore + PlanStore + JobStore + RowStore + GenerationExecutionStore
{
}
