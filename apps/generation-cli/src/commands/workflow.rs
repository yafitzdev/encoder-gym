use std::{collections::BTreeMap, future::Future, pin::Pin, time::Duration};

use advisor_fake::FakeAnalysisAdvisor;
use advisor_openai_compatible::OpenAICompatibleAdvisor;
use analysis_core::{ports::AnalysisStore, runner::run_analysis};
use anyhow::{Context, ensure};
use dataset_quality_core::{lifecycle::QualityAuditRunState, ports::DatasetQualityStore};
use evaluation_core::ports::EvaluationStore;
use generation_core::{
    coverage::calculate_coverage,
    dimensions::expand_generation_cells,
    jobs::JobState,
    ports::{DatasetStore, JobQuery, JobStore, PlanStore, RowStore},
};
use optimization_core::{
    ports::OptimizationStore,
    reviews::{ProposalReviewRecord, ProposalReviewState},
};
use project_config::ProjectConfigurationStore;
use recovery_core::{RecoveryState, RecoveryStore, WorkflowKind};
use semantic_catalog::SemanticCatalogStore;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    advisor::{
        AdvisorEgressPolicy, AdvisoryActionKind, AdvisoryExample, AdvisoryFinding, AdvisoryRequest,
        AnalysisAdvisor, run_advisor,
    },
    allocation::{
        InitialAllocationFeasibility, InitialAllocationRecord, InitialAllocationRequest,
        InitialCellCoverage, allocate_initial_budget,
    },
    approval::{WorkflowApprovalDecision, WorkflowApprovalMode},
    benchmark::{BenchmarkSuiteKind, CohortAssessmentInput, assess_benchmark},
    governance::{
        DisclosureLevel, EvidenceExposure, EvidenceExposureRequest, ExposurePurpose,
        summarize_exposure_risk,
    },
    ports::{
        AcceptanceAssessmentQuery, AdvisorStore, AdvisoryAssessmentQuery, BenchmarkStore,
        ExposureQuery, GovernanceStore, InitialAllocationQuery, InitialAllocationStore,
        PromotionStore, StopDecisionStore, WorkflowApprovalStore, WorkflowDefinitionQuery,
        WorkflowRunQuery, WorkflowRunStore,
    },
    promotion::ModelPromotion,
    stop::decide,
    workflow::{
        StageAttemptState, StageOutcome, TrainingIterationPolicy, WorkflowArtifactLink,
        WorkflowBudgetUsage, WorkflowDefinition, WorkflowDefinitionRequest, WorkflowRun,
        WorkflowRunState, WorkflowStage, WorkflowStageAttempt,
    },
};

use crate::cli::{WorkflowCommand, WorkflowRunStateArg};
use crate::document::read as read_document;

mod artifacts;
mod quality_gate;
mod queries;
mod stage_execution;

use artifacts::{artifact_id, artifact_ids, artifact_ids_for_stage, artifact_link, link};
use queries::{print_status, require_definition, require_run, resolve_pending_workflow_recovery};
use stage_execution::{StageExecution, execute_initial_stage};

