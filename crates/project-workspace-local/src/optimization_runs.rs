//! Project-owned run roots for input-first optimization.
//!
//! This module reserves and verifies the root journal only. Slice execution is
//! attached later through its own adapters and stores.

use crate::{
    connect, load_provider_catalog_history, open_workspace, optimization_launch, optimization_setup,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    OptimizationLaunchAuthorization, ProjectOptimizationEvent, ProjectOptimizationRun,
    ProjectOptimizationRunView, replay_project_optimization,
};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

pub async fn start(
    folder: &Path,
    request: optimization_launch::LaunchRequest,
    authorized_by: &str,
) -> Result<ProjectOptimizationRunView> {
    // Authorization and reservation are separately idempotent. If the process
    // stops between them, the same request finishes the missing reservation.
    let authorization = optimization_launch::authorize(folder, request, authorized_by).await?;
    let verified = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&verified.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &verified).await?;
    let providers = load_provider_catalog_history(&mut transaction, &verified.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &verified, &setups, &providers).await?;
    let persisted = launches
        .iter()
        .find(|value| value.id == authorization.id)
        .context("Optimization launch authorization is missing.")?;
    ensure!(
        persisted == &authorization,
        "Optimization launch authorization changed."
    );
    let existing = load(&mut transaction, verified.manifest.id, &launches).await?;
    if let Some(run) = existing
        .into_iter()
        .find(|run| run.run.launch.id == authorization.id.to_string())
    {
        transaction.commit().await?;
        database.close().await?;
        return Ok(run);
    }

    // A launch authorized earlier but not yet reserved must still be current at
    // the moment it becomes executable work.
    let setup = setups
        .iter()
        .find(|value| value.id.to_string() == authorization.scope.setup.id)
        .context("Optimization setup is missing.")?;
    ensure!(
        setups.last().is_some_and(|current| current.id == setup.id),
        "Optimization inputs changed. Review them again."
    );
    let provider = providers
        .iter()
        .find(|value| value.id.to_string() == authorization.scope.provider_catalog.id)
        .context("Optimization provider settings are missing.")?;
    ensure!(
        providers
            .last()
            .is_some_and(|current| current.id == provider.id),
        "Provider settings changed. Review Optimize again."
    );
    let benchmark = verified
        .benchmark_versions
        .iter()
        .find(|value| value.id.to_string() == setup.inputs.benchmark.id)
        .context("Optimization benchmark is missing.")?;
    authorization.validate(setup, provider, benchmark)?;
    let active: String = sqlx::query_scalar(
        "SELECT active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        active == setup.inputs.baseline_revision.id,
        "Baseline changed. Review Optimize inputs again."
    );

    let created_at = Utc::now();
    let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &authorization, created_at)?;
    let event = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, created_at)?;
    sqlx::query("INSERT INTO project_optimization_runs (id, project_id, launch_id, launch_fingerprint, setup_id, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(run.id.to_string()).bind(run.project_id.to_string()).bind(&run.launch.id)
        .bind(&run.launch.fingerprint).bind(&run.setup.id).bind(&run.fingerprint)
        .bind(serde_json::to_string(&run)?).bind(run.created_at.to_rfc3339())
        .execute(&mut *transaction).await?;
    sqlx::query("INSERT INTO project_optimization_events (id, run_id, sequence, previous_event_fingerprint, kind, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(event.id.to_string()).bind(event.run_id.to_string()).bind(i64::try_from(event.sequence)?)
        .bind(&event.previous_event_fingerprint).bind("reserved").bind(&event.fingerprint)
        .bind(serde_json::to_string(&event)?).bind(event.created_at.to_rfc3339())
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    replay_project_optimization(&run, &authorization, &[event]).map_err(Into::into)
}

pub async fn list(folder: &Path) -> Result<Vec<ProjectOptimizationRunView>> {
    let workspace = open_workspace(folder, false).await?;
    let launches = optimization_launch::list(folder).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let runs = load(&mut database, workspace.manifest.id, &launches).await?;
    database.close().await?;
    Ok(runs)
}

pub async fn show(folder: &Path, run_id: Uuid) -> Result<ProjectOptimizationRunView> {
    list(folder)
        .await?
        .into_iter()
        .find(|run| run.run.id == run_id)
        .context("Project optimization run was not found.")
}

async fn load(
    database: &mut SqliteConnection,
    project_id: Uuid,
    launches: &[OptimizationLaunchAuthorization],
) -> Result<Vec<ProjectOptimizationRunView>> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_optimization_runs'",
    )
    .fetch_one(&mut *database)
    .await?
        == 0
    {
        return Ok(vec![]);
    }
    let records = sqlx::query("SELECT * FROM project_optimization_runs ORDER BY created_at, id")
        .fetch_all(&mut *database)
        .await?;
    let mut result = Vec::new();
    for row in records {
        let run: ProjectOptimizationRun =
            serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?;
        ensure!(
            run.project_id == project_id
                && row.try_get::<String, _>("id")? == run.id.to_string()
                && row.try_get::<String, _>("project_id")? == project_id.to_string()
                && row.try_get::<String, _>("launch_id")? == run.launch.id
                && row.try_get::<String, _>("launch_fingerprint")? == run.launch.fingerprint
                && row.try_get::<String, _>("setup_id")? == run.setup.id
                && row.try_get::<String, _>("fingerprint")? == run.fingerprint
                && row.try_get::<String, _>("created_at")? == run.created_at.to_rfc3339(),
            "Project optimization run storage changed."
        );
        let launch = launches
            .iter()
            .find(|launch| launch.id.to_string() == run.launch.id)
            .context("Project optimization launch is missing.")?;
        let event_rows = sqlx::query(
            "SELECT * FROM project_optimization_events WHERE run_id=? ORDER BY sequence",
        )
        .bind(run.id.to_string())
        .fetch_all(&mut *database)
        .await?;
        let mut events = Vec::new();
        for event_row in event_rows {
            let event: ProjectOptimizationEvent =
                serde_json::from_str(&event_row.try_get::<String, _>("metadata_json")?)?;
            ensure!(
                event.run_id == run.id
                    && event_row.try_get::<String, _>("id")? == event.id.to_string()
                    && event_row.try_get::<String, _>("run_id")? == run.id.to_string()
                    && u64::try_from(event_row.try_get::<i64, _>("sequence")?)? == event.sequence
                    && event_row.try_get::<Option<String>, _>("previous_event_fingerprint")?
                        == event.previous_event_fingerprint
                    && event_row.try_get::<String, _>("kind")? == "reserved"
                    && event_row.try_get::<String, _>("fingerprint")? == event.fingerprint
                    && event_row.try_get::<String, _>("created_at")?
                        == event.created_at.to_rfc3339(),
                "Project optimization event storage changed."
            );
            events.push(event);
        }
        result.push(replay_project_optimization(&run, launch, &events)?);
    }
    Ok(result)
}
