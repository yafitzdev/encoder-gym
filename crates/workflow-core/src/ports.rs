//! Persistence contracts owned by the controlled-workflow core.

use std::{future::Future, pin::Pin};

use generation_core::domain::GenerationPlan;
use thiserror::Error;
use uuid::Uuid;

use crate::allocation::InitialAllocationRecord;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub struct InitialAllocationQuery {
    pub dataset_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("workflow persistence operation failed: {0}")]
pub struct WorkflowStoreError(pub String);

pub trait InitialAllocationStore: Send + Sync {
    /// Atomically persists one immutable allocation record and its ordinary Slice 1 plan.
    fn create_initial_allocation(
        &self,
        allocation: &InitialAllocationRecord,
        plan: &GenerationPlan,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_initial_allocation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<InitialAllocationRecord>, WorkflowStoreError>>;

    fn query_initial_allocations(
        &self,
        query: InitialAllocationQuery,
    ) -> BoxFuture<'_, Result<Vec<InitialAllocationRecord>, WorkflowStoreError>>;
}
