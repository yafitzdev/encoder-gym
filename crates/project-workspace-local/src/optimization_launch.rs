//! Durable finite authority for the exact inputs selected by the user.
use crate::{
    ManagedWorkspace, connect, load_provider_catalog_history, open_workspace, optimization_setup,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    OptimizationAgentSettings, OptimizationLaunchAuthorization, OptimizationLaunchScope,
    OptimizationSetup, ProviderCatalog,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPreview {
    pub scope: OptimizationLaunchScope,
    pub model_name: String,
    pub dataset_rows: u64,
    pub benchmark_number: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchRequest {
    pub id: Uuid,
    pub scope: OptimizationLaunchScope,
}

pub async fn preview(folder: &Path, setup_id: Uuid) -> Result<LaunchPreview> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let setups = optimization_setup::load(&mut database, &workspace).await?;
    let setup = current_setup(&setups, setup_id)?;
    let providers = workspace
        .provider_catalog
        .as_ref()
        .context("Configure providers before optimizing.")?;
    let preview = resolved_preview(&workspace, &mut database, setup, providers).await?;
    database.close().await?;
    Ok(preview)
}

/// Read-only preview of new bounded agent-loop settings. Does not dispatch work.
pub async fn preview_agentic(
    folder: &Path,
    setup_id: Uuid,
    settings: OptimizationAgentSettings,
) -> Result<LaunchPreview> {
    settings.validate()?;
    let mut value = preview(folder, setup_id).await?;
    value.scope = value.scope.with_agentic_settings(settings)?;
    Ok(value)
}

pub async fn list(folder: &Path) -> Result<Vec<OptimizationLaunchAuthorization>> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let setups = optimization_setup::load(&mut database, &workspace).await?;
    let providers = load_provider_catalog_history(&mut database, &workspace.manifest).await?;
    let launches = load(&mut database, &workspace, &setups, &providers).await?;
    database.close().await?;
    Ok(launches)
}

pub async fn authorize(
    folder: &Path,
    request: LaunchRequest,
    authorized_by: &str,
) -> Result<OptimizationLaunchAuthorization> {
    ensure!(!request.id.is_nil(), "Launch retry ID cannot be nil.");
    let workspace = open_workspace(folder, false).await?;
    ensure!(
        request.scope.project_id == workspace.manifest.id,
        "Launch belongs to another project."
    );
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let setups = optimization_setup::load(&mut database, &workspace).await?;
    let providers = load_provider_catalog_history(&mut database, &workspace.manifest).await?;
    let launches = load(&mut database, &workspace, &setups, &providers).await?;
    if let Some(existing) = launches.iter().find(|launch| launch.id == request.id) {
        ensure!(
            existing.scope == request.scope && existing.authorized_by == authorized_by,
            "Launch retry ID already contains another scope or authorizer."
        );
        return Ok(existing.clone());
    }

    // A new authorization verifies every managed artifact byte before it can
    // become execution authority. No secret is read here.
    let verified = open_workspace(folder, true).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &verified).await?;
    let setup = current_setup(&setups, request.scope.setup.id.parse()?)?;
    let providers = load_provider_catalog_history(&mut transaction, &verified.manifest).await?;
    let provider = providers
        .last()
        .context("Configure providers before optimizing.")?;
    let launches = load(&mut transaction, &verified, &setups, &providers).await?;
    if let Some(existing) = launches.iter().find(|launch| launch.id == request.id) {
        ensure!(
            existing.scope == request.scope && existing.authorized_by == authorized_by,
            "Launch retry ID already contains another scope or authorizer."
        );
        return Ok(existing.clone());
    }
    let active_baseline: String = sqlx::query_scalar(
        "SELECT active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        active_baseline == setup.inputs.baseline_revision.id,
        "Baseline changed. Review Optimize inputs again."
    );
    let mut expected = resolved_preview(&verified, &mut transaction, setup, provider)
        .await?
        .scope;
    if let Some(settings) = &request.scope.agentic {
        expected = expected.with_agentic_settings(settings.clone())?;
    }
    ensure!(
        expected == request.scope,
        "Optimization launch changed since preview. Review it again."
    );
    let authorization = OptimizationLaunchAuthorization::create(
        request.id,
        request.scope,
        authorized_by,
        Utc::now(),
    )?;
    sqlx::query("INSERT INTO optimization_launch_authorizations (id, project_id, setup_id, provider_catalog_id, scope_fingerprint, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(authorization.id.to_string()).bind(authorization.scope.project_id.to_string())
        .bind(&authorization.scope.setup.id).bind(&authorization.scope.provider_catalog.id)
        .bind(&authorization.scope.fingerprint).bind(&authorization.fingerprint)
        .bind(serde_json::to_string(&authorization)?).bind(authorization.created_at.to_rfc3339())
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(authorization)
}