pub async fn execute(command: WorkflowCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        WorkflowCommand::Define { definition } => {
            let request: WorkflowDefinitionRequest = read_document(&definition)?;
            let definition = WorkflowDefinition::new(request)?;
            store.create_workflow_definition(&definition).await?;
            crate::presentation::print(&definition)
        }
        WorkflowCommand::DefinitionShow { id } => {
            crate::presentation::print(&require_definition(store, id).await?)
        }
        WorkflowCommand::DefinitionList { dataset_id, page } => {
            let definitions = store
                .query_workflow_definitions(WorkflowDefinitionQuery {
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&definitions, definitions.len(), page)
        }
        WorkflowCommand::Start {
            definition_id,
            initialize_only,
        } => {
            let definition = require_definition(store, definition_id).await?;
            let mut run = WorkflowRun::queued(&definition)?;
            let attempt = WorkflowStageAttempt::start(
                &definition,
                &mut run,
                WorkflowStage::InitialAllocation,
                None,
                1,
            )?;
            store.create_workflow_run(&run, &attempt).await?;
            if initialize_only {
                crate::presentation::print(&serde_json::json!({
                    "run": run,
                    "attempt": attempt,
                }))
            } else {
                let run = drive_initial_pipeline(store, definition, run, attempt).await?;
                print_status(store, run).await
            }
        }
        WorkflowCommand::Resume { id } => {
            let mut run = require_run(store, id).await?;
            let definition = require_definition(store, run.definition_id).await?;
            let previous = store
                .list_workflow_attempts(id)
                .await?
                .pop()
                .context("workflow run has no stage attempt")?;
            let retrying = previous.state == StageAttemptState::Failed && previous.retryable;
            let attempt = if retrying {
                WorkflowStageAttempt::start(
                    &definition,
                    &mut run,
                    previous.stage,
                    Some(&previous),
                    previous.attempt.saturating_add(1),
                )?
            } else {
                previous
            };
            if retrying {
                store
                    .commit_workflow_attempt(
                        &run,
                        &attempt,
                        attempt
                            .predecessor_id
                            .context("retry attempt has no predecessor")?,
                    )
                    .await?;
            }
            let run = drive_initial_pipeline(store, definition, run, attempt).await?;
            resolve_pending_workflow_recovery(store, id).await?;
            print_status(store, run).await
        }
        WorkflowCommand::Watch { id, poll_ms } => {
            ensure!(poll_ms > 0, "poll interval must be positive");
            loop {
                let run = require_run(store, id).await?;
                if !matches!(
                    run.state,
                    WorkflowRunState::Queued | WorkflowRunState::Running
                ) {
                    break print_status(store, run).await;
                }
                tokio::time::sleep(Duration::from_millis(poll_ms)).await;
            }
        }
        WorkflowCommand::Approve {
            id,
            recommendation_ids,
            note,
        } => approve_and_resume(store, id, recommendation_ids, note).await,
        WorkflowCommand::ApprovalShow { id } => crate::presentation::print(
            &store
                .get_workflow_approval(id)
                .await?
                .with_context(|| format!("workflow approval not found: {id}"))?,
        ),
        WorkflowCommand::StopShow { id } => crate::presentation::print(
            &store
                .get_stop_decision(id)
                .await?
                .with_context(|| format!("workflow stop decision not found: {id}"))?,
        ),
        WorkflowCommand::Finalize { id } => finalize(store, id).await,
        WorkflowCommand::Promote { id } => promote(store, id).await,
        WorkflowCommand::PromotionShow { id } => crate::presentation::print(
            &store
                .get_promotion(id)
                .await?
                .with_context(|| format!("model promotion not found: {id}"))?,
        ),
        WorkflowCommand::Status { id } => {
            let run = require_run(store, id).await?;
            print_status(store, run).await
        }
        WorkflowCommand::List {
            definition_id,
            state,
            page,
        } => {
            let runs = store
                .query_workflow_runs(WorkflowRunQuery {
                    definition_id,
                    state: state.map(run_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&runs, runs.len(), page)
        }
        WorkflowCommand::History { id } => {
            require_run(store, id).await?;
            crate::presentation::print(&store.list_workflow_attempts(id).await?)
        }
        WorkflowCommand::Cancel { id } => {
            let mut run = require_run(store, id).await?;
            let attempts = store.list_workflow_attempts(id).await?;
            if let Some(current) = attempts.last() {
                let plan_kind = match current.stage {
                    WorkflowStage::Generation => Some("generation_plan"),
                    WorkflowStage::DatasetDiffGeneration => Some("iteration_generation_plan"),
                    _ => None,
                };
                if let Some(kind) = plan_kind
                    && let Ok(plan_id) = artifact_id(&attempts, kind)
                {
                    for job in store
                        .list_jobs(JobQuery {
                            plan_id: Some(plan_id),
                            limit: 10_000,
                            ..JobQuery::default()
                        })
                        .await?
                    {
                        if matches!(job.state, JobState::Queued | JobState::Running) {
                            store.request_job_cancellation(job.id).await?;
                        }
                    }
                }
                if matches!(
                    current.stage,
                    WorkflowStage::QualityAudit | WorkflowStage::IterationQualityAudit
                ) {
                    let definition = require_definition(store, run.definition_id).await?;
                    let audit_run_id =
                        quality_gate::audit_run_id(&definition, &run, current.stage)?;
                    if let Some(mut audit_run) = store.get_audit_run(audit_run_id).await?
                        && matches!(
                            audit_run.state,
                            QualityAuditRunState::Queued | QualityAuditRunState::Running
                        )
                    {
                        if audit_run.state == QualityAuditRunState::Queued {
                            audit_run.cancel()?;
                        } else {
                            audit_run.request_cancel()?;
                        }
                        store.save_audit_run(&audit_run).await?;
                    }
                }
            }
            let expected = run.latest_attempt_id;
            run.request_cancel()?;
            store.save_workflow_run(&run, expected).await?;
            crate::presentation::print(&run)
        }
    }
}

async fn drive_initial_pipeline(
    store: &SqliteStore,
    definition: WorkflowDefinition,
    run: WorkflowRun,
    attempt: WorkflowStageAttempt,
) -> anyhow::Result<WorkflowRun> {
    const WORKFLOW_STACK_BYTES: usize = 16 * 1024 * 1024;

    let store = store.clone();
    let runtime = tokio::runtime::Handle::current();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("encoder-workflow".into())
        .stack_size(WORKFLOW_STACK_BYTES)
        .spawn(move || {
            let result = runtime.block_on(drive_initial_pipeline_on_thread(
                &store, definition, run, attempt,
            ));
            let _ = sender.send(result);
        })
        .context("could not start workflow execution thread")?;
    receiver
        .await
        .context("workflow execution thread exited unexpectedly")?
}

async fn drive_initial_pipeline_on_thread(
    store: &SqliteStore,
    definition: WorkflowDefinition,
    run: WorkflowRun,
    attempt: WorkflowStageAttempt,
) -> anyhow::Result<WorkflowRun> {
    let run_id = run.id;
    store
        .acquire_execution_lease(WorkflowKind::EncoderWorkflow, run_id)
        .await?;
    let result = drive_pipeline_inner(store, definition, run, attempt).await;
    let release = store
        .release_execution_lease(WorkflowKind::EncoderWorkflow, run_id)
        .await;
    let run = result?;
    release?;
    Ok(run)
}

async fn drive_pipeline_inner(
    store: &SqliteStore,
    definition: WorkflowDefinition,
    mut run: WorkflowRun,
    mut attempt: WorkflowStageAttempt,
) -> anyhow::Result<WorkflowRun> {
    let persisted_configuration = store
        .get_project_configuration(definition.project_configuration_id)
        .await?
        .context("workflow project configuration not found")?;
    ensure!(
        persisted_configuration.fingerprint == definition.project_configuration_fingerprint,
        "workflow project configuration fingerprint changed"
    );
    loop {
        let cancellable_attempt = attempt.state == StageAttemptState::Running
            || matches!(
                attempt.state,
                StageAttemptState::AwaitingApproval | StageAttemptState::AwaitingUser
            )
            || (attempt.state == StageAttemptState::Failed && attempt.retryable);
        if run.cancel_requested && cancellable_attempt {
            let usage = run.usage.clone();
            let finished = attempt.finish(
                &definition,
                &mut run,
                StageOutcome {
                    state: StageAttemptState::Cancelled,
                    reason: Some("workflow cancellation requested".into()),
                    retryable: false,
                    artifacts: Vec::new(),
                    usage_after: usage,
                },
            )?;
            store
                .commit_workflow_attempt(&run, &finished, attempt.id)
                .await?;
            return Ok(run);
        }
        if attempt.state == StageAttemptState::Completed {
            if run.state == WorkflowRunState::DevelopmentComplete {
                return Ok(run);
            }
            if attempt.stage == WorkflowStage::StopDecision
                && !attempt
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == "workflow_continue")
            {
                return Ok(run);
            }
            let Some(next) = initial_successor(
                attempt.stage,
                definition.policy.enable_advisor,
                definition.quality_gate.is_some(),
            ) else {
                return Ok(run);
            };
            let started =
                WorkflowStageAttempt::start(&definition, &mut run, next, Some(&attempt), 1)?;
            store
                .commit_workflow_attempt(&run, &started, attempt.id)
                .await?;
            attempt = started;
            continue;
        }
        if attempt.state == StageAttemptState::AwaitingApproval
            && attempt.stage == WorkflowStage::Approval
            && store
                .get_iteration_workflow_approval(run.id, run.iteration)
                .await?
                .is_some()
        {
            let started = WorkflowStageAttempt::start(
                &definition,
                &mut run,
                WorkflowStage::ProposalApplication,
                Some(&attempt),
                1,
            )?;
            store
                .commit_workflow_attempt(&run, &started, attempt.id)
                .await?;
            attempt = started;
            continue;
        }
        if attempt.state == StageAttemptState::AwaitingUser
            && matches!(
                attempt.stage,
                WorkflowStage::CurationReview | WorkflowStage::IterationCurationReview
            )
        {
            let history = store.list_workflow_attempts(run.id).await?;
            let gate = quality_gate::approved_manifest(store, &definition, &history, attempt.stage)
                .await?;
            if gate.approved.is_none() {
                let proposal_kind = quality_gate::proposal_kind(attempt.stage)?;
                let paused_proposal_id = attempt
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == proposal_kind)
                    .map(|artifact| artifact.artifact_id);
                if paused_proposal_id != Some(gate.proposal.id) {
                    // Append a replacement pause only when new append-only row
                    // reviews produced an immutable successor proposal. This
                    // keeps status pointed at the exact proposal to approve
                    // without counting a retry or rewriting history.
                    let started = WorkflowStageAttempt::start(
                        &definition,
                        &mut run,
                        attempt.stage,
                        Some(&attempt),
                        attempt.attempt,
                    )?;
                    store
                        .commit_workflow_attempt(&run, &started, attempt.id)
                        .await?;
                    attempt = started;
                    continue;
                }
                // Repeated resumes before the exact latest proposal is
                // approved are intentionally no-ops: the persisted pause and
                // proposal identity remain unchanged and consume no retries.
                return Ok(run);
            }
            let approval_stage = match attempt.stage {
                WorkflowStage::CurationReview => WorkflowStage::CurationApproval,
                WorkflowStage::IterationCurationReview => WorkflowStage::IterationCurationApproval,
                _ => unreachable!("matched curation review stage"),
            };
            let started = WorkflowStageAttempt::start(
                &definition,
                &mut run,
                approval_stage,
                Some(&attempt),
                1,
            )?;
            store
                .commit_workflow_attempt(&run, &started, attempt.id)
                .await?;
            attempt = started;
            continue;
        }
        if attempt.state != StageAttemptState::Running {
            return Ok(run);
        }
        let execution = execute_initial_stage(
            store,
            &definition,
            &run,
            &attempt,
            &persisted_configuration.resolved,
        )
        .await;
        let persisted_run = require_run(store, run.id).await?;
        if persisted_run.cancel_requested {
            run = persisted_run;
            let (artifacts, usage_after) = execution.ok().map_or_else(
                || (Vec::new(), run.usage.clone()),
                |value| (value.artifacts, value.usage),
            );
            let cancelled = attempt.finish(
                &definition,
                &mut run,
                StageOutcome {
                    state: StageAttemptState::Cancelled,
                    reason: Some("workflow cancellation requested".into()),
                    retryable: false,
                    artifacts,
                    usage_after,
                },
            )?;
            store
                .commit_workflow_attempt(&run, &cancelled, attempt.id)
                .await?;
            return Ok(run);
        }
        let execution = match execution {
            Ok(execution) => execution,
            Err(error) => {
                let retryable = attempt.attempt < definition.budget.maximum_stage_attempts;
                let usage = run.usage.clone();
                let failed = attempt.finish(
                    &definition,
                    &mut run,
                    StageOutcome {
                        state: StageAttemptState::Failed,
                        reason: Some(error.to_string()),
                        retryable,
                        artifacts: Vec::new(),
                        usage_after: usage,
                    },
                )?;
                store
                    .commit_workflow_attempt(&run, &failed, attempt.id)
                    .await?;
                return Ok(run);
            }
        };
        let finished = attempt.finish(
            &definition,
            &mut run,
            StageOutcome {
                state: execution.state,
                reason: execution.reason,
                retryable: false,
                artifacts: execution.artifacts,
                usage_after: execution.usage,
            },
        )?;
        store
            .commit_workflow_attempt(&run, &finished, attempt.id)
            .await?;
        attempt = finished;
    }
}

