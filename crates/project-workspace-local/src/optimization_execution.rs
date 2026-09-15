//! Attempt ownership for the existing Agent coordinator. The caller acquires
//! the exact root execution lease before beginning or recovering an attempt.
use crate::{
    connect, optimization_completions, optimization_iterations, optimization_launch,
    optimization_runs,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    OptimizationLaunchAuthorization, OptimizationSetup, ProjectBenchmarkVersion,
    ProjectOptimizationRunState, ProjectOptimizationRunView,
    optimization_execution::{
        self as domain, AgentExecutionChange, AgentExecutionEvent, AgentExecutionState,
        AgentExecutionView,
    },
    optimization_loop::IterationCompletion,
};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

/// Called only after acquiring the exclusive PID/start-time-verified execution
/// lease. Thus an unfinished prior attempt belongs to a worker that is no longer
/// alive; a timeout or a missing provider response alone cannot enter this path.
pub async fn begin_attempt(
    folder: &Path,
    run_id: Uuid,
    resume: Option<&str>,
) -> Result<Option<Uuid>> {
    let mut view = optimization_runs::show(folder, run_id).await?;
    if let Some(previous) = view.agent_execution.clone() {
        if previous.state == AgentExecutionState::Completed {
            return Ok(None);
        }
        if let Some(expected) = resume {
            ensure!(
                matches!(
                    previous.state,
                    AgentExecutionState::StopRequested | AgentExecutionState::Paused
                ) && previous.head_fingerprint == expected,
                "Resume state changed. Read the current run and resume its exact stopped execution head."
            );
        }
        if matches!(
            previous.state,
            AgentExecutionState::StopRequested | AgentExecutionState::Paused
        ) {
            ensure!(
                resume.is_some(),
                "Run is stopped. Resume explicitly with drive-agent --resume <EXECUTION_HEAD>."
            );
            if previous.state == AgentExecutionState::StopRequested {
                view.agent_execution = Some(
                    append(
                        folder,
                        &view,
                        previous.attempt_id,
                        AgentExecutionChange::Paused,
                        None,
                    )
                    .await?,
                );
            }
        }
        if previous.state == AgentExecutionState::Running {
            view.agent_execution = Some(
                append(
                    folder,
                    &view,
                    previous.attempt_id,
                    AgentExecutionChange::Interrupted,
                    None,
                )
                .await?,
            );
        }
    } else {
        ensure!(
            resume.is_none(),
            "Resume state changed: this run has no stopped execution head."
        );
    }
    let attempt = Uuid::new_v4();
    append(folder, &view, attempt, AgentExecutionChange::Started, None).await?;
    Ok(Some(attempt))
}

/// Stop intent is durable before the worker is asked to interrupt. This never
/// asserts that work has already stopped; only the owner or a lease holder can
/// acknowledge it after active work has unwound.
pub async fn request_stop(
    folder: &Path,
    run_id: Uuid,
    request_id: Uuid,
) -> Result<AgentExecutionView> {
    ensure!(
        !request_id.is_nil(),
        "Stop needs a non-nil request identity"
    );
    // Retry only compare-and-append conflicts. A new Stop during paused/failed
    // state still changes the head and fences any already-issued Resume.
    for _ in 0..8 {
        let view = optimization_runs::show(folder, run_id).await?;
        let mut db = connect(folder, true, false).await?;
        let events = read(&mut db, run_id).await?;
        db.close().await?;
        if let Some(event) = events.iter().find(|e| e.id == request_id) {
            ensure!(
                event.change == AgentExecutionChange::StopRequested,
                "Stop identity belongs to another execution action"
            );
            return optimization_runs::show(folder, run_id)
                .await?
                .agent_execution
                .context("Stop execution is missing");
        }
        let attempt = view
            .agent_execution
            .as_ref()
            .map_or_else(Uuid::new_v4, |v| v.attempt_id);
        match append(
            folder,
            &view,
            attempt,
            AgentExecutionChange::StopRequested,
            Some(request_id),
        )
        .await
        {
            Err(error) if error.is::<ExecutionChanged>() => continue,
            result => return result,
        }
    }
    anyhow::bail!("Run changed repeatedly while saving Stop. Retry the same request identity.")
}

