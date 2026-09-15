//! Reconstruct recorded model inputs without altering checkpoints or run history.
mod completed;
pub use completed::{
    CompletedTrainingData, RecordedTrainingInput, adopt_completed, adopt_materialized,
};

use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    BASELINE, DatasetImport, DatasetPurpose, ModelCatalog, ModelDatasetLink, ModelOrigin,
    ModelTrainingEvidence, TrainingDatasetInput,
};
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    ManagedWorkspace, connect, dataset_versions,
    files::{contained, json},
    open_workspace, verify_file,
};

pub(crate) async fn load_links(
    database: &mut SqliteConnection,
    root: &Path,
    catalog: Option<&ModelCatalog>,
    imports: &[DatasetImport],
) -> Result<Vec<ModelDatasetLink>> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_dataset_links'",
    )
    .fetch_one(&mut *database)
    .await?
        == 0
    {
        return Ok(vec![]);
    }
    let mut links = vec![];
    for row in sqlx::query("SELECT model_id, version_id, fingerprint, metadata_json FROM model_dataset_links ORDER BY rowid").fetch_all(&mut *database).await? {
        let link: ModelDatasetLink = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        ensure!(row.get::<String, _>("model_id") == link.model_id.to_string() && row.get::<String, _>("version_id") == link.version.id.to_string() && row.get::<String, _>("fingerprint") == link.fingerprint, "Model dataset link storage changed.");
        let model = catalog.and_then(|catalog| catalog.artifacts.iter().find(|model| model.id == link.model_id)).context("Training dataset refers to an absent model.")?;
        let version = dataset_versions::load_reference(database, model.project_id, link.version.id).await?;
        link.validate_reference(model, &version)?;
        let mut identity = link.evidence.manifest().clone();
        identity.path = format!("{}/{}", model.path, identity.path);
        // The small manifest is always verified, including on metadata-only opens.
        verify_file(root, &identity, true)?;
        let manifest: serde_json::Value = json(&contained(root, &identity.path)?)?;
        let keys = manifest.get("inputs").and_then(serde_json::Value::as_array).context("Training manifest has no input list.")?;
        let actual_keys = keys.iter().map(|key| key.as_str().map(|key| key.replace('\\', "/")).context("Training input key is invalid.")).collect::<Result<Vec<_>>>()?;
        ensure!(actual_keys == link.inputs.iter().map(|input| input.key.clone()).collect::<Vec<_>>(), "Model training input order changed.");
        let count_field = match &link.evidence { ModelTrainingEvidence::ImportedManifest { .. } => "input_state_counts", ModelTrainingEvidence::CompletedTraining { .. } | ModelTrainingEvidence::MaterializedTraining { .. } => "input_row_counts" };
        for (key, input) in keys.iter().zip(&link.inputs) {
            ensure!(manifest.get(count_field).and_then(|counts| counts.get(key.as_str().expect("validated input key"))).and_then(serde_json::Value::as_u64) == Some(input.rows), "Model training manifest row counts changed.");
        }
        for input in &link.inputs {
            let source = imports.iter().find(|source| source.id == input.import_id).context("Training source is missing.")?;
            ensure!(source.purpose == DatasetPurpose::Training && source.artifact.fingerprint == input.fingerprint && source.rows == input.rows, "Model training input changed.");
            if matches!(link.evidence, ModelTrainingEvidence::ImportedManifest { .. }) {
                let recorded = source.training_source.as_ref().context("Training source has no recorded model provenance.")?;
                ensure!(recorded.baseline_fingerprint == model.fingerprint && recorded.manifest_fingerprint == link.evidence.manifest().fingerprint && recorded.input == input.key && recorded.declared_rows == input.rows, "Imported training provenance changed.");
            }
        }
        links.push(link);
    }
    Ok(links)
}

pub(crate) async fn verify_members(workspace: &ManagedWorkspace) -> Result<()> {
    if workspace.model_dataset_links.is_empty() {
        return Ok(());
    }
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    for link in &workspace.model_dataset_links {
        let version =
            dataset_versions::load_version(&mut database, workspace.manifest.id, link.version.id)
                .await?;
        let model = workspace
            .model_catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .artifacts
                    .iter()
                    .find(|model| model.id == link.model_id)
            })
            .context("Linked model is missing.")?;
        link.validate_for(model, &version)?;
        dataset_versions::rows::verify_members(workspace, &version.members)?;
        verify_materialized_members(workspace, link)?;
    }
    database.close().await?;
    Ok(())
}

fn verify_materialized_members(
    workspace: &ManagedWorkspace,
    link: &ModelDatasetLink,
) -> Result<()> {
    let ModelTrainingEvidence::MaterializedTraining {
        ordered_content_fingerprint,
        ..
    } = &link.evidence
    else {
        return Ok(());
    };
    let mut contents = Vec::new();
    for input in &link.inputs {
        let members = dataset_versions::rows::source_members(workspace, input.import_id)?;
        ensure!(
            members.len() as u64 == input.rows,
            "Materialized training source count changed."
        );
        contents.extend(members.into_iter().map(|row| row.content_fingerprint));
    }
    ensure!(
        ModelDatasetLink::ordered_content_fingerprint(contents.iter().map(String::as_str))?
            == *ordered_content_fingerprint,
        "Materialized training rows or their order differ from the recorded version."
    );
    Ok(())
}

