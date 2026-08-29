//! Persistence contracts owned by the controlled-workflow core.

use std::{future::Future, pin::Pin};

use generation_core::domain::GenerationPlan;
use thiserror::Error;
use uuid::Uuid;

use crate::allocation::InitialAllocationRecord;
use crate::benchmark::{AcceptanceAssessment, AcceptanceState, BenchmarkSuite, BenchmarkSuiteKind};
use crate::contamination::{ContaminationOverride, ContaminationReport, ContaminationStatus};
use crate::governance::{CohortRoleDecision, EvaluationCohort, EvidenceExposure, ExposurePurpose};

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

#[derive(Debug, Clone, Copy)]
pub struct CohortQuery {
    pub snapshot_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ExposureQuery {
    pub cohort_id: Uuid,
    pub purpose: Option<ExposurePurpose>,
    pub limit: u32,
    pub offset: u32,
}

pub trait GovernanceStore: Send + Sync {
    /// Atomically creates a cohort and its first append-only role decision.
    fn create_cohort(
        &self,
        cohort: &EvaluationCohort,
        initial_role: &CohortRoleDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_cohort(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationCohort>, WorkflowStoreError>>;

    fn query_cohorts(
        &self,
        query: CohortQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationCohort>, WorkflowStoreError>>;

    fn get_current_cohort_role(
        &self,
        cohort_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CohortRoleDecision>, WorkflowStoreError>>;

    fn append_cohort_role(
        &self,
        decision: &CohortRoleDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn list_cohort_role_history(
        &self,
        cohort_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CohortRoleDecision>, WorkflowStoreError>>;

    /// Atomically records an exposure and any mandatory sealed-cohort resolution.
    fn append_exposure(
        &self,
        exposure: &EvidenceExposure,
        resolution: Option<&CohortRoleDecision>,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn query_exposures(
        &self,
        query: ExposureQuery,
    ) -> BoxFuture<'_, Result<Vec<EvidenceExposure>, WorkflowStoreError>>;
}

#[derive(Debug, Clone, Copy)]
pub struct ContaminationQuery {
    pub cohort_id: Option<Uuid>,
    pub status: Option<ContaminationStatus>,
    pub limit: u32,
    pub offset: u32,
}

pub trait ContaminationStore: Send + Sync {
    fn create_contamination_report(
        &self,
        report: &ContaminationReport,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_contamination_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ContaminationReport>, WorkflowStoreError>>;

    fn query_contamination_reports(
        &self,
        query: ContaminationQuery,
    ) -> BoxFuture<'_, Result<Vec<ContaminationReport>, WorkflowStoreError>>;

    fn append_contamination_override(
        &self,
        value: &ContaminationOverride,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_contamination_override(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ContaminationOverride>, WorkflowStoreError>>;
}

#[derive(Debug, Clone, Copy)]
pub struct BenchmarkSuiteQuery {
    pub kind: Option<BenchmarkSuiteKind>,
    pub cohort_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct AcceptanceAssessmentQuery {
    pub suite_id: Option<Uuid>,
    pub checkpoint_id: Option<Uuid>,
    pub state: Option<AcceptanceState>,
    pub limit: u32,
    pub offset: u32,
}

pub trait BenchmarkStore: Send + Sync {
    fn create_benchmark_suite(
        &self,
        suite: &BenchmarkSuite,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_benchmark_suite(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkSuite>, WorkflowStoreError>>;

    fn query_benchmark_suites(
        &self,
        query: BenchmarkSuiteQuery,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkSuite>, WorkflowStoreError>>;

    fn create_acceptance_assessment(
        &self,
        assessment: &AcceptanceAssessment,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_acceptance_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AcceptanceAssessment>, WorkflowStoreError>>;

    fn query_acceptance_assessments(
        &self,
        query: AcceptanceAssessmentQuery,
    ) -> BoxFuture<'_, Result<Vec<AcceptanceAssessment>, WorkflowStoreError>>;
}
