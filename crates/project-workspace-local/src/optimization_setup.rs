//! Immutable project input selections; no scientific execution or approval.
use crate::{ManagedWorkspace, connect, dataset_versions, open_workspace};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{OptimizationInputs, OptimizationSetup};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupPreview {
    pub expected_parent: Option<Uuid>,
    pub inputs: OptimizationInputs,
    pub model_name: String,
    pub dataset_rows: u64,
    pub benchmark_number: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetupRequest {
    pub id: Uuid,
    pub expected_parent: Option<Uuid>,
    pub inputs: OptimizationInputs,
}

pub async fn preview(
    folder: &Path,
    model: Uuid,
    dataset: Uuid,
    benchmark: Uuid,
) -> Result<SetupPreview> {
    let workspace = open_workspace(folder, false).await?;
    let (inputs, rows, number) = resolve(&workspace, model, dataset, benchmark).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Upgrade the model catalog first.")?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let history = load(&mut database, &workspace).await?;
    database.close().await?;
    Ok(SetupPreview {
        expected_parent: history.last().map(|s| s.id),
        inputs,
        model_name: catalog.active_model().name.clone(),
        dataset_rows: rows,
        benchmark_number: number,
    })
}

async fn resolve(
    workspace: &ManagedWorkspace,
    model: Uuid,
    dataset: Uuid,
    benchmark: Uuid,
) -> Result<(OptimizationInputs, u64, u64)> {
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Upgrade the model catalog first.")?;
    let benchmark = workspace
        .benchmark_versions
        .iter()
        .find(|v| v.id == benchmark)
        .context("Choose a benchmark version belonging to this project.")?;
    let dataset = dataset_versions::inspect(Path::new(&workspace.folder), dataset).await?;
    let inputs =
        OptimizationInputs::bind(workspace.manifest.id, model, catalog, &dataset, benchmark)?;
    Ok((inputs, dataset.members.len() as u64, benchmark.number))
}

pub async fn list(folder: &Path) -> Result<Vec<OptimizationSetup>> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let setups = load(&mut database, &workspace).await?;
    database.close().await?;
    Ok(setups)
}

async fn load(
    database: &mut SqliteConnection,
    workspace: &ManagedWorkspace,
) -> Result<Vec<OptimizationSetup>> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_setups'",
    )
    .fetch_one(&mut *database)
    .await?
        == 0
    {
        return Ok(vec![]);
    }
    let records = sqlx::query("SELECT * FROM optimization_setups ORDER BY number")
        .fetch_all(&mut *database)
        .await?;
    let mut history = Vec::new();
    for row in records {
        let setup: OptimizationSetup =
            serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?;
        setup.validate(history.last())?;
        let inputs = &setup.inputs;
        ensure!(
            inputs.project_id == workspace.manifest.id
                && row.try_get::<String, _>("project_id")? == inputs.project_id.to_string()
                && row.try_get::<String, _>("id")? == setup.id.to_string()
                && u64::try_from(row.try_get::<i64, _>("number")?)? == setup.number
                && row.try_get::<Option<String>, _>("parent_id")?
                    == setup.parent.as_ref().map(|p| p.id.clone())
                && row.try_get::<String, _>("fingerprint")? == setup.fingerprint
                && row.try_get::<String, _>("baseline_revision_id")? == inputs.baseline_revision.id
                && row.try_get::<String, _>("model_id")? == inputs.model.id
                && row.try_get::<String, _>("dataset_version_id")? == inputs.dataset.id.to_string()
                && row.try_get::<String, _>("benchmark_version_id")? == inputs.benchmark.id,
            "Optimization setup metadata changed."
        );
        let catalog = workspace
            .model_catalog
            .as_ref()
            .context("Setup model catalog is missing.")?;
        let revision = catalog
            .baseline_revisions
            .iter()
            .find(|r| r.id.to_string() == inputs.baseline_revision.id)
            .context("Setup baseline revision is missing.")?;
        let model = catalog
            .artifacts
            .iter()
            .find(|m| m.id.to_string() == inputs.model.id)
            .context("Setup model is missing.")?;
        let benchmark = workspace
            .benchmark_versions
            .iter()
            .find(|v| v.id.to_string() == inputs.benchmark.id)
            .context("Setup benchmark is missing.")?;
        ensure!(
            revision.fingerprint == inputs.baseline_revision.fingerprint
                && revision.model_artifact_id == model.id
                && model.fingerprint == inputs.model.fingerprint
                && benchmark.fingerprint == inputs.benchmark.fingerprint,
            "Optimization setup artifact binding changed."
        );
        ensure!(
            dataset_versions::load_reference(database, workspace.manifest.id, inputs.dataset.id)
                .await?
                == inputs.dataset,
            "Optimization setup dataset binding changed."
        );
        history.push(setup);
    }
    Ok(history)
}

pub async fn save(folder: &Path, request: SetupRequest) -> Result<OptimizationSetup> {
    ensure!(!request.id.is_nil(), "Setup retry ID cannot be nil.");
    request.inputs.validate()?;
    let workspace = open_workspace(folder, false).await?;
    ensure!(
        request.inputs.project_id == workspace.manifest.id,
        "Setup belongs to another project."
    );
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    // An exact old retry reads its original record, never reactivating it.
    let history = load(&mut database, &workspace).await?;
    if let Some(existing) = history.iter().find(|s| s.id == request.id) {
        ensure!(
            existing.inputs == request.inputs
                && existing.parent.as_ref().map(|p| p.id.parse()).transpose()?
                    == request.expected_parent,
            "Setup retry ID already contains different inputs or predecessor."
        );
        return Ok(existing.clone());
    }
    // A new selection verifies custody bytes. It still grants no right to train.
    let verified = open_workspace(folder, true).await?;
    let (resolved, _, _) = resolve(
        &verified,
        request.inputs.model.id.parse()?,
        request.inputs.dataset.id,
        request.inputs.benchmark.id.parse()?,
    )
    .await?;
    ensure!(
        resolved == request.inputs,
        "Optimization inputs changed since preview. Review them again."
    );
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let history = load(&mut transaction, &verified).await?;
    // Another process may have saved the same request during file verification.
    if let Some(existing) = history.iter().find(|s| s.id == request.id) {
        ensure!(
            existing.inputs == request.inputs
                && existing.parent.as_ref().map(|p| p.id.parse()).transpose()?
                    == request.expected_parent,
            "Setup retry ID already contains different inputs or predecessor."
        );
        return Ok(existing.clone());
    }
    let active: String = sqlx::query_scalar(
        "SELECT active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        active == request.inputs.baseline_revision.id,
        "Baseline changed since preview. Select the current baseline."
    );
    ensure!(
        history.last().map(|s| s.id) == request.expected_parent,
        "Optimization setup changed. Reload before saving."
    );
    let setup = OptimizationSetup::create(request.id, history.last(), request.inputs, Utc::now())?;
    sqlx::query("INSERT INTO optimization_setups (id, project_id, number, parent_id, baseline_revision_id, model_id, dataset_version_id, benchmark_version_id, fingerprint, metadata_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(setup.id.to_string()).bind(setup.inputs.project_id.to_string()).bind(i64::try_from(setup.number)?)
        .bind(setup.parent.as_ref().map(|p| &p.id)).bind(&setup.inputs.baseline_revision.id).bind(&setup.inputs.model.id)
        .bind(setup.inputs.dataset.id.to_string()).bind(&setup.inputs.benchmark.id).bind(&setup.fingerprint).bind(serde_json::to_string(&setup)?)
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(setup)
}
