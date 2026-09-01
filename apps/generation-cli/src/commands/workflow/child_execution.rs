use anyhow::{Context, ensure};
use dataset_quality_core::{lifecycle::QualityAuditRunState, ports::DatasetQualityStore};
use evaluation_core::ports::EvaluationStore;
use generation_core::{jobs::JobState, ports::JobStore};
use generation_supervisor_core::{
    lifecycle::{SupervisorRunEvent, SupervisorRunState},
    ports::GenerationSupervisorStore,
};
use recovery_core::{RecoveryState, RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;
use training_core::{domain::TrainingRunState, ports::TrainingStore};
use uuid::Uuid;
use workflow_core::{
    execution::{WorkflowChildExecution, WorkflowChildKind},
    ports::WorkflowRunStore,
    workflow::WorkflowStageAttempt,
};

pub(crate) async fn reserve(
    store: &SqliteStore,
    attempt: &WorkflowStageAttempt,
    child_kind: WorkflowChildKind,
    logical_key: impl Into<String>,
    child_execution_id: Uuid,
) -> anyhow::Result<WorkflowChildExecution> {
    let logical_key = logical_key.into();
    let existing = store.list_workflow_child_executions(attempt.id).await?;
    if let Some(link) = existing
        .iter()
        .find(|link| link.child_kind == child_kind && link.child_execution_id == child_execution_id)
    {
        ensure!(
            link.logical_key == logical_key,
            "workflow child execution identity is already reserved for another logical input"
        );
        return Ok(link.clone());
    }
    let ordinal = u32::try_from(existing.len())
        .context("workflow attempt has too many child executions")?
        .checked_add(1)
        .context("workflow child execution ordinal overflow")?;
    let link = WorkflowChildExecution::new(
        attempt,
        ordinal,
        child_kind,
        logical_key,
        child_execution_id,
    )?;
    store.create_workflow_child_execution(&link).await?;
    Ok(link)
}

pub(super) async fn reserve_next(
    store: &SqliteStore,
    attempt: &WorkflowStageAttempt,
    child_kind: WorkflowChildKind,
    logical_key: impl Into<String>,
) -> anyhow::Result<WorkflowChildExecution> {
    let logical_key = logical_key.into();
    let existing = store.list_workflow_child_executions(attempt.id).await?;
    let previous = existing
        .iter()
        .rev()
        .find(|link| link.child_kind == child_kind && link.logical_key == logical_key);
    if let Some(link) = previous {
        if child_identity_is_reusable(store, link).await? {
            return Ok(link.clone());
        }
        if child_kind == WorkflowChildKind::GenerationJob
            && pending_recovery(store, WorkflowKind::Generation, link.child_execution_id).await?
        {
            store
                .prepare_generation_resume(link.child_execution_id)
                .await?;
            return Ok(link.clone());
        }
    }
    let replacement = reserve(store, attempt, child_kind, logical_key, Uuid::new_v4()).await?;
    if let Some(previous) = previous
        && let Some(kind) = recovery_kind(previous.child_kind)
        && pending_recovery(store, kind, previous.child_execution_id).await?
    {
        store
            .resolve_recovery(
                kind,
                previous.child_execution_id,
                RecoveryState::Restarted,
                Some(replacement.child_execution_id),
            )
            .await?;
    }
    Ok(replacement)
}

pub(crate) async fn synchronize_parent_before_start(
    store: &SqliteStore,
    workflow_run_id: Uuid,
    child: &WorkflowChildExecution,
) -> anyhow::Result<()> {
    let run = store
        .get_workflow_run(workflow_run_id)
        .await?
        .context("workflow parent disappeared after child reservation")?;
    if run.cancel_requested {
        request_child_cancellation(store, child).await?;
        return Ok(());
    }
    ensure!(
        run.latest_attempt_id == Some(child.workflow_stage_attempt_id),
        "workflow parent advanced before child execution startup"
    );
    Ok(())
}

pub(super) async fn request_attempt_cancellation(
    store: &SqliteStore,
    attempt_id: Uuid,
) -> anyhow::Result<usize> {
    let children = store.list_workflow_child_executions(attempt_id).await?;
    for child in &children {
        request_child_cancellation(store, child).await?;
    }
    Ok(children.len())
}

async fn request_child_cancellation(
    store: &SqliteStore,
    child: &WorkflowChildExecution,
) -> anyhow::Result<bool> {
    match child.child_kind {
        WorkflowChildKind::GenerationJob => Ok(store
            .request_job_cancellation(child.child_execution_id)
            .await?),
        WorkflowChildKind::GenerationSupervisorRun => Ok(store
            .request_supervisor_cancellation(child.child_execution_id)
            .await?),
        WorkflowChildKind::TrainingRun => Ok(store
            .request_training_cancellation(child.child_execution_id)
            .await?),
        WorkflowChildKind::EvaluationRun => Ok(store
            .request_evaluation_cancellation(child.child_execution_id)
            .await?),
        WorkflowChildKind::QualityAuditRun => {
            let Some(mut run) = store.get_audit_run(child.child_execution_id).await? else {
                return Ok(false);
            };
            match run.state {
                QualityAuditRunState::Queued => run.cancel()?,
                QualityAuditRunState::Running => run.request_cancel()?,
                QualityAuditRunState::Completed
                | QualityAuditRunState::Failed
                | QualityAuditRunState::Cancelled => return Ok(false),
            }
            store.save_audit_run(&run).await?;
            Ok(true)
        }
    }
}

async fn child_identity_is_reusable(
    store: &SqliteStore,
    child: &WorkflowChildExecution,
) -> anyhow::Result<bool> {
    match child.child_kind {
        WorkflowChildKind::GenerationJob => Ok(store
            .get_job(child.child_execution_id)
            .await?
            .is_none_or(|job| {
                matches!(
                    job.state,
                    JobState::Queued | JobState::Running | JobState::Completed
                )
            })),
        WorkflowChildKind::GenerationSupervisorRun => {
            let Some(run) = store.get_supervisor_run(child.child_execution_id).await? else {
                return Ok(true);
            };
            let state =
                SupervisorRunEvent::verify_chain(run.id, &store.list_run_events(run.id).await?)?;
            Ok(!matches!(
                state,
                SupervisorRunState::Failed | SupervisorRunState::Cancelled
            ))
        }
        WorkflowChildKind::TrainingRun => Ok(store
            .get_training_run(child.child_execution_id)
            .await?
            .is_none_or(|run| {
                matches!(
                    run.state,
                    TrainingRunState::Queued
                        | TrainingRunState::Running
                        | TrainingRunState::Completed
                )
            })),
        WorkflowChildKind::EvaluationRun => Ok(store
            .get_evaluation_run(child.child_execution_id)
            .await?
            .is_none_or(|run| {
                matches!(
                    run.state,
                    evaluation_core::domain::EvaluationRunState::Queued
                        | evaluation_core::domain::EvaluationRunState::Running
                        | evaluation_core::domain::EvaluationRunState::Completed
                )
            })),
        WorkflowChildKind::QualityAuditRun => Ok(store
            .get_audit_run(child.child_execution_id)
            .await?
            .is_none_or(|run| {
                matches!(
                    run.state,
                    QualityAuditRunState::Queued
                        | QualityAuditRunState::Running
                        | QualityAuditRunState::Completed
                )
            })),
    }
}

const fn recovery_kind(kind: WorkflowChildKind) -> Option<WorkflowKind> {
    match kind {
        WorkflowChildKind::GenerationJob => Some(WorkflowKind::Generation),
        WorkflowChildKind::GenerationSupervisorRun => None,
        WorkflowChildKind::TrainingRun => Some(WorkflowKind::Training),
        WorkflowChildKind::EvaluationRun => Some(WorkflowKind::Evaluation),
        WorkflowChildKind::QualityAuditRun => None,
    }
}

async fn pending_recovery(
    store: &SqliteStore,
    kind: WorkflowKind,
    id: Uuid,
) -> anyhow::Result<bool> {
    Ok(store
        .list_recovery_records(false)
        .await?
        .iter()
        .any(|record| record.workflow_kind == kind && record.workflow_id == id))
}
