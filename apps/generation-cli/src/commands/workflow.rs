use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, ensure};
use evaluation_core::ports::EvaluationStore;
use generation_core::{
    dimensions::expand_generation_cells,
    jobs::JobState,
    ports::{DatasetStore, JobQuery, JobStore, RowStore},
};
use project_config::ProjectConfigurationStore;
use serde::de::DeserializeOwned;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    allocation::{
        InitialAllocationFeasibility, InitialAllocationRecord, InitialAllocationRequest,
        InitialCellCoverage, allocate_initial_budget,
    },
    benchmark::{BenchmarkSuiteKind, CohortAssessmentInput, assess_benchmark},
    governance::{EvidenceExposure, EvidenceExposureRequest, ExposurePurpose},
    ports::{
        AcceptanceAssessmentQuery, BenchmarkStore, GovernanceStore, InitialAllocationQuery,
        InitialAllocationStore, WorkflowDefinitionQuery, WorkflowRunQuery, WorkflowRunStore,
    },
    workflow::{
        StageAttemptState, StageOutcome, WorkflowArtifactLink, WorkflowBudgetUsage,
        WorkflowDefinition, WorkflowDefinitionRequest, WorkflowRun, WorkflowRunState,
        WorkflowStage, WorkflowStageAttempt,
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
            let run = require_run(store, id).await?;
            let definition = require_definition(store, run.definition_id).await?;
            let attempt = store
                .list_workflow_attempts(id)
                .await?
                .pop()
                .context("workflow run has no stage attempt")?;
            let run = drive_initial_pipeline(store, definition, run, attempt).await?;
            print_status(store, run).await
        }
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
            let Some(next) = initial_successor(attempt.stage) else {
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
        if attempt.state != StageAttemptState::Running {
            return Ok(run);
        }
        let (artifacts, usage) = execute_initial_stage(
            store,
            &definition,
            &run,
            &attempt,
            &persisted_configuration.resolved,
        )
        .await?;
        let finished = attempt.finish(
            &definition,
            &mut run,
            StageOutcome {
                state: StageAttemptState::Completed,
                reason: None,
                retryable: false,
                artifacts,
                usage_after: usage,
            },
        )?;
        store
            .commit_workflow_attempt(&run, &finished, attempt.id)
            .await?;
        attempt = finished;
    }
}

async fn execute_initial_stage(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    attempt: &WorkflowStageAttempt,
    configured: &project_config::ResolvedProjectConfig,
) -> anyhow::Result<(Vec<WorkflowArtifactLink>, WorkflowBudgetUsage)> {
    let history = store.list_workflow_attempts(run.id).await?;
    let mut usage = run.usage.clone();
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
                None => super::generation::run_workflow(plan_id, configured, store.clone()).await?,
            };
            ensure!(
                job.state == JobState::Completed,
                "generation job did not complete"
            );
            usage.accepted_rows = usage.accepted_rows.saturating_add(job.accepted_rows);
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
            let expected_name = format!("{}-workflow-{}", configured.snapshot.name, run.iteration);
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
                    super::training::run_workflow(snapshot_id, configured, store.clone()).await?
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
                    for cohort in &suite.cohorts {
                        let current_role = store
                            .get_current_cohort_role(cohort.cohort_id)
                            .await?
                            .context("development cohort has no current role")?;
                        let evaluation_run_id = assessment.evaluation_run_ids[&cohort.cohort_id];
                        let exposure = EvidenceExposure::new(
                            &store
                                .get_cohort(cohort.cohort_id)
                                .await?
                                .context("development cohort not found")?,
                            &current_role,
                            EvidenceExposureRequest {
                                evaluation_run_id: Some(evaluation_run_id),
                                workflow_run_id: Some(run.id),
                                workflow_iteration: Some(run.iteration),
                                purpose: ExposurePurpose::DevelopmentEvaluation,
                                disclosure: cohort.disclosure,
                                adaptation_eligible: true,
                                note: Some("workflow development assessment".into()),
                            },
                        )?;
                        store.append_exposure(&exposure, None).await?;
                    }
                    store.create_acceptance_assessment(&assessment).await?;
                    assessment
                }
            };
            vec![link(
                "acceptance_assessment",
                assessment.id,
                &assessment.fingerprint,
            )]
        }
        other => anyhow::bail!("workflow stage {other:?} is not connected yet"),
    };
    usage.validate_against(&definition.budget)?;
    Ok((artifacts, usage))
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

const fn initial_successor(stage: WorkflowStage) -> Option<WorkflowStage> {
    match stage {
        WorkflowStage::InitialAllocation => Some(WorkflowStage::Generation),
        WorkflowStage::Generation => Some(WorkflowStage::Snapshot),
        WorkflowStage::Snapshot => Some(WorkflowStage::Training),
        WorkflowStage::Training => Some(WorkflowStage::DevelopmentEvaluation),
        WorkflowStage::DevelopmentEvaluation => Some(WorkflowStage::AcceptanceAssessment),
        WorkflowStage::AcceptanceAssessment => None,
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

fn artifact_ids(history: &[WorkflowStageAttempt], kind: &str) -> Vec<uuid::Uuid> {
    history
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .collect()
}

async fn print_status(store: &SqliteStore, run: WorkflowRun) -> anyhow::Result<()> {
    let attempts = store.list_workflow_attempts(run.id).await?;
    crate::presentation::print(&serde_json::json!({
        "run": run,
        "attempt_count": attempts.len(),
        "latest_attempt": attempts.last(),
        "attempts": attempts,
    }))
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