async fn execute_approval_stage(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    configured: &project_config::ResolvedProjectConfig,
    usage: WorkflowBudgetUsage,
) -> anyhow::Result<StageExecution> {
    if let Some(decision) = store
        .get_iteration_workflow_approval(run.id, run.iteration)
        .await?
    {
        return Ok(StageExecution::completed(
            vec![
                link("workflow_approval", decision.id, &decision.fingerprint),
                link(
                    "proposal_review",
                    decision.proposal_review_id,
                    &decision.proposal_review_fingerprint,
                ),
            ],
            usage,
        ));
    }
    let history = store.list_workflow_attempts(run.id).await?;
    let proposal_id = artifact_id(&history, "optimization_proposal")?;
    let proposal = store
        .get_optimization_proposal(proposal_id)
        .await?
        .context("workflow optimization proposal not found")?;
    let approved_rows = proposal.allocated_count();
    if approved_rows == 0 {
        return Ok(StageExecution {
            artifacts: Vec::new(),
            usage,
            state: StageAttemptState::Inconclusive,
            reason: Some("optimization proposal has no eligible data recommendation".into()),
        });
    }
    let workflow_core::workflow::IterationGovernance::PreauthorizedBounded { envelope } =
        &definition.governance
    else {
        return Ok(StageExecution {
            artifacts: Vec::new(),
            usage,
            state: StageAttemptState::AwaitingApproval,
            reason: Some("explicit proposal approval is required".into()),
        });
    };
    validate_preauthorization(definition, configured, envelope, &usage, approved_rows)?;
    let review = ProposalReviewRecord::new(
        &proposal,
        ProposalReviewState::ApprovedForPlanCreation,
        Vec::new(),
        Some("approved by persisted bounded preauthorization envelope".into()),
        None,
        None,
    )?;
    store.append_proposal_review(&review).await?;
    let decision = WorkflowApprovalDecision::new(
        run.id,
        run.iteration,
        proposal.id,
        proposal.fingerprint.clone(),
        review.id,
        review.fingerprint.clone(),
        WorkflowApprovalMode::PreauthorizedEnvelope,
        approved_rows,
        usage.clone(),
        Some(envelope.clone()),
        Some("exact action fits persisted preauthorization".into()),
    )?;
    store.create_workflow_approval(&decision).await?;
    Ok(StageExecution::completed(
        vec![
            link("workflow_approval", decision.id, &decision.fingerprint),
            link("proposal_review", review.id, &review.fingerprint),
        ],
        usage,
    ))
}

