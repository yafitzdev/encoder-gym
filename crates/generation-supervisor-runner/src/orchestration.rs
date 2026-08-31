//! Finite, restartable generation -> audit -> decision orchestration.
//!
//! The application runner composes existing slice contracts. It does not
//! duplicate generation, quality-assessment, or supervisor policy logic.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use agent_runtime_core::AgentRuntime;
use artifact_core::{FingerprintError, fingerprint};
use chrono::Utc;
use dataset_core::domain::{SourceProvenance, SourceRow};
use dataset_quality_core::{
    QualityError,
    assessment::{EvaluatorGuidance, RowQualityAssessment},
    lifecycle::{QualityAuditRun, QualityAuditRunState},
    policy::{AuditMode, QualityPolicy},
    population::{AuditPlan, GuidanceReference, GuidanceReferences},
    ports::{DatasetQualityStore, QualityAdapterError, QualityCandidateSource, QualityEvaluator},
};
use dataset_quality_runner::{DatasetQualityRunner, QualityRunnerError};
use generation_core::{
    construction::RowConstructionPlan,
    domain::{
        DatasetDefinition, GeneratedRow, GenerationParameters, GenerationPlan, PlannedCell,
        ValidationStatus,
    },
    jobs::{
        GenerationExecutionPolicy, GenerationExecutionSpec, GenerationJob, JobRunner,
        JobRunnerError, JobRunnerPolicy, JobState,
    },
    planning::calculate_generation_needs,
    ports::{GenerationBackend, GenerationStore, RowQuery, StoreError},
    prompting::{PromptBuilder, SupervisedGenerationSchedule, SupervisedRowGuidance},
    validation::{SourceExcerptNoveltyValidator, TextLengthValidator, ValidationPipeline},
};
use generation_supervisor_core::{
    SupervisorError,
    advisor::{AdvisorConfiguration, AdvisorSessionState, GenerationQualityDiagnosisBrief},
    contract::{
        AcceptedCoverageBinding, BaselinePolicy, GenerationQualityContract, MonitoringScope,
    },
    decision::{DeterministicQualityDecision, SupervisorDecisionState},
    lifecycle::{
        ChildKind, ChildOutcome, ChildOutcomeState, ChildReservation, SupervisorRun,
        SupervisorRunEvent, SupervisorRunState, SupervisorUsage,
    },
    observation::{
        AssessmentEvidence, BatchQualityObservation, ContractRowVerdict, QualityScope,
        QualityWindowKind, RowQualityObservation, StructuralOutcome,
    },
    ports::{GenerationSupervisorStore, SupervisorAdvisorStore, SupervisorIntegrityReport},
    revision::{
        PromptGuidanceVersion, PromptRevisionActivation, PromptRevisionAuthorization,
        PromptRevisionReview, RevisionReviewDecision,
    },
    strategy::StrategyAssignmentSet,
};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::{AdvisorOutcome, GenerationSupervisorAdvisorRunner, RunnerError};

