//! Local project custody. No training execution or dataset split decisions.
mod activity;
pub mod benchmarks;
pub mod dataset_versions;
mod datasets;
mod files;
mod model;
pub mod model_datasets;
mod model_registration;
pub mod optimization_setup;

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    BASELINE, BaselineChange, BaselineRevision, BoundIdentity, DATABASE, DIRECTORIES,
    DatasetImport, LocalModel, MANIFEST, ModelArtifact, ModelCatalog, ModelOrigin, ProjectManifest,
    ProviderCatalog, ScientificBinding, validate_name,
};
use serde::Serialize;
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

pub use activity::{
    ActivityInitialization, AppendActivity, append_activity, export_activity, initialize_activity,
    read_action, read_activity,
};
pub use datasets::{DatasetPreview, backfill_nomos, import_dataset, inspect_dataset};
use files::{canonical_plain, contained, copy_verified, hash, json, plain, write_new};
pub use model::inspect_model;
pub use model_registration::{CompletedModelRegistration, register_completed_model};

#[derive(Debug, Clone)]
pub struct AcceptedModelPromotion {
    pub expected_baseline_revision_id: Uuid,
    pub name: String,
    pub source_model: BoundIdentity,
    pub source_model_format: String,
    pub source_model_bytes: u64,
    pub producing_run: BoundIdentity,
    pub training_snapshot: BoundIdentity,
    pub trainer: BoundIdentity,
    pub effective_configuration_fingerprint: String,
    pub source_revision: String,
    pub decision_id: String,
    pub decision_fingerprint: String,
    pub actor: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorkspace {
    pub folder: String,
    pub manifest: ProjectManifest,
    pub datasets: Vec<DatasetImport>,
    pub model_dataset_links: Vec<project_workspace_core::ModelDatasetLink>,
    pub benchmark_versions: Vec<project_workspace_core::ProjectBenchmarkVersion>,
    /// Absent only when an older workspace requires an explicit registry upgrade.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_catalog: Option<ModelCatalog>,
    /// The latest explicit slice-store/runtime binding, if one has been configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scientific_binding: Option<ScientificBinding>,
    /// Latest non-secret provider settings revision, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_catalog: Option<ProviderCatalog>,
    pub verified: bool,
}

/// The expected fingerprint binds confirmation to the exact previewed checkpoint.
pub async fn create_workspace(
    destination: &Path,
    name: &str,
    model_source: &Path,
    expected_fingerprint: &str,
    task: Option<String>,
) -> Result<ManagedWorkspace> {
    validate_name(name)?;
    if let Some(task) = &task {
        validate_name(task)?;
    }
    let destination = std::path::absolute(destination)?;
    let parent = canonical_plain(
        destination
            .parent()
            .context("Choose a project destination with an existing parent.")?,
    )?;
    let leaf = destination
        .file_name()
        .context("Choose a new project folder, not a filesystem root.")?;
    project_workspace_core::validate_relative(
        leaf.to_str().context("Folder name must be UTF-8.")?,
    )?;
    let destination = parent.join(leaf);
    ensure!(
        !destination.try_exists()?,
        "Destination already exists. Choose a new folder, or use Open project."
    );
    let baseline = inspect_model(model_source)?;
    ensure!(
        baseline.fingerprint == expected_fingerprint,
        "The checkpoint changed after preview. Inspect it again before creating the project."
    );
    let manifest = ProjectManifest {
        version: 1,
        id: Uuid::new_v4(),
        name: name.trim().into(),
        created_at: Utc::now(),
        task,
        baseline,
    };
    manifest.validate()?;
    let staging = tempfile::Builder::new()
        .prefix(".encoder-gym-create-")
        .tempdir_in(&parent)?;
    for directory in DIRECTORIES {
        fs::create_dir_all(staging.path().join(directory))?;
    }
    copy_model(&manifest.baseline, &staging.path().join(BASELINE))?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    write_new(&staging.path().join(MANIFEST), &manifest_bytes)?;
    let mut database = connect(staging.path(), false, true).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    sqlx::query("INSERT INTO workspace_identity VALUES (1, ?, ?)")
        .bind(manifest.id.to_string())
        .bind(hash(&staging.path().join(MANIFEST), MANIFEST)?.fingerprint)
        .execute(&mut database)
        .await?;
    initialize_model_catalog(&mut database, &manifest).await?;
    database.close().await?;
    // Reserve an absent destination without replacing an existing file or folder.
    // Publish the manifest last: an interrupted publication is never openable.
    fs::create_dir(&destination)
        .context("Could not reserve a new project folder; nothing was overwritten.")?;
    for name in [
        "models",
        "datasets",
        "runs",
        "evaluations",
        DATABASE,
        MANIFEST,
    ] {
        fs::rename(staging.path().join(name), destination.join(name)).with_context(|| {
            format!(
                "Project publication interrupted at {name}. Incomplete folder: {}",
                destination.display()
            )
        })?;
    }
    open_workspace(&destination, true).await
}

fn copy_model(model: &LocalModel, destination: &Path) -> Result<()> {
    let source = canonical_plain(Path::new(&model.source))?;
    for file in &model.files {
        let target = destination.join(&file.path);
        fs::create_dir_all(target.parent().context("Model file has no parent.")?)?;
        copy_verified(&contained(&source, &file.path)?, &target, file)?;
    }
    // Some supported modules (Normalize) deliberately have an empty directory.
    if model.format == "sentence-transformers" {
        let modules: Vec<serde_json::Value> = json(&source.join("modules.json"))?;
        for module in modules {
            if let Some(path) = module
                .get("path")
                .and_then(serde_json::Value::as_str)
                .filter(|p| !p.is_empty())
            {
                project_workspace_core::validate_relative(path)?;
                fs::create_dir_all(destination.join(path))?;
            }
        }
    }
    ensure!(
        inspect_model(&source)?.fingerprint == model.fingerprint,
        "Checkpoint inventory changed during import."
    );
    Ok(())
}

pub async fn open_workspace(folder: &Path, verify: bool) -> Result<ManagedWorkspace> {
    let root = canonical_plain(folder)?;
    let manifest_path = contained(&root, MANIFEST).context("This is not an Encoder Gym workspace. Create a new project from a local checkpoint instead.")?;
    let manifest: ProjectManifest = json(&manifest_path)?;
    manifest.validate()?;
    for directory in DIRECTORIES {
        ensure!(
            plain(&contained(&root, directory)?)?.is_dir(),
            "Managed artifact directory is missing: {directory}"
        );
    }
    contained(&root, DATABASE)?;
    let mut database = connect(&root, true, false).await?;
    let binding =
        sqlx::query("SELECT project_id, manifest_sha256 FROM workspace_identity WHERE singleton=1")
            .fetch_one(&mut database)
            .await
            .context("Invalid workspace database.")?;
    ensure!(
        binding.get::<String, _>("project_id") == manifest.id.to_string()
            && binding.get::<String, _>("manifest_sha256")
                == hash(&manifest_path, MANIFEST)?.fingerprint,
        "Project manifest and database do not match. Restore the matching project files."
    );
    let mut datasets = vec![];
    for row in
        sqlx::query("SELECT id, content_sha256, metadata_json FROM dataset_imports ORDER BY rowid")
            .fetch_all(&mut database)
            .await?
    {
        let dataset: DatasetImport = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        dataset.validate()?;
        ensure!(
            row.get::<String, _>("id") == dataset.id.to_string()
                && row.get::<String, _>("content_sha256") == dataset.artifact.fingerprint,
            "Dataset identity does not match its database key."
        );
        if let Some(provenance) = &dataset.training_source {
            ensure!(
                provenance.baseline_fingerprint == manifest.baseline.fingerprint
                    && manifest
                        .baseline
                        .files
                        .iter()
                        .any(|file| file.path == "nomos_training_manifest.json"
                            && file.fingerprint == provenance.manifest_fingerprint),
                "Dataset training provenance does not belong to this baseline."
            );
        }
        verify_file(&root, &dataset.artifact, verify)?;
        if verify {
            let preview =
                inspect_dataset(&contained(&root, &dataset.artifact.path)?, dataset.purpose)?;
            ensure!(
                preview.rows == dataset.rows,
                "Imported dataset row count changed."
            );
            if let Some(provenance) = &dataset.training_source {
                let native: serde_json::Value = json(&contained(
                    &root,
                    &format!("{BASELINE}/nomos_training_manifest.json"),
                )?)?;
                let named_input = native.get("inputs").and_then(serde_json::Value::as_array)
                    .and_then(|inputs| inputs.iter().filter_map(serde_json::Value::as_str).find(|input| input.replace('\\', "/") == provenance.input))
                    .context("Imported training provenance names an input absent from the baseline manifest.")?;
                ensure!(
                    native
                        .get("input_state_counts")
                        .and_then(|counts| counts.get(named_input))
                        .and_then(serde_json::Value::as_u64)
                        == Some(dataset.rows)
                        && preview.native_nomos_training_rows == dataset.rows,
                    "Imported data no longer matches its native training-manifest provenance."
                );
            }
        }
        datasets.push(dataset);
    }
    let model_catalog = load_model_catalog(&mut database, &manifest).await?;
    let scientific_binding = load_scientific_bindings(&mut database, &manifest)
        .await?
        .pop();
    let provider_catalog = load_provider_catalog(&mut database, &manifest).await?;
    let benchmark_versions = benchmarks::load(&mut database, manifest.id).await?;
    let model_dataset_links =
        model_datasets::load_links(&mut database, &root, model_catalog.as_ref(), &datasets).await?;
    database.close().await?;
    for file in &manifest.baseline.files {
        let mut identity = file.clone();
        identity.path = format!("{BASELINE}/{}", file.path);
        verify_file(&root, &identity, verify)?;
    }
    if verify {
        ensure!(
            inspect_model(&root.join(BASELINE))?.fingerprint == manifest.baseline.fingerprint,
            "Managed baseline inventory changed."
        );
        if let Some(catalog) = &model_catalog {
            for artifact in &catalog.artifacts {
                let inspected = inspect_model(&contained(&root, &artifact.path)?)?;
                ensure!(
                    inspected.format == artifact.format
                        && inspected.bytes == artifact.bytes
                        && inspected.fingerprint == artifact.fingerprint,
                    "Managed model artifact changed: {}",
                    artifact.path
                );
            }
        }
    }
    let workspace = ManagedWorkspace {
        folder: root.to_string_lossy().into_owned(),
        manifest,
        datasets,
        model_dataset_links,
        benchmark_versions,
        model_catalog,
        scientific_binding,
        provider_catalog,
        verified: verify,
    };
    if verify {
        model_datasets::verify_members(&workspace).await?;
    }
    Ok(workspace)
}

/// Append and activate one non-secret provider-settings revision.
pub async fn record_provider_catalog(
    folder: &Path,
    catalog: ProviderCatalog,
    expected_active_revision_id: Option<Uuid>,
) -> Result<ManagedWorkspace> {
    catalog.validate()?;
    let workspace = open_workspace(folder, false).await?;
    ensure!(
        catalog.project_id == workspace.manifest.id,
        "Provider settings belong to another managed project."
    );
    let root = Path::new(&workspace.folder);
    let expected_sequence = workspace
        .provider_catalog
        .as_ref()
        .map_or(1, |value| value.sequence + 1);
    let mut database = connect(root, false, false).await?;
    ensure!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='provider_catalog_state'",
        )
        .fetch_one(&mut database)
        .await?
            == 1,
        "Upgrade this managed workspace before configuring providers."
    );
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let current = sqlx::query_scalar::<_, String>(
        "SELECT active_revision_id FROM provider_catalog_state WHERE singleton=1",
    )
    .fetch_optional(&mut *transaction)
    .await?
    .map(|value| Uuid::parse_str(&value))
    .transpose()?;
    if let Some(row) = sqlx::query("SELECT catalog_json FROM provider_catalog_revisions WHERE id=?")
        .bind(catalog.id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
    {
        let existing: ProviderCatalog =
            serde_json::from_str(&row.get::<String, _>("catalog_json"))?;
        ensure!(
            existing == catalog && current == Some(catalog.id),
            "Provider settings identity already exists with different or inactive state."
        );
        transaction.rollback().await?;
        database.close().await?;
        return open_workspace(root, false).await;
    }
    ensure!(
        current == expected_active_revision_id
            && catalog.previous_revision_id == current
            && catalog.sequence == expected_sequence,
        "Provider settings changed after inspection. Reload before saving again."
    );
    sqlx::query(
        "INSERT INTO provider_catalog_revisions \
         (id, project_id, sequence, previous_revision_id, fingerprint, catalog_json) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(catalog.id.to_string())
    .bind(catalog.project_id.to_string())
    .bind(i64::try_from(catalog.sequence)?)
    .bind(catalog.previous_revision_id.map(|value| value.to_string()))
    .bind(&catalog.fingerprint)
    .bind(serde_json::to_string(&catalog)?)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO provider_catalog_state (singleton, project_id, active_revision_id) \
         VALUES (1, ?, ?) ON CONFLICT(singleton) DO UPDATE SET active_revision_id=excluded.active_revision_id",
    )
    .bind(catalog.project_id.to_string())
    .bind(catalog.id.to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    database.close().await?;
    // This mutation changes only the append-only provider catalog. Do not
    // rehash unrelated model and dataset artifacts before acknowledging it.
    open_workspace(root, false).await
}

/// Append and activate a verified scientific runtime/store binding.
///
/// Adapter-specific verification must happen before this boundary. The
/// compare-and-append requirement prevents stale project views from replacing
/// a newer binding.
pub async fn record_scientific_binding(
    folder: &Path,
    binding: ScientificBinding,
    expected_active_binding_id: Option<Uuid>,
) -> Result<ManagedWorkspace> {
    binding.validate()?;
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Upgrade this managed workspace before configuring execution.")?;
    ensure!(
        binding.project_id == workspace.manifest.id
            && binding.baseline_revision_id == catalog.active_baseline_revision_id,
        "Scientific binding must target this project's current baseline revision."
    );
    let root = Path::new(&workspace.folder);
    let mut database = connect(root, false, false).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let current = sqlx::query_scalar::<_, String>(
        "SELECT active_binding_id FROM scientific_binding_state WHERE singleton=1",
    )
    .fetch_optional(&mut *transaction)
    .await?
    .map(|value| Uuid::parse_str(&value))
    .transpose()?;
    if let Some(row) = sqlx::query("SELECT binding_json FROM scientific_bindings WHERE id = ?")
        .bind(binding.id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
    {
        let existing: ScientificBinding =
            serde_json::from_str(&row.get::<String, _>("binding_json"))?;
        ensure!(
            existing == binding && current == Some(binding.id),
            "Scientific binding identity already exists with different or inactive state."
        );
        transaction.rollback().await?;
        database.close().await?;
        return open_workspace(root, true).await;
    }
    ensure!(
        current == expected_active_binding_id && binding.previous_binding_id == current,
        "Scientific binding changed after it was inspected. Reload before binding again."
    );
    sqlx::query(
        "INSERT INTO scientific_bindings \
         (id, project_id, baseline_revision_id, previous_binding_id, specification_fingerprint, fingerprint, binding_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(binding.id.to_string())
    .bind(binding.project_id.to_string())
    .bind(binding.baseline_revision_id.to_string())
    .bind(binding.previous_binding_id.map(|value| value.to_string()))
    .bind(&binding.specification_fingerprint)
    .bind(&binding.fingerprint)
    .bind(serde_json::to_string(&binding)?)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO scientific_binding_state (singleton, project_id, active_binding_id) \
         VALUES (1, ?, ?) ON CONFLICT(singleton) DO UPDATE SET active_binding_id=excluded.active_binding_id",
    )
    .bind(binding.project_id.to_string())
    .bind(binding.id.to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    database.close().await?;
    open_workspace(root, true).await
}

/// Copy one scientifically accepted checkpoint into managed custody and
/// atomically advance the active baseline revision.
///
/// The caller owns acceptance-policy validation. This boundary independently
/// verifies the source as a supported local encoder, publishes it under a
/// content-addressed managed path, and compare-and-appends the catalog change.
pub async fn record_accepted_model_promotion(
    folder: &Path,
    source: &Path,
    request: AcceptedModelPromotion,
) -> Result<ManagedWorkspace> {
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Upgrade this managed workspace before promoting a model.")?;
    if let Some(existing) = catalog.baseline_revisions.iter().find(|revision| {
        matches!(
            &revision.change,
            BaselineChange::Promotion { decision_id, .. } if decision_id == &request.decision_id
        )
    }) {
        ensure!(
            existing.id == catalog.active_baseline_revision_id
                && matches!(
                    &existing.change,
                    BaselineChange::Promotion { decision_fingerprint, .. }
                        if decision_fingerprint == &request.decision_fingerprint
                ),
            "This promotion decision was already recorded with different or historical baseline state."
        );
        return open_workspace(Path::new(&workspace.folder), true).await;
    }
    ensure!(
        catalog.active_baseline_revision_id == request.expected_baseline_revision_id,
        "The active baseline changed after this candidate was accepted. Reload before promoting."
    );
    validate_name(&request.name)?;
    validate_name(&request.actor)?;
    validate_name(&request.reason)?;
    request.source_model.validate("Accepted source model")?;
    request.producing_run.validate("Producing run")?;
    request.training_snapshot.validate("Training snapshot")?;
    request.trainer.validate("Trainer")?;
    project_workspace_core::validate_hash(&request.effective_configuration_fingerprint)?;
    project_workspace_core::validate_hash(&request.decision_fingerprint)?;
    let source_model = inspect_model(source)?;
    ensure!(
        source_model.format == request.source_model_format
            && source_model.bytes == request.source_model_bytes,
        "The accepted scientific model format or size differs from the selected checkpoint."
    );
    let digest = source_model
        .fingerprint
        .strip_prefix("sha256:")
        .context("Managed model inventory has no SHA-256 identity.")?;
    let relative = format!("models/candidates/{digest}");
    let root = Path::new(&workspace.folder);
    publish_model_copy(&source_model, root, &relative)?;
    let mut artifact = ModelArtifact::trained(
        workspace.manifest.id,
        Uuid::new_v4(),
        request.name,
        relative,
        &source_model,
        catalog.active_model().id,
        request.producing_run,
        request.source_model,
        request.training_snapshot,
        request.trainer,
        request.effective_configuration_fingerprint,
        request.source_revision,
        Utc::now(),
    )?;
    if let Some(existing) = catalog.artifacts.iter().find(|value| {
        value.source_model == artifact.source_model
            && value.producing_run.as_ref().map(|value| &value.id)
                == artifact.producing_run.as_ref().map(|value| &value.id)
    }) {
        ensure!(
            existing.fingerprint == artifact.fingerprint
                && existing.parent_model_id == artifact.parent_model_id
                && existing.training_snapshot == artifact.training_snapshot
                && existing.trainer == artifact.trainer
                && existing.effective_configuration_fingerprint
                    == artifact.effective_configuration_fingerprint
                && existing.source_revision == artifact.source_revision,
            "The accepted model differs from its registered output."
        );
        artifact = existing.clone();
    }
    let artifact_id = artifact.id;
    let next = catalog.with_promotion(
        artifact,
        Uuid::new_v4(),
        request.decision_id,
        request.decision_fingerprint,
        request.actor,
        request.reason,
        Utc::now(),
    )?;
    let artifact = next
        .artifacts
        .iter()
        .find(|value| value.id == artifact_id)
        .expect("promotion references an artifact");
    let revision = next
        .baseline_revisions
        .last()
        .expect("promotion appends a revision");
    let mut database = connect(root, false, false).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let current = Uuid::parse_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
        )
        .fetch_one(&mut *transaction)
        .await?,
    )?;
    ensure!(
        current == request.expected_baseline_revision_id,
        "The active baseline changed while the accepted model was being copied. Reload before promoting."
    );
    sqlx::query(
        "INSERT INTO model_artifacts \
         (id, project_id, content_fingerprint, origin, metadata_json) VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(artifact.id.to_string())
    .bind(artifact.project_id.to_string())
    .bind(&artifact.fingerprint)
    .bind(model_origin(artifact.origin))
    .bind(serde_json::to_string(artifact)?)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO baseline_revisions \
         (id, project_id, sequence, model_artifact_id, previous_revision_id, change_kind, fingerprint, metadata_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(revision.id.to_string())
    .bind(revision.project_id.to_string())
    .bind(i64::try_from(revision.sequence)?)
    .bind(revision.model_artifact_id.to_string())
    .bind(revision.previous_revision_id.map(|value| value.to_string()))
    .bind(baseline_change(&revision.change))
    .bind(&revision.fingerprint)
    .bind(serde_json::to_string(revision)?)
    .execute(&mut *transaction)
    .await?;
    let updated = sqlx::query(
        "UPDATE model_catalog_state SET active_baseline_revision_id=? \
         WHERE singleton=1 AND project_id=? AND active_baseline_revision_id=?",
    )
    .bind(revision.id.to_string())
    .bind(workspace.manifest.id.to_string())
    .bind(current.to_string())
    .execute(&mut *transaction)
    .await?;
    ensure!(
        updated.rows_affected() == 1,
        "The active baseline changed while promotion was committing."
    );
    transaction.commit().await?;
    database.close().await?;
    open_workspace(root, true).await
}

fn publish_model_copy(model: &LocalModel, root: &Path, relative: &str) -> Result<()> {
    project_workspace_core::validate_relative(relative)?;
    let unresolved = root.join(relative);
    let leaf = unresolved
        .file_name()
        .context("Accepted model path has no final component.")?;
    let parent = canonical_plain(
        unresolved
            .parent()
            .context("Accepted model path has no parent directory.")?,
    )?;
    ensure!(
        parent.starts_with(root),
        "Accepted model path escaped the managed workspace."
    );
    let target = parent.join(leaf);
    if target.exists() {
        plain(&target)?;
        let existing = inspect_model(&target)?;
        ensure!(
            existing.format == model.format
                && existing.bytes == model.bytes
                && existing.fingerprint == model.fingerprint,
            "A different model already occupies the accepted checkpoint path."
        );
        return Ok(());
    }
    let staging = tempfile::Builder::new()
        .prefix(".accepted-model-")
        .tempdir_in(parent)?;
    copy_model(model, staging.path())?;
    let copied = inspect_model(staging.path())?;
    ensure!(
        copied.format == model.format
            && copied.bytes == model.bytes
            && copied.fingerprint == model.fingerprint,
        "Accepted checkpoint changed while entering managed custody."
    );
    let staged = staging.keep();
    match fs::rename(&staged, &target) {
        Ok(()) => {}
        Err(error) if target.exists() => {
            let existing = inspect_model(&target)?;
            fs::remove_dir_all(&staged)?;
            ensure!(
                existing.format == model.format
                    && existing.bytes == model.bytes
                    && existing.fingerprint == model.fingerprint,
                "Another model occupied the accepted checkpoint path: {error}"
            );
        }
        Err(error) => {
            fs::remove_dir_all(&staged)?;
            return Err(error.into());
        }
    }
    Ok(())
}

/// Apply project-registry migrations and initialize the imported baseline catalog.
/// Scientific stores and the immutable manifest are not changed.
pub async fn upgrade_workspace(folder: &Path) -> Result<ManagedWorkspace> {
    let root = canonical_plain(folder)?;
    let manifest_path = contained(&root, MANIFEST)
        .context("This is not an Encoder Gym workspace. Open a managed project folder.")?;
    let manifest: ProjectManifest = json(&manifest_path)?;
    manifest.validate()?;
    let mut database = connect(&root, false, false).await?;
    let binding =
        sqlx::query("SELECT project_id, manifest_sha256 FROM workspace_identity WHERE singleton=1")
            .fetch_one(&mut database)
            .await
            .context("Invalid workspace database.")?;
    ensure!(
        binding.get::<String, _>("project_id") == manifest.id.to_string()
            && binding.get::<String, _>("manifest_sha256")
                == hash(&manifest_path, MANIFEST)?.fingerprint,
        "Project manifest and database do not match. Restore the matching project files."
    );
    sqlx::migrate!("./migrations").run(&mut database).await?;
    initialize_model_catalog(&mut database, &manifest).await?;
    database.close().await?;
    open_workspace(&root, true).await
}

async fn initialize_model_catalog(
    database: &mut SqliteConnection,
    manifest: &ProjectManifest,
) -> Result<()> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM model_catalog_state")
        .fetch_one(&mut *database)
        .await?
        > 0
    {
        return Ok(());
    }
    let artifact = ModelArtifact::imported_baseline(
        manifest.id,
        Uuid::new_v4(),
        format!("{} baseline", manifest.name),
        &manifest.baseline,
        manifest.created_at,
    )?;
    let catalog = ModelCatalog::initialize(
        manifest.id,
        artifact,
        Uuid::new_v4(),
        manifest.baseline.fingerprint.clone(),
        manifest.created_at,
    )?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let artifact = &catalog.artifacts[0];
    sqlx::query(
        "INSERT INTO model_artifacts \
         (id, project_id, content_fingerprint, origin, metadata_json) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(artifact.id.to_string())
    .bind(artifact.project_id.to_string())
    .bind(&artifact.fingerprint)
    .bind(model_origin(artifact.origin))
    .bind(serde_json::to_string(artifact)?)
    .execute(&mut *transaction)
    .await?;
    let revision = &catalog.baseline_revisions[0];
    sqlx::query(
        "INSERT INTO baseline_revisions \
         (id, project_id, sequence, model_artifact_id, previous_revision_id, change_kind, fingerprint, metadata_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(revision.id.to_string())
    .bind(revision.project_id.to_string())
    .bind(i64::try_from(revision.sequence)?)
    .bind(revision.model_artifact_id.to_string())
    .bind(Option::<String>::None)
    .bind(baseline_change(&revision.change))
    .bind(&revision.fingerprint)
    .bind(serde_json::to_string(revision)?)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO model_catalog_state \
         (singleton, project_id, active_baseline_revision_id) VALUES (1, ?, ?)",
    )
    .bind(manifest.id.to_string())
    .bind(revision.id.to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn load_model_catalog(
    database: &mut SqliteConnection,
    manifest: &ProjectManifest,
) -> Result<Option<ModelCatalog>> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_catalog_state'",
    )
    .fetch_one(&mut *database)
    .await?
        == 1;
    if !exists {
        return Ok(None);
    }
    let Some(state) = sqlx::query(
        "SELECT project_id, active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
    )
    .fetch_optional(&mut *database)
    .await?
    else {
        return Ok(None);
    };
    ensure!(
        state.get::<String, _>("project_id") == manifest.id.to_string(),
        "Model catalog belongs to another project."
    );
    let active_baseline_revision_id =
        Uuid::parse_str(&state.get::<String, _>("active_baseline_revision_id"))?;
    let mut artifacts = Vec::new();
    for row in sqlx::query(
        "SELECT id, project_id, content_fingerprint, origin, metadata_json \
         FROM model_artifacts ORDER BY rowid",
    )
    .fetch_all(&mut *database)
    .await?
    {
        let artifact: ModelArtifact = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        artifact.validate()?;
        ensure!(
            row.get::<String, _>("id") == artifact.id.to_string()
                && row.get::<String, _>("project_id") == artifact.project_id.to_string()
                && row.get::<String, _>("content_fingerprint") == artifact.fingerprint
                && row.get::<String, _>("origin") == model_origin(artifact.origin),
            "Model artifact projection does not match its immutable record."
        );
        artifacts.push(artifact);
    }
    let mut baseline_revisions = Vec::new();
    for row in sqlx::query(
        "SELECT id, project_id, sequence, model_artifact_id, previous_revision_id, \
         change_kind, fingerprint, metadata_json FROM baseline_revisions ORDER BY sequence",
    )
    .fetch_all(&mut *database)
    .await?
    {
        let revision: BaselineRevision =
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        revision.validate()?;
        let sequence = u64::try_from(row.get::<i64, _>("sequence"))?;
        let previous = row
            .get::<Option<String>, _>("previous_revision_id")
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        ensure!(
            row.get::<String, _>("id") == revision.id.to_string()
                && row.get::<String, _>("project_id") == revision.project_id.to_string()
                && sequence == revision.sequence
                && row.get::<String, _>("model_artifact_id")
                    == revision.model_artifact_id.to_string()
                && previous == revision.previous_revision_id
                && row.get::<String, _>("change_kind") == baseline_change(&revision.change)
                && row.get::<String, _>("fingerprint") == revision.fingerprint,
            "Baseline revision projection does not match its immutable record."
        );
        baseline_revisions.push(revision);
    }
    let catalog = ModelCatalog {
        project_id: manifest.id,
        artifacts,
        baseline_revisions,
        active_baseline_revision_id,
    };
    catalog.validate()?;
    let initial = &catalog.baseline_revisions[0];
    ensure!(
        catalog.artifacts.iter().any(|artifact| {
            artifact.id == initial.model_artifact_id
                && artifact.origin == ModelOrigin::Imported
                && artifact.path == BASELINE
                && artifact.fingerprint == manifest.baseline.fingerprint
                && matches!(
                    &initial.change,
                    BaselineChange::Initialization { source_fingerprint }
                        if source_fingerprint == &manifest.baseline.fingerprint
                )
        }),
        "Initial model catalog baseline does not match the immutable workspace manifest."
    );
    Ok(Some(catalog))
}

/// Verified append-only history; callers must still validate each bound store.
pub async fn scientific_binding_history(folder: &Path) -> Result<Vec<ScientificBinding>> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let bindings = load_scientific_bindings(&mut database, &workspace.manifest).await?;
    database.close().await?;
    Ok(bindings)
}

async fn load_scientific_bindings(
    database: &mut SqliteConnection,
    manifest: &ProjectManifest,
) -> Result<Vec<ScientificBinding>> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='scientific_binding_state'",
    )
    .fetch_one(&mut *database)
    .await?
        == 1;
    if !exists {
        return Ok(vec![]);
    }
    let Some(state) = sqlx::query(
        "SELECT project_id, active_binding_id FROM scientific_binding_state WHERE singleton=1",
    )
    .fetch_optional(&mut *database)
    .await?
    else {
        return Ok(vec![]);
    };
    ensure!(
        state.get::<String, _>("project_id") == manifest.id.to_string(),
        "Scientific binding belongs to another managed project."
    );
    let active_id = Uuid::parse_str(&state.get::<String, _>("active_binding_id"))?;
    let rows = sqlx::query(
        "SELECT id, project_id, baseline_revision_id, previous_binding_id, \
         specification_fingerprint, fingerprint, binding_json \
         FROM scientific_bindings ORDER BY rowid",
    )
    .fetch_all(&mut *database)
    .await?;
    let mut previous = None;
    let mut bindings = vec![];
    for row in rows {
        let binding: ScientificBinding =
            serde_json::from_str(&row.get::<String, _>("binding_json"))?;
        binding.validate()?;
        let row_previous = row
            .get::<Option<String>, _>("previous_binding_id")
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        ensure!(
            row.get::<String, _>("id") == binding.id.to_string()
                && row.get::<String, _>("project_id") == binding.project_id.to_string()
                && row.get::<String, _>("baseline_revision_id")
                    == binding.baseline_revision_id.to_string()
                && row_previous == binding.previous_binding_id
                && row.get::<String, _>("specification_fingerprint")
                    == binding.specification_fingerprint
                && row.get::<String, _>("fingerprint") == binding.fingerprint
                && binding.project_id == manifest.id
                && binding.previous_binding_id == previous,
            "Scientific binding history or normalized projection is invalid."
        );
        previous = Some(binding.id);
        bindings.push(binding);
    }
    ensure!(
        previous == Some(active_id),
        "The active scientific binding must be the latest append-only record."
    );
    Ok(bindings)
}