async fn approve_and_resume(
    store: &SqliteStore,
    id: uuid::Uuid,
    recommendation_ids: Vec<String>,
    note: Option<String>,
) -> anyhow::Result<()> {
    let run = require_run(store, id).await?;
    ensure!(
        run.state == WorkflowRunState::AwaitingApproval,
        "workflow is not awaiting approval"
    );
    let definition = require_definition(store, run.definition_id).await?;
    ensure!(
        matches!(
            definition.governance,
            workflow_core::workflow::IterationGovernance::ReviewEachIteration
        ),
        "workflow uses bounded preauthorization and does not accept manual approval here"
    );
    let mut attempts = store.list_workflow_attempts(id).await?;
    let attempt = attempts.pop().context("workflow has no stage attempt")?;
    ensure!(
        attempt.stage == WorkflowStage::Approval
            && attempt.state == StageAttemptState::AwaitingApproval,
        "workflow is not paused at the approval stage"
    );
    if store
        .get_iteration_workflow_approval(id, run.iteration)
        .await?
        .is_none()
    {
        let proposal_id = artifact_id(&attempts, "optimization_proposal")?;
        let proposal = store
            .get_optimization_proposal(proposal_id)
            .await?
            .context("workflow optimization proposal not found")?;
        let state = if recommendation_ids.is_empty() {
            ProposalReviewState::ApprovedForPlanCreation
        } else {
            ProposalReviewState::PartiallyAccepted
        };
        let review = ProposalReviewRecord::new(
            &proposal,
            state,
            recommendation_ids,
            note.clone(),
            None,
            None,
        )?;
        let selected = review.selected_data_recommendation_ids(&proposal)?;
        let approved_rows = proposal
            .normalized_recommendations
            .iter()
            .filter(|recommendation| selected.contains(&recommendation.id))
            .map(|recommendation| u64::from(recommendation.additional_count))
            .sum();
        ensure!(approved_rows > 0, "approval selected no generated rows");
        ensure!(
            run.usage.accepted_rows.saturating_add(approved_rows)
                <= definition.budget.maximum_cumulative_rows,
            "approval exceeds the workflow row budget"
        );
        store.append_proposal_review(&review).await?;
        let decision = WorkflowApprovalDecision::new(
            run.id,
            run.iteration,
            proposal.id,
            proposal.fingerprint,
            review.id,
            review.fingerprint.clone(),
            WorkflowApprovalMode::HumanReview,
            approved_rows,
            run.usage.clone(),
            None,
            note,
        )?;
        store.create_workflow_approval(&decision).await?;
    }
    let run = drive_initial_pipeline(store, definition, run, attempt).await?;
    print_status(store, run).await
}

