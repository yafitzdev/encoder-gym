//! Per-iteration training custody, distinct from the legacy root candidate.
use crate::{
    connect, dataset_versions, optimization_iterations, optimization_launch, optimization_runs,
};
use anyhow::{Context, Result, ensure};
use encoder_experiment_core::{
    domain::ExternalProjectSnapshot, journal::ExperimentEvent, protocol::ExperimentProtocol,
};
use project_workspace_core::optimization_iteration_execution::{
    IterationDevelopmentResult, IterationTrainingBinding,
};
use serde::de::DeserializeOwned;
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

async fn validate(folder: &Path, run_id: Uuid, binding: &IterationTrainingBinding) -> Result<()> {
    let iteration = optimization_iterations::list(folder, run_id)
        .await?
        .into_iter()
        .find(|item| item.id.to_string() == binding.iteration.id)
        .context("Iteration inputs are missing")?;
    let run = optimization_runs::show(folder, run_id).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|item| item.id.to_string() == run.run.launch.id)
        .context("Launch is missing")?;
    binding.validate_for(&iteration, &launch)?;
    let publication =
        crate::optimization_dataset::publication(folder, run_id, iteration.scope.iteration).await?;
    ensure!(
        publication.parent == iteration.dataset && publication.version == binding.qualified_dataset,
        "Training does not use this iteration's published Agent dataset"
    );
    let qualified = dataset_versions::inspect(folder, binding.qualified_dataset.id).await?;
    let training = dataset_versions::inspect(folder, binding.training_dataset.id).await?;
    ensure!(
        qualified.reference() == binding.qualified_dataset
            && training.reference() == binding.training_dataset
            && training.members.len() as u64 == binding.training_rows,
        "Iteration dataset identity changed"
    );
    let mut expected = qualified.members.clone();
    if let Some(limit) = launch
        .scope
        .agentic
        .as_ref()
        .unwrap()
        .training
        .maximum_training_rows
    {
        // Stable source identities, independent of source file order or RNG.
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        expected.truncate(limit as usize);
        let chosen: std::collections::BTreeSet<_> = expected.iter().map(|item| &item.id).collect();
        let expected: Vec<_> = qualified
            .members
            .iter()
            .filter(|item| chosen.contains(&item.id))
            .collect();
        ensure!(
            training.members.iter().collect::<Vec<_>>() == expected,
            "Training does not match the deterministic qualified subset"
        );
    } else {
        ensure!(
            training == qualified,
            "Training population differs from its clearance"
        );
    }
    Ok(())
}

pub async fn training(
    folder: &Path,
    run_id: Uuid,
    iteration_id: Uuid,
) -> Result<Option<IterationTrainingBinding>> {
    let mut database = connect(folder, true, false).await?;
    let value: Option<IterationTrainingBinding> = read(
        &mut database,
        "optimization_iteration_training",
        iteration_id,
    )
    .await?;
    database.close().await?;
    if let Some(binding) = &value {
        ensure!(
            binding.iteration.id == iteration_id.to_string(),
            "Training record belongs to another iteration"
        );
        validate(folder, run_id, binding).await?;
    }
    Ok(value)
}

pub async fn record_training(
    folder: &Path,
    run_id: Uuid,
    mut binding: IterationTrainingBinding,
) -> Result<IterationTrainingBinding> {
    validate(folder, run_id, &binding).await?;
    let view = optimization_runs::show(folder, run_id).await?;
    ensure!(
        !view.state.is_terminal(),
        "Run stopped before training reservation"
    );
    let mut database = connect(folder, false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut tx = database.begin_with("BEGIN IMMEDIATE").await?;
    check_head(&mut tx, run_id, &view.head_fingerprint).await?;
    let iteration_id = binding.iteration.id.parse()?;
    if let Some(existing) =
        read::<IterationTrainingBinding>(&mut tx, "optimization_iteration_training", iteration_id)
            .await?
    {
        binding.created_at = existing.created_at;
        binding.fingerprint = binding.reproduce()?;
        ensure!(
            binding == existing,
            "Iteration already reserved different training inputs"
        );
    } else {
        sqlx::query("INSERT INTO optimization_iteration_training(iteration_id,fingerprint,metadata_json) VALUES(?,?,?)")
            .bind(iteration_id.to_string()).bind(&binding.fingerprint).bind(serde_json::to_string(&binding)?)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    database.close().await?;
    Ok(binding)
}

pub async fn record_result(
    folder: &Path,
    run_id: Uuid,
    iteration_id: Uuid,
    project: &ExternalProjectSnapshot,
    protocol: &ExperimentProtocol,
    events: &[ExperimentEvent],
) -> Result<IterationDevelopmentResult> {
    let binding = training(folder, run_id, iteration_id)
        .await?
        .context("Training reservation is missing")?;
    let result = IterationDevelopmentResult::from_journal(&binding, project, protocol, events)?;
    let view = optimization_runs::show(folder, run_id).await?;
    ensure!(
        !view.state.is_terminal(),
        "Run stopped before iteration completion"
    );
    let mut database = connect(folder, false, false).await?;
    let mut tx = database.begin_with("BEGIN IMMEDIATE").await?;
    check_head(&mut tx, run_id, &view.head_fingerprint).await?;
    if let Some(existing) =
        read::<IterationDevelopmentResult>(&mut tx, "optimization_iteration_results", iteration_id)
            .await?
    {
        ensure!(
            existing == result,
            "Recorded iteration result differs from scientific evidence"
        );
    } else {
        sqlx::query("INSERT INTO optimization_iteration_results(iteration_id,fingerprint,metadata_json) VALUES(?,?,?)")
            .bind(iteration_id.to_string()).bind(&result.fingerprint).bind(serde_json::to_string(&result)?)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    database.close().await?;
    Ok(result)
}

async fn check_head(database: &mut SqliteConnection, run_id: Uuid, expected: &str) -> Result<()> {
    let head: String = sqlx::query_scalar("SELECT fingerprint FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1")
        .bind(run_id.to_string()).fetch_one(database).await?;
    ensure!(head == expected, "Run changed before iteration persistence");
    Ok(())
}

async fn read<T: DeserializeOwned>(
    database: &mut SqliteConnection,
    table: &str,
    iteration_id: Uuid,
) -> Result<Option<T>> {
    // Table names are private constants, never input from CLI or renderer.
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(&mut *database)
            .await?;
    if exists == 0 {
        return Ok(None);
    }
    let row = sqlx::query(&format!(
        "SELECT fingerprint,metadata_json FROM {table} WHERE iteration_id=?"
    ))
    .bind(iteration_id.to_string())
    .fetch_optional(database)
    .await?;
    row.map(|row| {
        let mut value: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        let fingerprint = value
            .as_object_mut()
            .context("Invalid iteration record")?
            .remove("fingerprint")
            .context("Missing iteration fingerprint")?;
        ensure!(
            fingerprint == row.get::<String, _>("fingerprint")
                && fingerprint == artifact_core::fingerprint(&value)?,
            "Iteration storage was altered"
        );
        Ok(serde_json::from_str(
            &row.get::<String, _>("metadata_json"),
        )?)
    })
    .transpose()
}
