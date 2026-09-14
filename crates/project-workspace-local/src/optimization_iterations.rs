//! Exact iteration input bindings, persisted before Agent dispatch. Scientific
//! protocols are loaded by the CLI through their owning read-only store; only
//! development references, never protocol payloads, enter this journal.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition, domain::ExternalProjectSnapshot, protocol::ExperimentProtocol,
};
use project_workspace_core::{
    OptimizationLaunchAuthorization, OptimizationSetup, ProjectBenchmarkVersion,
    ProjectOptimizationRunView,
    optimization_iteration::{IterationDevelopmentEvidence, ProjectOptimizationIteration},
};
use sqlx::{Connection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{connect, open_workspace, optimization_launch, optimization_runs, optimization_setup};

struct ContextBinding {
    run: ProjectOptimizationRunView,
    launch: OptimizationLaunchAuthorization,
    setup: OptimizationSetup,
    benchmark: ProjectBenchmarkVersion,
}

impl ContextBinding {
    async fn load(folder: &Path, run_id: Uuid) -> Result<Self> {
        let run = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|launch| launch.id.to_string() == run.run.launch.id)
            .context("Iteration launch authority is missing")?;
        let setup = optimization_setup::list(folder)
            .await?
            .into_iter()
            .find(|setup| setup.id.to_string() == run.run.setup.id)
            .context("Iteration setup is missing")?;
        let workspace = open_workspace(folder, false).await?;
        let benchmark = workspace
            .benchmark_versions
            .into_iter()
            .find(|benchmark| benchmark.id.to_string() == setup.inputs.benchmark.id)
            .context("Iteration benchmark is missing")?;
        Ok(Self {
            run,
            launch,
            setup,
            benchmark,
        })
    }

    fn validate(&self, iteration: &ProjectOptimizationIteration) -> Result<()> {
        let preparation = self
            .run
            .preparation
            .as_ref()
            .context("Verify the run inputs before binding an Agent iteration")?;
        iteration.validate_first(&self.run.run, &self.launch, &self.setup, preparation)?;
        iteration.validate_benchmark(&self.benchmark)?;
        Ok(())
    }
}

/// The native adapter must have normalized `definition` from `project`; the
/// source protocol must be loaded from the pinned scientific store, not JSON
/// submitted by the renderer. No provider, training or evaluation is invoked.
pub async fn begin_first(
    folder: &Path,
    run_id: Uuid,
    project: &ExternalProjectSnapshot,
    protocol: &ExperimentProtocol,
    definition: &BenchmarkDefinition,
) -> Result<ProjectOptimizationIteration> {
    let context = ContextBinding::load(folder, run_id).await?;
    ensure!(
        !context.run.state.is_terminal() && definition == &context.benchmark.definition,
        "The run is terminal or its native benchmark changed"
    );
    let preparation = context
        .run
        .preparation
        .as_ref()
        .context("Verify the run inputs before binding an Agent iteration")?;
    let development = IterationDevelopmentEvidence::baseline(project, protocol, definition)?;
    let mut iteration = ProjectOptimizationIteration::first(
        &context.run.run,
        &context.launch,
        &context.setup,
        preparation,
        development,
        Utc::now(),
    )?;
    context.validate(&iteration)?;
    let mut database = connect(folder, false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    // Cancellation/other journal advances win over the context read above.
    let head: String = sqlx::query_scalar(
        "SELECT fingerprint FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1",
    ).bind(run_id.to_string()).fetch_one(&mut *transaction).await?;
    ensure!(
        head == context.run.head_fingerprint,
        "Run changed before iteration reservation; reload it"
    );
    if let Some(existing) = read(&mut transaction, run_id).await?.into_iter().next() {
        context.validate(&existing)?;
        // Replay retains the original timestamp and exact identity, even if
        // the first response was lost after commit.
        iteration.created_at = existing.created_at;
        iteration.fingerprint = iteration.reproduce()?;
        ensure!(
            iteration == existing,
            "Iteration inputs differ from the recorded reservation"
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(existing);
    }
    insert(&mut transaction, &iteration).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(iteration)
}

/// Historical reads never upgrade a project or rehash model files.
pub async fn list(folder: &Path, run_id: Uuid) -> Result<Vec<ProjectOptimizationIteration>> {
    let context = ContextBinding::load(folder, run_id).await?;
    let mut database = connect(folder, true, false).await?;
    let rows = read(&mut database, run_id).await?;
    database.close().await?;
    for iteration in &rows {
        context.validate(iteration)?;
    }
    Ok(rows)
}

async fn insert(
    database: &mut SqliteConnection,
    iteration: &ProjectOptimizationIteration,
) -> Result<()> {
    // A first provider reservation must not invent its own evidence binding.
    // Existing unbound component journals cannot silently become executable.
    let scope: Option<String> = sqlx::query_scalar(
        "SELECT fingerprint FROM optimization_agent_scopes WHERE run_id=? AND iteration=?",
    )
    .bind(iteration.scope.run_id.to_string())
    .bind(i64::from(iteration.scope.iteration))
    .fetch_optional(&mut *database)
    .await?;
    ensure!(
        scope.is_none(),
        "Agent work already exists without this iteration reservation"
    );
    sqlx::query("INSERT INTO project_optimization_iterations (id,run_id,iteration,scope_fingerprint,fingerprint,metadata_json,created_at) VALUES (?,?,?,?,?,?,?)")
        .bind(iteration.id.to_string()).bind(iteration.scope.run_id.to_string())
        .bind(i64::from(iteration.scope.iteration)).bind(iteration.scope.fingerprint()?)
        .bind(&iteration.fingerprint).bind(serde_json::to_string(iteration)?)
        .bind(iteration.created_at.to_rfc3339()).execute(database).await?;
    Ok(())
}

async fn read(
    database: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<ProjectOptimizationIteration>> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_optimization_iterations'")
        .fetch_one(&mut *database).await?;
    if exists == 0 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT id,iteration,scope_fingerprint,fingerprint,metadata_json,created_at FROM project_optimization_iterations WHERE run_id=? ORDER BY iteration")
        .bind(run_id.to_string()).fetch_all(database).await?;
    rows.into_iter()
        .map(|row| {
            let iteration: ProjectOptimizationIteration =
                serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
            iteration.development.validate_identity()?;
            ensure!(
                iteration.scope.run_id == run_id
                    && iteration.id.to_string() == row.get::<String, _>("id")
                    && i64::from(iteration.scope.iteration) == row.get::<i64, _>("iteration")
                    && iteration.scope.fingerprint()? == row.get::<String, _>("scope_fingerprint")
                    && iteration.fingerprint == row.get::<String, _>("fingerprint")
                    && iteration.fingerprint == iteration.reproduce()?
                    && iteration.created_at.to_rfc3339() == row.get::<String, _>("created_at"),
                "Persisted optimization iteration binding changed"
            );
            Ok(iteration)
        })
        .collect()
}