fn validate_preauthorization(
    definition: &WorkflowDefinition,
    configured: &project_config::ResolvedProjectConfig,
    envelope: &workflow_core::workflow::ApprovalEnvelope,
    usage: &WorkflowBudgetUsage,
    approved_rows: u64,
) -> anyhow::Result<()> {
    let generation_backend = match configured.generation.backend {
        project_config::GenerationBackendKind::Fake => "fake",
        project_config::GenerationBackendKind::OpenaiCompatible => "openai-compatible",
    };
    let training_backend = match configured.training.backend {
        project_config::TrainingBackendKind::HashingLinear => "hashing-linear",
        project_config::TrainingBackendKind::BertCpu => "bert-cpu",
    };
    ensure!(
        generation_backend == envelope.permitted_generation_backend
            && configured.generation.model == envelope.permitted_generation_model
            && training_backend == envelope.permitted_training_backend,
        "configured provider or training backend is outside the preauthorization envelope"
    );
    let initial_rows =
        definition.initial_allocation.total_rows - definition.initial_allocation.reserved_rows;
    let estimated_generation_requests =
        approved_rows.div_ceil(u64::from(configured.generation.batch_size.max(1)));
    ensure!(
        usage
            .accepted_rows
            .saturating_sub(initial_rows)
            .saturating_add(approved_rows)
            <= envelope.maximum_additional_rows
            && usage.iterations < envelope.maximum_iterations
            && usage
                .generation_requests
                .saturating_add(estimated_generation_requests)
                <= envelope.maximum_generation_requests
            && usage.advisor_calls <= envelope.maximum_advisor_calls
            && envelope
                .maximum_advisor_tokens
                .is_none_or(|maximum| usage.advisor_tokens <= maximum),
        "workflow action exceeds the preauthorization envelope"
    );
    let fingerprint = artifact_core::fingerprint(&configured.training)?;
    ensure!(
        envelope
            .permitted_training_configuration_fingerprints
            .is_empty()
            || envelope
                .permitted_training_configuration_fingerprints
                .contains(&fingerprint),
        "training configuration is outside the preauthorization envelope"
    );
    Ok(())
}

