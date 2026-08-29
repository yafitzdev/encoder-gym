use std::{collections::BTreeMap, future::Future, path::Path, pin::Pin, time::Duration};

use advisor_fake::FakeAnalysisAdvisor;
use advisor_openai_compatible::OpenAICompatibleAdvisor;
use analysis_core::{ports::AnalysisStore, runner::run_analysis};
use anyhow::{Context, ensure};
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
use serde::de::DeserializeOwned;
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
        if run.cancel_requested && attempt.state == StageAttemptState::Running {
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
            let Some(next) = initial_successor(attempt.stage, definition.policy.enable_advisor)
            else {
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

fn execute_initial_stage<'a>(
    store: &'a SqliteStore,
    definition: &'a WorkflowDefinition,
    run: &'a WorkflowRun,
    attempt: &'a WorkflowStageAttempt,
    configured: &'a project_config::ResolvedProjectConfig,
) -> Pin<Box<dyn Future<Output = anyhow::Result<StageExecution>> + Send + 'a>> {
    Box::pin(async move {
        let history = store.list_workflow_attempts(run.id).await?;
        let mut usage = run.usage.clone();
        if attempt.stage == WorkflowStage::Approval {
            return execute_approval_stage(store, definition, run, configured, usage).await;
        }
        let artifacts = match attempt.stage {
            WorkflowStage::InitialAllocation => {
                let record = execute_initial_allocation(store, definition).await?;
                vec![
                    link("initial_allocation", record.id, &record.fingerprint),
                    link(
                        "generation_plan",
                        record.generation_plan_id,
                        &record.result.fingerprint,
                    ),
                ]
            }
            WorkflowStage::Generation => {
                let plan_id = artifact_id(&history, "generation_plan")?;
                let allocation_id = artifact_id(&history, "initial_allocation")?;
                let allocation = store
                    .get_initial_allocation(allocation_id)
                    .await?
                    .context("workflow initial allocation not found")?;
                let existing = store
                    .list_jobs(JobQuery {
                        plan_id: Some(plan_id),
                        state: Some(JobState::Completed),
                        limit: 1,
                        ..JobQuery::default()
                    })
                    .await?
                    .into_iter()
                    .next();
                let job = match existing {
                    Some(job) => job,
                    None => {
                        super::generation::run_workflow(plan_id, configured, store.clone()).await?
                    }
                };
                ensure!(
                    job.state == JobState::Completed,
                    "generation job did not complete"
                );
                usage.accepted_rows = u64::from(allocation.result.initial_target_rows);
                usage.generation_attempts =
                    usage.generation_attempts.saturating_add(job.generated_rows);
                let batch = u64::from(configured.generation.batch_size.max(1));
                usage.generation_requests = usage
                    .generation_requests
                    .saturating_add(job.generated_rows.div_ceil(batch))
                    .saturating_add(job.failed_requests);
                vec![link(
                    "generation_job",
                    job.id,
                    &artifact_core::fingerprint(&job)?,
                )]
            }
            WorkflowStage::Snapshot => {
                let expected_name = format!(
                    "{}-workflow-{}-{}",
                    configured.snapshot.name, run.id, run.iteration
                );
                let existing = dataset_core::ports::SnapshotStore::query_snapshots(
                    store,
                    dataset_core::ports::SnapshotQuery {
                        dataset_id: Some(definition.dataset_id),
                        limit: 10_000,
                        offset: 0,
                    },
                )
                .await?
                .into_iter()
                .find(|snapshot| snapshot.name == expected_name);
                let snapshot = match existing {
                    Some(snapshot) => snapshot,
                    None => {
                        super::snapshot::create_workflow(
                            definition.dataset_id,
                            run.id,
                            run.iteration,
                            configured,
                            store,
                        )
                        .await?
                    }
                };
                vec![link("snapshot", snapshot.id, &snapshot.fingerprint)]
            }
            WorkflowStage::Training => {
                let snapshot_id = artifact_id(&history, "snapshot")?;
                let existing = training_core::ports::TrainingStore::query_training_runs(
                    store,
                    training_core::ports::TrainingRunQuery {
                        snapshot_id: Some(snapshot_id),
                        state: Some(training_core::domain::TrainingRunState::Completed),
                        limit: 1,
                        offset: 0,
                    },
                )
                .await?
                .into_iter()
                .next();
                let completed = match existing {
                    Some(run) => super::training::CompletedTraining {
                        checkpoints: training_core::ports::TrainingStore::list_checkpoints(
                            store, run.id,
                        )
                        .await?,
                        run,
                    },
                    None => {
                        super::training::run_workflow(snapshot_id, configured, store.clone())
                            .await?
                    }
                };
                let checkpoint = completed
                    .checkpoints
                    .iter()
                    .find(|value| value.is_final)
                    .context("training completed without a final checkpoint")?;
                vec![
                    link(
                        "training_run",
                        completed.run.id,
                        &artifact_core::fingerprint(&completed.run)?,
                    ),
                    link("checkpoint", checkpoint.id, &checkpoint.artifact_checksum),
                ]
            }
            WorkflowStage::DevelopmentEvaluation => {
                let checkpoint_id = artifact_id(&history, "checkpoint")?;
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                ensure!(
                    suite.kind == BenchmarkSuiteKind::Development,
                    "suite is not development"
                );
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let existing = store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        });
                    let evaluation = match existing {
                        Some(evaluation) => evaluation,
                        None => {
                            super::evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    links.push(link(
                        "evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                links
            }
            WorkflowStage::AcceptanceAssessment => {
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                let evaluation_ids = artifact_ids(&history, "evaluation_run");
                let mut inputs = Vec::new();
                for cohort in &suite.cohorts {
                    inputs.push(CohortAssessmentInput {
                        cohort_id: cohort.cohort_id,
                        run: Some(load_matching_evaluation(store, cohort, &evaluation_ids).await?),
                        comparison: None,
                    });
                }
                let assessment = assess_benchmark(&suite, inputs)?;
                let existing = store
                    .query_acceptance_assessments(AcceptanceAssessmentQuery {
                        suite_id: Some(suite.id),
                        checkpoint_id: assessment.checkpoint_id,
                        state: None,
                        limit: 10_000,
                        offset: 0,
                    })
                    .await?
                    .into_iter()
                    .find(|value| value.evaluation_run_ids == assessment.evaluation_run_ids);
                let assessment = match existing {
                    Some(value) => value,
                    None => {
                        store.create_acceptance_assessment(&assessment).await?;
                        assessment
                    }
                };
                record_suite_exposures(
                    store,
                    &suite,
                    assessment.evaluation_run_ids.values().copied().collect(),
                    run,
                    ExposurePurpose::DevelopmentEvaluation,
                    None,
                    "workflow development assessment",
                )
                .await?;
                let mut links = vec![link(
                    "acceptance_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                )];
                if assessment.state == workflow_core::benchmark::AcceptanceState::Pass {
                    links.push(link(
                        "development_acceptance_pass",
                        assessment.id,
                        &assessment.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::ErrorAnalysis => {
                let evaluation_ids = artifact_ids(&history, "evaluation_run");
                let protocol = definition
                    .analysis_protocol
                    .as_ref()
                    .context("legacy workflow definition has no resolved analysis protocol")?;
                let protocol_fingerprint = protocol.fingerprint()?;
                let mut links = Vec::new();
                for evaluation_id in evaluation_ids {
                    let existing = store
                        .query_analysis_reports(analysis_core::ports::AnalysisReportQuery {
                            evaluation_run_id: Some(evaluation_id),
                            limit: 10_000,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|report| report.protocol_fingerprint == protocol_fingerprint);
                    let report = match existing {
                        Some(report) => report,
                        None => run_analysis(store, store, evaluation_id, protocol.clone()).await?,
                    };
                    links.push(link("analysis_report", report.id, &report.fingerprint));
                }
                ensure!(
                    !links.is_empty(),
                    "development analysis produced no reports"
                );
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                record_suite_exposures(
                    store,
                    &suite,
                    artifact_ids(&history, "evaluation_run"),
                    run,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                    "workflow error analysis",
                )
                .await?;
                links
            }
            WorkflowStage::Advisor => {
                let configuration = definition
                    .advisor
                    .as_ref()
                    .context("workflow advisor is enabled without resolved configuration")?;
                let analysis_report_id = artifact_id(&history, "analysis_report")?;
                let assessment_id = artifact_id(&history, "acceptance_assessment")?;
                let report = store
                    .get_analysis_report(analysis_report_id)
                    .await?
                    .context("workflow analysis report not found")?;
                let acceptance = store
                    .get_acceptance_assessment(assessment_id)
                    .await?
                    .context("workflow acceptance assessment not found")?;
                let existing = store
                    .query_advisory_assessments(AdvisoryAssessmentQuery {
                        workflow_run_id: Some(run.id),
                        analysis_report_id: Some(report.id),
                        limit: 10_000,
                        offset: 0,
                    })
                    .await?
                    .into_iter()
                    .find(|value| {
                        value.request.workflow_iteration == run.iteration
                            && value.backend == configuration.backend
                            && value.model == configuration.model
                    });
                let assessment = match existing {
                    Some(value) => value,
                    None => {
                        let dataset = store
                            .get_dataset(definition.dataset_id)
                            .await?
                            .context("workflow dataset not found")?;
                        let finding_limit = usize::from(configuration.maximum_findings);
                        let findings = report
                            .findings
                            .iter()
                            .take(finding_limit)
                            .map(|finding| AdvisoryFinding {
                                key: finding.key.clone(),
                                fingerprint: finding.fingerprint.clone(),
                                kind: serde_json::to_value(finding.kind)
                                    .ok()
                                    .and_then(|value| value.as_str().map(str::to_owned))
                                    .unwrap_or_else(|| "unknown".into()),
                                support: finding.support,
                                error_count: finding.error_count,
                                error_rate: finding.error_rate,
                            })
                            .collect();
                        let representative_errors = if configuration.egress_policy
                            == AdvisorEgressPolicy::DevelopmentText
                        {
                            report
                                .errors
                                .iter()
                                .take(usize::from(configuration.maximum_representative_errors))
                                .map(|error| AdvisoryExample {
                                    finding_key: report
                                        .findings
                                        .first()
                                        .map_or_else(String::new, |finding| finding.key.clone()),
                                    text: error.text.clone(),
                                    expected_label: error.expected_label.clone(),
                                    predicted_label: error.predicted_label.clone(),
                                    dimensions: error.dimensions.clone(),
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let request = AdvisoryRequest {
                            id: uuid::Uuid::new_v4(),
                            workflow_run_id: run.id,
                            workflow_iteration: run.iteration,
                            task: dataset.task_description.clone(),
                            labels: dataset.labels.clone(),
                            dimensions: dataset
                                .dimensions
                                .iter()
                                .map(|dimension| (dimension.name.clone(), dimension.values.clone()))
                                .collect(),
                            analysis_report_id: report.id,
                            analysis_report_fingerprint: report.fingerprint.clone(),
                            acceptance_assessment_id: acceptance.id,
                            acceptance_assessment_fingerprint: acceptance.fingerprint.clone(),
                            acceptance_state: serde_json::to_value(acceptance.state)?
                                .as_str()
                                .unwrap_or("invalid")
                                .to_owned(),
                            prediction_count: report.prediction_count,
                            error_count: report.error_count,
                            findings,
                            representative_errors,
                            allowed_cells: expand_generation_cells(&dataset),
                            allowed_actions: vec![
                                AdvisoryActionKind::Stop,
                                AdvisoryActionKind::Inspect,
                                AdvisoryActionKind::ConsiderExperiment,
                            ],
                            remaining_row_budget: definition
                                .budget
                                .maximum_cumulative_rows
                                .saturating_sub(usage.accepted_rows),
                            remaining_iteration_budget: definition
                                .budget
                                .maximum_iterations
                                .saturating_sub(usage.iterations),
                            egress_policy: configuration.egress_policy,
                        };
                        let backend: Box<dyn AnalysisAdvisor> = match configuration.backend.as_str()
                        {
                            "fake" => Box::<FakeAnalysisAdvisor>::default(),
                            "openai-compatible" => {
                                let base_url = configuration
                                    .base_url
                                    .as_deref()
                                    .context("OpenAI-compatible advisor requires base_url")?;
                                let api_key = std::env::var(&configuration.api_key_env).ok();
                                Box::new(OpenAICompatibleAdvisor::new(base_url, api_key)?)
                            }
                            value => anyhow::bail!("unsupported advisor backend: {value}"),
                        };
                        let value = run_advisor(backend.as_ref(), configuration, request).await?;
                        store.create_advisory_assessment(&value).await?;
                        value
                    }
                };
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                record_suite_exposures(
                    store,
                    &suite,
                    artifact_ids(&history, "evaluation_run"),
                    run,
                    ExposurePurpose::Advisor,
                    Some(match configuration.egress_policy {
                        AdvisorEgressPolicy::AggregateOnly => DisclosureLevel::Aggregate,
                        AdvisorEgressPolicy::DevelopmentText => DisclosureLevel::RowContent,
                    }),
                    "workflow advisory assessment",
                )
                .await?;
                usage.advisor_calls = usage.advisor_calls.saturating_add(1);
                usage.advisor_tokens = usage.advisor_tokens.saturating_add(
                    assessment.usage.total_tokens.unwrap_or_else(|| {
                        assessment.usage.input_tokens.unwrap_or_default()
                            + assessment.usage.output_tokens.unwrap_or_default()
                    }),
                );
                vec![link(
                    "advisory_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                )]
            }
            WorkflowStage::OptimizationProposal => {
                let analysis_report_id = artifact_id(&history, "followup_analysis_report")
                    .or_else(|_| artifact_id(&history, "analysis_report"))?;
                let report = store
                    .get_analysis_report(analysis_report_id)
                    .await?
                    .context("workflow analysis report not found")?;
                let proposal = super::optimization::propose_workflow(
                    store,
                    analysis_report_id,
                    definition.optimization_protocol.as_ref().context(
                        "legacy workflow definition has no resolved optimization protocol",
                    )?,
                )
                .await?;
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                record_suite_exposures(
                    store,
                    &suite,
                    vec![report.evaluation_run_id],
                    run,
                    ExposurePurpose::Optimization,
                    Some(DisclosureLevel::Slices),
                    "workflow optimization proposal",
                )
                .await?;
                vec![link(
                    "optimization_proposal",
                    proposal.id,
                    &proposal.fingerprint,
                )]
            }
            WorkflowStage::ProposalApplication => {
                let decision = store
                    .get_iteration_workflow_approval(run.id, run.iteration)
                    .await?
                    .context("workflow proposal has no approval decision")?;
                let (plan, application) = super::optimization::apply_workflow(
                    store,
                    decision.proposal_id,
                    decision.proposal_review_id,
                )
                .await?;
                vec![
                    link("workflow_approval", decision.id, &decision.fingerprint),
                    link(
                        "proposal_review",
                        decision.proposal_review_id,
                        &decision.proposal_review_fingerprint,
                    ),
                    link(
                        "proposal_application",
                        application.proposal_id,
                        &artifact_core::fingerprint(&application)?,
                    ),
                    link(
                        "iteration_generation_plan",
                        plan.id,
                        &artifact_core::fingerprint(&plan)?,
                    ),
                ]
            }
            WorkflowStage::DatasetDiffGeneration => {
                let plan_id = artifact_id(&history, "iteration_generation_plan")?;
                let existing = store
                    .list_jobs(JobQuery {
                        plan_id: Some(plan_id),
                        state: Some(JobState::Completed),
                        limit: 1,
                        ..JobQuery::default()
                    })
                    .await?
                    .into_iter()
                    .next();
                let job = match existing {
                    Some(job) => job,
                    None => {
                        super::generation::run_workflow(plan_id, configured, store.clone()).await?
                    }
                };
                ensure!(
                    job.state == JobState::Completed,
                    "dataset diff did not complete"
                );
                usage.accepted_rows = usage.accepted_rows.saturating_add(job.accepted_rows);
                usage.generation_attempts =
                    usage.generation_attempts.saturating_add(job.generated_rows);
                let batch = u64::from(configured.generation.batch_size.max(1));
                usage.generation_requests = usage
                    .generation_requests
                    .saturating_add(job.generated_rows.div_ceil(batch))
                    .saturating_add(job.failed_requests);
                usage.iterations = usage.iterations.saturating_add(1);
                vec![link(
                    "dataset_diff_generation_job",
                    job.id,
                    &artifact_core::fingerprint(&job)?,
                )]
            }
            WorkflowStage::IterationSnapshot => {
                let expected_name = format!(
                    "{}-workflow-{}-{}",
                    configured.snapshot.name, run.id, run.iteration
                );
                let existing = dataset_core::ports::SnapshotStore::query_snapshots(
                    store,
                    dataset_core::ports::SnapshotQuery {
                        dataset_id: Some(definition.dataset_id),
                        limit: 10_000,
                        offset: 0,
                    },
                )
                .await?
                .into_iter()
                .find(|snapshot| snapshot.name == expected_name);
                let snapshot = match existing {
                    Some(snapshot) => snapshot,
                    None => {
                        super::snapshot::create_workflow(
                            definition.dataset_id,
                            run.id,
                            run.iteration,
                            configured,
                            store,
                        )
                        .await?
                    }
                };
                vec![link(
                    "iteration_snapshot",
                    snapshot.id,
                    &snapshot.fingerprint,
                )]
            }
            WorkflowStage::IterationTraining => {
                ensure!(
                    definition
                        .training_iteration_policy
                        .unwrap_or(TrainingIterationPolicy::Fresh)
                        == TrainingIterationPolicy::Fresh,
                    "unsupported workflow iteration training policy"
                );
                let snapshot_id = artifact_id(&history, "iteration_snapshot")?;
                let existing = training_core::ports::TrainingStore::query_training_runs(
                    store,
                    training_core::ports::TrainingRunQuery {
                        snapshot_id: Some(snapshot_id),
                        state: Some(training_core::domain::TrainingRunState::Completed),
                        limit: 1,
                        offset: 0,
                    },
                )
                .await?
                .into_iter()
                .next();
                let completed = match existing {
                    Some(training_run) => super::training::CompletedTraining {
                        checkpoints: training_core::ports::TrainingStore::list_checkpoints(
                            store,
                            training_run.id,
                        )
                        .await?,
                        run: training_run,
                    },
                    None => {
                        super::training::run_workflow(snapshot_id, configured, store.clone())
                            .await?
                    }
                };
                let checkpoint = completed
                    .checkpoints
                    .iter()
                    .find(|value| value.is_final)
                    .context("iteration training has no final checkpoint")?;
                vec![
                    link(
                        "iteration_training_run",
                        completed.run.id,
                        &artifact_core::fingerprint(&completed.run)?,
                    ),
                    link(
                        "iteration_checkpoint",
                        checkpoint.id,
                        &checkpoint.artifact_checksum,
                    ),
                ]
            }
            WorkflowStage::IterationEvaluation => {
                let checkpoint_id = artifact_id(&history, "iteration_checkpoint")?;
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let evaluation = match store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        }) {
                        Some(value) => value,
                        None => {
                            super::evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    links.push(link(
                        "iteration_evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                links
            }
            WorkflowStage::Comparison => {
                let baseline_ids = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::DevelopmentEvaluation,
                    0,
                    "evaluation_run",
                );
                let iteration_ids = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::IterationEvaluation,
                    run.iteration,
                    "iteration_evaluation_run",
                );
                ensure!(
                    baseline_ids.len() == iteration_ids.len() && !baseline_ids.is_empty(),
                    "iteration evaluations are incompatible with the baseline"
                );
                let mut links = Vec::new();
                for (left, right) in baseline_ids.into_iter().zip(iteration_ids) {
                    let comparison =
                        super::evaluation::compare_workflow(left, right, store).await?;
                    links.push(link(
                        "evaluation_comparison",
                        comparison.id,
                        &comparison.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::FollowupAnalysis => {
                let evaluations = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::IterationEvaluation,
                    run.iteration,
                    "iteration_evaluation_run",
                );
                let comparisons = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::Comparison,
                    run.iteration,
                    "evaluation_comparison",
                );
                ensure!(
                    evaluations.len() == comparisons.len() && !evaluations.is_empty(),
                    "follow-up analysis inputs are incomplete"
                );
                let exposure_evaluations = evaluations.clone();
                let base_protocol = definition
                    .analysis_protocol
                    .as_ref()
                    .context("workflow has no resolved analysis protocol")?;
                let mut links = Vec::new();
                for (evaluation_id, comparison_id) in evaluations.into_iter().zip(comparisons) {
                    let mut protocol = base_protocol.clone();
                    protocol.comparison_id = Some(comparison_id);
                    let fingerprint = protocol.fingerprint()?;
                    let existing = store
                        .query_analysis_reports(analysis_core::ports::AnalysisReportQuery {
                            evaluation_run_id: Some(evaluation_id),
                            limit: 10_000,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|report| report.protocol_fingerprint == fingerprint);
                    let report = match existing {
                        Some(value) => value,
                        None => run_analysis(store, store, evaluation_id, protocol).await?,
                    };
                    links.push(link(
                        "followup_analysis_report",
                        report.id,
                        &report.fingerprint,
                    ));
                }
                let suite = store
                    .get_benchmark_suite(definition.development_suite_id)
                    .await?
                    .context("development benchmark suite not found")?;
                record_suite_exposures(
                    store,
                    &suite,
                    exposure_evaluations,
                    run,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                    "workflow follow-up analysis",
                )
                .await?;
                links
            }
            WorkflowStage::StopDecision => {
                let decision =
                    execute_stop_decision(store, definition, run, &history, &usage).await?;
                let mut links = vec![
                    link("stop_decision", decision.id, &decision.fingerprint),
                    link(
                        "development_acceptance_assessment",
                        decision.acceptance_assessment_id,
                        &decision.acceptance_assessment_fingerprint,
                    ),
                ];
                if decision.should_continue {
                    links.push(link(
                        "workflow_continue",
                        decision.id,
                        &decision.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::SealedEvaluation => {
                let suite_id = definition
                    .sealed_suite_id
                    .context("workflow has no sealed benchmark suite")?;
                let suite = store
                    .get_benchmark_suite(suite_id)
                    .await?
                    .context("sealed benchmark suite not found")?;
                ensure!(
                    suite.kind == BenchmarkSuiteKind::SealedAcceptance
                        && definition.sealed_suite_fingerprint.as_deref()
                            == Some(&suite.fingerprint),
                    "sealed benchmark suite identity changed"
                );
                let checkpoint_id = artifact_id(&history, "iteration_checkpoint")
                    .or_else(|_| artifact_id(&history, "checkpoint"))?;
                let mut evaluation_ids = Vec::new();
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let evaluation = match store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        }) {
                        Some(value) => value,
                        None => {
                            super::evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    evaluation_ids.push(evaluation.id);
                    links.push(link(
                        "sealed_evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                let mut inputs = Vec::new();
                for cohort in &suite.cohorts {
                    inputs.push(CohortAssessmentInput {
                        cohort_id: cohort.cohort_id,
                        run: Some(load_matching_evaluation(store, cohort, &evaluation_ids).await?),
                        comparison: None,
                    });
                }
                let candidate = assess_benchmark(&suite, inputs)?;
                let assessment = match store
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
                        for cohort in &suite.cohorts {
                            let current_role = store
                                .get_current_cohort_role(cohort.cohort_id)
                                .await?
                                .context("sealed cohort has no current role")?;
                            let exposure = EvidenceExposure::new(
                                &store
                                    .get_cohort(cohort.cohort_id)
                                    .await?
                                    .context("sealed cohort not found")?,
                                &current_role,
                                EvidenceExposureRequest {
                                    evaluation_run_id: Some(
                                        candidate.evaluation_run_ids[&cohort.cohort_id],
                                    ),
                                    workflow_run_id: Some(run.id),
                                    workflow_iteration: Some(run.iteration),
                                    purpose: ExposurePurpose::Acceptance,
                                    disclosure: cohort.disclosure,
                                    adaptation_eligible: false,
                                    note: Some("explicit final sealed assessment".into()),
                                },
                            )?;
                            store.append_exposure(&exposure, None).await?;
                        }
                        store.create_acceptance_assessment(&candidate).await?;
                        candidate
                    }
                };
                links.push(link(
                    "sealed_acceptance_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                ));
                links
            }
            WorkflowStage::Promotion => {
                let existing = store.get_workflow_promotion(run.id).await?;
                let promotion = match existing {
                    Some(value) => value,
                    None => create_promotion(store, definition, run, &history).await?,
                };
                vec![link(
                    "model_promotion",
                    promotion.id,
                    &promotion.fingerprint,
                )]
            }
            other => anyhow::bail!("workflow stage {other:?} is not connected yet"),
        };
        usage.validate_against(&definition.budget)?;
        Ok(StageExecution::completed(artifacts, usage))
    })
}

struct StageExecution {
    artifacts: Vec<WorkflowArtifactLink>,
    usage: WorkflowBudgetUsage,
    state: StageAttemptState,
    reason: Option<String>,
}

impl StageExecution {
    fn completed(artifacts: Vec<WorkflowArtifactLink>, usage: WorkflowBudgetUsage) -> Self {
        Self {
            artifacts,
            usage,
            state: StageAttemptState::Completed,
            reason: None,
        }
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

const fn initial_successor(stage: WorkflowStage, advisor: bool) -> Option<WorkflowStage> {
    match stage {
        WorkflowStage::InitialAllocation => Some(WorkflowStage::Generation),
        WorkflowStage::Generation => Some(WorkflowStage::Snapshot),
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
        WorkflowStage::DatasetDiffGeneration => Some(WorkflowStage::IterationSnapshot),
        WorkflowStage::IterationSnapshot => Some(WorkflowStage::IterationTraining),
        WorkflowStage::IterationTraining => Some(WorkflowStage::IterationEvaluation),
        WorkflowStage::IterationEvaluation => Some(WorkflowStage::Comparison),
        WorkflowStage::Comparison => Some(WorkflowStage::FollowupAnalysis),
        WorkflowStage::FollowupAnalysis => Some(WorkflowStage::StopDecision),
        WorkflowStage::StopDecision => Some(WorkflowStage::OptimizationProposal),
        _ => None,
    }
}

fn link(kind: &str, artifact_id: uuid::Uuid, artifact_fingerprint: &str) -> WorkflowArtifactLink {
    WorkflowArtifactLink {
        kind: kind.to_owned(),
        artifact_id,
        artifact_fingerprint: artifact_fingerprint.to_owned(),
    }
}

fn artifact_id(history: &[WorkflowStageAttempt], kind: &str) -> anyhow::Result<uuid::Uuid> {
    history
        .iter()
        .rev()
        .flat_map(|attempt| attempt.artifacts.iter().rev())
        .find(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .with_context(|| format!("workflow artifact is missing: {kind}"))
}

fn artifact_link(
    history: &[WorkflowStageAttempt],
    kind: &str,
) -> anyhow::Result<WorkflowArtifactLink> {
    history
        .iter()
        .rev()
        .flat_map(|attempt| attempt.artifacts.iter().rev())
        .find(|artifact| artifact.kind == kind)
        .cloned()
        .with_context(|| format!("workflow artifact is missing: {kind}"))
}

fn artifact_ids(history: &[WorkflowStageAttempt], kind: &str) -> Vec<uuid::Uuid> {
    history
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .collect()
}

fn artifact_ids_for_stage(
    history: &[WorkflowStageAttempt],
    stage: WorkflowStage,
    iteration: u32,
    kind: &str,
) -> Vec<uuid::Uuid> {
    history
        .iter()
        .filter(|attempt| {
            attempt.stage == stage
                && attempt.iteration == iteration
                && attempt.state == StageAttemptState::Completed
        })
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .collect()
}

async fn print_status(store: &SqliteStore, run: WorkflowRun) -> anyhow::Result<()> {
    let attempts = store.list_workflow_attempts(run.id).await?;
    let mut generation = Vec::new();
    for plan_id in attempts
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| {
            artifact.kind == "generation_plan" || artifact.kind == "iteration_generation_plan"
        })
        .map(|artifact| artifact.artifact_id)
    {
        let Some(plan) = store.get_plan(plan_id).await? else {
            continue;
        };
        let jobs = store
            .list_jobs(JobQuery {
                plan_id: Some(plan_id),
                limit: 10_000,
                ..JobQuery::default()
            })
            .await?;
        let coverage =
            calculate_coverage(&plan, &store.dataset_cell_counts(plan.dataset_id).await?);
        generation.push(serde_json::json!({
            "plan_id": plan_id,
            "jobs": jobs,
            "coverage": coverage,
        }));
    }
    let definition = require_definition(store, run.definition_id).await?;
    let mut evidence_risk = Vec::new();
    for suite_id in
        std::iter::once(definition.development_suite_id).chain(definition.sealed_suite_id)
    {
        let Some(suite) = store.get_benchmark_suite(suite_id).await? else {
            continue;
        };
        for cohort in &suite.cohorts {
            let exposures = store
                .query_exposures(ExposureQuery {
                    cohort_id: cohort.cohort_id,
                    purpose: None,
                    limit: 10_000,
                    offset: 0,
                })
                .await?;
            evidence_risk.push(serde_json::json!({
                "suite_id": suite.id,
                "suite_kind": suite.kind,
                "cohort_id": cohort.cohort_id,
                "risk": summarize_exposure_risk(cohort.cohort_id, &exposures)?,
            }));
        }
    }
    crate::presentation::print(&serde_json::json!({
        "run": run,
        "attempt_count": attempts.len(),
        "latest_attempt": attempts.last(),
        "attempts": attempts,
        "generation": generation,
        "evidence_risk": evidence_risk,
    }))
}

async fn resolve_pending_workflow_recovery(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<()> {
    if store
        .list_recovery_records(false)
        .await?
        .iter()
        .any(|record| {
            record.workflow_kind == WorkflowKind::EncoderWorkflow && record.workflow_id == id
        })
    {
        store
            .resolve_recovery(
                WorkflowKind::EncoderWorkflow,
                id,
                RecoveryState::Resumed,
                None,
            )
            .await?;
    }
    Ok(())
}

async fn require_definition(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<WorkflowDefinition> {
    store
        .get_workflow_definition(id)
        .await?
        .with_context(|| format!("workflow definition not found: {id}"))
}

async fn require_run(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<WorkflowRun> {
    store
        .get_workflow_run(id)
        .await?
        .with_context(|| format!("workflow run not found: {id}"))
}

fn read_document<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        _ => {
            toml::from_str(&contents).with_context(|| format!("invalid TOML in {}", path.display()))
        }
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
