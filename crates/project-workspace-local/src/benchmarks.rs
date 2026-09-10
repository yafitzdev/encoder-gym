//! Append-only project catalog for experiment-owned benchmark definitions.
use crate::{connect, open_workspace};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{ProjectBenchmarkVersion, ScientificBinding};
use sqlx::{Connection, Row, SqliteConnection};
use std::{collections::BTreeSet, path::Path};
use uuid::Uuid;

pub async fn list(folder: &Path) -> Result<Vec<ProjectBenchmarkVersion>> {
    Ok(open_workspace(folder, false).await?.benchmark_versions)
}

pub async fn inspect(
    folder: &Path,
    id: Uuid,
) -> Result<(ProjectBenchmarkVersion, ScientificBinding)> {
    let workspace = open_workspace(folder, false).await?;
    let version = workspace
        .benchmark_versions
        .into_iter()
        .find(|v| v.id == id)
        .context("Benchmark version is missing.")?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let json: String =
        sqlx::query_scalar("SELECT binding_json FROM scientific_bindings WHERE id=?")
            .bind(&version.source.scientific_binding.id)
            .fetch_one(&mut database)
            .await?;
    database.close().await?;
    Ok((version, serde_json::from_str(&json)?))
}

pub(crate) async fn load(
    database: &mut SqliteConnection,
    project_id: Uuid,
) -> Result<Vec<ProjectBenchmarkVersion>> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_benchmark_versions'")
        .fetch_one(&mut *database).await? == 0 { return Ok(vec![]); }
    let rows = sqlx::query("SELECT id, project_id, number, parent_id, definition_fingerprint, source_binding_id, fingerprint, metadata_json FROM project_benchmark_versions ORDER BY number")
        .fetch_all(&mut *database).await?;
    let mut versions: Vec<ProjectBenchmarkVersion> = vec![];
    let mut definitions = BTreeSet::new();
    for row in rows {
        let version: ProjectBenchmarkVersion =
            serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?;
        version.validate(versions.last())?;
        ensure!(
            version.project_id == project_id
                && row.try_get::<String, _>("project_id")? == project_id.to_string()
                && row.try_get::<String, _>("id")? == version.id.to_string()
                && u64::try_from(row.try_get::<i64, _>("number")?)? == version.number
                && row.try_get::<Option<String>, _>("parent_id")?
                    == version.parent.as_ref().map(|p| p.id.clone())
                && row.try_get::<String, _>("definition_fingerprint")?
                    == version.definition.fingerprint
                && row.try_get::<String, _>("source_binding_id")?
                    == version.source.scientific_binding.id
                && row.try_get::<String, _>("fingerprint")? == version.fingerprint
                && definitions.insert(version.definition.fingerprint.clone()),
            "Benchmark catalog metadata or history changed."
        );
        let binding_json: String =
            sqlx::query_scalar("SELECT binding_json FROM scientific_bindings WHERE id=?")
                .bind(&version.source.scientific_binding.id)
                .fetch_optional(&mut *database)
                .await?
                .context("Benchmark source binding is missing.")?;
        let binding: ScientificBinding = serde_json::from_str(&binding_json)?;
        binding.validate()?;
        ensure!(
            binding.project_id == project_id
                && binding.id.to_string() == version.source.scientific_binding.id
                && binding.fingerprint == version.source.scientific_binding.fingerprint
                && binding.runtime.project_snapshot == version.source.project_snapshot
                && binding.adapter.key == version.definition.backend.name
                && binding.adapter.protocol == version.definition.backend.protocol_version
                && binding.adapter.configuration_fingerprint
                    == version.definition.backend.configuration_fingerprint,
            "Benchmark source belongs to another project or binding."
        );
        versions.push(version);
    }
    Ok(versions)
}

/// The caller verifies the scientific protocol and adapter-normalized definition.
/// This transaction preserves project custody, the version chain and retry IDs.
pub async fn record(
    folder: &Path,
    id: Uuid,
    expected_parent: Option<Uuid>,
    definition: encoder_experiment_core::benchmark::BenchmarkDefinition,
    source: project_workspace_core::BenchmarkSource,
) -> Result<ProjectBenchmarkVersion> {
    ensure!(!id.is_nil(), "Benchmark version ID must not be nil.");
    definition.validate_integrity()?;
    source.validate()?;
    let workspace = open_workspace(folder, false).await?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect an evaluation runtime first.")?;
    ensure!(
        source.scientific_binding.id == binding.id.to_string()
            && source.scientific_binding.fingerprint == binding.fingerprint
            && source.project_snapshot == binding.runtime.project_snapshot
            && binding.adapter.key == definition.backend.name
            && binding.adapter.protocol == definition.backend.protocol_version
            && binding.adapter.configuration_fingerprint
                == definition.backend.configuration_fingerprint,
        "Benchmark source does not match this project's current scientific binding."
    );
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let active: String = sqlx::query_scalar(
        "SELECT active_binding_id FROM scientific_binding_state WHERE singleton=1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        active == source.scientific_binding.id,
        "Scientific binding changed. Reload before recording the benchmark."
    );
    let versions = load(&mut transaction, workspace.manifest.id).await?;
    if let Some(existing) = versions.iter().find(|version| version.id == id) {
        ensure!(
            existing.definition == definition,
            "Benchmark version ID already has different contents."
        );
        return Ok(existing.clone());
    }
    if let Some(existing) = versions
        .iter()
        .find(|version| version.definition.fingerprint == definition.fingerprint)
    {
        ensure!(
            existing.definition == definition,
            "Conflicting benchmark definitions."
        );
        return Ok(existing.clone());
    }
    ensure!(
        versions.last().map(|v| v.id) == expected_parent,
        "Benchmark changed. Reload before adding a version."
    );
    let version = ProjectBenchmarkVersion::create(
        id,
        workspace.manifest.id,
        versions.last(),
        definition,
        source,
        Utc::now(),
    )?;
    sqlx::query("INSERT INTO project_benchmark_versions (id, project_id, number, parent_id, definition_fingerprint, source_binding_id, fingerprint, metadata_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(version.id.to_string()).bind(version.project_id.to_string()).bind(i64::try_from(version.number)?)
        .bind(version.parent.as_ref().map(|p| &p.id)).bind(&version.definition.fingerprint)
        .bind(&version.source.scientific_binding.id).bind(&version.fingerprint).bind(serde_json::to_string(&version)?)
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(version)
}