async fn execute_stop_decision(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    history: &[WorkflowStageAttempt],
    usage: &WorkflowBudgetUsage,
) -> anyhow::Result<workflow_core::stop::StopDecision> {
    if let Some(existing) = store
        .get_iteration_stop_decision(run.id, run.iteration)
        .await?
    {
        return Ok(existing);
    }
    let suite = store
        .get_benchmark_suite(definition.development_suite_id)
        .await?
        .context("development benchmark suite not found")?;
    let evaluation_ids = artifact_ids_for_stage(
        history,
        WorkflowStage::IterationEvaluation,
        run.iteration,
        "iteration_evaluation_run",
    );
    let mut inputs = Vec::new();
    for cohort in &suite.cohorts {
        inputs.push(CohortAssessmentInput {
            cohort_id: cohort.cohort_id,
            run: Some(load_matching_evaluation(store, cohort, &evaluation_ids).await?),
            comparison: None,
        });
    }
    let candidate = assess_benchmark(&suite, inputs)?;
    let acceptance = match store
        .query_acceptance_assessments(AcceptanceAssessmentQuery {
            suite_id: Some(suite.id),
            checkpoint_id: candidate.checkpoint_id,
            state: None,
            limit: 10_000,
            offset: 0,
        })
        .await?
        .into_iter()
        .find(|value| value.evaluation_run_ids == candidate.evaluation_run_ids)
    {
        Some(value) => value,
        None => {
            store.create_acceptance_assessment(&candidate).await?;
            candidate
        }
    };
    record_suite_exposures(
        store,
        &suite,
        acceptance.evaluation_run_ids.values().copied().collect(),
        run,
        ExposurePurpose::Comparison,
        None,
        "workflow iteration stop assessment",
    )
    .await?;
    let comparison_ids = artifact_ids_for_stage(
        history,
        WorkflowStage::Comparison,
        run.iteration,
        "evaluation_comparison",
    );
    let mut comparisons = Vec::new();
    for id in comparison_ids {
        comparisons.push(
            store
                .get_comparison(id)
                .await?
                .with_context(|| format!("workflow comparison not found: {id}"))?,
        );
    }
    let decision = decide(
        run.id,
        run.iteration,
        &acceptance,
        &comparisons,
        usage,
        &definition.budget,
        &definition.policy,
    )?;
    store.create_stop_decision(&decision).await?;
    Ok(decision)
}

async fn finalize(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, id).await?;
    let definition = require_definition(store, run.definition_id).await?;
    ensure!(
        definition.sealed_suite_id.is_some(),
        "workflow definition has no sealed acceptance suite"
    );
    let mut attempts = store.list_workflow_attempts(id).await?;
    let previous = attempts.pop().context("workflow has no stage attempt")?;
    if previous.stage == WorkflowStage::SealedEvaluation
        && previous.state == StageAttemptState::Completed
    {
        return print_status(store, run).await;
    }
    ensure!(
        run.state == WorkflowRunState::DevelopmentComplete,
        "workflow development must be complete before final sealed evaluation"
    );
    let started = WorkflowStageAttempt::start(
        &definition,
        &mut run,
        WorkflowStage::SealedEvaluation,
        Some(&previous),
        1,
    )?;
    store
        .commit_workflow_attempt(&run, &started, previous.id)
        .await?;
    let run = drive_initial_pipeline(store, definition, run, started).await?;
    print_status(store, run).await
}

async fn promote(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, id).await?;
    if store.get_workflow_promotion(id).await?.is_some() {
        return print_status(store, run).await;
    }
    let definition = require_definition(store, run.definition_id).await?;
    let mut attempts = store.list_workflow_attempts(id).await?;
    let previous = attempts.pop().context("workflow has no stage attempt")?;
    ensure!(
        previous.stage == WorkflowStage::SealedEvaluation
            && previous.state == StageAttemptState::Completed,
        "explicit sealed evaluation must complete before promotion"
    );
    let started = WorkflowStageAttempt::start(
        &definition,
        &mut run,
        WorkflowStage::Promotion,
        Some(&previous),
        1,
    )?;
    store
        .commit_workflow_attempt(&run, &started, previous.id)
        .await?;
    let run = drive_initial_pipeline(store, definition, run, started).await?;
    print_status(store, run).await
}

