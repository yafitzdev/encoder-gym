//! Finite, budgeted, append-only workflow lifecycle.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::advisor::AdvisorConfiguration;
use crate::allocation::{InitialAllocationPolicy, InitialCellConstraint};
use crate::benchmark_bundle::{BenchmarkBundleBinding, BenchmarkBundleError};
use crate::benchmark_qualification::{
    ApprovedBenchmarkQualificationBinding, BenchmarkQualificationError,
};
use analysis_core::protocol::AnalysisProtocol;
use dataset_quality_core::policy::{
    AuditMode, EvaluatorEgressPolicy, QualityPolicy, QualityPolicyPresetControls, QualityPreset,
};
use optimization_core::protocol::OptimizationProtocol;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinitionRequest {
    pub name: String,
    pub dataset_id: Uuid,
    pub project_configuration_id: Uuid,
    pub project_configuration_fingerprint: String,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    #[serde(default)]
    pub sealed_suite_id: Option<Uuid>,
    #[serde(default)]
    pub sealed_suite_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_bundle: Option<BenchmarkBundleBinding>,
    pub initial_allocation: WorkflowInitialAllocation,
    #[serde(default)]
    pub analysis_protocol: Option<AnalysisProtocol>,
    #[serde(default)]
    pub optimization_protocol: Option<OptimizationProtocol>,
    #[serde(default)]
    pub advisor: Option<AdvisorConfiguration>,
    #[serde(default)]
    pub training_iteration_policy: Option<TrainingIterationPolicy>,
    /// Optional immutable qualification gate. Definitions which omit it keep
    /// the legacy generation -> snapshot path unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_gate: Option<WorkflowQualityGateRequest>,
    pub governance: IterationGovernance,
    pub budget: WorkflowBudget,
    pub policy: WorkflowPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowInitialAllocation {
    pub total_rows: u64,
    #[serde(default)]
    pub reserved_rows: u64,
    pub policy: InitialAllocationPolicy,
    #[serde(default)]
    pub constraints: Vec<InitialCellConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum IterationGovernance {
    ReviewEachIteration,
    PreauthorizedBounded { envelope: ApprovalEnvelope },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalEnvelope {
    pub maximum_iterations: u32,
    pub maximum_additional_rows: u64,
    pub maximum_generation_requests: u64,
    pub maximum_advisor_calls: u32,
    #[serde(default)]
    pub maximum_advisor_tokens: Option<u64>,
    pub permitted_generation_backend: String,
    pub permitted_generation_model: String,
    pub permitted_training_backend: String,
    #[serde(default)]
    pub permitted_training_configuration_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowBudget {
    pub maximum_iterations: u32,
    pub maximum_initial_rows: u64,
    pub maximum_cumulative_rows: u64,
    pub maximum_generation_attempts: u64,
    pub maximum_generation_requests: u64,
    pub maximum_advisor_calls: u32,
    #[serde(default)]
    pub maximum_advisor_tokens: Option<u64>,
    pub maximum_stage_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPolicy {
    pub minimum_improvement: f64,
    pub maximum_tolerated_regression: f64,
    pub stop_on_inconclusive: bool,
    pub stop_on_invalid: bool,
    pub enable_advisor: bool,
    pub require_fresh_development_cohort_after_iterations: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingIterationPolicy {
    /// Train every candidate independently from the configured base model.
    Fresh,
}

/// Whether authenticity evidence is part of the immutable workflow quality
/// policy. `Required` deliberately fails closed if no approved context can be
/// pinned when the audit is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowQualityAuthenticity {
    Off,
    Required,
}

/// The only evaluator adapter that governed workflows can currently execute
/// unattended without persisting a credential selector. Unsupported backend
/// identities are rejected when the immutable definition is constructed.
pub const WORKFLOW_QUALITY_EVALUATOR_BACKEND: &str = "deterministic-fake";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowQualityGateRequest {
    pub preset: QualityPreset,
    pub egress_policy: EvaluatorEgressPolicy,
    pub authenticity: WorkflowQualityAuthenticity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microusd: Option<u64>,
    /// Provider-neutral adapter identity. The application rejects adapters it
    /// cannot reconstruct exactly; no credentials are persisted here.
    pub evaluator_backend: String,
    pub evaluator_protocol_version: String,
}

/// Fully resolved, fingerprinted quality policy pinned into a workflow
/// definition. Runtime code consumes these exact values and never
/// reinterprets the preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowQualityGate {
    pub policy: QualityPolicy,
    pub authenticity: WorkflowQualityAuthenticity,
    pub evaluator_backend: String,
    pub evaluator_protocol_version: String,
}

impl WorkflowQualityGate {
    fn resolve(request: WorkflowQualityGateRequest) -> Result<Self, WorkflowError> {
        let evaluator_backend = required(request.evaluator_backend, "quality evaluator backend")?;
        let evaluator_protocol_version = required(
            request.evaluator_protocol_version,
            "quality evaluator protocol version",
        )?;
        let policy = request
            .preset
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: request.egress_policy,
                evaluate_authenticity: request.authenticity
                    == WorkflowQualityAuthenticity::Required,
                maximum_cost_microusd: request.maximum_cost_microusd,
            })
            .map_err(|_| WorkflowError::InvalidQualityGate)?;
        let value = Self {
            policy,
            authenticity: request.authenticity,
            evaluator_backend,
            evaluator_protocol_version,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), WorkflowError> {
        self.policy
            .verify_integrity()
            .map_err(|_| WorkflowError::InvalidQualityGate)?;
        if self.evaluator_backend.trim().is_empty()
            || self.evaluator_backend != WORKFLOW_QUALITY_EVALUATOR_BACKEND
            || self.evaluator_protocol_version.trim().is_empty()
            || self.policy.audit_mode != AuditMode::FullPopulation
            || (self.authenticity == WorkflowQualityAuthenticity::Required)
                != self.policy.thresholds.minimum_authenticity_score.is_some()
        {
            return Err(WorkflowError::InvalidQualityGate);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: Uuid,
    pub name: String,
    pub dataset_id: Uuid,
    pub project_configuration_id: Uuid,
    pub project_configuration_fingerprint: String,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Option<Uuid>,
    pub sealed_suite_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_bundle: Option<BenchmarkBundleBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_qualification: Option<ApprovedBenchmarkQualificationBinding>,
    pub initial_allocation: WorkflowInitialAllocation,
    #[serde(default)]
    pub analysis_protocol: Option<AnalysisProtocol>,
    #[serde(default)]
    pub optimization_protocol: Option<OptimizationProtocol>,
    #[serde(default)]
    pub advisor: Option<AdvisorConfiguration>,
    #[serde(default)]
    pub training_iteration_policy: Option<TrainingIterationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_gate: Option<WorkflowQualityGate>,
    pub governance: IterationGovernance,
    pub budget: WorkflowBudget,
    pub policy: WorkflowPolicy,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl WorkflowDefinition {
    pub fn new(request: WorkflowDefinitionRequest) -> Result<Self, WorkflowError> {
        Self::new_with_qualification(request, None)
    }

    pub fn new_with_qualification(
        request: WorkflowDefinitionRequest,
        benchmark_qualification: Option<ApprovedBenchmarkQualificationBinding>,
    ) -> Result<Self, WorkflowError> {
        validate_request(&request)?;
        validate_benchmark_qualification_binding(
            benchmark_qualification.as_ref(),
            request.benchmark_bundle.as_ref(),
        )?;
        let analysis_protocol = request.analysis_protocol.unwrap_or_default();
        analysis_protocol
            .validate()
            .map_err(|_| WorkflowError::InvalidAnalysisProtocol)?;
        let optimization_protocol = match request.optimization_protocol {
            Some(protocol) => protocol
                .normalize()
                .map_err(|_| WorkflowError::InvalidOptimizationProtocol)?,
            None => {
                let available = request
                    .budget
                    .maximum_cumulative_rows
                    .saturating_sub(request.initial_allocation.total_rows)
                    .min(request.initial_allocation.reserved_rows);
                let available = u32::try_from(available)
                    .map_err(|_| WorkflowError::InvalidOptimizationProtocol)?;
                if available == 0 {
                    return Err(WorkflowError::InvalidOptimizationProtocol);
                }
                OptimizationProtocol::legacy(available, 1)
                    .normalize()
                    .map_err(|_| WorkflowError::InvalidOptimizationProtocol)?
            }
        };
        if request.policy.enable_advisor != request.advisor.is_some() {
            return Err(WorkflowError::InvalidAdvisorConfiguration);
        }
        if let Some(advisor) = &request.advisor {
            advisor
                .validate()
                .map_err(|_| WorkflowError::InvalidAdvisorConfiguration)?;
        }
        let quality_gate = request
            .quality_gate
            .map(WorkflowQualityGate::resolve)
            .transpose()?;
        let mut value = Self {
            id: Uuid::new_v4(),
            name: required(request.name, "workflow name")?,
            dataset_id: request.dataset_id,
            project_configuration_id: request.project_configuration_id,
            project_configuration_fingerprint: required(
                request.project_configuration_fingerprint,
                "project configuration fingerprint",
            )?,
            development_suite_id: request.development_suite_id,
            development_suite_fingerprint: required(
                request.development_suite_fingerprint,
                "development suite fingerprint",
            )?,
            sealed_suite_id: request.sealed_suite_id,
            sealed_suite_fingerprint: request.sealed_suite_fingerprint,
            benchmark_bundle: request.benchmark_bundle,
            benchmark_qualification,
            initial_allocation: request.initial_allocation,
            analysis_protocol: Some(analysis_protocol),
            optimization_protocol: Some(optimization_protocol),
            advisor: request.advisor,
            training_iteration_policy: Some(
                request
                    .training_iteration_policy
                    .unwrap_or(TrainingIterationPolicy::Fresh),
            ),
            quality_gate,
            governance: request.governance,
            budget: request.budget,
            policy: request.policy,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = definition_fingerprint(&value)?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, WorkflowError> {
        definition_fingerprint(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Queued,
    Running,
    AwaitingApproval,
    AwaitingUser,
    DevelopmentComplete,
    Completed,
    Failed,
    Cancelled,
    Exhausted,
    Inconclusive,
}

impl WorkflowRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Exhausted | Self::Inconclusive
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStage {
    InitialAllocation,
    Generation,
    QualityAudit,
    CurationReview,
    CurationApproval,
    Snapshot,
    Training,
    DevelopmentEvaluation,
    AcceptanceAssessment,
    ErrorAnalysis,
    Advisor,
    OptimizationProposal,
    Approval,
    ProposalApplication,
    DatasetDiffGeneration,
    IterationQualityAudit,
    IterationCurationReview,
    IterationCurationApproval,
    IterationSnapshot,
    IterationTraining,
    IterationEvaluation,
    Comparison,
    FollowupAnalysis,
    StopDecision,
    SealedEvaluation,
    Promotion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowBudgetUsage {
    pub iterations: u32,
    pub accepted_rows: u64,
    pub generation_attempts: u64,
    pub generation_requests: u64,
    pub advisor_calls: u32,
    pub advisor_tokens: u64,
}

impl WorkflowBudgetUsage {
    pub const fn zero() -> Self {
        Self {
            iterations: 0,
            accepted_rows: 0,
            generation_attempts: 0,
            generation_requests: 0,
            advisor_calls: 0,
            advisor_tokens: 0,
        }
    }

    pub fn validate_against(&self, budget: &WorkflowBudget) -> Result<(), WorkflowError> {
        if self.iterations > budget.maximum_iterations
            || self.accepted_rows > budget.maximum_cumulative_rows
            || self.generation_attempts > budget.maximum_generation_attempts
            || self.generation_requests > budget.maximum_generation_requests
            || self.advisor_calls > budget.maximum_advisor_calls
            || budget
                .maximum_advisor_tokens
                .is_some_and(|maximum| self.advisor_tokens > maximum)
        {
            return Err(WorkflowError::BudgetExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub definition_id: Uuid,
    pub definition_fingerprint: String,
    pub state: WorkflowRunState,
    pub current_stage: Option<WorkflowStage>,
    pub iteration: u32,
    pub usage: WorkflowBudgetUsage,
    pub latest_attempt_id: Option<Uuid>,
    pub latest_attempt_fingerprint: Option<String>,
    pub cancel_requested: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowRun {
    pub fn queued(definition: &WorkflowDefinition) -> Result<Self, WorkflowError> {
        validate_executable_definition(definition)?;
        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4(),
            definition_id: definition.id,
            definition_fingerprint: definition.fingerprint.clone(),
            state: WorkflowRunState::Queued,
            current_stage: None,
            iteration: 0,
            usage: WorkflowBudgetUsage::zero(),
            latest_attempt_id: None,
            latest_attempt_fingerprint: None,
            cancel_requested: false,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn request_cancel(&mut self) -> Result<(), WorkflowError> {
        if self.state.is_terminal() || self.state == WorkflowRunState::DevelopmentComplete {
            return Err(WorkflowError::TerminalRun);
        }
        self.cancel_requested = true;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Reactivates the exact running attempt that process recovery placed in
    /// `awaiting_user`. Paused approval/user stages cannot use this path.
    pub fn resume_interrupted(
        &mut self,
        attempt: &WorkflowStageAttempt,
    ) -> Result<(), WorkflowError> {
        if self.state != WorkflowRunState::AwaitingUser
            || self.cancel_requested
            || attempt.state != StageAttemptState::Running
            || attempt.workflow_run_id != self.id
            || self.latest_attempt_id != Some(attempt.id)
            || self.latest_attempt_fingerprint.as_deref() != Some(&attempt.fingerprint)
            || self.current_stage != Some(attempt.stage)
        {
            return Err(WorkflowError::NotInterrupted);
        }
        self.state = WorkflowRunState::Running;
        self.updated_at = Utc::now();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageAttemptState {
    Running,
    Completed,
    Failed,
    Cancelled,
    AwaitingApproval,
    AwaitingUser,
    Inconclusive,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowArtifactLink {
    pub kind: String,
    pub artifact_id: Uuid,
    pub artifact_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStageAttempt {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub sequence: u32,
    pub iteration: u32,
    pub stage: WorkflowStage,
    pub attempt: u32,
    pub state: StageAttemptState,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub reason: Option<String>,
    pub retryable: bool,
    pub artifacts: Vec<WorkflowArtifactLink>,
    pub usage_after: WorkflowBudgetUsage,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
}

impl WorkflowStageAttempt {
    pub fn start(
        definition: &WorkflowDefinition,
        run: &mut WorkflowRun,
        stage: WorkflowStage,
        previous: Option<&Self>,
        attempt: u32,
    ) -> Result<Self, WorkflowError> {
        validate_executable_definition(definition)?;
        validate_run_identity(definition, run)?;
        if attempt == 0 || attempt > definition.budget.maximum_stage_attempts {
            return Err(WorkflowError::StageAttemptsExceeded);
        }
        if run.cancel_requested {
            return Err(WorkflowError::CancellationRequested);
        }
        validate_stage_start(definition, run, stage, previous)?;
        let now = Utc::now();
        let mut value = Self {
            id: Uuid::new_v4(),
            workflow_run_id: run.id,
            sequence: previous.map_or(0, |value| value.sequence + 1),
            iteration: run.iteration,
            stage,
            attempt,
            state: StageAttemptState::Running,
            predecessor_id: previous.map(|value| value.id),
            predecessor_fingerprint: previous.map(|value| value.fingerprint.clone()),
            reason: None,
            retryable: false,
            artifacts: Vec::new(),
            usage_after: run.usage.clone(),
            started_at: now,
            finished_at: None,
            fingerprint: String::new(),
        };
        value.fingerprint = attempt_fingerprint(&value)?;
        run.state = WorkflowRunState::Running;
        run.current_stage = Some(stage);
        run.latest_attempt_id = Some(value.id);
        run.latest_attempt_fingerprint = Some(value.fingerprint.clone());
        run.updated_at = now;
        Ok(value)
    }

    pub fn finish(
        &self,
        definition: &WorkflowDefinition,
        run: &mut WorkflowRun,
        outcome: StageOutcome,
    ) -> Result<Self, WorkflowError> {
        validate_definition(definition)?;
        validate_run_identity(definition, run)?;
        let cancelling_pause = outcome.state == StageAttemptState::Cancelled
            && run.cancel_requested
            && (matches!(
                self.state,
                StageAttemptState::AwaitingApproval | StageAttemptState::AwaitingUser
            ) || (self.state == StageAttemptState::Failed && self.retryable));
        if (self.state != StageAttemptState::Running && !cancelling_pause)
            || run.latest_attempt_id != Some(self.id)
            || run.latest_attempt_fingerprint.as_deref() != Some(&self.fingerprint)
        {
            return Err(WorkflowError::StaleAttempt);
        }
        outcome.usage_after.validate_against(&definition.budget)?;
        let state = outcome.state;
        if state == StageAttemptState::Running {
            return Err(WorkflowError::InvalidOutcome);
        }
        let now = Utc::now();
        let mut finished = Self {
            id: Uuid::new_v4(),
            workflow_run_id: self.workflow_run_id,
            sequence: self.sequence + 1,
            iteration: self.iteration,
            stage: self.stage,
            attempt: self.attempt,
            state,
            predecessor_id: Some(self.id),
            predecessor_fingerprint: Some(self.fingerprint.clone()),
            reason: outcome.reason.map(|value| value.trim().to_owned()),
            retryable: outcome.retryable,
            artifacts: outcome.artifacts,
            usage_after: outcome.usage_after,
            started_at: self.started_at,
            finished_at: Some(now),
            fingerprint: String::new(),
        };
        if finished.reason.as_deref() == Some("") {
            return Err(WorkflowError::Empty("stage outcome reason"));
        }
        finished.fingerprint = attempt_fingerprint(&finished)?;
        apply_outcome(definition, run, &finished)?;
        run.latest_attempt_id = Some(finished.id);
        run.latest_attempt_fingerprint = Some(finished.fingerprint.clone());
        run.updated_at = now;
        Ok(finished)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, WorkflowError> {
        attempt_fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageOutcome {
    pub state: StageAttemptState,
    pub reason: Option<String>,
    pub retryable: bool,
    pub artifacts: Vec<WorkflowArtifactLink>,
    pub usage_after: WorkflowBudgetUsage,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum WorkflowError {
    #[error("{0} must not be empty")]
    Empty(&'static str),
    #[error("workflow budgets must be finite and internally consistent")]
    InvalidBudget,
    #[error("workflow policy contains invalid finite bounds")]
    InvalidPolicy,
    #[error("workflow analysis protocol is invalid")]
    InvalidAnalysisProtocol,
    #[error("workflow optimization protocol is invalid or exceeds the iteration budget")]
    InvalidOptimizationProtocol,
    #[error("workflow advisor configuration does not match the workflow policy")]
    InvalidAdvisorConfiguration,
    #[error("workflow quality-gate configuration is invalid")]
    InvalidQualityGate,
    #[error("sealed suite id and fingerprint must either both be present or both absent")]
    SealedSuitePair,
    #[error("workflow definition requires a benchmark bundle binding")]
    BenchmarkBundleRequired,
    #[error("workflow benchmark bundle binding is invalid")]
    InvalidBenchmarkBundleBinding,
    #[error("workflow benchmark bundle fingerprint does not reproduce")]
    BenchmarkBundleFingerprint,
    #[error("workflow benchmark suite references do not match the benchmark bundle binding")]
    BenchmarkBundleSuiteMismatch,
    #[error("workflow benchmark qualification binding is invalid")]
    InvalidBenchmarkQualificationBinding,
    #[error("workflow benchmark qualification does not match the benchmark bundle")]
    BenchmarkQualificationBundleMismatch,
    #[error("preauthorization exceeds the workflow budget or has invalid permissions")]
    InvalidEnvelope,
    #[error("workflow definition fingerprint mismatch")]
    DefinitionFingerprint,
    #[error("workflow run references a different definition")]
    DefinitionMismatch,
    #[error("workflow run is terminal")]
    TerminalRun,
    #[error("workflow cancellation was requested")]
    CancellationRequested,
    #[error("workflow run is not paused on the exact interrupted running attempt")]
    NotInterrupted,
    #[error("stage {next:?} cannot follow {previous:?} in state {state:?}")]
    IllegalStage {
        previous: Option<WorkflowStage>,
        next: WorkflowStage,
        state: WorkflowRunState,
    },
    #[error("stage attempt ceiling exceeded")]
    StageAttemptsExceeded,
    #[error("stage attempt is stale or does not own the run")]
    StaleAttempt,
    #[error("invalid stage outcome")]
    InvalidOutcome,
    #[error("workflow budget exceeded")]
    BudgetExceeded,
    #[error("artifact fingerprint failed: {0}")]
    Fingerprint(String),
}

fn validate_request(request: &WorkflowDefinitionRequest) -> Result<(), WorkflowError> {
    if request.sealed_suite_id.is_some() != request.sealed_suite_fingerprint.is_some() {
        return Err(WorkflowError::SealedSuitePair);
    }
    let benchmark_bundle = request
        .benchmark_bundle
        .as_ref()
        .ok_or(WorkflowError::BenchmarkBundleRequired)?;
    validate_benchmark_bundle_binding(benchmark_bundle)?;
    validate_benchmark_suite_binding(
        request.development_suite_id,
        &request.development_suite_fingerprint,
        request.sealed_suite_id,
        request.sealed_suite_fingerprint.as_deref(),
        benchmark_bundle,
    )?;
    let budget = &request.budget;
    if budget.maximum_iterations == 0
        || budget.maximum_initial_rows == 0
        || budget.maximum_cumulative_rows < budget.maximum_initial_rows
        || budget.maximum_generation_attempts == 0
        || budget.maximum_generation_requests == 0
        || budget.maximum_stage_attempts == 0
        || request.initial_allocation.total_rows == 0
        || request.initial_allocation.total_rows > budget.maximum_initial_rows
        || request.initial_allocation.reserved_rows > request.initial_allocation.total_rows
    {
        return Err(WorkflowError::InvalidBudget);
    }
    if !request.policy.minimum_improvement.is_finite()
        || request.policy.minimum_improvement < 0.0
        || !request.policy.maximum_tolerated_regression.is_finite()
        || request.policy.maximum_tolerated_regression < 0.0
    {
        return Err(WorkflowError::InvalidPolicy);
    }
    if let Some(protocol) = &request.optimization_protocol {
        let normalized = protocol
            .clone()
            .normalize()
            .map_err(|_| WorkflowError::InvalidOptimizationProtocol)?;
        let available = request
            .budget
            .maximum_cumulative_rows
            .saturating_sub(request.initial_allocation.total_rows);
        if u64::from(normalized.additional_example_budget) > available {
            return Err(WorkflowError::InvalidOptimizationProtocol);
        }
    }
    if let IterationGovernance::PreauthorizedBounded { envelope } = &request.governance {
        if envelope.maximum_iterations == 0
            || envelope.maximum_iterations > budget.maximum_iterations
            || envelope.maximum_additional_rows
                > budget
                    .maximum_cumulative_rows
                    .saturating_sub(request.initial_allocation.total_rows)
            || envelope.maximum_generation_requests > budget.maximum_generation_requests
            || envelope.maximum_advisor_calls > budget.maximum_advisor_calls
            || envelope.maximum_advisor_tokens.is_some_and(|tokens| {
                budget
                    .maximum_advisor_tokens
                    .is_none_or(|maximum| tokens > maximum)
            })
            || [
                &envelope.permitted_generation_backend,
                &envelope.permitted_generation_model,
                &envelope.permitted_training_backend,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(WorkflowError::InvalidEnvelope);
        }
    }
    Ok(())
}

fn validate_definition(value: &WorkflowDefinition) -> Result<(), WorkflowError> {
    if let Some(gate) = &value.quality_gate {
        gate.validate()?;
    }
    if value.reproduce_fingerprint()? != value.fingerprint {
        return Err(WorkflowError::DefinitionFingerprint);
    }
    if let Some(benchmark_bundle) = &value.benchmark_bundle {
        validate_benchmark_bundle_binding(benchmark_bundle)?;
        validate_benchmark_suite_binding(
            value.development_suite_id,
            &value.development_suite_fingerprint,
            value.sealed_suite_id,
            value.sealed_suite_fingerprint.as_deref(),
            benchmark_bundle,
        )?;
    }
    validate_benchmark_qualification_binding(
        value.benchmark_qualification.as_ref(),
        value.benchmark_bundle.as_ref(),
    )?;
    Ok(())
}

fn validate_benchmark_qualification_binding(
    qualification: Option<&ApprovedBenchmarkQualificationBinding>,
    bundle: Option<&BenchmarkBundleBinding>,
) -> Result<(), WorkflowError> {
    let Some(qualification) = qualification else {
        return Ok(());
    };
    qualification
        .validate_shape()
        .map_err(|error| match error {
            BenchmarkQualificationError::BindingMismatch => {
                WorkflowError::InvalidBenchmarkQualificationBinding
            }
            other => WorkflowError::Fingerprint(other.to_string()),
        })?;
    let Some(bundle) = bundle else {
        return Err(WorkflowError::BenchmarkBundleRequired);
    };
    if qualification.benchmark_bundle_id != bundle.bundle_id
        || qualification.benchmark_bundle_fingerprint != bundle.bundle_fingerprint
    {
        return Err(WorkflowError::BenchmarkQualificationBundleMismatch);
    }
    Ok(())
}

fn validate_executable_definition(value: &WorkflowDefinition) -> Result<(), WorkflowError> {
    validate_definition(value)?;
    if value.benchmark_bundle.is_none() {
        return Err(WorkflowError::BenchmarkBundleRequired);
    }
    Ok(())
}

fn validate_benchmark_bundle_binding(
    binding: &BenchmarkBundleBinding,
) -> Result<(), WorkflowError> {
    let sealed_pair_is_complete =
        binding.sealed_suite_id.is_some() == binding.sealed_suite_fingerprint.is_some();
    let fingerprints = [
        binding.bundle_fingerprint.as_str(),
        binding.development_suite_fingerprint.as_str(),
        binding.contamination_report_fingerprint.as_str(),
    ];
    if binding.bundle_id.is_nil()
        || binding.development_suite_id.is_nil()
        || binding.contamination_report_id.is_nil()
        || !sealed_pair_is_complete
        || binding.sealed_suite_id.is_some_and(|id| id.is_nil())
        || fingerprints
            .into_iter()
            .chain(binding.sealed_suite_fingerprint.as_deref())
            .any(|value| value.trim().is_empty() || value.trim() != value)
        || binding
            .sealed_suite_id
            .is_some_and(|sealed_id| sealed_id == binding.development_suite_id)
        || binding
            .sealed_suite_fingerprint
            .as_ref()
            .is_some_and(|sealed| sealed == &binding.development_suite_fingerprint)
    {
        return Err(WorkflowError::InvalidBenchmarkBundleBinding);
    }
    binding
        .validate_fingerprint()
        .map_err(|error| match error {
            BenchmarkBundleError::BundleFingerprintMismatch => {
                WorkflowError::BenchmarkBundleFingerprint
            }
            other => WorkflowError::Fingerprint(other.to_string()),
        })?;
    Ok(())
}

fn validate_benchmark_suite_binding(
    development_suite_id: Uuid,
    development_suite_fingerprint: &str,
    sealed_suite_id: Option<Uuid>,
    sealed_suite_fingerprint: Option<&str>,
    binding: &BenchmarkBundleBinding,
) -> Result<(), WorkflowError> {
    if development_suite_id != binding.development_suite_id
        || development_suite_fingerprint != binding.development_suite_fingerprint
        || sealed_suite_id != binding.sealed_suite_id
        || sealed_suite_fingerprint != binding.sealed_suite_fingerprint.as_deref()
    {
        return Err(WorkflowError::BenchmarkBundleSuiteMismatch);
    }
    Ok(())
}

fn validate_run_identity(
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
) -> Result<(), WorkflowError> {
    if run.definition_id != definition.id || run.definition_fingerprint != definition.fingerprint {
        return Err(WorkflowError::DefinitionMismatch);
    }
    if run.state.is_terminal() {
        return Err(WorkflowError::TerminalRun);
    }
    Ok(())
}

fn validate_stage_start(
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    stage: WorkflowStage,
    previous: Option<&WorkflowStageAttempt>,
) -> Result<(), WorkflowError> {
    let previous_stage = previous.map(|value| value.stage);
    let allowed = if previous.is_none() {
        run.state == WorkflowRunState::Queued && stage == WorkflowStage::InitialAllocation
    } else if run.state == WorkflowRunState::DevelopmentComplete {
        stage == WorkflowStage::SealedEvaluation && definition.sealed_suite_id.is_some()
    } else {
        previous.is_some_and(|value| {
            value.state == StageAttemptState::Completed
                && legal_successors(
                    value.stage,
                    definition.policy.enable_advisor,
                    definition.quality_gate.is_some(),
                )
                .contains(&stage)
        }) || previous.is_some_and(|value| {
            value.state == StageAttemptState::Failed && value.retryable && value.stage == stage
        }) || (run.state == WorkflowRunState::AwaitingApproval
            && previous_stage == Some(WorkflowStage::Approval)
            && stage == WorkflowStage::ProposalApplication)
            || (run.state == WorkflowRunState::AwaitingUser
                && previous.is_some_and(|value| value.state == StageAttemptState::AwaitingUser)
                && matches!(
                    (previous_stage, stage),
                    (
                        Some(WorkflowStage::CurationReview),
                        WorkflowStage::CurationApproval
                    ) | (
                        Some(WorkflowStage::CurationReview),
                        WorkflowStage::CurationReview
                    ) | (
                        Some(WorkflowStage::IterationCurationReview),
                        WorkflowStage::IterationCurationApproval
                    ) | (
                        Some(WorkflowStage::IterationCurationReview),
                        WorkflowStage::IterationCurationReview
                    )
                ))
    };
    if !allowed {
        return Err(WorkflowError::IllegalStage {
            previous: previous_stage,
            next: stage,
            state: run.state,
        });
    }
    Ok(())
}

fn legal_successors(
    stage: WorkflowStage,
    advisor: bool,
    quality_gate: bool,
) -> &'static [WorkflowStage] {
    use WorkflowStage::*;
    match stage {
        InitialAllocation => &[Generation],
        Generation if quality_gate => &[QualityAudit],
        Generation => &[Snapshot],
        QualityAudit => &[CurationReview],
        CurationReview => &[CurationApproval],
        CurationApproval => &[Snapshot],
        Snapshot => &[Training],
        Training => &[DevelopmentEvaluation],
        DevelopmentEvaluation => &[AcceptanceAssessment],
        AcceptanceAssessment => &[ErrorAnalysis],
        ErrorAnalysis if advisor => &[Advisor, OptimizationProposal, StopDecision],
        ErrorAnalysis => &[OptimizationProposal, StopDecision],
        Advisor => &[OptimizationProposal, StopDecision],
        OptimizationProposal => &[Approval],
        Approval => &[ProposalApplication],
        ProposalApplication => &[DatasetDiffGeneration],
        DatasetDiffGeneration if quality_gate => &[IterationQualityAudit],
        DatasetDiffGeneration => &[IterationSnapshot],
        IterationQualityAudit => &[IterationCurationReview],
        IterationCurationReview => &[IterationCurationApproval],
        IterationCurationApproval => &[IterationSnapshot],
        IterationSnapshot => &[IterationTraining],
        IterationTraining => &[IterationEvaluation],
        IterationEvaluation => &[Comparison],
        Comparison => &[FollowupAnalysis],
        FollowupAnalysis => &[StopDecision],
        StopDecision => &[OptimizationProposal],
        SealedEvaluation => &[Promotion],
        Promotion => &[],
    }
}

fn apply_outcome(
    definition: &WorkflowDefinition,
    run: &mut WorkflowRun,
    attempt: &WorkflowStageAttempt,
) -> Result<(), WorkflowError> {
    if attempt.state == StageAttemptState::Completed
        && attempt.stage == WorkflowStage::DatasetDiffGeneration
        && run.iteration >= definition.budget.maximum_iterations
    {
        return Err(WorkflowError::BudgetExceeded);
    }
    run.usage = attempt.usage_after.clone();
    run.current_stage = Some(attempt.stage);
    match attempt.state {
        StageAttemptState::Completed => {
            if attempt.stage == WorkflowStage::AcceptanceAssessment
                && attempt
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == "development_acceptance_pass")
            {
                run.state = WorkflowRunState::DevelopmentComplete;
            } else if attempt.stage == WorkflowStage::StopDecision {
                run.state = if attempt
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == "workflow_continue")
                {
                    WorkflowRunState::Running
                } else {
                    WorkflowRunState::DevelopmentComplete
                };
            } else if attempt.stage == WorkflowStage::Promotion {
                run.state = WorkflowRunState::Completed;
            } else if attempt.stage == WorkflowStage::SealedEvaluation {
                run.state = WorkflowRunState::AwaitingUser;
            } else {
                run.state = WorkflowRunState::Running;
            }
            if attempt.stage == WorkflowStage::DatasetDiffGeneration {
                run.iteration = run.iteration.saturating_add(1);
            }
        }
        StageAttemptState::Failed => {
            run.state = if attempt.retryable {
                WorkflowRunState::AwaitingUser
            } else {
                WorkflowRunState::Failed
            }
        }
        StageAttemptState::Cancelled => run.state = WorkflowRunState::Cancelled,
        StageAttemptState::AwaitingApproval => run.state = WorkflowRunState::AwaitingApproval,
        StageAttemptState::AwaitingUser => run.state = WorkflowRunState::AwaitingUser,
        StageAttemptState::Inconclusive => run.state = WorkflowRunState::Inconclusive,
        StageAttemptState::Exhausted => run.state = WorkflowRunState::Exhausted,
        StageAttemptState::Running => return Err(WorkflowError::InvalidOutcome),
    }
    Ok(())
}

fn definition_fingerprint(value: &WorkflowDefinition) -> Result<String, WorkflowError> {
    let mut document = serde_json::json!({
        "name": value.name, "dataset_id": value.dataset_id,
        "project_configuration_id": value.project_configuration_id,
        "project_configuration_fingerprint": value.project_configuration_fingerprint,
        "development_suite_id": value.development_suite_id,
        "development_suite_fingerprint": value.development_suite_fingerprint,
        "sealed_suite_id": value.sealed_suite_id,
        "sealed_suite_fingerprint": value.sealed_suite_fingerprint,
        "initial_allocation": value.initial_allocation, "governance": value.governance,
        "budget": value.budget, "policy": value.policy,
    });
    if value.analysis_protocol.is_some()
        || value.optimization_protocol.is_some()
        || value.advisor.is_some()
        || value.training_iteration_policy.is_some()
    {
        let object = document
            .as_object_mut()
            .expect("definition document object");
        object.insert(
            "analysis_protocol".into(),
            serde_json::to_value(&value.analysis_protocol)
                .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
        );
        object.insert(
            "optimization_protocol".into(),
            serde_json::to_value(&value.optimization_protocol)
                .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
        );
        object.insert(
            "advisor".into(),
            serde_json::to_value(&value.advisor)
                .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
        );
        object.insert(
            "training_iteration_policy".into(),
            serde_json::to_value(value.training_iteration_policy)
                .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
        );
    }
    if value.quality_gate.is_some() {
        document
            .as_object_mut()
            .expect("definition document object")
            .insert(
                "quality_gate".into(),
                serde_json::to_value(&value.quality_gate)
                    .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
            );
    }
    if value.benchmark_bundle.is_some() {
        document
            .as_object_mut()
            .expect("definition document object")
            .insert(
                "benchmark_bundle".into(),
                serde_json::to_value(&value.benchmark_bundle)
                    .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
            );
    }
    if value.benchmark_qualification.is_some() {
        document
            .as_object_mut()
            .expect("definition document object")
            .insert(
                "benchmark_qualification".into(),
                serde_json::to_value(&value.benchmark_qualification)
                    .map_err(|error| WorkflowError::Fingerprint(error.to_string()))?,
            );
    }
    artifact_core::fingerprint(&document)
        .map_err(|error| WorkflowError::Fingerprint(error.to_string()))
}

fn attempt_fingerprint(value: &WorkflowStageAttempt) -> Result<String, WorkflowError> {
    artifact_core::fingerprint(&serde_json::json!({
        "workflow_run_id": value.workflow_run_id, "sequence": value.sequence,
        "iteration": value.iteration, "stage": value.stage, "attempt": value.attempt,
        "state": value.state, "predecessor_id": value.predecessor_id,
        "predecessor_fingerprint": value.predecessor_fingerprint,
        "reason": value.reason, "retryable": value.retryable,
        "artifacts": value.artifacts, "usage_after": value.usage_after,
    }))
    .map_err(|error| WorkflowError::Fingerprint(error.to_string()))
}

fn required(value: String, field: &'static str) -> Result<String, WorkflowError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(WorkflowError::Empty(field))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn benchmark_bundle_binding(
        development_suite_id: Uuid,
        development_suite_fingerprint: &str,
        sealed_suite_id: Option<Uuid>,
        sealed_suite_fingerprint: Option<&str>,
    ) -> BenchmarkBundleBinding {
        let contamination_report_id = Uuid::new_v4();
        let contamination_report_fingerprint = "sha256:contamination".to_owned();
        let mut binding = BenchmarkBundleBinding {
            bundle_id: Uuid::new_v4(),
            bundle_fingerprint: String::new(),
            development_suite_id,
            development_suite_fingerprint: development_suite_fingerprint.to_owned(),
            sealed_suite_id,
            sealed_suite_fingerprint: sealed_suite_fingerprint.map(str::to_owned),
            contamination_report_id,
            contamination_report_fingerprint,
        };
        binding.bundle_fingerprint = binding
            .reproduce_bundle_fingerprint()
            .expect("bundle fingerprint");
        binding
    }

    fn definition_request() -> WorkflowDefinitionRequest {
        let development_suite_id = Uuid::new_v4();
        let development_suite_fingerprint = "sha256:development";
        let sealed_suite_id = Uuid::new_v4();
        let sealed_suite_fingerprint = "sha256:sealed";
        WorkflowDefinitionRequest {
            name: "encoder loop".into(),
            dataset_id: Uuid::new_v4(),
            project_configuration_id: Uuid::new_v4(),
            project_configuration_fingerprint: "sha256:config".into(),
            development_suite_id,
            development_suite_fingerprint: development_suite_fingerprint.into(),
            sealed_suite_id: Some(sealed_suite_id),
            sealed_suite_fingerprint: Some(sealed_suite_fingerprint.into()),
            benchmark_bundle: Some(benchmark_bundle_binding(
                development_suite_id,
                development_suite_fingerprint,
                Some(sealed_suite_id),
                Some(sealed_suite_fingerprint),
            )),
            initial_allocation: WorkflowInitialAllocation {
                total_rows: 100,
                reserved_rows: 20,
                policy: InitialAllocationPolicy::Balanced,
                constraints: Vec::new(),
            },
            analysis_protocol: None,
            optimization_protocol: None,
            advisor: Some(AdvisorConfiguration {
                backend: "fake".into(),
                model: "deterministic-v1".into(),
                base_url: None,
                api_key_env: "SYNTH_ADVISOR_API_KEY".into(),
                egress_policy: crate::advisor::AdvisorEgressPolicy::AggregateOnly,
                maximum_findings: 10,
                maximum_representative_errors: 0,
                maximum_actions: 5,
                maximum_output_tokens: 1_000,
                temperature: Some(0.0),
            }),
            training_iteration_policy: Some(TrainingIterationPolicy::Fresh),
            quality_gate: None,
            governance: IterationGovernance::ReviewEachIteration,
            budget: WorkflowBudget {
                maximum_iterations: 2,
                maximum_initial_rows: 100,
                maximum_cumulative_rows: 150,
                maximum_generation_attempts: 500,
                maximum_generation_requests: 100,
                maximum_advisor_calls: 2,
                maximum_advisor_tokens: Some(10_000),
                maximum_stage_attempts: 3,
            },
            policy: WorkflowPolicy {
                minimum_improvement: 0.01,
                maximum_tolerated_regression: 0.02,
                stop_on_inconclusive: true,
                stop_on_invalid: true,
                enable_advisor: true,
                require_fresh_development_cohort_after_iterations: Some(2),
            },
        }
    }

    fn benchmark_qualification_binding(
        bundle: &BenchmarkBundleBinding,
    ) -> ApprovedBenchmarkQualificationBinding {
        ApprovedBenchmarkQualificationBinding {
            qualification_id: Uuid::new_v4(),
            qualification_fingerprint: format!("sha256:{}", "a".repeat(64)),
            review_id: Uuid::new_v4(),
            review_fingerprint: format!("sha256:{}", "b".repeat(64)),
            benchmark_bundle_id: bundle.bundle_id,
            benchmark_bundle_fingerprint: bundle.bundle_fingerprint.clone(),
        }
    }

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition::new(definition_request()).expect("definition")
    }

    #[test]
    fn definition_creation_requires_a_valid_matching_benchmark_bundle() {
        let mut missing = definition_request();
        missing.benchmark_bundle = None;
        assert_eq!(
            WorkflowDefinition::new(missing),
            Err(WorkflowError::BenchmarkBundleRequired)
        );

        let mut invalid = definition_request();
        invalid
            .benchmark_bundle
            .as_mut()
            .expect("benchmark bundle")
            .bundle_fingerprint = "sha256:tampered".into();
        assert_eq!(
            WorkflowDefinition::new(invalid),
            Err(WorkflowError::BenchmarkBundleFingerprint)
        );

        let mut malformed = definition_request();
        malformed
            .benchmark_bundle
            .as_mut()
            .expect("benchmark bundle")
            .contamination_report_id = Uuid::nil();
        assert_eq!(
            WorkflowDefinition::new(malformed),
            Err(WorkflowError::InvalidBenchmarkBundleBinding)
        );

        let mut mismatched = definition_request();
        mismatched.development_suite_fingerprint = "sha256:substituted-suite".into();
        assert_eq!(
            WorkflowDefinition::new(mismatched),
            Err(WorkflowError::BenchmarkBundleSuiteMismatch)
        );

        let mut mismatched = definition_request();
        mismatched.development_suite_id = Uuid::new_v4();
        assert_eq!(
            WorkflowDefinition::new(mismatched),
            Err(WorkflowError::BenchmarkBundleSuiteMismatch)
        );

        let mut mismatched = definition_request();
        mismatched.sealed_suite_id = Some(Uuid::new_v4());
        assert_eq!(
            WorkflowDefinition::new(mismatched),
            Err(WorkflowError::BenchmarkBundleSuiteMismatch)
        );

        let mut mismatched = definition_request();
        mismatched.sealed_suite_fingerprint = Some("sha256:substituted-sealed-suite".into());
        assert_eq!(
            WorkflowDefinition::new(mismatched),
            Err(WorkflowError::BenchmarkBundleSuiteMismatch)
        );

        let mut development_only = definition_request();
        development_only.sealed_suite_id = None;
        development_only.sealed_suite_fingerprint = None;
        development_only.benchmark_bundle = Some(benchmark_bundle_binding(
            development_only.development_suite_id,
            &development_only.development_suite_fingerprint,
            None,
            None,
        ));
        assert!(WorkflowDefinition::new(development_only).is_ok());

        let request = definition_request();
        let qualification =
            benchmark_qualification_binding(request.benchmark_bundle.as_ref().expect("bundle"));
        let definition = WorkflowDefinition::new_with_qualification(
            request.clone(),
            Some(qualification.clone()),
        )
        .expect("qualified definition");
        assert_eq!(
            definition.benchmark_qualification.as_ref(),
            Some(&qualification)
        );

        let mut wrong_bundle = qualification.clone();
        wrong_bundle.benchmark_bundle_id = Uuid::new_v4();
        assert_eq!(
            WorkflowDefinition::new_with_qualification(request.clone(), Some(wrong_bundle)),
            Err(WorkflowError::BenchmarkQualificationBundleMismatch)
        );

        let mut malformed = qualification;
        malformed.review_fingerprint = "sha256:not-a-digest".into();
        assert_eq!(
            WorkflowDefinition::new_with_qualification(request, Some(malformed)),
            Err(WorkflowError::InvalidBenchmarkQualificationBinding)
        );
    }

    #[test]
    fn legacy_definition_reproduces_its_fingerprint_but_cannot_be_queued() {
        let mut legacy = definition();
        legacy.benchmark_bundle = None;
        let historical_document = serde_json::json!({
            "name": legacy.name,
            "dataset_id": legacy.dataset_id,
            "project_configuration_id": legacy.project_configuration_id,
            "project_configuration_fingerprint": legacy.project_configuration_fingerprint,
            "development_suite_id": legacy.development_suite_id,
            "development_suite_fingerprint": legacy.development_suite_fingerprint,
            "sealed_suite_id": legacy.sealed_suite_id,
            "sealed_suite_fingerprint": legacy.sealed_suite_fingerprint,
            "initial_allocation": legacy.initial_allocation,
            "governance": legacy.governance,
            "budget": legacy.budget,
            "policy": legacy.policy,
            "analysis_protocol": legacy.analysis_protocol,
            "optimization_protocol": legacy.optimization_protocol,
            "advisor": legacy.advisor,
            "training_iteration_policy": legacy.training_iteration_policy,
        });
        legacy.fingerprint =
            artifact_core::fingerprint(&historical_document).expect("historical fingerprint");

        let encoded = serde_json::to_value(&legacy).expect("legacy definition JSON");
        assert!(
            !encoded
                .as_object()
                .expect("definition object")
                .contains_key("benchmark_bundle")
        );
        assert!(
            !encoded
                .as_object()
                .expect("definition object")
                .contains_key("benchmark_qualification")
        );
        let decoded: WorkflowDefinition =
            serde_json::from_value(encoded).expect("legacy definition remains readable");
        assert_eq!(
            decoded.reproduce_fingerprint().expect("reproduce"),
            decoded.fingerprint
        );
        assert_eq!(
            WorkflowRun::queued(&decoded),
            Err(WorkflowError::BenchmarkBundleRequired)
        );
        let now = Utc::now();
        let mut queued = WorkflowRun {
            id: Uuid::new_v4(),
            definition_id: decoded.id,
            definition_fingerprint: decoded.fingerprint.clone(),
            state: WorkflowRunState::Queued,
            current_stage: None,
            iteration: 0,
            usage: WorkflowBudgetUsage::zero(),
            latest_attempt_id: None,
            latest_attempt_fingerprint: None,
            cancel_requested: false,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(
            WorkflowStageAttempt::start(
                &decoded,
                &mut queued,
                WorkflowStage::InitialAllocation,
                None,
                1,
            ),
            Err(WorkflowError::BenchmarkBundleRequired)
        );

        let request_json = serde_json::to_value({
            let mut request = definition_request();
            request.benchmark_bundle = None;
            request
        })
        .expect("legacy request JSON");
        assert!(
            !request_json
                .as_object()
                .expect("request object")
                .contains_key("benchmark_bundle")
        );
        let request: WorkflowDefinitionRequest =
            serde_json::from_value(request_json).expect("legacy request remains readable");
        assert_eq!(
            WorkflowDefinition::new(request),
            Err(WorkflowError::BenchmarkBundleRequired)
        );
    }

    fn gated_definition() -> WorkflowDefinition {
        let definition = definition();
        WorkflowDefinition::new(WorkflowDefinitionRequest {
            name: definition.name,
            dataset_id: definition.dataset_id,
            project_configuration_id: definition.project_configuration_id,
            project_configuration_fingerprint: definition.project_configuration_fingerprint,
            development_suite_id: definition.development_suite_id,
            development_suite_fingerprint: definition.development_suite_fingerprint,
            sealed_suite_id: definition.sealed_suite_id,
            sealed_suite_fingerprint: definition.sealed_suite_fingerprint,
            benchmark_bundle: definition.benchmark_bundle,
            initial_allocation: definition.initial_allocation,
            analysis_protocol: definition.analysis_protocol,
            optimization_protocol: definition.optimization_protocol,
            advisor: definition.advisor,
            training_iteration_policy: definition.training_iteration_policy,
            quality_gate: Some(WorkflowQualityGateRequest {
                preset: QualityPreset::Fast,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                authenticity: WorkflowQualityAuthenticity::Off,
                maximum_cost_microusd: Some(10_000),
                evaluator_backend: "deterministic-fake".into(),
                evaluator_protocol_version: "quality-evaluator-v1".into(),
            }),
            governance: definition.governance,
            budget: definition.budget,
            policy: definition.policy,
        })
        .expect("gated definition")
    }

    #[test]
    fn rejects_illegal_transitions_and_stale_attempts() {
        let definition = definition();
        let mut run = WorkflowRun::queued(&definition).expect("run");
        assert!(matches!(
            WorkflowStageAttempt::start(&definition, &mut run, WorkflowStage::Training, None, 1),
            Err(WorkflowError::IllegalStage { .. })
        ));
        let running = WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::InitialAllocation,
            None,
            1,
        )
        .expect("start");
        let finished = running
            .finish(
                &definition,
                &mut run,
                StageOutcome {
                    state: StageAttemptState::Completed,
                    reason: None,
                    retryable: false,
                    artifacts: Vec::new(),
                    usage_after: WorkflowBudgetUsage::zero(),
                },
            )
            .expect("finish");
        assert!(matches!(
            running.finish(
                &definition,
                &mut run,
                StageOutcome {
                    state: StageAttemptState::Completed,
                    reason: None,
                    retryable: false,
                    artifacts: Vec::new(),
                    usage_after: WorkflowBudgetUsage::zero(),
                }
            ),
            Err(WorkflowError::StaleAttempt)
        ));
        WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::Generation,
            Some(&finished),
            1,
        )
        .expect("legal successor");
    }

    #[test]
    fn only_the_exact_interrupted_running_attempt_can_reactivate_a_run() {
        let definition = definition();
        let mut run = WorkflowRun::queued(&definition).expect("run");
        let attempt = WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::InitialAllocation,
            None,
            1,
        )
        .expect("running attempt");
        run.state = WorkflowRunState::AwaitingUser;
        run.resume_interrupted(&attempt)
            .expect("exact interrupted attempt resumes");
        assert_eq!(run.state, WorkflowRunState::Running);

        let mut stale = run.clone();
        stale.state = WorkflowRunState::AwaitingUser;
        stale.latest_attempt_id = Some(Uuid::new_v4());
        assert_eq!(
            stale.resume_interrupted(&attempt),
            Err(WorkflowError::NotInterrupted)
        );
    }

    #[test]
    fn finite_budgets_and_preauthorization_are_enforced() {
        let mut definition = definition();
        definition.governance = IterationGovernance::PreauthorizedBounded {
            envelope: ApprovalEnvelope {
                maximum_iterations: 3,
                maximum_additional_rows: 10,
                maximum_generation_requests: 10,
                maximum_advisor_calls: 1,
                maximum_advisor_tokens: Some(100),
                permitted_generation_backend: "fake".into(),
                permitted_generation_model: "fake-v1".into(),
                permitted_training_backend: "linear".into(),
                permitted_training_configuration_fingerprints: Vec::new(),
            },
        };
        let request = WorkflowDefinitionRequest {
            name: definition.name,
            dataset_id: definition.dataset_id,
            project_configuration_id: definition.project_configuration_id,
            project_configuration_fingerprint: definition.project_configuration_fingerprint,
            development_suite_id: definition.development_suite_id,
            development_suite_fingerprint: definition.development_suite_fingerprint,
            sealed_suite_id: definition.sealed_suite_id,
            sealed_suite_fingerprint: definition.sealed_suite_fingerprint,
            benchmark_bundle: definition.benchmark_bundle,
            initial_allocation: definition.initial_allocation,
            analysis_protocol: definition.analysis_protocol,
            optimization_protocol: definition.optimization_protocol,
            advisor: definition.advisor,
            training_iteration_policy: definition.training_iteration_policy,
            quality_gate: definition
                .quality_gate
                .map(|gate| WorkflowQualityGateRequest {
                    preset: gate.policy.preset.expect("preset-backed test gate"),
                    egress_policy: gate.policy.egress_policy,
                    authenticity: gate.authenticity,
                    maximum_cost_microusd: gate.policy.budgets.maximum_cost_microusd,
                    evaluator_backend: gate.evaluator_backend,
                    evaluator_protocol_version: gate.evaluator_protocol_version,
                }),
            governance: definition.governance,
            budget: definition.budget,
            policy: definition.policy,
        };
        assert_eq!(
            WorkflowDefinition::new(request),
            Err(WorkflowError::InvalidEnvelope)
        );
    }

    #[test]
    fn retryable_failure_restarts_only_the_same_stage_within_the_ceiling() {
        let definition = definition();
        let mut run = WorkflowRun::queued(&definition).expect("run");
        let running = WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::InitialAllocation,
            None,
            1,
        )
        .expect("start");
        let failed = running
            .finish(
                &definition,
                &mut run,
                StageOutcome {
                    state: StageAttemptState::Failed,
                    reason: Some("retry me".into()),
                    retryable: true,
                    artifacts: Vec::new(),
                    usage_after: WorkflowBudgetUsage::zero(),
                },
            )
            .expect("fail");
        assert_eq!(run.state, WorkflowRunState::AwaitingUser);
        assert!(matches!(
            WorkflowStageAttempt::start(
                &definition,
                &mut run,
                WorkflowStage::Generation,
                Some(&failed),
                2,
            ),
            Err(WorkflowError::IllegalStage { .. })
        ));
        WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::InitialAllocation,
            Some(&failed),
            2,
        )
        .expect("same-stage retry");
        assert!(matches!(
            WorkflowStageAttempt::start(
                &definition,
                &mut run,
                WorkflowStage::InitialAllocation,
                Some(&failed),
                4,
            ),
            Err(WorkflowError::StageAttemptsExceeded)
        ));
    }

    #[test]
    fn optional_quality_gate_resolves_a_full_population_policy_and_changes_both_paths() {
        let legacy = definition();
        let gated = gated_definition();
        let gate = gated.quality_gate.as_ref().expect("quality gate");
        assert_eq!(gate.policy.audit_mode, AuditMode::FullPopulation);
        assert_eq!(gate.policy.preset, Some(QualityPreset::Fast));
        assert!(gate.policy.verify_integrity().is_ok());

        assert_eq!(
            legal_successors(WorkflowStage::Generation, true, false),
            &[WorkflowStage::Snapshot]
        );
        assert_eq!(
            legal_successors(WorkflowStage::Generation, true, true),
            &[WorkflowStage::QualityAudit]
        );
        assert_eq!(
            legal_successors(WorkflowStage::DatasetDiffGeneration, true, false),
            &[WorkflowStage::IterationSnapshot]
        );
        assert_eq!(
            legal_successors(WorkflowStage::DatasetDiffGeneration, true, true),
            &[WorkflowStage::IterationQualityAudit]
        );
        assert!(
            !serde_json::to_value(&legacy)
                .expect("legacy definition JSON")
                .as_object()
                .expect("definition object")
                .contains_key("quality_gate"),
            "an absent quality gate must not change the persisted legacy shape"
        );
        assert_ne!(legacy.fingerprint, gated.fingerprint);

        let definition = definition();
        let unsupported = WorkflowDefinition::new(WorkflowDefinitionRequest {
            name: definition.name,
            dataset_id: definition.dataset_id,
            project_configuration_id: definition.project_configuration_id,
            project_configuration_fingerprint: definition.project_configuration_fingerprint,
            development_suite_id: definition.development_suite_id,
            development_suite_fingerprint: definition.development_suite_fingerprint,
            sealed_suite_id: definition.sealed_suite_id,
            sealed_suite_fingerprint: definition.sealed_suite_fingerprint,
            benchmark_bundle: definition.benchmark_bundle,
            initial_allocation: definition.initial_allocation,
            analysis_protocol: definition.analysis_protocol,
            optimization_protocol: definition.optimization_protocol,
            advisor: definition.advisor,
            training_iteration_policy: definition.training_iteration_policy,
            quality_gate: Some(WorkflowQualityGateRequest {
                preset: QualityPreset::Fast,
                egress_policy: EvaluatorEgressPolicy::ExternalCandidateText,
                authenticity: WorkflowQualityAuthenticity::Off,
                maximum_cost_microusd: Some(10_000),
                evaluator_backend: "openai-compatible".into(),
                evaluator_protocol_version: "quality-evaluator-v1".into(),
            }),
            governance: definition.governance,
            budget: definition.budget,
            policy: definition.policy,
        });
        assert_eq!(unsupported, Err(WorkflowError::InvalidQualityGate));
    }

    #[test]
    fn curation_pause_allows_only_the_explicit_approval_stage() {
        let definition = gated_definition();
        let mut run = WorkflowRun::queued(&definition).expect("run");
        run.state = WorkflowRunState::Running;
        run.current_stage = Some(WorkflowStage::CurationReview);
        let now = Utc::now();
        let mut review = WorkflowStageAttempt {
            id: Uuid::new_v4(),
            workflow_run_id: run.id,
            sequence: 7,
            iteration: 0,
            stage: WorkflowStage::CurationReview,
            attempt: 1,
            state: StageAttemptState::AwaitingUser,
            predecessor_id: Some(Uuid::new_v4()),
            predecessor_fingerprint: Some("sha256:predecessor".into()),
            reason: Some("approve exact proposal".into()),
            retryable: false,
            artifacts: Vec::new(),
            usage_after: run.usage.clone(),
            started_at: now,
            finished_at: Some(now),
            fingerprint: String::new(),
        };
        review.fingerprint = attempt_fingerprint(&review).expect("review fingerprint");
        run.state = WorkflowRunState::AwaitingUser;
        run.latest_attempt_id = Some(review.id);
        run.latest_attempt_fingerprint = Some(review.fingerprint.clone());

        assert!(matches!(
            WorkflowStageAttempt::start(
                &definition,
                &mut run.clone(),
                WorkflowStage::Snapshot,
                Some(&review),
                1,
            ),
            Err(WorkflowError::IllegalStage { .. })
        ));
        let mut cancelled_run = run.clone();
        cancelled_run.cancel_requested = true;
        let cancelled = review
            .finish(
                &definition,
                &mut cancelled_run,
                StageOutcome {
                    state: StageAttemptState::Cancelled,
                    reason: Some("operator cancelled while approval was pending".into()),
                    retryable: false,
                    artifacts: Vec::new(),
                    usage_after: run.usage.clone(),
                },
            )
            .expect("paused review cancellation");
        assert_eq!(cancelled.state, StageAttemptState::Cancelled);
        assert_eq!(cancelled_run.state, WorkflowRunState::Cancelled);
        WorkflowStageAttempt::start(
            &definition,
            &mut run,
            WorkflowStage::CurationApproval,
            Some(&review),
            1,
        )
        .expect("approval continuation");
    }
}
