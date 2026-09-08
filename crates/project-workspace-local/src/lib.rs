//! Local project custody. No training execution or dataset split decisions.
mod datasets;
mod files;
mod model;

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    BASELINE, DATABASE, DIRECTORIES, DatasetImport, LocalModel, MANIFEST, ProjectManifest,
    validate_name,
};
use serde::Serialize;
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

pub use datasets::{DatasetPreview, backfill_nomos, import_dataset, inspect_dataset};
use files::{canonical_plain, contained, copy_verified, hash, json, plain, write_new};
pub use model::inspect_model;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorkspace {
    pub folder: String,
    pub manifest: ProjectManifest,
    pub datasets: Vec<DatasetImport>,
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
    }
    Ok(ManagedWorkspace {
        folder: root.to_string_lossy().into_owned(),
        manifest,
        datasets,
        verified: verify,
    })
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
