//! Persistence contracts owned by the controlled-workflow core.

use std::{future::Future, pin::Pin};

use generation_core::domain::GenerationPlan;
use thiserror::Error;
use uuid::Uuid;

use crate::advisor::AdvisoryAssessment;
use crate::allocation::InitialAllocationRecord;
use crate::approval::WorkflowApprovalDecision;
use crate::benchmark::{AcceptanceAssessment, AcceptanceState, BenchmarkSuite, BenchmarkSuiteKind};
use crate::contamination::{ContaminationOverride, ContaminationReport, ContaminationStatus};
use crate::governance::{CohortRoleDecision, EvaluationCohort, EvidenceExposure, ExposurePurpose};
use crate::stop::StopDecision;
use crate::workflow::{WorkflowDefinition, WorkflowRun, WorkflowRunState, WorkflowStageAttempt};

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
pub struct AdvisoryAssessmentQuery {
    pub workflow_run_id: Option<Uuid>,
    pub analysis_report_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

pub trait AdvisorStore: Send + Sync {
    fn create_advisory_assessment(
        &self,
        assessment: &AdvisoryAssessment,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_advisory_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AdvisoryAssessment>, WorkflowStoreError>>;

    fn query_advisory_assessments(
        &self,
        query: AdvisoryAssessmentQuery,
    ) -> BoxFuture<'_, Result<Vec<AdvisoryAssessment>, WorkflowStoreError>>;
}

pub trait WorkflowApprovalStore: Send + Sync {
    fn create_workflow_approval(
        &self,
        decision: &WorkflowApprovalDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_workflow_approval(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowApprovalDecision>, WorkflowStoreError>>;

    fn get_iteration_workflow_approval(
        &self,
        workflow_run_id: Uuid,
        workflow_iteration: u32,
    ) -> BoxFuture<'_, Result<Option<WorkflowApprovalDecision>, WorkflowStoreError>>;
}

pub trait StopDecisionStore: Send + Sync {
    fn create_stop_decision(
        &self,
        decision: &StopDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_stop_decision(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<StopDecision>, WorkflowStoreError>>;

    fn get_iteration_stop_decision(
        &self,
        workflow_run_id: Uuid,
        workflow_iteration: u32,
    ) -> BoxFuture<'_, Result<Option<StopDecision>, WorkflowStoreError>>;
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

#[derive(Debug, Clone, Copy)]
pub struct WorkflowDefinitionQuery {
    pub dataset_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkflowRunQuery {
    pub definition_id: Option<Uuid>,
    pub state: Option<WorkflowRunState>,
    pub limit: u32,
    pub offset: u32,
}

pub trait WorkflowRunStore: Send + Sync {
    fn create_workflow_definition(
        &self,
        definition: &WorkflowDefinition,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_workflow_definition(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowDefinition>, WorkflowStoreError>>;

    fn query_workflow_definitions(
        &self,
        query: WorkflowDefinitionQuery,
    ) -> BoxFuture<'_, Result<Vec<WorkflowDefinition>, WorkflowStoreError>>;

    /// Atomically creates a run and its first running stage attempt.
    fn create_workflow_run(
        &self,
        run: &WorkflowRun,
        initial_attempt: &WorkflowStageAttempt,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn get_workflow_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowRun>, WorkflowStoreError>>;

    fn query_workflow_runs(
        &self,
        query: WorkflowRunQuery,
    ) -> BoxFuture<'_, Result<Vec<WorkflowRun>, WorkflowStoreError>>;

    /// Optimistically updates the run and appends exactly one attempt event.
    fn commit_workflow_attempt(
        &self,
        run: &WorkflowRun,
        attempt: &WorkflowStageAttempt,
        expected_previous_attempt_id: Uuid,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;

    fn list_workflow_attempts(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowStageAttempt>, WorkflowStoreError>>;

    fn save_workflow_run(
        &self,
        run: &WorkflowRun,
        expected_latest_attempt_id: Option<Uuid>,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>>;
}