async fn create_promotion(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    history: &[WorkflowStageAttempt],
) -> anyhow::Result<ModelPromotion> {
    let checkpoint = artifact_link(history, "iteration_checkpoint")
        .or_else(|_| artifact_link(history, "checkpoint"))?;
    let snapshot = artifact_link(history, "iteration_snapshot")
        .or_else(|_| artifact_link(history, "snapshot"))?;
    let development = artifact_link(history, "development_acceptance_assessment")
        .or_else(|_| artifact_link(history, "acceptance_assessment"))?;
    let sealed = artifact_link(history, "sealed_acceptance_assessment")?;
    let sealed_assessment = store
        .get_acceptance_assessment(sealed.artifact_id)
        .await?
        .context("sealed acceptance assessment not found")?;
    let sealed_suite_id = definition
        .sealed_suite_id
        .context("workflow has no sealed suite")?;
    let promotion = ModelPromotion::new(
        run.id,
        checkpoint.artifact_id,
        checkpoint.artifact_fingerprint,
        snapshot.artifact_id,
        snapshot.artifact_fingerprint,
        development.artifact_id,
        development.artifact_fingerprint,
        &sealed_assessment,
        definition.development_suite_id,
        definition.development_suite_fingerprint.clone(),
        sealed_suite_id,
        definition
            .sealed_suite_fingerprint
            .clone()
            .context("workflow has no sealed suite fingerprint")?,
        artifact_core::fingerprint(&definition.policy)?,
    )?;
    store.create_promotion(&promotion).await?;
    Ok(promotion)
}