/// Adopt only the original imported checkpoint's recorded final-stage inputs.
/// This never reconstructs unknown ancestor pretraining or changes the baseline.
pub async fn adopt_baseline(folder: &Path) -> Result<ModelDatasetLink> {
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Initialize model history first.")?;
    let model = catalog
        .artifacts
        .iter()
        .find(|model| {
            model.origin == ModelOrigin::Imported
                && model.path == BASELINE
                && model.fingerprint == workspace.manifest.baseline.fingerprint
        })
        .context("The original imported model is missing.")?;
    let manifest = workspace
        .manifest
        .baseline
        .files
        .iter()
        .find(|file| file.path == "nomos_training_manifest.json")
        .context("The imported model has no recorded training manifest.")?
        .clone();
    let value: serde_json::Value = json(&contained(
        Path::new(&workspace.folder),
        &format!("{BASELINE}/{}", manifest.path),
    )?)?;
    let inputs = value
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .context("Training manifest has no inputs.")?;
    ensure!(
        !inputs.is_empty() && inputs.len() <= 32,
        "Training manifest has unsupported input count."
    );
    let mut recorded = vec![];
    for key in inputs {
        let key = key
            .as_str()
            .context("Invalid training input key.")?
            .replace('\\', "/");
        let sources: Vec<_> = workspace
            .datasets
            .iter()
            .filter(|source| {
                source.training_source.as_ref().is_some_and(|record| {
                    record.input == key
                        && record.baseline_fingerprint == model.fingerprint
                        && record.manifest_fingerprint == manifest.fingerprint
                })
            })
            .collect();
        ensure!(
            sources.len() == 1,
            "Import the model's exact recorded training inputs before adopting its dataset."
        );
        let source = sources[0];
        recorded.push(TrainingDatasetInput {
            key,
            import_id: source.id,
            fingerprint: source.artifact.fingerprint.clone(),
            rows: source.rows,
        });
    }
    let evidence = ModelTrainingEvidence::ImportedManifest { manifest };
    if let Some(existing) = workspace
        .model_dataset_links
        .iter()
        .find(|link| link.model_id == model.id)
    {
        ensure!(
            existing.inputs == recorded && existing.evidence == evidence,
            "Recorded model training inputs changed."
        );
        return Ok(existing.clone());
    }
    let mut matched = None;
    let entries = dataset_versions::list(folder).await?;
    for entry in &entries {
        if entry.dataset.origin.is_some() {
            continue;
        }
        for summary in &entry.versions {
            let version = dataset_versions::inspect(folder, summary.version.id).await?;
            if ModelDatasetLink::new(
                model,
                &version,
                recorded.clone(),
                evidence.clone(),
                Utc::now(),
            )
            .is_ok()
            {
                matched = Some(version);
                break;
            }
        }
    }
    let version = match matched {
        Some(version) => version,
        None => {
            ensure!(
                entries.is_empty(),
                "The base dataset does not contain this model's recorded training population. Existing datasets were not changed."
            );
            let sources = recorded
                .iter()
                .map(|input| input.import_id)
                .collect::<Vec<_>>();
            ensure!(
                sources.iter().collect::<BTreeSet<_>>().len() == sources.len(),
                "Training inputs are repeated."
            );
            dataset_versions::create_base(
                folder,
                reserved_id(workspace.manifest.id, model.id, "base"),
                reserved_id(workspace.manifest.id, model.id, "version-1"),
                "Base dataset",
                &sources,
            )
            .await?
        }
    };
    dataset_versions::rows::verify_members(&workspace, &version.members)?;
    let link = ModelDatasetLink::new(model, &version, recorded, evidence, Utc::now())?;
    persist_link(&workspace, link, &version).await
}

async fn persist_link(
    workspace: &ManagedWorkspace,
    mut link: ModelDatasetLink,
    version: &dataset_core::versions::DatasetVersion,
) -> Result<ModelDatasetLink> {
    let model = workspace
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog
                .artifacts
                .iter()
                .find(|model| model.id == link.model_id)
        })
        .context("Linked model is missing.")?;
    link.validate_for(model, version)?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(json) = sqlx::query_scalar::<_, String>(
        "SELECT metadata_json FROM model_dataset_links WHERE model_id=?",
    )
    .bind(model.id.to_string())
    .fetch_optional(&mut *transaction)
    .await?
    {
        let existing: ModelDatasetLink = serde_json::from_str(&json)?;
        link.created_at = existing.created_at;
        link.fingerprint.clone_from(&existing.fingerprint);
        ensure!(
            link == existing,
            "This model already has a different training dataset link."
        );
        existing.validate_for(model, version)?;
        transaction.rollback().await?;
    } else {
        sqlx::query("INSERT INTO model_dataset_links(model_id, version_id, fingerprint, metadata_json) VALUES (?, ?, ?, ?)").bind(link.model_id.to_string()).bind(link.version.id.to_string()).bind(&link.fingerprint).bind(serde_json::to_string(&link)?).execute(&mut *transaction).await?;
        transaction.commit().await?;
    }
    database.close().await?;
    Ok(link)
}

// Project/model-scoped UUIDv8 reservations keep retries after partial creation
// on the same objects. They are neither credentials nor content fingerprints.
fn reserved_id(project: Uuid, model: Uuid, slot: &str) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(b"model-training-dataset-v1");
    hash.update(project.as_bytes());
    hash.update(model.as_bytes());
    hash.update(slot.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().expect("16 digest bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