async fn load_provider_catalog(
    database: &mut SqliteConnection,
    manifest: &ProjectManifest,
) -> Result<Option<ProviderCatalog>> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='provider_catalog_state'",
    )
    .fetch_one(&mut *database)
    .await?
        == 1;
    if !exists {
        return Ok(None);
    }
    let Some(state) = sqlx::query(
        "SELECT project_id, active_revision_id FROM provider_catalog_state WHERE singleton=1",
    )
    .fetch_optional(&mut *database)
    .await?
    else {
        return Ok(None);
    };
    ensure!(
        state.get::<String, _>("project_id") == manifest.id.to_string(),
        "Provider settings belong to another managed project."
    );
    let active_id = Uuid::parse_str(&state.get::<String, _>("active_revision_id"))?;
    let rows = sqlx::query(
        "SELECT id, project_id, sequence, previous_revision_id, fingerprint, catalog_json \
         FROM provider_catalog_revisions ORDER BY sequence",
    )
    .fetch_all(&mut *database)
    .await?;
    let mut previous = None;
    let mut active = None;
    for (index, row) in rows.into_iter().enumerate() {
        let catalog: ProviderCatalog = serde_json::from_str(&row.get::<String, _>("catalog_json"))?;
        catalog.validate()?;
        let row_previous = row
            .get::<Option<String>, _>("previous_revision_id")
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        ensure!(
            row.get::<String, _>("id") == catalog.id.to_string()
                && row.get::<String, _>("project_id") == catalog.project_id.to_string()
                && u64::try_from(row.get::<i64, _>("sequence"))? == catalog.sequence
                && row_previous == catalog.previous_revision_id
                && row.get::<String, _>("fingerprint") == catalog.fingerprint
                && catalog.project_id == manifest.id
                && catalog.sequence == index as u64 + 1
                && catalog.previous_revision_id == previous,
            "Provider settings history or normalized projection is invalid."
        );
        previous = Some(catalog.id);
        if catalog.id == active_id {
            active = Some(catalog);
        }
    }
    ensure!(
        previous == Some(active_id) && active.is_some(),
        "Active provider settings must be the latest append-only revision."
    );
    Ok(active)
}