async fn execute_initial_allocation(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
) -> anyhow::Result<InitialAllocationRecord> {
    let dataset = store
        .get_dataset(definition.dataset_id)
        .await?
        .context("workflow dataset not found")?;
    let counts = store.dataset_cell_counts(dataset.id).await?;
    let cells = expand_generation_cells(&dataset)
        .into_iter()
        .map(|cell| (cell.key(), cell))
        .collect::<BTreeMap<_, _>>();
    let current_coverage = counts
        .into_iter()
        .map(|(key, counts)| {
            Ok(InitialCellCoverage {
                cell: cells
                    .get(&key)
                    .cloned()
                    .with_context(|| format!("persisted coverage contains unknown cell: {key}"))?,
                accepted: counts.accepted,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let result = allocate_initial_budget(
        &dataset,
        InitialAllocationRequest {
            total_rows: u32::try_from(definition.initial_allocation.total_rows)
                .context("initial row budget exceeds Slice 1 limits")?,
            reserved_rows: u32::try_from(definition.initial_allocation.reserved_rows)
                .context("reserved row budget exceeds Slice 1 limits")?,
            policy: definition.initial_allocation.policy.clone(),
            current_coverage,
            constraints: definition.initial_allocation.constraints.clone(),
        },
    )?;
    ensure!(
        result.feasibility == InitialAllocationFeasibility::Feasible,
        "initial allocation is infeasible: {}",
        serde_json::to_string(&result.issues)?
    );
    if let Some(existing) = store
        .query_initial_allocations(InitialAllocationQuery {
            dataset_id: Some(dataset.id),
            limit: 10_000,
            offset: 0,
        })
        .await?
        .into_iter()
        .find(|value| value.result.fingerprint == result.fingerprint)
    {
        return Ok(existing);
    }
    let plan = result.to_generation_plan(&dataset)?;
    let record = InitialAllocationRecord::new(result, plan.id)?;
    store.create_initial_allocation(&record, &plan).await?;
    Ok(record)
}

async fn load_matching_evaluation(
    store: &SqliteStore,
    cohort: &workflow_core::benchmark::BenchmarkCohort,
    evaluation_ids: &[uuid::Uuid],
) -> anyhow::Result<evaluation_core::domain::EvaluationRun> {
    for id in evaluation_ids {
        let Some(run) = store.get_evaluation_run(*id).await? else {
            continue;
        };
        if run.snapshot_id == cohort.snapshot_id
            && run.split == cohort.split
            && run.protocol_fingerprint == cohort.protocol_fingerprint
            && run.state == evaluation_core::domain::EvaluationRunState::Completed
        {
            return Ok(run);
        }
    }
    anyhow::bail!(
        "no completed compatible evaluation found for cohort {}",
        cohort.cohort_id
    )
}

#[allow(clippy::too_many_arguments)]
async fn record_suite_exposures(
    store: &SqliteStore,
    suite: &workflow_core::benchmark::BenchmarkSuite,
    evaluation_ids: Vec<uuid::Uuid>,
    run: &WorkflowRun,
    purpose: ExposurePurpose,
    disclosure: Option<DisclosureLevel>,
    note: &str,
) -> anyhow::Result<()> {
    for cohort in &suite.cohorts {
        let evaluation = load_matching_evaluation(store, cohort, &evaluation_ids).await?;
        let existing = store
            .query_exposures(ExposureQuery {
                cohort_id: cohort.cohort_id,
                purpose: Some(purpose),
                limit: 10_000,
                offset: 0,
            })
            .await?
            .into_iter()
            .any(|value| {
                value.evaluation_run_id == Some(evaluation.id)
                    && value.workflow_run_id == Some(run.id)
                    && value.workflow_iteration == Some(run.iteration)
            });
        if existing {
            continue;
        }
        let persisted_cohort = store
            .get_cohort(cohort.cohort_id)
            .await?
            .context("benchmark cohort not found")?;
        let role = store
            .get_current_cohort_role(cohort.cohort_id)
            .await?
            .context("benchmark cohort has no current role")?;
        let exposure = EvidenceExposure::new(
            &persisted_cohort,
            &role,
            EvidenceExposureRequest {
                evaluation_run_id: Some(evaluation.id),
                workflow_run_id: Some(run.id),
                workflow_iteration: Some(run.iteration),
                purpose,
                disclosure: disclosure.unwrap_or(cohort.disclosure),
                adaptation_eligible: true,
                note: Some(note.to_owned()),
            },
        )?;
        store.append_exposure(&exposure, None).await?;
    }
    Ok(())
}

const fn initial_successor(
    stage: WorkflowStage,
    advisor: bool,
    quality_gate: bool,
) -> Option<WorkflowStage> {
    match stage {
        WorkflowStage::InitialAllocation => Some(WorkflowStage::Generation),
        WorkflowStage::Generation if quality_gate => Some(WorkflowStage::QualityAudit),
        WorkflowStage::Generation => Some(WorkflowStage::Snapshot),
        WorkflowStage::QualityAudit => Some(WorkflowStage::CurationReview),
        WorkflowStage::CurationApproval => Some(WorkflowStage::Snapshot),
        WorkflowStage::Snapshot => Some(WorkflowStage::Training),
        WorkflowStage::Training => Some(WorkflowStage::DevelopmentEvaluation),
        WorkflowStage::DevelopmentEvaluation => Some(WorkflowStage::AcceptanceAssessment),
        WorkflowStage::AcceptanceAssessment => Some(WorkflowStage::ErrorAnalysis),
        WorkflowStage::ErrorAnalysis if advisor => Some(WorkflowStage::Advisor),
        WorkflowStage::ErrorAnalysis => Some(WorkflowStage::OptimizationProposal),
        WorkflowStage::Advisor => Some(WorkflowStage::OptimizationProposal),
        WorkflowStage::OptimizationProposal => Some(WorkflowStage::Approval),
        WorkflowStage::Approval => Some(WorkflowStage::ProposalApplication),
        WorkflowStage::ProposalApplication => Some(WorkflowStage::DatasetDiffGeneration),
        WorkflowStage::DatasetDiffGeneration if quality_gate => {
            Some(WorkflowStage::IterationQualityAudit)
        }
        WorkflowStage::DatasetDiffGeneration => Some(WorkflowStage::IterationSnapshot),
        WorkflowStage::IterationQualityAudit => Some(WorkflowStage::IterationCurationReview),
        WorkflowStage::IterationCurationApproval => Some(WorkflowStage::IterationSnapshot),
        WorkflowStage::IterationSnapshot => Some(WorkflowStage::IterationTraining),
        WorkflowStage::IterationTraining => Some(WorkflowStage::IterationEvaluation),
        WorkflowStage::IterationEvaluation => Some(WorkflowStage::Comparison),
        WorkflowStage::Comparison => Some(WorkflowStage::FollowupAnalysis),
        WorkflowStage::FollowupAnalysis => Some(WorkflowStage::StopDecision),
        WorkflowStage::StopDecision => Some(WorkflowStage::OptimizationProposal),
        _ => None,
    }
}

const fn run_state(value: WorkflowRunStateArg) -> WorkflowRunState {
    match value {
        WorkflowRunStateArg::Queued => WorkflowRunState::Queued,
        WorkflowRunStateArg::Running => WorkflowRunState::Running,
        WorkflowRunStateArg::AwaitingApproval => WorkflowRunState::AwaitingApproval,
        WorkflowRunStateArg::AwaitingUser => WorkflowRunState::AwaitingUser,
        WorkflowRunStateArg::DevelopmentComplete => WorkflowRunState::DevelopmentComplete,
        WorkflowRunStateArg::Completed => WorkflowRunState::Completed,
        WorkflowRunStateArg::Failed => WorkflowRunState::Failed,
        WorkflowRunStateArg::Cancelled => WorkflowRunState::Cancelled,
        WorkflowRunStateArg::Exhausted => WorkflowRunState::Exhausted,
        WorkflowRunStateArg::Inconclusive => WorkflowRunState::Inconclusive,
    }
}