async fn resolved_preview(
    workspace: &ManagedWorkspace,
    database: &mut SqliteConnection,
    setup: &OptimizationSetup,
    providers: &ProviderCatalog,
) -> Result<LaunchPreview> {
    let benchmark = workspace
        .benchmark_versions
        .iter()
        .find(|version| version.id.to_string() == setup.inputs.benchmark.id)
        .context("Selected benchmark version is missing.")?;
    let dataset = crate::dataset_versions::load_reference(
        database,
        workspace.manifest.id,
        setup.inputs.dataset.id,
    )
    .await?;
    ensure!(
        dataset == setup.inputs.dataset,
        "Selected dataset version changed."
    );
    let rows: i64 = sqlx::query_scalar(
        "SELECT json_array_length(metadata_json, '$.members') FROM dataset_versions WHERE id=?",
    )
    .bind(setup.inputs.dataset.id.to_string())
    .fetch_one(&mut *database)
    .await?;
    let model = workspace
        .model_catalog
        .as_ref()
        .context("Model catalog is missing.")?
        .artifacts
        .iter()
        .find(|model| model.id.to_string() == setup.inputs.model.id)
        .context("Selected model is missing.")?;
    Ok(LaunchPreview {
        scope: OptimizationLaunchScope::bind(setup, providers, benchmark)?,
        model_name: model.name.clone(),
        dataset_rows: u64::try_from(rows)?,
        benchmark_number: benchmark.number,
    })
}

fn current_setup(setups: &[OptimizationSetup], setup_id: Uuid) -> Result<&OptimizationSetup> {
    let setup = setups
        .iter()
        .find(|setup| setup.id == setup_id)
        .context("Optimization setup is missing.")?;
    ensure!(
        setups.last().is_some_and(|current| current.id == setup_id),
        "Choose the current Optimization setup."
    );
    Ok(setup)
}

pub(crate) async fn load(
    database: &mut SqliteConnection,
    workspace: &ManagedWorkspace,
    setups: &[OptimizationSetup],
    providers: &[ProviderCatalog],
) -> Result<Vec<OptimizationLaunchAuthorization>> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_launch_authorizations'")
        .fetch_one(&mut *database).await? == 0 { return Ok(vec![]); }
    let rows = sqlx::query("SELECT id, project_id, setup_id, provider_catalog_id, scope_fingerprint, fingerprint, metadata_json, created_at FROM optimization_launch_authorizations ORDER BY created_at, id")
        .fetch_all(&mut *database).await?;
    let mut launches = Vec::new();
    for row in rows {
        let launch: OptimizationLaunchAuthorization =
            serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?;
        let setup = setups
            .iter()
            .find(|setup| setup.id.to_string() == launch.scope.setup.id)
            .context("Launch setup is missing.")?;
        let provider = providers
            .iter()
            .find(|provider| provider.id.to_string() == launch.scope.provider_catalog.id)
            .context("Launch provider settings are missing.")?;
        let benchmark = workspace
            .benchmark_versions
            .iter()
            .find(|version| version.id.to_string() == setup.inputs.benchmark.id)
            .context("Launch benchmark is missing.")?;
        launch.validate(setup, provider, benchmark)?;
        ensure!(
            row.try_get::<String, _>("id")? == launch.id.to_string()
                && row.try_get::<String, _>("project_id")? == launch.scope.project_id.to_string()
                && row.try_get::<String, _>("setup_id")? == launch.scope.setup.id
                && row.try_get::<String, _>("provider_catalog_id")?
                    == launch.scope.provider_catalog.id
                && row.try_get::<String, _>("scope_fingerprint")? == launch.scope.fingerprint
                && row.try_get::<String, _>("fingerprint")? == launch.fingerprint
                && row.try_get::<String, _>("created_at")? == launch.created_at.to_rfc3339(),
            "Optimization launch storage changed."
        );
        launches.push(launch);
    }
    Ok(launches)
}
