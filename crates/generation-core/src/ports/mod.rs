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
    jobs::GenerationJob,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GenerationBackendError {
    #[error("generation backend configuration is invalid: {0}")]
    Configuration(String),
    #[error("generation backend request failed: {0}")]
    Request(String),
    #[error("generation backend response was invalid: {0}")]
    InvalidResponse(String),
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

pub trait GenerationStore: DatasetStore + PlanStore + JobStore + RowStore {}

impl<T> GenerationStore for T where T: DatasetStore + PlanStore + JobStore + RowStore {}
