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
pub async fn begin_attempt(folder: &Path, run_id: Uuid) -> Result<Option<Uuid>> {
    let view = optimization_runs::show(folder, run_id).await?;
    if let Some(previous) = &view.agent_execution {
        if previous.state == AgentExecutionState::Completed {
            return Ok(None);
        }
        if previous.state == AgentExecutionState::Running {
            append(
                folder,
                run_id,
                previous.attempt_id,
                AgentExecutionChange::Interrupted,
            )
            .await?;
        }
    }
    let attempt = Uuid::new_v4();
    append(folder, run_id, attempt, AgentExecutionChange::Started).await?;
    Ok(Some(attempt))
}

pub async fn fail_attempt(
    folder: &Path,
    run_id: Uuid,
    attempt: Uuid,
) -> Result<AgentExecutionView> {
    append(folder, run_id, attempt, AgentExecutionChange::Failed).await
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
    append(
        folder,
        run_id,
        attempt,
        AgentExecutionChange::Completed {
            completion: completion.identity(),
        },
    )
    .await
}

async fn append(
    folder: &Path,
    run_id: Uuid,
    attempt: Uuid,
    change: AgentExecutionChange,
) -> Result<AgentExecutionView> {
    let view = optimization_runs::show(folder, run_id).await?;
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
    ensure!(
        root_head == view.head_fingerprint
            && previous == view.agent_execution
            && optimization_completions::read(&mut tx, run_id).await? == completions,
        "Run or execution attempt changed before its outcome was saved"
    );
    let event = AgentExecutionEvent::create(
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
            AgentExecutionState::Interrupted => ProjectOptimizationRunState::AgentInterrupted,
            AgentExecutionState::Failed => ProjectOptimizationRunState::AgentFailed,
            AgentExecutionState::Completed => ProjectOptimizationRunState::AgentCompleted,
        };
    }
    view.updated_at = view.updated_at.max(execution.updated_at);
    view.agent_execution = Some(execution);
    Ok(())
}