fn model_origin(origin: ModelOrigin) -> &'static str {
    match origin {
        ModelOrigin::Imported => "imported",
        ModelOrigin::Trained => "trained",
        ModelOrigin::Transformed => "transformed",
    }
}

fn baseline_change(change: &BaselineChange) -> &'static str {
    match change {
        BaselineChange::Initialization { .. } => "initialization",
        BaselineChange::Promotion { .. } => "promotion",
        BaselineChange::Restoration { .. } => "restoration",
    }
}

fn verify_file(
    root: &Path,
    identity: &project_workspace_core::FileIdentity,
    verify: bool,
) -> Result<()> {
    let path = contained(root, &identity.path)?;
    let metadata = plain(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() == identity.bytes,
        "Artifact missing or size changed: {}",
        identity.path
    );
    if verify {
        ensure!(
            hash(&path, &identity.path)? == *identity,
            "Artifact checksum changed: {}",
            identity.path
        );
    }
    Ok(())
}

async fn connect(root: &Path, read_only: bool, create: bool) -> Result<SqliteConnection> {
    let options = SqliteConnectOptions::new()
        .filename(root.join(DATABASE))
        .read_only(read_only)
        .create_if_missing(create)
        .busy_timeout(std::time::Duration::from_secs(10));
    Ok(SqliteConnection::connect_with(&options).await?)
}
