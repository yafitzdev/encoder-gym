//! Maps an adapter-verified completed run to ordinary immutable dataset versions.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    BoundIdentity, DatasetPurpose, FileIdentity, ModelDatasetLink, ModelOrigin,
    ModelTrainingEvidence, TrainingDatasetInput, validate_relative,
};
use uuid::Uuid;

use crate::{
    dataset_versions::{self, DatasetVersionUpdate, RecordSelection},
    datasets::persist_import,
    files::{contained, json},
    inspect_dataset, inspect_model, open_workspace, verify_file,
};

/// Adapter-owned paths are accepted only by this application custody boundary,
/// never through a renderer request or an operator-authored arbitrary binding.
#[derive(Debug, Clone)]
pub struct RecordedTrainingInput {
    pub key: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub fingerprint: String,
    pub rows: u64,
}

#[derive(Debug, Clone)]
pub struct CompletedTrainingData {
    pub model_id: Uuid,
    /// Exact managed dataset version selected when this training run began.
    /// Legacy registrations may omit it and fall back to the parent model's
    /// recorded training version.
    pub parent_version_id: Option<Uuid>,
    pub snapshot: BoundIdentity,
    pub run: BoundIdentity,
    pub manifest: FileIdentity,
    pub inputs: Vec<RecordedTrainingInput>,
}

/// The caller must verify the native receipt and scientific completion journal.
/// This boundary verifies managed custody and creates/reuses dataset artifacts;
/// it neither trains a model nor makes a scientific acceptance decision.
pub async fn adopt_completed(
    folder: &Path,
    request: CompletedTrainingData,
) -> Result<ModelDatasetLink> {
    adopt(folder, request, false).await
}

/// Bind an adapter-verified rendering to its existing exact dataset version.
/// Unlike legacy adoption this never creates substitute row/version identities.
pub async fn adopt_materialized(
    folder: &Path,
    request: CompletedTrainingData,
) -> Result<ModelDatasetLink> {
    adopt(folder, request, true).await
}