/// Caller owns the execution lease and has finished unwinding active work.
pub async fn acknowledge_stop(
    folder: &Path,
    run_id: Uuid,
    attempt: Uuid,
) -> Result<AgentExecutionView> {
    let view = optimization_runs::show(folder, run_id).await?;
    append(folder, &view, attempt, AgentExecutionChange::Paused, None).await
}

/// Caller acquired the exclusive lease without starting work. Read-only status
/// remains read-only; this explicit recovery operation records abandoned work.
pub async fn reconcile(folder: &Path, run_id: Uuid) -> Result<()> {
    let view = optimization_runs::show(folder, run_id).await?;
    if let Some(previous) = &view.agent_execution {
        match previous.state {
            AgentExecutionState::Running => {
                append(
                    folder,
                    &view,
                    previous.attempt_id,
                    AgentExecutionChange::Interrupted,
                    None,
                )
                .await?;
            }
            AgentExecutionState::StopRequested => {
                acknowledge_stop(folder, run_id, previous.attempt_id).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Lightweight dispatch fence; no dataset reads, migration or credential access.
pub async fn stopped(folder: &Path, run_id: Uuid) -> Result<bool> {
    let mut db = connect(folder, true, false).await?;
    let stopped = dispatch_stopped(&mut db, run_id).await?;
    db.close().await?;
    Ok(stopped)
}

pub async fn fail_attempt(
    folder: &Path,
    run_id: Uuid,
    attempt: Uuid,
) -> Result<AgentExecutionView> {
    let view = optimization_runs::show(folder, run_id).await?;
    append(folder, &view, attempt, AgentExecutionChange::Failed, None).await
}

/// The caller first replays original scientific journals through the normal
/// completion contract. This adapter additionally validates the saved chain and
/// binds the exact immutable terminal record, never caller-provided outcomes.
pub async fn complete_attempt(
    folder: &Path,
    run_id: Uuid,
    attempt: Uuid,
    completion: &IterationCompletion,
) -> Result<AgentExecutionView> {
    let saved = optimization_completions::list(folder, run_id).await?;
    ensure!(
        saved.last() == Some(completion),
        "Execution outcome is not the last verified completion"
    );
    let view = optimization_runs::show(folder, run_id).await?;
    append(
        folder,
        &view,
        attempt,
        AgentExecutionChange::Completed {
            completion: completion.identity(),
        },
        None,
    )
    .await
}

async fn append(
    folder: &Path,
    view: &ProjectOptimizationRunView,
    attempt: Uuid,
    change: AgentExecutionChange,
    event_id: Option<Uuid>,
) -> Result<AgentExecutionView> {
    let run_id = view.run.id;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|l| l.id.to_string() == view.run.launch.id)
        .context("Launch missing")?;
    ensure!(
        launch.scope.agentic.is_some()
            && view.materialization.is_none()
            && view.experiment.is_none(),
        "Run does not belong to the Agent coordinator"
    );
    ensure!(
        !view.state.is_terminal(),
        "Terminal run cannot accept another execution attempt"
    );
    let completions = optimization_completions::list(folder, run_id).await?;
    let mut db = connect(folder, false, false).await?;
    sqlx::migrate!("./migrations").run(&mut db).await?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let root_head: String = sqlx::query_scalar("SELECT fingerprint FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1")
        .bind(run_id.to_string()).fetch_one(&mut *tx).await?;
    let mut events = read(&mut tx, run_id).await?;
    let previous = domain::replay(&view.run.identity(), &events, &completions)?;
    if !(root_head == view.head_fingerprint
        && previous == view.agent_execution
        && optimization_completions::read(&mut tx, run_id).await? == completions)
    {
        return Err(ExecutionChanged.into());
    }
    let event = AgentExecutionEvent::create_with_id(
        event_id.unwrap_or_else(Uuid::new_v4),
        view.run.identity(),
        events.last(),
        attempt,
        change,
        Utc::now(),
    )?;
    events.push(event.clone());
    let result = domain::replay(&view.run.identity(), &events, &completions)?
        .context("Execution view missing")?;
    sqlx::query("INSERT INTO optimization_agent_execution_events(id,run_id,sequence,attempt_id,kind,fingerprint,metadata_json) VALUES(?,?,?,?,?,?,?)")
        .bind(event.id.to_string()).bind(run_id.to_string()).bind(i64::try_from(event.sequence)?)
        .bind(event.attempt_id.to_string()).bind(event.change.storage_key()).bind(&event.fingerprint)
        .bind(serde_json::to_string(&event)?).execute(&mut *tx).await?;
    tx.commit().await?;
    db.close().await?;
    Ok(result)
}

#[derive(Debug)]
struct ExecutionChanged;

impl std::fmt::Display for ExecutionChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Run or execution attempt changed before its outcome was saved")
    }
}

impl std::error::Error for ExecutionChanged {}

pub(crate) async fn read(db: &mut SqliteConnection, run: Uuid) -> Result<Vec<AgentExecutionEvent>> {
    let exists:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_agent_execution_events'").fetch_one(&mut *db).await?;
    if exists == 0 {
        return Ok(vec![]);
    }
    let rows = sqlx::query(
        "SELECT * FROM optimization_agent_execution_events WHERE run_id=? ORDER BY sequence",
    )
    .bind(run.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|row| {
            let event: AgentExecutionEvent =
                serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
            ensure!(
                event.id.to_string() == row.get::<String, _>("id")
                    && event.run.id == run.to_string()
                    && event.sequence == u64::try_from(row.get::<i64, _>("sequence"))?
                    && event.attempt_id.to_string() == row.get::<String, _>("attempt_id")
                    && event.change.storage_key() == row.get::<String, _>("kind")
                    && event.fingerprint == row.get::<String, _>("fingerprint"),
                "Execution event storage changed"
            );
            Ok(event)
        })
        .collect()
}

/// Checked inside the same transaction as provider reservation. Historical
/// single-cycle commands without an execution journal retain their behavior.
pub(crate) async fn dispatch_stopped(db: &mut SqliteConnection, run: Uuid) -> Result<bool> {
    let root:Option<String> = sqlx::query_scalar("SELECT kind FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1").bind(run.to_string()).fetch_optional(&mut *db).await?;
    let events = read(db, run).await?;
    Ok(root.as_deref() == Some("cancelled")
        || events
            .last()
            .is_some_and(|event| event.change != AgentExecutionChange::Started))
}

/// Decorates the ordinary root view without changing its old journal or creating
/// recursive calls through public iteration/root readers.
pub(crate) async fn project(
    db: &mut SqliteConnection,
    view: &mut ProjectOptimizationRunView,
    launch: &OptimizationLaunchAuthorization,
    setup: &OptimizationSetup,
    benchmark: &ProjectBenchmarkVersion,
) -> Result<()> {
    let events = read(db, view.run.id).await?;
    if events.is_empty() {
        return Ok(());
    }
    ensure!(
        launch.scope.agentic.is_some()
            && view.materialization.is_none()
            && view.experiment.is_none(),
        "Agent execution attached to a fixed-recipe run"
    );
    optimization_iterations::validate_for_view(db, view, launch, setup, benchmark).await?;
    let completions = optimization_completions::read(db, view.run.id).await?;
    let execution = domain::replay(&view.run.identity(), &events, &completions)?
        .context("Execution history missing")?;
    if view.state != ProjectOptimizationRunState::Cancelled {
        view.state = match execution.state {
            AgentExecutionState::Running => ProjectOptimizationRunState::AgentRunning,
            AgentExecutionState::StopRequested => ProjectOptimizationRunState::AgentStopping,
            AgentExecutionState::Paused => ProjectOptimizationRunState::AgentPaused,
            AgentExecutionState::Interrupted => ProjectOptimizationRunState::AgentInterrupted,
            AgentExecutionState::Failed => ProjectOptimizationRunState::AgentFailed,
            AgentExecutionState::Completed => ProjectOptimizationRunState::AgentCompleted,
        };
    }
    view.updated_at = view.updated_at.max(execution.updated_at);
    view.agent_execution = Some(execution);
    Ok(())
}

#[cfg(test)]
mod tests;