#[derive(Debug, Error)]
pub enum SupervisorLoopError {
    #[error("generation supervisor artifact was not found: {0}")]
    NotFound(String),
    #[error("generation supervisor runtime rejected the operation: {0}")]
    Validation(String),
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
    #[error(transparent)]
    GenerationStore(#[from] StoreError),
    #[error(transparent)]
    Generation(#[from] JobRunnerError),
    #[error(transparent)]
    Quality(#[from] QualityError),
    #[error(transparent)]
    QualityAdapter(#[from] QualityAdapterError),
    #[error(transparent)]
    QualityRunner(#[from] QualityRunnerError),
    #[error(transparent)]
    Fingerprint(#[from] FingerprintError),
    #[error(transparent)]
    Advisor(#[from] RunnerError),
    #[error("generation supervisor metadata was invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct SupervisorExecutionConfiguration {
    pub generation_parameters: GenerationParameters,
    pub generation_policy: JobRunnerPolicy,
    pub quality_policy: QualityPolicy,
    pub evaluator_guidance: EvaluatorGuidance,
    pub text_length: Option<TextLengthValidator>,
    pub construction_plan: RowConstructionPlan,
    pub semantic_context: Option<semantic_catalog::ResolvedSemanticContext>,
    pub authenticity_context: Option<research_core::profile::ResolvedAuthenticityContext>,
    /// Exact source excerpts used only by the structural novelty validator.
    /// They are never exposed to the diagnosis advisor.
    pub authenticity_source_excerpts: Vec<String>,
    pub strategy_context: Option<generation_core::strategy::ResolvedGenerationStrategyContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupervisorStatus {
    pub run: SupervisorRun,
    pub state: SupervisorRunState,
    pub usage: SupervisorUsage,
    pub active_prompt: PromptGuidanceVersion,
    pub generation_segments: u32,
    pub quality_audits: u32,
    pub observed_rows: u64,
    pub quality_windows: u64,
    pub latest_decisions: Vec<DeterministicQualityDecision>,
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupervisorAdvanceOutcome {
    pub status: SupervisorStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_job_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_audit_run_id: Option<Uuid>,
    #[serde(default)]
    pub decisions: Vec<DeterministicQualityDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevisionReviewInput {
    pub decision: RevisionReviewDecision,
    pub reviewer: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevisionAuthorizationOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<PromptRevisionReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<PromptRevisionAuthorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_prompt: Option<PromptGuidanceVersion>,
    pub status: SupervisorStatus,
}

pub struct GenerationQualitySupervisorRunner {
    generation_store: Arc<dyn GenerationStore>,
    supervisor_store: Arc<dyn GenerationSupervisorStore>,
    advisor_store: Arc<dyn SupervisorAdvisorStore>,
    quality_store: Arc<dyn DatasetQualityStore>,
    candidates: Arc<dyn QualityCandidateSource>,
    backend: Arc<dyn GenerationBackend>,
    evaluators: Vec<Arc<dyn QualityEvaluator>>,
    configuration: SupervisorExecutionConfiguration,
}

impl GenerationQualitySupervisorRunner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generation_store: Arc<dyn GenerationStore>,
        supervisor_store: Arc<dyn GenerationSupervisorStore>,
        advisor_store: Arc<dyn SupervisorAdvisorStore>,
        quality_store: Arc<dyn DatasetQualityStore>,
        candidates: Arc<dyn QualityCandidateSource>,
        backend: Arc<dyn GenerationBackend>,
        evaluators: Vec<Arc<dyn QualityEvaluator>>,
        configuration: SupervisorExecutionConfiguration,
    ) -> Result<Self, SupervisorLoopError> {
        configuration.quality_policy.verify_integrity()?;
        if evaluators.is_empty() {
            return Err(SupervisorLoopError::Validation(
                "at least one independent quality evaluator is required".into(),
            ));
        }
        if !matches!(
            configuration.quality_policy.audit_mode,
            AuditMode::FullPopulation
        ) {
            return Err(SupervisorLoopError::Validation(
                "supervisor segment audits must assess the full segment population".into(),
            ));
        }
        Ok(Self {
            generation_store,
            supervisor_store,
            advisor_store,
            quality_store,
            candidates,
            backend,
            evaluators,
            configuration,
        })
    }

    /// Creates an immutable queued run. No provider or evaluator call occurs.
    pub async fn start(
        &self,
        contract_id: Uuid,
        initial_guidance: Vec<String>,
        strategy_seed: u64,
    ) -> Result<SupervisorStatus, SupervisorLoopError> {
        let contract = self.load_contract(contract_id).await?;
        let dataset = self.load_dataset(contract.dataset.id).await?;
        let plan = self.load_plan(contract.plan.id).await?;
        self.verify_runtime(&contract, &dataset, &plan)?;
        let starting_coverage = AcceptedCoverageBinding::from_counts(
            &self
                .generation_store
                .dataset_cell_counts(dataset.id)
                .await?,
        )?;
        if starting_coverage != contract.starting_coverage {
            return Err(SupervisorLoopError::Validation(
                "dataset coverage changed after the immutable supervisor contract was created"
                    .into(),
            ));
        }

        let run_id = Uuid::new_v4();
        let base_prompt = self.base_prompt_identity()?;
        let protected_fields_fingerprint = fingerprint(&(
            &contract.dataset,
            &contract.plan,
            &contract.semantic_context,
            &contract.authenticity_context,
            &contract.construction_context,
            &contract.strategy_context,
            &contract.row_thresholds,
            &contract.batch_thresholds,
            &contract.budgets,
        ))?;
        let initial_prompt = PromptGuidanceVersion::initial(
            Uuid::new_v4(),
            run_id,
            base_prompt.fingerprint,
            protected_fields_fingerprint,
            initial_guidance,
            Utc::now(),
        )?;
        let run = SupervisorRun::create(
            run_id,
            &contract,
            initial_prompt.id,
            initial_prompt.fingerprint.clone(),
            Utc::now(),
        )?;
        let assignments = self
            .configuration
            .strategy_context
            .as_ref()
            .map(|context| {
                StrategyAssignmentSet::compile(
                    Uuid::new_v4(),
                    &plan,
                    context,
                    strategy_seed,
                    Utc::now(),
                )
            })
            .transpose()?;
        validate_monitoring_support(
            &contract,
            &plan,
            &self
                .generation_store
                .dataset_cell_counts(dataset.id)
                .await?,
            assignments.as_ref(),
        )?;
        self.supervisor_store
            .create_run(&run, &initial_prompt, assignments.as_ref())
            .await?;
        self.status(run.id).await
    }

    /// Runs finite segments until a human/diagnosis boundary or terminal state.
    pub async fn run_until_boundary(
        &self,
        run_id: Uuid,
    ) -> Result<SupervisorAdvanceOutcome, SupervisorLoopError> {
        let contract = self.contract_for_run(run_id).await?;
        let maximum_steps = contract
            .budgets
            .maximum_generation_segments
            .saturating_add(2);
        let mut latest = None;
        for _ in 0..maximum_steps {
            let outcome = self.advance(run_id).await?;
            let state = outcome.status.state;
            latest = Some(outcome);
            if state.is_terminal()
                || matches!(
                    state,
                    SupervisorRunState::Paused
                        | SupervisorRunState::Diagnosing
                        | SupervisorRunState::AwaitingReview
                        | SupervisorRunState::Canary
                )
            {
                break;
            }
        }
        latest.ok_or_else(|| {
            SupervisorLoopError::Validation("supervisor made no finite progress".into())
        })
    }

    /// Executes at most one generation segment and its exact audit.
    pub async fn advance(
        &self,
        run_id: Uuid,
    ) -> Result<SupervisorAdvanceOutcome, SupervisorLoopError> {
        let run = self.load_run(run_id).await?;
        let contract = self.load_contract(run.contract_id).await?;
        let dataset = self.load_dataset(contract.dataset.id).await?;
        let master_plan = self.load_plan(contract.plan.id).await?;
        self.verify_runtime(&contract, &dataset, &master_plan)?;

        let mut state = self.current_state(run.id).await?;
        if state == SupervisorRunState::Queued {
            self.transition(
                run.id,
                SupervisorRunState::Queued,
                SupervisorRunState::Running,
                "operator started finite generation supervision",
            )
            .await?;
            state = SupervisorRunState::Running;
        }
        if state.is_terminal()
            || matches!(
                state,
                SupervisorRunState::Paused
                    | SupervisorRunState::Diagnosing
                    | SupervisorRunState::AwaitingReview
                    | SupervisorRunState::Canary
            )
        {
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run.id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }
        if state != SupervisorRunState::Running {
            return Err(SupervisorLoopError::Validation(format!(
                "run {run_id} is in interrupted state {state:?}; recover it before continuing"
            )));
        }
        if self.cancel_requested(run.id).await? {
            self.transition(
                run.id,
                state,
                SupervisorRunState::Cancelled,
                "durable cancellation observed before the next segment",
            )
            .await?;
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run.id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }
        let elapsed_seconds = Utc::now()
            .signed_duration_since(run.created_at)
            .num_seconds()
            .max(0) as u64;
        if elapsed_seconds >= contract.budgets.maximum_duration_seconds {
            self.transition(
                run.id,
                SupervisorRunState::Running,
                SupervisorRunState::Failed,
                "supervisor wall-clock budget was exhausted before the next segment",
            )
            .await?;
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run.id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }

        let prompt = self.active_prompt(run.id).await?;
        let assignments = self
            .supervisor_store
            .get_strategy_assignments(run.id)
            .await?;
        let coverage = self
            .generation_store
            .dataset_cell_counts(dataset.id)
            .await?;
        let prior_observations = self.supervisor_store.list_row_observations(run.id).await?;
        let rows_per_cell = if prior_observations.is_empty() {
            contract.monitoring.initial_canary_rows_per_scope
        } else {
            contract.monitoring.rolling_window_rows_per_scope
        };
        let segment_plan = segment_plan(
            &contract,
            &master_plan,
            &coverage,
            &prior_observations,
            rows_per_cell,
        )?;
        let requested_rows = segment_plan
            .cells
            .iter()
            .map(|planned| {
                let accepted = coverage
                    .get(&planned.cell.key())
                    .map_or(0, |counts| counts.accepted);
                u64::from(planned.target_count.saturating_sub(accepted))
            })
            .sum::<u64>();
        if requested_rows == 0 {
            self.transition(
                run.id,
                SupervisorRunState::Running,
                SupervisorRunState::Completed,
                "master generation plan coverage is complete",
            )
            .await?;
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run.id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }

        let schedule = build_schedule(
            &run,
            &prompt,
            &segment_plan,
            &coverage,
            &prior_observations,
            assignments.as_ref(),
            &contract,
        )?;
        let job = GenerationJob::queued(
            dataset.id,
            segment_plan.id,
            self.backend.name(),
            self.backend.model(),
            requested_rows,
        );
        let novelty_guard = self.novelty_guard()?;
        let execution = self.generation_execution(
            &contract,
            &segment_plan,
            &coverage,
            &job,
            &schedule,
            novelty_guard.as_ref(),
        )?;
        let input_fingerprint = fingerprint(&(&segment_plan, &job, &execution, &schedule))?;
        let reserved_generated_rows = requested_rows
            .checked_mul(u64::from(
                self.configuration
                    .generation_policy
                    .max_attempt_multiplier
                    .max(1),
            ))
            .ok_or_else(|| {
                SupervisorLoopError::Validation("segment generated-row budget overflowed".into())
            })?;
        let reservation = ChildReservation::create(
            Uuid::new_v4(),
            run.id,
            ChildKind::GenerationSegment,
            format!("segment:{}:{}", prompt.sequence, prior_observations.len()),
            job.id,
            1,
            None,
            input_fingerprint,
            Utc::now(),
        )?
        .with_reserved_usage(SupervisorUsage {
            generation_segments: 1,
            generated_rows: reserved_generated_rows,
            ..SupervisorUsage::default()
        })?;

        self.transition(
            run.id,
            SupervisorRunState::Running,
            SupervisorRunState::Generating,
            "reserved the next finite generation segment",
        )
        .await?;
        self.supervisor_store.reserve_child(&reservation).await?;
        self.generation_store.create_plan(&segment_plan).await?;
        self.generation_store
            .create_generation_execution(&job, &execution)
            .await?;

        let runner = self.job_runner(schedule, novelty_guard);
        let generated = match runner
            .run(job.id, self.configuration.generation_parameters.clone())
            .await
        {
            Ok(job) if job.state == JobState::Completed => job,
            Ok(job) => {
                let reason = job
                    .error_message
                    .clone()
                    .unwrap_or_else(|| format!("generation child ended in {:?}", job.state));
                self.finish_failed_child(&reservation, child_state(job.state), reason.clone())
                    .await?;
                let next = if job.state == JobState::Cancelled {
                    SupervisorRunState::Cancelled
                } else {
                    SupervisorRunState::Failed
                };
                self.transition(run.id, SupervisorRunState::Generating, next, reason)
                    .await?;
                return Ok(SupervisorAdvanceOutcome {
                    status: self.status(run.id).await?,
                    generation_job_id: Some(job.id),
                    quality_audit_run_id: None,
                    decisions: vec![],
                });
            }
            Err(error) => {
                let reason = error.to_string();
                self.finish_failed_child(&reservation, ChildOutcomeState::Failed, reason.clone())
                    .await?;
                self.transition(
                    run.id,
                    SupervisorRunState::Generating,
                    SupervisorRunState::Failed,
                    reason,
                )
                .await?;
                return Ok(SupervisorAdvanceOutcome {
                    status: self.status(run.id).await?,
                    generation_job_id: Some(job.id),
                    quality_audit_run_id: None,
                    decisions: vec![],
                });
            }
        };
        self.supervisor_store
            .finish_child(&ChildOutcome::record(
                Uuid::new_v4(),
                &reservation,
                ChildOutcomeState::Succeeded,
                Some((generated.id, fingerprint(&generated)?)),
                None,
                Utc::now(),
            )?)
            .await?;
        self.transition(
            run.id,
            SupervisorRunState::Generating,
            SupervisorRunState::Assessing,
            "generation segment completed; auditing only its persisted rows",
        )
        .await?;

        let generated_rows = self.list_job_rows(generated.id).await?;
        let accepted_ids = generated_rows
            .iter()
            .filter(|row| row.validation_status == ValidationStatus::Accepted)
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let source_rows = if accepted_ids.is_empty() {
            vec![]
        } else {
            self.candidates
                .get_source_rows(dataset.id, accepted_ids)
                .await?
        };
        verify_segment_sources(generated.id, &source_rows)?;

        let (quality_run_id, assessments) = if source_rows.is_empty() {
            (None, vec![])
        } else {
            match self
                .audit_segment(&run, &contract, &dataset, source_rows.clone())
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.transition(
                        run.id,
                        SupervisorRunState::Assessing,
                        SupervisorRunState::Failed,
                        format!("segment quality audit failed: {error}"),
                    )
                    .await?;
                    return Ok(SupervisorAdvanceOutcome {
                        status: self.status(run.id).await?,
                        generation_job_id: Some(generated.id),
                        quality_audit_run_id: None,
                        decisions: vec![],
                    });
                }
            }
        };

        let observations = build_row_observations(
            &contract,
            &run,
            &prompt,
            generated.id,
            &generated_rows,
            &source_rows,
            &assessments,
            assignments.as_ref(),
        )?;
        self.supervisor_store
            .save_row_observations(&observations)
            .await?;
        let decisions = self
            .persist_windows_and_decisions(&contract, &run, &prompt, &observations, false)
            .await?;

        let refreshed_coverage = self
            .generation_store
            .dataset_cell_counts(dataset.id)
            .await?;
        let all_observations = self.supervisor_store.list_row_observations(run.id).await?;
        let effective = effective_coverage(
            &contract,
            &master_plan,
            &refreshed_coverage,
            &all_observations,
        );
        let plan_complete = master_plan.cells.iter().all(|planned| {
            effective.get(&planned.cell.key()).copied().unwrap_or(0) >= planned.target_count
        });
        let must_pause = decisions.iter().any(|decision| {
            matches!(
                decision.state,
                SupervisorDecisionState::PauseForDiagnosis
                    | SupervisorDecisionState::RevisionFailed
                    | SupervisorDecisionState::Escalate
            )
        }) || (plan_complete
            && decisions
                .iter()
                .any(|decision| decision.state == SupervisorDecisionState::InsufficientEvidence));
        self.transition(
            run.id,
            SupervisorRunState::Assessing,
            if must_pause {
                SupervisorRunState::Paused
            } else {
                SupervisorRunState::Running
            },
            if must_pause {
                "deterministic quality evidence paused generation"
            } else {
                "segment quality passed or needs more authorized evidence"
            },
        )
        .await?;

        Ok(SupervisorAdvanceOutcome {
            status: self.status(run.id).await?,
            generation_job_id: Some(generated.id),
            quality_audit_run_id: quality_run_id,
            decisions,
        })
    }

    pub async fn request_cancel(
        &self,
        run_id: Uuid,
    ) -> Result<SupervisorStatus, SupervisorLoopError> {
        if !self
            .supervisor_store
            .request_supervisor_cancellation(run_id)
            .await?
        {
            return Err(SupervisorLoopError::NotFound(format!(
                "supervisor run {run_id}"
            )));
        }
        let outcomes = self
            .supervisor_store
            .list_child_outcomes(run_id)
            .await?
            .into_iter()
            .map(|value| value.reservation_id)
            .collect::<BTreeSet<_>>();
        for reservation in self
            .supervisor_store
            .list_child_reservations(run_id)
            .await?
        {
            if outcomes.contains(&reservation.id) {
                continue;
            }
            match reservation.kind {
                ChildKind::GenerationSegment | ChildKind::RevisionCanary => {
                    let _ = self
                        .generation_store
                        .request_job_cancellation(reservation.child_id)
                        .await;
                }
                ChildKind::QualityAudit => {
                    if let Some(mut run) = self
                        .quality_store
                        .get_audit_run(reservation.child_id)
                        .await?
                    {
                        run.request_cancel()?;
                        self.quality_store.save_audit_run(&run).await?;
                    }
                }
                ChildKind::PiModelTurn | ChildKind::PiToolCall => {}
            }
        }
        let state = self.current_state(run_id).await?;
        if !state.is_terminal() {
            self.transition(
                run_id,
                state,
                SupervisorRunState::Cancelled,
                "operator requested durable cancellation",
            )
            .await?;
        }
        self.status(run_id).await
    }

    /// Runs the bounded Pi advisor against one exact redacted pause. The
    /// deterministic pause remains authoritative; Pi can only propose a
    /// guidance replacement or escalate.
    pub async fn diagnose(
        &self,
        run_id: Uuid,
        runtime: Arc<dyn AgentRuntime>,
        advisor: AdvisorConfiguration,
    ) -> Result<AdvisorOutcome, SupervisorLoopError> {
        if self.current_state(run_id).await? != SupervisorRunState::Paused {
            return Err(SupervisorLoopError::Validation(
                "only a deterministically paused run can enter diagnosis".into(),
            ));
        }
        let run = self.load_run(run_id).await?;
        let contract = self.load_contract(run.contract_id).await?;
        let decision = self
            .supervisor_store
            .list_decisions(run_id)
            .await?
            .into_iter()
            .rev()
            .find(|value| value.state == SupervisorDecisionState::PauseForDiagnosis)
            .ok_or_else(|| {
                SupervisorLoopError::Validation(
                    "paused run has no deterministic diagnosis decision".into(),
                )
            })?;
        let window = self
            .supervisor_store
            .get_quality_window(decision.window_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!("quality window {}", decision.window_id))
            })?;
        let prompt = self
            .supervisor_store
            .get_prompt_version(decision.prompt_version_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!(
                    "prompt version {}",
                    decision.prompt_version_id
                ))
            })?;
        let brief = GenerationQualityDiagnosisBrief::create(
            Uuid::new_v4(),
            run_id,
            contract,
            decision,
            window,
            prompt,
            advisor,
            Utc::now(),
        )?;
        self.transition(
            run_id,
            SupervisorRunState::Paused,
            SupervisorRunState::Diagnosing,
            "bounded Pi diagnosis started from redacted aggregate evidence",
        )
        .await?;
        let runner =
            GenerationSupervisorAdvisorRunner::new(runtime, Arc::clone(&self.advisor_store));
        let session = runner.queue(brief).await?;
        let outcome = match runner.run(session.id).await {
            Ok(value) => value,
            Err(error) => {
                self.transition(
                    run_id,
                    SupervisorRunState::Diagnosing,
                    SupervisorRunState::Failed,
                    format!("bounded diagnosis failed: {error}"),
                )
                .await?;
                return Err(error.into());
            }
        };
        let (next, reason) = match outcome.session.state {
            AdvisorSessionState::AwaitingReview if outcome.proposal.is_some() => (
                SupervisorRunState::AwaitingReview,
                "Pi proposed a bounded guidance revision awaiting authorization",
            ),
            AdvisorSessionState::Escalated => (
                SupervisorRunState::Paused,
                "Pi escalated without changing generation guidance",
            ),
            AdvisorSessionState::Cancelled => (
                SupervisorRunState::Cancelled,
                "bounded diagnosis was cancelled",
            ),
            AdvisorSessionState::Failed => (
                SupervisorRunState::Failed,
                "bounded diagnosis ended without a valid proposal or escalation",
            ),
            state => {
                return Err(SupervisorLoopError::Validation(format!(
                    "advisor ended in nonterminal state {state:?}"
                )));
            }
        };
        self.transition(run_id, SupervisorRunState::Diagnosing, next, reason)
            .await?;
        Ok(outcome)
    }

    /// Appends an optional human review and, when authorized, creates an
    /// immutable candidate prompt. The candidate is not active until its
    /// deterministic canary passes.
    pub async fn authorize_revision(
        &self,
        run_id: Uuid,
        advisor_session_id: Uuid,
        review_input: Option<RevisionReviewInput>,
    ) -> Result<RevisionAuthorizationOutcome, SupervisorLoopError> {
        if self.current_state(run_id).await? != SupervisorRunState::AwaitingReview {
            return Err(SupervisorLoopError::Validation(
                "run is not awaiting a prompt revision decision".into(),
            ));
        }
        let session = self
            .advisor_store
            .get_session(advisor_session_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!("advisor session {advisor_session_id}"))
            })?;
        if session.supervisor_run_id != run_id
            || session.state != AdvisorSessionState::AwaitingReview
        {
            return Err(SupervisorLoopError::Validation(
                "advisor session is not the awaiting-review session for this run".into(),
            ));
        }
        let proposal = self
            .advisor_store
            .latest_proposal(advisor_session_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!(
                    "revision proposal for session {advisor_session_id}"
                ))
            })?;
        let predecessor = self
            .supervisor_store
            .latest_revision_review(proposal.id)
            .await?;
        let review = review_input
            .map(|input| {
                PromptRevisionReview::create(
                    Uuid::new_v4(),
                    &proposal,
                    predecessor.as_ref(),
                    input.decision,
                    input.reviewer,
                    input.rationale,
                    Utc::now(),
                )
            })
            .transpose()?;
        if let Some(review) = &review {
            self.supervisor_store.append_revision_review(review).await?;
            if review.decision != RevisionReviewDecision::Approve {
                self.transition(
                    run_id,
                    SupervisorRunState::AwaitingReview,
                    SupervisorRunState::Paused,
                    "prompt revision was not approved; current prompt remains active",
                )
                .await?;
                return Ok(RevisionAuthorizationOutcome {
                    review: Some(review.clone()),
                    authorization: None,
                    candidate_prompt: None,
                    status: self.status(run_id).await?,
                });
            }
        }
        let run = self.load_run(run_id).await?;
        let contract = self.load_contract(run.contract_id).await?;
        let parent = self
            .supervisor_store
            .get_prompt_version(proposal.parent_prompt_version_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!(
                    "parent prompt {}",
                    proposal.parent_prompt_version_id
                ))
            })?;
        let revision_count = self
            .supervisor_store
            .list_prompt_versions(run_id)
            .await?
            .into_iter()
            .filter(|value| value.sequence > 0)
            .count();
        if revision_count
            >= usize::try_from(contract.budgets.maximum_prompt_revisions).unwrap_or(usize::MAX)
        {
            return Err(SupervisorLoopError::Supervisor(
                SupervisorError::BudgetExhausted(
                    "prompt revision budget is exhausted before authorization".into(),
                ),
            ));
        }
        let (authorization, candidate) = PromptRevisionAuthorization::authorize(
            &contract,
            &proposal,
            &parent,
            review.as_ref(),
            Utc::now(),
        )?;
        self.supervisor_store
            .save_revision_authorization(&authorization, &candidate)
            .await?;
        self.transition(
            run_id,
            SupervisorRunState::AwaitingReview,
            SupervisorRunState::Canary,
            "authorized candidate prompt requires deterministic canary evidence",
        )
        .await?;
        Ok(RevisionAuthorizationOutcome {
            review,
            authorization: Some(authorization),
            candidate_prompt: Some(candidate),
            status: self.status(run_id).await?,
        })
    }

    /// Generates and audits only the candidate prompt's authorized scopes.
    /// Activation is persisted only when every scoped deterministic decision
    /// is `RevisionPassed`.
    pub async fn run_revision_canary(
        &self,
        run_id: Uuid,
    ) -> Result<SupervisorAdvanceOutcome, SupervisorLoopError> {
        if self.current_state(run_id).await? != SupervisorRunState::Canary {
            return Err(SupervisorLoopError::Validation(
                "run is not awaiting a revision canary".into(),
            ));
        }
        let run = self.load_run(run_id).await?;
        let contract = self.load_contract(run.contract_id).await?;
        if self.cancel_requested(run_id).await? {
            self.transition(
                run_id,
                SupervisorRunState::Canary,
                SupervisorRunState::Cancelled,
                "durable cancellation observed before revision canary I/O",
            )
            .await?;
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run_id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }
        if Utc::now()
            .signed_duration_since(run.created_at)
            .num_seconds()
            .max(0) as u64
            >= contract.budgets.maximum_duration_seconds
        {
            self.transition(
                run_id,
                SupervisorRunState::Canary,
                SupervisorRunState::Failed,
                "supervisor wall-clock budget was exhausted before revision canary I/O",
            )
            .await?;
            return Ok(SupervisorAdvanceOutcome {
                status: self.status(run_id).await?,
                generation_job_id: None,
                quality_audit_run_id: None,
                decisions: vec![],
            });
        }
        let dataset = self.load_dataset(contract.dataset.id).await?;
        let master_plan = self.load_plan(contract.plan.id).await?;
        self.verify_runtime(&contract, &dataset, &master_plan)?;
        let candidate = self.candidate_prompt(run_id).await?;
        let proposal_id = candidate.source_proposal_id.ok_or_else(|| {
            SupervisorLoopError::Validation("candidate prompt has no source proposal".into())
        })?;
        let proposal = self
            .advisor_store
            .get_proposal(proposal_id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!("revision proposal {proposal_id}"))
            })?;
        let authorization = self
            .supervisor_store
            .get_revision_authorization(proposal.id)
            .await?
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!(
                    "revision authorization for proposal {}",
                    proposal.id
                ))
            })?;
        let assignments = self
            .supervisor_store
            .get_strategy_assignments(run_id)
            .await?;
        let coverage = self
            .generation_store
            .dataset_cell_counts(dataset.id)
            .await?;
        let prior_observations = self.supervisor_store.list_row_observations(run.id).await?;
        let (segment_plan, schedule) = build_canary_inputs(
            &run,
            &candidate,
            &master_plan,
            &coverage,
            &prior_observations,
            assignments.as_ref(),
            &contract,
            &proposal.affected_scopes,
        )?;
        let requested_rows = segment_plan
            .cells
            .iter()
            .map(|planned| {
                let accepted = coverage
                    .get(&planned.cell.key())
                    .map_or(0, |counts| counts.accepted);
                u64::from(planned.target_count.saturating_sub(accepted))
            })
            .sum::<u64>();
        if requested_rows == 0 {
            return Err(SupervisorLoopError::Validation(
                "revision canary has no authorized rows to generate".into(),
            ));
        }
        let job = GenerationJob::queued(
            dataset.id,
            segment_plan.id,
            self.backend.name(),
            self.backend.model(),
            requested_rows,
        );
        let novelty_guard = self.novelty_guard()?;
        let execution = self.generation_execution(
            &contract,
            &segment_plan,
            &coverage,
            &job,
            &schedule,
            novelty_guard.as_ref(),
        )?;
        let reserved_generated_rows = requested_rows
            .checked_mul(u64::from(
                self.configuration
                    .generation_policy
                    .max_attempt_multiplier
                    .max(1),
            ))
            .ok_or_else(|| {
                SupervisorLoopError::Validation("canary generated-row budget overflowed".into())
            })?;
        let reservation = ChildReservation::create(
            Uuid::new_v4(),
            run.id,
            ChildKind::RevisionCanary,
            format!("revision-canary:{}", candidate.id),
            job.id,
            1,
            None,
            fingerprint(&(&segment_plan, &job, &execution, &schedule, &authorization))?,
            Utc::now(),
        )?
        .with_reserved_usage(SupervisorUsage {
            generation_segments: 1,
            generated_rows: reserved_generated_rows,
            revision_canaries: 1,
            ..SupervisorUsage::default()
        })?;
        self.supervisor_store.reserve_child(&reservation).await?;
        self.generation_store.create_plan(&segment_plan).await?;
        self.generation_store
            .create_generation_execution(&job, &execution)
            .await?;

        let generated = match self
            .job_runner(schedule, novelty_guard)
            .run(job.id, self.configuration.generation_parameters.clone())
            .await
        {
            Ok(job) if job.state == JobState::Completed => job,
            Ok(job) => {
                let reason = job
                    .error_message
                    .clone()
                    .unwrap_or_else(|| format!("revision canary ended in {:?}", job.state));
                self.finish_failed_child(&reservation, child_state(job.state), reason.clone())
                    .await?;
                self.transition(
                    run.id,
                    SupervisorRunState::Canary,
                    if job.state == JobState::Cancelled {
                        SupervisorRunState::Cancelled
                    } else {
                        SupervisorRunState::Failed
                    },
                    reason,
                )
                .await?;
                return Ok(SupervisorAdvanceOutcome {
                    status: self.status(run.id).await?,
                    generation_job_id: Some(job.id),
                    quality_audit_run_id: None,
                    decisions: vec![],
                });
            }
            Err(error) => {
                let reason = error.to_string();
                self.finish_failed_child(&reservation, ChildOutcomeState::Failed, reason.clone())
                    .await?;
                self.transition(
                    run.id,
                    SupervisorRunState::Canary,
                    SupervisorRunState::Failed,
                    reason,
                )
                .await?;
                return Ok(SupervisorAdvanceOutcome {
                    status: self.status(run.id).await?,
                    generation_job_id: Some(job.id),
                    quality_audit_run_id: None,
                    decisions: vec![],
                });
            }
        };
        self.supervisor_store
            .finish_child(&ChildOutcome::record(
                Uuid::new_v4(),
                &reservation,
                ChildOutcomeState::Succeeded,
                Some((generated.id, fingerprint(&generated)?)),
                None,
                Utc::now(),
            )?)
            .await?;
        let generated_rows = self.list_job_rows(generated.id).await?;
        let accepted_ids = generated_rows
            .iter()
            .filter(|row| row.validation_status == ValidationStatus::Accepted)
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let source_rows = if accepted_ids.is_empty() {
            vec![]
        } else {
            self.candidates
                .get_source_rows(dataset.id, accepted_ids)
                .await?
        };
        verify_segment_sources(generated.id, &source_rows)?;
        let (quality_run_id, assessments) = if source_rows.is_empty() {
            (None, vec![])
        } else {
            match self
                .audit_segment(&run, &contract, &dataset, source_rows.clone())
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.transition(
                        run.id,
                        SupervisorRunState::Canary,
                        SupervisorRunState::Failed,
                        format!("revision canary quality audit failed: {error}"),
                    )
                    .await?;
                    return Ok(SupervisorAdvanceOutcome {
                        status: self.status(run.id).await?,
                        generation_job_id: Some(generated.id),
                        quality_audit_run_id: None,
                        decisions: vec![],
                    });
                }
            }
        };
        let observations = build_row_observations(
            &contract,
            &run,
            &candidate,
            generated.id,
            &generated_rows,
            &source_rows,
            &assessments,
            assignments.as_ref(),
        )?;
        self.supervisor_store
            .save_row_observations(&observations)
            .await?;
        let decisions = self
            .persist_windows_and_decisions(&contract, &run, &candidate, &observations, true)
            .await?;
        let actual_scopes = decisions
            .iter()
            .map(|decision| decision.scope.clone())
            .collect::<BTreeSet<_>>();
        let passed = actual_scopes == proposal.affected_scopes
            && decisions
                .iter()
                .all(|decision| decision.state == SupervisorDecisionState::RevisionPassed);
        if passed {
            let activation = PromptRevisionActivation::create_for_decisions(
                Uuid::new_v4(),
                &candidate,
                &authorization,
                &decisions,
                Utc::now(),
            )?;
            self.supervisor_store
                .save_revision_activation(&activation)
                .await?;
            self.transition(
                run.id,
                SupervisorRunState::Canary,
                SupervisorRunState::Running,
                "every authorized revision scope passed; candidate prompt activated",
            )
            .await?;
        } else {
            self.transition(
                run.id,
                SupervisorRunState::Canary,
                SupervisorRunState::Paused,
                "revision canary failed or lacked evidence; current prompt remains active",
            )
            .await?;
        }
        Ok(SupervisorAdvanceOutcome {
            status: self.status(run.id).await?,
            generation_job_id: Some(generated.id),
            quality_audit_run_id: quality_run_id,
            decisions,
        })
    }

    /// Fails closed after a host interruption. Started external calls are
    /// marked uncertain and are never replayed by this recovery operation.
    pub async fn recover(&self, run_id: Uuid) -> Result<SupervisorStatus, SupervisorLoopError> {
        let state = self.current_state(run_id).await?;
        if state.is_terminal()
            || matches!(
                state,
                SupervisorRunState::Queued
                    | SupervisorRunState::Running
                    | SupervisorRunState::Paused
                    | SupervisorRunState::AwaitingReview
            )
        {
            return self.status(run_id).await;
        }
        let finished = self
            .supervisor_store
            .list_child_outcomes(run_id)
            .await?
            .into_iter()
            .map(|value| value.reservation_id)
            .collect::<BTreeSet<_>>();
        for reservation in self
            .supervisor_store
            .list_child_reservations(run_id)
            .await?
        {
            if finished.contains(&reservation.id) {
                continue;
            }
            if matches!(
                reservation.kind,
                ChildKind::GenerationSegment | ChildKind::RevisionCanary
            ) {
                let _ = self
                    .generation_store
                    .interrupt_open_generation_attempts(reservation.child_id)
                    .await;
            }
            self.supervisor_store
                .finish_child(&ChildOutcome::record(
                    Uuid::new_v4(),
                    &reservation,
                    ChildOutcomeState::Interrupted,
                    None,
                    Some("host stopped with an uncertain child outcome".into()),
                    Utc::now(),
                )?)
                .await?;
        }
        self.transition(
            run_id,
            state,
            SupervisorRunState::Failed,
            "recovery preserved uncertain calls and failed closed without replay",
        )
        .await?;
        self.status(run_id).await
    }

    pub async fn status(&self, run_id: Uuid) -> Result<SupervisorStatus, SupervisorLoopError> {
        let run = self.load_run(run_id).await?;
        let state = self.current_state(run_id).await?;
        let reservations = self
            .supervisor_store
            .list_child_reservations(run_id)
            .await?;
        let mut usage = reservations
            .iter()
            .try_fold(SupervisorUsage::default(), |usage, reservation| {
                usage.checked_add(&reservation.reserved_usage)
            })?;
        let sessions = self.advisor_store.list_sessions(run_id).await?;
        for session in &sessions {
            usage = usage.checked_add(&SupervisorUsage {
                pi_model_turns: session.usage.model_turns,
                pi_tool_calls: session.usage.tool_calls,
                pi_input_tokens: session.usage.input_tokens,
                pi_output_tokens: session.usage.output_tokens,
                cost_microunits: session.usage.cost_microunits.unwrap_or(0),
                ..SupervisorUsage::default()
            })?;
        }
        usage.prompt_revisions = self
            .supervisor_store
            .list_prompt_versions(run_id)
            .await?
            .into_iter()
            .filter(|version| version.sequence > 0)
            .count()
            .try_into()
            .map_err(|_| {
                SupervisorLoopError::Validation("prompt revision count exceeds u32".into())
            })?;
        usage.elapsed_seconds = Utc::now()
            .signed_duration_since(run.created_at)
            .num_seconds()
            .max(0) as u64;
        let observations = self.supervisor_store.list_row_observations(run_id).await?;
        let windows = self.supervisor_store.list_quality_windows(run_id).await?;
        let decisions = self.supervisor_store.list_decisions(run_id).await?;
        let mut latest_by_scope = BTreeMap::new();
        for decision in decisions {
            latest_by_scope.insert(decision.scope.clone(), decision);
        }
        Ok(SupervisorStatus {
            run,
            state,
            usage,
            active_prompt: self.active_prompt(run_id).await?,
            generation_segments: reservations
                .iter()
                .filter(|value| {
                    matches!(
                        value.kind,
                        ChildKind::GenerationSegment | ChildKind::RevisionCanary
                    )
                })
                .count()
                .try_into()
                .unwrap_or(u32::MAX),
            quality_audits: reservations
                .iter()
                .filter(|value| value.kind == ChildKind::QualityAudit)
                .count()
                .try_into()
                .unwrap_or(u32::MAX),
            observed_rows: observations.len().try_into().unwrap_or(u64::MAX),
            quality_windows: windows.len().try_into().unwrap_or(u64::MAX),
            latest_decisions: latest_by_scope.into_values().collect(),
            cancel_requested: self.cancel_requested(run_id).await?,
        })
    }

    pub async fn verify_integrity(&self) -> Result<SupervisorIntegrityReport, SupervisorLoopError> {
        Ok(self.supervisor_store.verify_supervisor_integrity().await?)
    }

    async fn audit_segment(
        &self,
        run: &SupervisorRun,
        contract: &GenerationQualityContract,
        dataset: &DatasetDefinition,
        source_rows: Vec<SourceRow>,
    ) -> Result<(Option<Uuid>, Vec<RowQualityAssessment>), SupervisorLoopError> {
        let guidance_references = guidance_references(contract)?;
        let guidance_fingerprint = self
            .configuration
            .evaluator_guidance
            .reproduce_fingerprint()?;
        let plan = AuditPlan::with_identity(
            Uuid::new_v4(),
            dataset,
            self.configuration.quality_policy.clone(),
            guidance_references,
            guidance_fingerprint,
            contract.evaluator.protocol_version.clone(),
            source_rows,
            Utc::now(),
        )?;
        self.configuration
            .evaluator_guidance
            .verify_against(&plan)?;
        let primary = self.primary_evaluator(contract)?;
        let audit_run = QualityAuditRun::queue(&plan, primary.identity(), vec![])?;
        let rounds = 1_u64;
        let requests = plan
            .policy
            .budgets
            .required_requests_for_rows(plan.selected_count(), rounds)?;
        let attempts = requests
            .checked_mul(u64::from(plan.policy.budgets.maximum_attempts_per_request))
            .ok_or_else(|| {
                SupervisorLoopError::Validation("quality attempt reservation overflowed".into())
            })?;
        let reservation = ChildReservation::create(
            Uuid::new_v4(),
            run.id,
            ChildKind::QualityAudit,
            format!("quality:{}", plan.source_set_fingerprint),
            audit_run.id,
            1,
            None,
            fingerprint(&(&plan, &audit_run, &self.configuration.evaluator_guidance))?,
            Utc::now(),
        )?
        .with_reserved_usage(SupervisorUsage {
            quality_audits: 1,
            evaluator_requests: requests.try_into().map_err(|_| {
                SupervisorLoopError::Validation("quality request count exceeds u32".into())
            })?,
            evaluator_attempts: attempts.try_into().map_err(|_| {
                SupervisorLoopError::Validation("quality attempt count exceeds u32".into())
            })?,
            evaluator_input_tokens: plan.policy.budgets.maximum_input_tokens,
            evaluator_output_tokens: plan.policy.budgets.maximum_output_tokens,
            evaluator_total_tokens: plan.policy.budgets.maximum_total_tokens,
            cost_microunits: plan.policy.budgets.maximum_cost_microusd.unwrap_or(0),
            ..SupervisorUsage::default()
        })?;
        self.supervisor_store.reserve_child(&reservation).await?;
        self.quality_store
            .create_audit(&plan, &audit_run, &self.configuration.evaluator_guidance)
            .await?;
        let runner = DatasetQualityRunner::new(
            Arc::clone(&self.quality_store),
            Arc::clone(&self.candidates),
            self.evaluators.clone(),
        )?;
        let outcome = match runner.execute(audit_run.id).await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.finish_failed_child(
                    &reservation,
                    ChildOutcomeState::Failed,
                    error.to_string(),
                )
                .await?;
                return Err(error.into());
            }
        };
        if outcome.run.state != QualityAuditRunState::Completed {
            let reason = outcome
                .run
                .error_message
                .clone()
                .unwrap_or_else(|| format!("quality audit ended in {:?}", outcome.run.state));
            self.finish_failed_child(
                &reservation,
                match outcome.run.state {
                    QualityAuditRunState::Cancelled => ChildOutcomeState::Cancelled,
                    _ => ChildOutcomeState::Failed,
                },
                reason.clone(),
            )
            .await?;
            return Err(SupervisorLoopError::Validation(reason));
        }
        self.supervisor_store
            .finish_child(&ChildOutcome::record(
                Uuid::new_v4(),
                &reservation,
                ChildOutcomeState::Succeeded,
                Some((outcome.run.id, fingerprint(&outcome.run)?)),
                None,
                Utc::now(),
            )?)
            .await?;
        let assessments = self.quality_store.list_assessments(outcome.run.id).await?;
        Ok((Some(outcome.run.id), assessments))
    }

    async fn persist_windows_and_decisions(
        &self,
        contract: &GenerationQualityContract,
        run: &SupervisorRun,
        prompt: &PromptGuidanceVersion,
        rows: &[RowQualityObservation],
        revision_canary: bool,
    ) -> Result<Vec<DeterministicQualityDecision>, SupervisorLoopError> {
        let prior_windows = self.supervisor_store.list_quality_windows(run.id).await?;
        let prior_decisions = self.supervisor_store.list_decisions(run.id).await?;
        let all_rows = self.supervisor_store.list_row_observations(run.id).await?;
        let mut grouped = BTreeMap::<QualityScope, Vec<RowQualityObservation>>::new();
        for row in rows {
            let scope = observation_scope(contract, row);
            grouped.entry(scope).or_default().push(row.clone());
        }
        let mut decisions = Vec::with_capacity(grouped.len());
        for (scope, _) in grouped {
            let previous_for_scope = prior_windows
                .iter()
                .filter(|window| window.scope == scope)
                .collect::<Vec<_>>();
            let kind = if revision_canary {
                QualityWindowKind::RevisionCanary
            } else if previous_for_scope.is_empty() {
                QualityWindowKind::InitialCanary
            } else {
                QualityWindowKind::Rolling
            };
            let mut scope_rows = all_rows
                .iter()
                .filter(|row| {
                    row.prompt_version_id == prompt.id && observation_scope(contract, row) == scope
                })
                .cloned()
                .collect::<Vec<_>>();
            scope_rows.sort_by_key(|row| (row.created_at, row.id));
            let window_size = match kind {
                QualityWindowKind::InitialCanary => {
                    contract.monitoring.initial_canary_rows_per_scope
                }
                QualityWindowKind::Rolling => contract.monitoring.rolling_window_rows_per_scope,
                QualityWindowKind::RevisionCanary => {
                    contract.monitoring.revision_canary_rows_per_scope
                }
            } as usize;
            if scope_rows.len() > window_size {
                if kind == QualityWindowKind::InitialCanary {
                    scope_rows.truncate(window_size);
                } else {
                    scope_rows = scope_rows.split_off(scope_rows.len() - window_size);
                }
            }
            let sequence = u32::try_from(previous_for_scope.len()).map_err(|_| {
                SupervisorLoopError::Validation("quality window sequence exceeds u32".into())
            })?;
            let aggregated = BatchQualityObservation::aggregate(
                Uuid::new_v4(),
                Uuid::new_v4(),
                contract,
                run.id,
                prompt.id,
                prompt.fingerprint.clone(),
                kind,
                sequence,
                scope.clone(),
                &scope_rows,
                Utc::now(),
            )?;
            self.supervisor_store
                .save_quality_window(&aggregated.manifest, &aggregated.observation)
                .await?;
            let baseline = select_baseline(contract, &scope, &prior_windows, &prior_decisions);
            let decision = DeterministicQualityDecision::evaluate(
                Uuid::new_v4(),
                contract,
                &aggregated.observation,
                baseline,
                Utc::now(),
            )?;
            self.supervisor_store.save_decision(&decision).await?;
            decisions.push(decision);
        }
        Ok(decisions)
    }

    fn generation_execution(
        &self,
        contract: &GenerationQualityContract,
        plan: &GenerationPlan,
        coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
        job: &GenerationJob,
        schedule: &SupervisedGenerationSchedule,
        novelty_guard: Option<&SourceExcerptNoveltyValidator>,
    ) -> Result<GenerationExecutionSpec, SupervisorLoopError> {
        let accepted = coverage
            .iter()
            .map(|(key, counts)| (key.clone(), counts.accepted))
            .collect();
        let base_prompt = self.base_prompt_identity()?;
        let prompt = PromptBuilder::supervision_template_identity(&base_prompt)?;
        let semantic = self
            .configuration
            .semantic_context
            .as_ref()
            .map_or_else(|| "none".to_owned(), |value| value.fingerprint.clone());
        let spec = GenerationExecutionSpec::new_with_all_contexts(
            job.id,
            plan.dataset_id,
            plan.id,
            calculate_generation_needs(plan, &accepted),
            contract.generator.backend.clone(),
            self.configuration.generation_parameters.clone(),
            GenerationExecutionPolicy::from(&self.configuration.generation_policy),
            prompt,
            semantic,
            self.configuration
                .authenticity_context
                .as_ref()
                .map(|value| value.fingerprint.clone()),
            novelty_guard.map(|value| value.fingerprint().to_owned()),
            None,
            self.configuration.construction_plan.clone(),
        )
        .map_err(|error| SupervisorLoopError::Validation(error.to_string()))?
        .with_supervision_schedule(schedule.fingerprint.clone())
        .map_err(|error| SupervisorLoopError::Validation(error.to_string()))?;
        Ok(spec)
    }

    fn job_runner(
        &self,
        schedule: SupervisedGenerationSchedule,
        novelty_guard: Option<SourceExcerptNoveltyValidator>,
    ) -> JobRunner {
        let mut runner = JobRunner::new(
            Arc::clone(&self.generation_store),
            Arc::clone(&self.backend),
            self.configuration.generation_policy.clone(),
            ValidationPipeline::standard(self.configuration.text_length),
        );
        if let Some(context) = &self.configuration.semantic_context {
            runner = runner.with_semantic_context(context.clone());
        }
        if let Some(context) = &self.configuration.authenticity_context {
            runner = runner.with_authenticity_context(context.clone());
        }
        if let Some(guard) = novelty_guard {
            runner = runner.with_source_novelty_guard(guard);
        }
        runner.with_supervision_schedule(schedule)
    }

    fn novelty_guard(&self) -> Result<Option<SourceExcerptNoveltyValidator>, SupervisorLoopError> {
        self.configuration
            .authenticity_context
            .as_ref()
            .map(|_| {
                SourceExcerptNoveltyValidator::new(
                    self.configuration
                        .authenticity_source_excerpts
                        .iter()
                        .map(String::as_str),
                )
            })
            .transpose()
            .map_err(Into::into)
    }

    fn base_prompt_identity(
        &self,
    ) -> Result<generation_core::jobs::PromptTemplateIdentity, SupervisorLoopError> {
        if self.configuration.authenticity_context.is_some() {
            Ok(PromptBuilder::authenticity_template_identity()?)
        } else {
            Ok(PromptBuilder::template_identity()?)
        }
    }

    fn verify_runtime(
        &self,
        contract: &GenerationQualityContract,
        dataset: &DatasetDefinition,
        plan: &GenerationPlan,
    ) -> Result<(), SupervisorLoopError> {
        contract.validate()?;
        if fingerprint(dataset)? != contract.dataset.fingerprint
            || plan.dataset_id != dataset.id
            || fingerprint(plan)? != contract.plan.fingerprint
            || self.backend.name() != contract.generator.backend.name
            || self.backend.model() != contract.generator.backend.model
        {
            return Err(SupervisorLoopError::Validation(
                "runtime dataset, plan, or generator does not match the immutable contract".into(),
            ));
        }
        let primary = self.primary_evaluator(contract)?;
        if primary.identity() != contract.evaluator {
            return Err(SupervisorLoopError::Validation(
                "runtime evaluator does not match the immutable contract".into(),
            ));
        }
        if self.configuration.quality_policy != contract.quality_policy {
            return Err(SupervisorLoopError::Validation(
                "quality evaluator policy does not match the immutable supervisor contract".into(),
            ));
        }
        match (
            &contract.semantic_context,
            &self.configuration.semantic_context,
        ) {
            (None, None) => {}
            (Some(binding), Some(context)) if binding.fingerprint == context.fingerprint => {}
            _ => {
                return Err(SupervisorLoopError::Validation(
                    "semantic runtime context does not match the contract".into(),
                ));
            }
        }
        match (
            &contract.authenticity_context,
            &self.configuration.authenticity_context,
        ) {
            (None, None) => {}
            (Some(binding), Some(context))
                if binding.id == context.binding_id
                    && binding.fingerprint == context.fingerprint => {}
            _ => {
                return Err(SupervisorLoopError::Validation(
                    "authenticity runtime context does not match the contract".into(),
                ));
            }
        }
        match (
            &contract.strategy_context,
            &self.configuration.strategy_context,
        ) {
            (None, None) => {}
            (Some(binding), Some(context))
                if binding.id == context.id && binding.fingerprint == context.fingerprint => {}
            _ => {
                return Err(SupervisorLoopError::Validation(
                    "strategy runtime context does not match the contract".into(),
                ));
            }
        }
        Ok(())
    }

    fn primary_evaluator(
        &self,
        contract: &GenerationQualityContract,
    ) -> Result<Arc<dyn QualityEvaluator>, SupervisorLoopError> {
        self.evaluators
            .iter()
            .find(|value| value.identity().fingerprint == contract.evaluator.fingerprint)
            .cloned()
            .ok_or_else(|| {
                SupervisorLoopError::Validation(format!(
                    "contract evaluator {} is not configured",
                    contract.evaluator.fingerprint
                ))
            })
    }

    async fn list_job_rows(&self, job_id: Uuid) -> Result<Vec<GeneratedRow>, SupervisorLoopError> {
        let mut output = Vec::new();
        let mut offset = 0_u32;
        loop {
            let batch = self
                .generation_store
                .list_rows(RowQuery {
                    job_id: Some(job_id),
                    limit: 10_000,
                    offset,
                    ..RowQuery::default()
                })
                .await?;
            let length = u32::try_from(batch.len()).unwrap_or(u32::MAX);
            output.extend(batch);
            if length < 10_000 {
                break;
            }
            offset = offset.checked_add(length).ok_or_else(|| {
                SupervisorLoopError::Validation("generated-row pagination overflowed".into())
            })?;
        }
        Ok(output)
    }

    async fn finish_failed_child(
        &self,
        reservation: &ChildReservation,
        state: ChildOutcomeState,
        reason: String,
    ) -> Result<(), SupervisorLoopError> {
        self.supervisor_store
            .finish_child(&ChildOutcome::record(
                Uuid::new_v4(),
                reservation,
                state,
                None,
                Some(reason),
                Utc::now(),
            )?)
            .await?;
        Ok(())
    }

    async fn transition(
        &self,
        run_id: Uuid,
        from: SupervisorRunState,
        to: SupervisorRunState,
        reason: impl Into<String>,
    ) -> Result<(), SupervisorLoopError> {
        let events = self.supervisor_store.list_run_events(run_id).await?;
        let actual = SupervisorRunEvent::verify_chain(run_id, &events)?;
        if actual != from {
            return Err(SupervisorLoopError::Validation(format!(
                "supervisor state changed concurrently: expected {from:?}, found {actual:?}"
            )));
        }
        let event = SupervisorRunEvent::transition(
            Uuid::new_v4(),
            run_id,
            events.last(),
            from,
            to,
            reason,
            Utc::now(),
        )?;
        self.supervisor_store.append_run_event(&event).await?;
        Ok(())
    }

    async fn current_state(&self, run_id: Uuid) -> Result<SupervisorRunState, SupervisorLoopError> {
        Ok(SupervisorRunEvent::verify_chain(
            run_id,
            &self.supervisor_store.list_run_events(run_id).await?,
        )?)
    }

    async fn active_prompt(
        &self,
        run_id: Uuid,
    ) -> Result<PromptGuidanceVersion, SupervisorLoopError> {
        let mut versions = self.supervisor_store.list_prompt_versions(run_id).await?;
        versions.sort_by_key(|value| value.sequence);
        for version in versions.into_iter().rev() {
            if version.sequence == 0
                || self
                    .supervisor_store
                    .get_revision_activation(version.id)
                    .await?
                    .is_some()
            {
                return Ok(version);
            }
        }
        Err(SupervisorLoopError::NotFound(format!(
            "active prompt for run {run_id}"
        )))
    }

    async fn candidate_prompt(
        &self,
        run_id: Uuid,
    ) -> Result<PromptGuidanceVersion, SupervisorLoopError> {
        let candidate = self
            .supervisor_store
            .list_prompt_versions(run_id)
            .await?
            .into_iter()
            .filter(|value| value.sequence > 0 && value.source_proposal_id.is_some())
            .max_by_key(|value| value.sequence)
            .ok_or_else(|| {
                SupervisorLoopError::NotFound(format!("candidate prompt for run {run_id}"))
            })?;
        if self
            .supervisor_store
            .get_revision_activation(candidate.id)
            .await?
            .is_some()
        {
            return Err(SupervisorLoopError::Validation(
                "latest candidate prompt is already active".into(),
            ));
        }
        Ok(candidate)
    }

    async fn cancel_requested(&self, run_id: Uuid) -> Result<bool, SupervisorLoopError> {
        self.supervisor_store
            .supervisor_cancel_requested(run_id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("supervisor run {run_id}")))
    }

    async fn contract_for_run(
        &self,
        run_id: Uuid,
    ) -> Result<GenerationQualityContract, SupervisorLoopError> {
        let run = self.load_run(run_id).await?;
        self.load_contract(run.contract_id).await
    }

    async fn load_run(&self, id: Uuid) -> Result<SupervisorRun, SupervisorLoopError> {
        self.supervisor_store
            .get_supervisor_run(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("supervisor run {id}")))
    }

    async fn load_contract(
        &self,
        id: Uuid,
    ) -> Result<GenerationQualityContract, SupervisorLoopError> {
        self.supervisor_store
            .get_contract(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("quality contract {id}")))
    }

    async fn load_dataset(&self, id: Uuid) -> Result<DatasetDefinition, SupervisorLoopError> {
        self.generation_store
            .get_dataset(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("dataset {id}")))
    }

    async fn load_plan(&self, id: Uuid) -> Result<GenerationPlan, SupervisorLoopError> {
        self.generation_store
            .get_plan(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("generation plan {id}")))
    }
}