async fn adopt(
    folder: &Path,
    request: CompletedTrainingData,
    materialized: bool,
) -> Result<ModelDatasetLink> {
    let mut workspace = open_workspace(folder, false).await?;
    let model = workspace
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog
                .artifacts
                .iter()
                .find(|model| model.id == request.model_id)
        })
        .context("Register the completed model first.")?
        .clone();
    ensure!(
        model.origin == ModelOrigin::Trained
            && model.training_snapshot.as_ref() == Some(&request.snapshot)
            && model.producing_run.as_ref() == Some(&request.run),
        "Training data belongs to another model, snapshot, or run."
    );
    let exact_version = if materialized {
        let version = dataset_versions::inspect(
            folder,
            request
                .parent_version_id
                .context("Materialized training needs its exact dataset version.")?,
        )
        .await?;
        ensure!(
            request.snapshot.id == version.id.to_string()
                && request.snapshot.fingerprint == version.fingerprint,
            "Materialized training snapshot differs from the selected version."
        );
        Some(version)
    } else {
        None
    };
    let checkpoint = inspect_model(&contained(Path::new(&workspace.folder), &model.path)?)?;
    ensure!(
        checkpoint.fingerprint == model.fingerprint
            && checkpoint.bytes == model.bytes
            && checkpoint.format == model.format,
        "The registered checkpoint changed."
    );
    let mut manifest_identity = request.manifest.clone();
    validate_relative(&manifest_identity.path)?;
    manifest_identity.path = format!("{}/{}", model.path, manifest_identity.path);
    verify_file(Path::new(&workspace.folder), &manifest_identity, true)?;
    let manifest: serde_json::Value = json(&contained(
        Path::new(&workspace.folder),
        &manifest_identity.path,
    )?)?;
    let keys = manifest
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .context("Training manifest has no input order.")?;
    ensure!(
        !request.inputs.is_empty()
            && request.inputs.len() <= 32
            && keys.len() == request.inputs.len(),
        "Unsupported training input count."
    );
    let mut fingerprints = BTreeSet::new();
    let mut input_keys = BTreeSet::new();
    let mut inspected = Vec::new();
    // Validate every source before publishing any new import.
    for (key, input) in keys.iter().zip(&request.inputs) {
        validate_relative(&input.key)?;
        ensure!(
            input_keys.insert(&input.key) && fingerprints.insert(&input.fingerprint),
            "Training inputs are repeated."
        );
        let key = key.as_str().context("Invalid training input key.")?;
        ensure!(
            key.replace('\\', "/") == input.key
                && manifest
                    .get("input_row_counts")
                    .and_then(|counts| counts.get(key))
                    .and_then(serde_json::Value::as_u64)
                    == Some(input.rows),
            "Training input order or row counts differ from the recorded manifest."
        );
        let preview = inspect_dataset(&input.path, DatasetPurpose::Training)?;
        ensure!(
            preview.artifact.bytes == input.bytes
                && preview.artifact.fingerprint == input.fingerprint
                && preview.rows == input.rows,
            "Training source differs from its verified native identity."
        );
        if let Some(existing) = workspace
            .datasets
            .iter()
            .find(|source| source.artifact.fingerprint == input.fingerprint)
        {
            ensure!(
                existing.purpose == DatasetPurpose::Training
                    && existing.rows == input.rows
                    && existing.artifact.bytes == input.bytes,
                "Existing source has incompatible purpose or contents."
            );
            verify_file(Path::new(&workspace.folder), &existing.artifact, true)?;
        }
        inspected.push(preview);
    }
    for preview in inspected {
        if !workspace
            .datasets
            .iter()
            .any(|source| source.artifact.fingerprint == preview.artifact.fingerprint)
        {
            persist_import(
                &workspace,
                &preview,
                &format!("{} training data", model.name),
                DatasetPurpose::Training,
                None,
            )
            .await?;
        }
    }
    workspace = open_workspace(folder, false).await?;
    let inputs = request
        .inputs
        .iter()
        .map(|input| {
            let source = workspace
                .datasets
                .iter()
                .find(|source| source.artifact.fingerprint == input.fingerprint)
                .context("Recorded training import is missing.")?;
            Ok(TrainingDatasetInput {
                key: input.key.clone(),
                import_id: source.id,
                fingerprint: input.fingerprint.clone(),
                rows: input.rows,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let evidence = match &exact_version {
        Some(version) => ModelTrainingEvidence::MaterializedTraining {
            manifest: request.manifest,
            snapshot: request.snapshot,
            run: request.run,
            ordered_content_fingerprint: ModelDatasetLink::materialized_content_fingerprint(
                version,
            )?,
        },
        None => ModelTrainingEvidence::CompletedTraining {
            manifest: request.manifest,
            snapshot: request.snapshot,
            run: request.run,
        },
    };
    if let Some(existing) = workspace
        .model_dataset_links
        .iter()
        .find(|link| link.model_id == model.id)
    {
        ensure!(
            existing.inputs == inputs && existing.evidence == evidence,
            "Completed model training provenance changed."
        );
        let version = dataset_versions::inspect(folder, existing.version.id).await?;
        existing.validate_for(&model, &version)?;
        dataset_versions::rows::verify_members(&workspace, &version.members)?;
        super::verify_materialized_members(&workspace, existing)?;
        return Ok(existing.clone());
    }
    if let Some(version) = exact_version {
        let link = ModelDatasetLink::new(&model, &version, inputs, evidence, Utc::now())?;
        dataset_versions::rows::verify_members(&workspace, &version.members)?;
        super::verify_materialized_members(&workspace, &link)?;
        return super::persist_link(&workspace, link, &version).await;
    }
    let parent_version = request
        .parent_version_id
        .or_else(|| {
            workspace
                .model_dataset_links
                .iter()
                .find(|link| Some(link.model_id) == model.parent_model_id)
                .map(|link| link.version.id)
        })
        .context("Select the training dataset or link the starting model's recorded dataset.")?;
    let original = dataset_versions::inspect(folder, parent_version).await?;
    let mut versions = vec![original.clone()];
    // Reuse any already-recorded exact population, even with a different native
    // input order. Models trained with different settings can share a version.
    for entry in dataset_versions::list(folder).await? {
        for summary in entry.versions {
            if summary.version.id != original.id {
                versions.push(dataset_versions::inspect(folder, summary.version.id).await?);
            }
        }
    }
    for version in versions {
        if let Ok(link) = ModelDatasetLink::new(
            &model,
            &version,
            inputs.clone(),
            evidence.clone(),
            Utc::now(),
        ) {
            dataset_versions::rows::verify_members(&workspace, &version.members)?;
            return super::persist_link(&workspace, link, &version).await;
        }
    }
    let mut target = Vec::new();
    for input in &inputs {
        target.extend(dataset_versions::rows::source_members(
            &workspace,
            input.import_id,
        )?);
    }
    let target_ids: BTreeSet<_> = target.iter().map(|member| member.id.as_str()).collect();
    let previous: BTreeMap<_, _> = original
        .members
        .iter()
        .map(|member| (member.id.as_str(), member))
        .collect();
    let added = target
        .iter()
        .filter(|member| !previous.contains_key(member.id.as_str()))
        .map(|member| RecordSelection {
            import_id: member.source.import_id,
            record: member.source.record,
        })
        .collect();
    let removed = original
        .members
        .iter()
        .filter(|member| !target_ids.contains(member.id.as_str()))
        .map(|member| member.id.clone())
        .collect();
    let dataset_id = super::reserved_id(workspace.manifest.id, model.id, "candidate-dataset");
    let first = dataset_versions::fork(
        folder,
        dataset_id,
        super::reserved_id(workspace.manifest.id, model.id, "candidate-version-1"),
        &format!("{} dataset", model.name),
        original.id,
    )
    .await?;
    let version = dataset_versions::revise(
        folder,
        DatasetVersionUpdate {
            dataset_id,
            version_id: super::reserved_id(workspace.manifest.id, model.id, "candidate-version-2"),
            parent_id: first.id,
            added,
            removed,
            replaced: vec![],
        },
    )
    .await?;
    let link = ModelDatasetLink::new(&model, &version, inputs, evidence, Utc::now())?;
    super::persist_link(&workspace, link, &version).await
}