fn observation_scope(
    contract: &GenerationQualityContract,
    row: &RowQualityObservation,
) -> QualityScope {
    match contract.monitoring.scope {
        MonitoringScope::Cell => QualityScope::Cell {
            cell_key: row.cell_key.clone(),
        },
        MonitoringScope::CellAndStrategy => QualityScope::CellStrategy {
            cell_key: row.cell_key.clone(),
            directive_id: row.strategy_directive_id,
        },
    }
}

fn validate_monitoring_support(
    contract: &GenerationQualityContract,
    plan: &GenerationPlan,
    coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
    assignments: Option<&StrategyAssignmentSet>,
) -> Result<(), SupervisorLoopError> {
    let minimum = contract.monitoring.minimum_evidence_rows_per_scope;
    match contract.monitoring.scope {
        MonitoringScope::Cell => {
            for planned in &plan.cells {
                let accepted = coverage
                    .get(&planned.cell.key())
                    .map_or(0, |counts| counts.accepted);
                let remaining = planned.target_count.saturating_sub(accepted);
                if remaining > 0 && remaining < minimum {
                    return Err(SupervisorLoopError::Validation(format!(
                        "cell {} has only {remaining} remaining rows but quality monitoring requires {minimum}",
                        planned.cell.key()
                    )));
                }
            }
        }
        MonitoringScope::CellAndStrategy => {
            let assignments = assignments.ok_or_else(|| {
                SupervisorLoopError::Validation(
                    "cell-and-strategy monitoring requires exact strategy assignments".into(),
                )
            })?;
            let mut remaining =
                BTreeMap::<generation_supervisor_core::strategy::StrategyScope, u32>::new();
            for assignment in &assignments.assignments {
                let accepted = coverage
                    .get(&assignment.cell_key)
                    .map_or(0, |counts| counts.accepted);
                if assignment.row_sequence >= accepted {
                    *remaining.entry(assignment.scope()).or_default() += 1;
                }
            }
            for (scope, count) in remaining {
                if count > 0 && count < minimum {
                    return Err(SupervisorLoopError::Validation(format!(
                        "strategy scope {} / {:?} has only {count} remaining rows but monitoring requires {minimum}",
                        scope.cell_key, scope.directive_id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn segment_plan(
    contract: &GenerationQualityContract,
    master: &GenerationPlan,
    coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
    observations: &[RowQualityObservation],
    rows_per_cell: u32,
) -> Result<GenerationPlan, SupervisorLoopError> {
    let effective = effective_coverage(contract, master, coverage, observations);
    let cells = master
        .cells
        .iter()
        .map(|planned| {
            let accepted = coverage
                .get(&planned.cell.key())
                .map_or(0, |counts| counts.accepted);
            let qualified = effective.get(&planned.cell.key()).copied().unwrap_or(0);
            let addition = planned
                .target_count
                .saturating_sub(qualified)
                .min(rows_per_cell);
            PlannedCell {
                cell: planned.cell.clone(),
                target_count: accepted.saturating_add(addition),
            }
        })
        .collect();
    GenerationPlan::new(master.dataset_id, cells)
        .map_err(|error| SupervisorLoopError::Validation(error.to_string()))
}

fn build_schedule(
    run: &SupervisorRun,
    prompt: &PromptGuidanceVersion,
    segment: &GenerationPlan,
    coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
    observations: &[RowQualityObservation],
    assignments: Option<&StrategyAssignmentSet>,
    contract: &GenerationQualityContract,
) -> Result<SupervisedGenerationSchedule, SupervisorLoopError> {
    let qualified_assignments = observations
        .iter()
        .filter(|row| row.contract_verdict(contract) == ContractRowVerdict::Qualified)
        .filter_map(|row| row.strategy_assignment_fingerprint.clone())
        .collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    for planned in &segment.cells {
        let cell_key = planned.cell.key();
        let accepted = coverage.get(&cell_key).map_or(0, |counts| counts.accepted);
        let supervised_accepted = observations
            .iter()
            .filter(|row| {
                row.cell_key == cell_key && row.structural_outcome == StructuralOutcome::Accepted
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let starting = accepted.saturating_sub(supervised_accepted);
        let pending = assignments
            .map(|set| {
                set.assignments
                    .iter()
                    .filter(|assignment| {
                        assignment.cell_key == cell_key
                            && assignment.row_sequence >= starting
                            && !qualified_assignments.contains(&assignment.fingerprint)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (offset, row_sequence) in (accepted..planned.target_count).enumerate() {
            let assignment = if assignments.is_some() {
                Some(*pending.get(offset).ok_or_else(|| {
                    SupervisorLoopError::Validation(format!(
                        "strategy schedule has no remaining assignment for {cell_key} replacement row {row_sequence}"
                    ))
                })?)
            } else {
                None
            };
            rows.push(SupervisedRowGuidance {
                cell_key: cell_key.clone(),
                row_sequence,
                strategy_assignment_fingerprint: assignment.map(|value| value.fingerprint.clone()),
                strategy_directive_id: assignment.and_then(|value| value.directive_id),
                strategy_instructions: assignment
                    .map(|value| value.instructions.clone())
                    .unwrap_or_default(),
            });
        }
    }
    SupervisedGenerationSchedule::create(
        run.id,
        prompt.id,
        prompt.fingerprint.clone(),
        prompt.guidance.clone(),
        assignments.map(|set| (set.id, set.fingerprint.clone())),
        rows,
    )
    .map_err(|error| SupervisorLoopError::Validation(error.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn build_canary_inputs(
    run: &SupervisorRun,
    prompt: &PromptGuidanceVersion,
    master: &GenerationPlan,
    coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
    observations: &[RowQualityObservation],
    assignments: Option<&StrategyAssignmentSet>,
    contract: &GenerationQualityContract,
    affected_scopes: &BTreeSet<QualityScope>,
) -> Result<(GenerationPlan, SupervisedGenerationSchedule), SupervisorLoopError> {
    if affected_scopes.is_empty() {
        return Err(SupervisorLoopError::Validation(
            "revision canary requires at least one affected scope".into(),
        ));
    }
    let qualified_assignments = observations
        .iter()
        .filter(|row| row.contract_verdict(contract) == ContractRowVerdict::Qualified)
        .filter_map(|row| row.strategy_assignment_fingerprint.clone())
        .collect::<BTreeSet<_>>();
    let mut selected_fingerprints = BTreeSet::new();
    let mut selected = BTreeMap::<
        String,
        Vec<Option<&generation_supervisor_core::strategy::StrategyAssignment>>,
    >::new();
    for scope in affected_scopes {
        let cell_key = scope.cell_key();
        if !master
            .cells
            .iter()
            .any(|planned| planned.cell.key() == cell_key)
        {
            return Err(SupervisorLoopError::Validation(format!(
                "revision scope references a cell outside the master plan: {cell_key}"
            )));
        }
        let required = usize::try_from(contract.monitoring.revision_canary_rows_per_scope)
            .unwrap_or(usize::MAX);
        if let Some(assignments) = assignments {
            let raw_accepted = coverage.get(cell_key).map_or(0, |counts| counts.accepted);
            let supervised_accepted = observations
                .iter()
                .filter(|row| {
                    row.cell_key == cell_key
                        && row.structural_outcome == StructuralOutcome::Accepted
                })
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            let starting = raw_accepted.saturating_sub(supervised_accepted);
            let candidates = assignments
                .assignments
                .iter()
                .filter(|assignment| {
                    assignment.cell_key == cell_key
                        && assignment.row_sequence >= starting
                        && !qualified_assignments.contains(&assignment.fingerprint)
                        && !selected_fingerprints.contains(&assignment.fingerprint)
                        && match scope {
                            QualityScope::Cell { .. } => true,
                            QualityScope::CellStrategy { directive_id, .. } => {
                                assignment.directive_id == *directive_id
                            }
                        }
                })
                .take(required)
                .collect::<Vec<_>>();
            if candidates.len() != required {
                return Err(SupervisorLoopError::Validation(format!(
                    "revision scope {cell_key} has {} unqualified assignments but requires {required} canary rows",
                    candidates.len()
                )));
            }
            for assignment in candidates {
                selected_fingerprints.insert(assignment.fingerprint.clone());
                selected
                    .entry(cell_key.to_owned())
                    .or_default()
                    .push(Some(assignment));
            }
        } else {
            if matches!(scope, QualityScope::CellStrategy { .. }) {
                return Err(SupervisorLoopError::Validation(
                    "cell-and-strategy canary has no exact assignment set".into(),
                ));
            }
            selected
                .entry(cell_key.to_owned())
                .or_default()
                .extend(std::iter::repeat_n(None, required));
        }
    }
    let cells = master
        .cells
        .iter()
        .map(|planned| {
            let cell_key = planned.cell.key();
            let raw_accepted = coverage.get(&cell_key).map_or(0, |counts| counts.accepted);
            let addition = selected
                .get(&cell_key)
                .map_or(0, |values| u32::try_from(values.len()).unwrap_or(u32::MAX));
            PlannedCell {
                cell: planned.cell.clone(),
                target_count: raw_accepted.saturating_add(addition),
            }
        })
        .collect();
    let plan = GenerationPlan::new(master.dataset_id, cells)
        .map_err(|error| SupervisorLoopError::Validation(error.to_string()))?;
    let mut rows = Vec::new();
    for planned in &plan.cells {
        let cell_key = planned.cell.key();
        let raw_accepted = coverage.get(&cell_key).map_or(0, |counts| counts.accepted);
        for (offset, assignment) in selected.get(&cell_key).into_iter().flatten().enumerate() {
            rows.push(SupervisedRowGuidance {
                cell_key: cell_key.clone(),
                row_sequence: raw_accepted
                    .saturating_add(u32::try_from(offset).unwrap_or(u32::MAX)),
                strategy_assignment_fingerprint: assignment.map(|value| value.fingerprint.clone()),
                strategy_directive_id: assignment.and_then(|value| value.directive_id),
                strategy_instructions: assignment
                    .map(|value| value.instructions.clone())
                    .unwrap_or_default(),
            });
        }
    }
    let schedule = SupervisedGenerationSchedule::create(
        run.id,
        prompt.id,
        prompt.fingerprint.clone(),
        prompt.guidance.clone(),
        assignments.map(|set| (set.id, set.fingerprint.clone())),
        rows,
    )
    .map_err(|error| SupervisorLoopError::Validation(error.to_string()))?;
    Ok((plan, schedule))
}

fn effective_coverage(
    contract: &GenerationQualityContract,
    master: &GenerationPlan,
    coverage: &BTreeMap<String, generation_core::coverage::CellCounts>,
    observations: &[RowQualityObservation],
) -> BTreeMap<String, u32> {
    master
        .cells
        .iter()
        .map(|planned| {
            let cell_key = planned.cell.key();
            let raw_accepted = coverage.get(&cell_key).map_or(0, |counts| counts.accepted);
            let supervised_accepted = observations
                .iter()
                .filter(|row| {
                    row.cell_key == cell_key
                        && row.structural_outcome == StructuralOutcome::Accepted
                })
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            let starting_accepted = raw_accepted.saturating_sub(supervised_accepted);
            let qualified_rows = observations.iter().filter(|row| {
                row.cell_key == cell_key
                    && row.contract_verdict(contract) == ContractRowVerdict::Qualified
            });
            let qualified = if contract.strategy_context.is_some() {
                qualified_rows
                    .filter_map(|row| row.strategy_assignment_fingerprint.as_deref())
                    .collect::<BTreeSet<_>>()
                    .len()
            } else {
                qualified_rows.count()
            }
            .try_into()
            .unwrap_or(u32::MAX);
            (cell_key, starting_accepted.saturating_add(qualified))
        })
        .collect()
}

fn guidance_references(
    contract: &GenerationQualityContract,
) -> Result<GuidanceReferences, SupervisorLoopError> {
    Ok(GuidanceReferences {
        semantic_context: contract
            .semantic_context
            .as_ref()
            .map(|value| GuidanceReference::new(value.id, value.fingerprint.clone()))
            .transpose()?,
        authenticity_context: contract
            .authenticity_context
            .as_ref()
            .map(|value| GuidanceReference::new(value.id, value.fingerprint.clone()))
            .transpose()?,
    })
}

fn verify_segment_sources(job_id: Uuid, rows: &[SourceRow]) -> Result<(), SupervisorLoopError> {
    if rows.iter().any(|row| {
        !matches!(
            row.provenance,
            SourceProvenance::Generated {
                generation_job_id,
                ..
            } if generation_job_id == job_id
        )
    }) {
        return Err(SupervisorLoopError::Validation(
            "quality audit source rows escaped the exact generation segment".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_row_observations(
    contract: &GenerationQualityContract,
    run: &SupervisorRun,
    prompt: &PromptGuidanceVersion,
    segment_id: Uuid,
    rows: &[GeneratedRow],
    sources: &[SourceRow],
    assessments: &[RowQualityAssessment],
    assignments: Option<&StrategyAssignmentSet>,
) -> Result<Vec<RowQualityObservation>, SupervisorLoopError> {
    let sources = sources
        .iter()
        .map(|value| (value.id, value))
        .collect::<BTreeMap<_, _>>();
    let assessments = assessments
        .iter()
        .filter(|value| value.evaluator.fingerprint == contract.evaluator.fingerprint)
        .map(|value| (value.source_row_id, value))
        .collect::<BTreeMap<_, _>>();
    let assignment_index = assignments
        .map(|set| {
            set.assignments
                .iter()
                .map(|value| (value.fingerprint.as_str(), value))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let difficulty = rows
        .first()
        .and_then(|row| {
            row.dimensions
                .keys()
                .find(|name| name.eq_ignore_ascii_case("difficulty"))
        })
        .map(String::as_str);
    let mut exact_seen = BTreeSet::new();
    let mut normalized_seen = BTreeSet::new();
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        let attempt_id = metadata_uuid(&row.generation_metadata, "generation_attempt_id")?;
        let guidance: SupervisedRowGuidance = serde_json::from_value(
            row.generation_metadata
                .get("supervision")
                .cloned()
                .ok_or_else(|| {
                    SupervisorLoopError::Validation(format!(
                        "generated row {} has no supervision metadata",
                        row.id
                    ))
                })?,
        )?;
        let assignment = guidance
            .strategy_assignment_fingerprint
            .as_deref()
            .map(|value| {
                assignment_index.get(value).copied().ok_or_else(|| {
                    SupervisorLoopError::Validation(
                        "generated row references an unknown strategy assignment".into(),
                    )
                })
            })
            .transpose()?;
        let source = sources.get(&row.id).copied();
        let assessment = assessments.get(&row.id).copied();
        let evidence = assessment
            .map(|value| AssessmentEvidence::from_assessment(value, difficulty, None))
            .transpose()?;
        let exact_duplicate = !exact_seen.insert(row.text.clone());
        let normalized_duplicate = row
            .validation_errors
            .iter()
            .any(|value| value.starts_with("duplicate_text:"))
            || !normalized_seen.insert(row.normalized_text.clone());
        let observed_patterns = contract
            .batch_thresholds
            .required_patterns
            .iter()
            .filter(|pattern| row.normalized_text.contains(&pattern.trim().to_lowercase()))
            .cloned()
            .collect();
        let source_binding = source
            .map(|value| {
                fingerprint(value)
                    .map(|source_fingerprint| (value.id, source_fingerprint))
                    .map_err(SupervisorLoopError::from)
            })
            .transpose()?;
        output.push(RowQualityObservation::create(
            Uuid::new_v4(),
            contract,
            run.id,
            segment_id,
            attempt_id,
            prompt.id,
            prompt.fingerprint.clone(),
            row,
            source_binding,
            assignment,
            exact_duplicate,
            normalized_duplicate,
            observed_patterns,
            evidence,
            Utc::now(),
        )?);
    }
    Ok(output)
}

fn metadata_uuid(value: &serde_json::Value, key: &str) -> Result<Uuid, SupervisorLoopError> {
    serde_json::from_value(value.get(key).cloned().ok_or_else(|| {
        SupervisorLoopError::Validation(format!("generated row metadata has no {key}"))
    })?)
    .map_err(Into::into)
}

fn select_baseline<'a>(
    contract: &GenerationQualityContract,
    scope: &QualityScope,
    windows: &'a [BatchQualityObservation],
    decisions: &[DeterministicQualityDecision],
) -> Option<&'a BatchQualityObservation> {
    let mut candidates = windows
        .iter()
        .filter(|window| {
            &window.scope == scope
                && window.counts.assessed >= contract.monitoring.baseline_minimum_rows_per_scope
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|window| (window.created_at, window.sequence, window.id));
    match contract.monitoring.baseline_policy {
        BaselinePolicy::InitialCanary => candidates
            .into_iter()
            .find(|window| window.kind == QualityWindowKind::InitialCanary),
        BaselinePolicy::FirstQualifiedWindow => candidates.into_iter().find(|window| {
            decisions.iter().any(|decision| {
                decision.window_id == window.id
                    && decision.state == SupervisorDecisionState::Healthy
            })
        }),
    }
}

fn child_state(state: JobState) -> ChildOutcomeState {
    match state {
        JobState::Cancelled => ChildOutcomeState::Cancelled,
        JobState::Queued | JobState::Running | JobState::Failed => ChildOutcomeState::Failed,
        JobState::Completed => ChildOutcomeState::Succeeded,
    }
}
