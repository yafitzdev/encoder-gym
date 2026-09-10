use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    BASELINE, DatasetImport, DatasetPurpose, FileIdentity, TrainingSource, validate_name,
    validate_relative,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{
    ManagedWorkspace, connect,
    files::{canonical_plain, contained, copy_verified, hash, json, plain},
    open_workspace,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetPreview {
    pub source: String,
    pub artifact: FileIdentity,
    pub rows: u64,
    pub partitions: BTreeMap<String, u64>,
    pub native_nomos_training_rows: u64,
}

pub fn inspect_dataset(source: &Path, purpose: DatasetPurpose) -> Result<DatasetPreview> {
    let source = canonical_plain(source)?;
    ensure!(plain(&source)?.is_file(), "Choose a local JSONL file.");
    let before = hash(&source, "data.jsonl")?;
    let mut reader = BufReader::new(File::open(&source)?);
    let mut rows = 0;
    let mut partitions = BTreeMap::new();
    let mut native_nomos_training_rows = 0;
    loop {
        let mut line = Vec::new();
        // Cap one record without ever allocating an unbounded line.
        let count = (&mut reader)
            .take(8 * 1_048_576 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        ensure!(
            line.len() <= 8 * 1_048_576,
            "JSONL record exceeds 8 MiB near row {}.",
            rows + 1
        );
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let row: Value = serde_json::from_slice(&line)
            .with_context(|| format!("Invalid JSONL record near row {}.", rows + 1))?;
        ensure!(
            row.is_object(),
            "Each JSONL record must be an object (row {}).",
            rows + 1
        );
        let mut partition = "unspecified";
        for field in ["evaluation_partition", "split", "partition"] {
            if let Some(value) = row.get(field) {
                let value = value.as_str().context("Partition metadata must be text.")?;
                ensure!(value.len() <= 80, "Partition name is too long.");
                partition = value;
                if purpose == DatasetPurpose::Training {
                    ensure!(
                        ["train", "training"].contains(&value),
                        "Row {} declares a non-training partition. Import it under its actual purpose; do not mix held-out evidence into training.",
                        rows + 1
                    );
                }
            }
        }
        if purpose == DatasetPurpose::Training {
            ensure!(
                row.get("sealed").and_then(Value::as_bool) != Some(true),
                "Sealed rows cannot be imported as training data."
            );
        }
        if row.get("evaluation_partition").and_then(Value::as_str) == Some("train")
            && row.get("accepted").and_then(Value::as_bool) == Some(true)
            && row
                .get("decision_state_id")
                .and_then(Value::as_str)
                .is_some()
            && row.get("tool_registry").is_some()
            && row.get("legal_candidate_ids").is_some()
        {
            native_nomos_training_rows += 1;
        }
        *partitions.entry(partition.to_owned()).or_default() += 1;
        ensure!(
            partitions.len() <= 128,
            "Too many distinct partition names."
        );
        rows += 1;
    }
    ensure!(rows > 0, "Dataset contains no JSON objects.");
    let artifact = hash(&source, "data.jsonl")?;
    ensure!(
        artifact == before,
        "Dataset changed while its records were being inspected."
    );
    Ok(DatasetPreview {
        source: source.to_string_lossy().into_owned(),
        artifact,
        rows,
        partitions,
        native_nomos_training_rows,
    })
}

pub async fn import_dataset(
    folder: &Path,
    source: &Path,
    name: &str,
    purpose: DatasetPurpose,
    expected_fingerprint: &str,
) -> Result<ManagedWorkspace> {
    let workspace = open_workspace(folder, false).await?;
    let preview = inspect_dataset(source, purpose)?;
    ensure!(
        preview.artifact.fingerprint == expected_fingerprint,
        "Dataset changed after preview. Inspect it again before importing."
    );
    persist_import(&workspace, &preview, name, purpose, None).await?;
    open_workspace(folder, true).await
}

pub(crate) async fn persist_import(
    workspace: &ManagedWorkspace,
    preview: &DatasetPreview,
    name: &str,
    purpose: DatasetPurpose,
    training_source: Option<TrainingSource>,
) -> Result<()> {
    validate_name(name)?;
    let root = Path::new(&workspace.folder);
    let hex = &preview.artifact.fingerprint[7..];
    let relative = format!("datasets/imports/{hex}/data.jsonl");
    let dataset = DatasetImport {
        id: Uuid::new_v4(),
        name: name.trim().into(),
        created_at: Utc::now(),
        source: preview.source.clone(),
        purpose,
        format: "jsonl".into(),
        artifact: FileIdentity {
            path: relative,
            ..preview.artifact.clone()
        },
        rows: preview.rows,
        training_source,
    };
    dataset.validate()?;
    let imports = contained(root, "datasets/imports")?;
    let staging = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(&imports)?;
    copy_verified(
        Path::new(&preview.source),
        &staging.path().join("data.jsonl"),
        &preview.artifact,
    )?;
    // Validate the copied bytes, not merely the source path inspected earlier.
    let copied = inspect_dataset(&staging.path().join("data.jsonl"), purpose)?;
    ensure!(
        copied.rows == preview.rows
            && copied.artifact.fingerprint == preview.artifact.fingerprint
            && copied.partitions == preview.partitions
            && copied.native_nomos_training_rows == preview.native_nomos_training_rows,
        "Dataset changed during validation."
    );
    let mut database = connect(root, false, false).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(row) =
        sqlx::query("SELECT metadata_json FROM dataset_imports WHERE content_sha256 = ?")
            .bind(&dataset.artifact.fingerprint)
            .fetch_optional(&mut *transaction)
            .await?
    {
        let existing: DatasetImport = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        ensure!(
            existing.purpose == dataset.purpose
                && existing.training_source == dataset.training_source,
            "This content is already imported with a different purpose or training provenance. Existing evidence was not changed."
        );
        transaction.rollback().await?;
        database.close().await?;
        return Ok(());
    }
    let destination = imports.join(hex);
    if destination.try_exists()? {
        // A process may have stopped after publication but before the DB commit.
        ensure!(
            hash(
                &contained(root, &dataset.artifact.path)?,
                &dataset.artifact.path
            )? == dataset.artifact,
            "An existing import artifact is corrupt; it was not overwritten."
        );
    } else {
        fs::rename(staging.path(), &destination)?;
    }
    sqlx::query("INSERT INTO dataset_imports (id, content_sha256, metadata_json) VALUES (?, ?, ?)")
        .bind(dataset.id.to_string())
        .bind(&dataset.artifact.fingerprint)
        .bind(serde_json::to_string(&dataset)?)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(())
}

/// Import only the final-stage inputs explicitly named by this retained model.
/// Native records stay native; this does not create an admitted slice snapshot.
pub async fn backfill_nomos(folder: &Path, source_root: &Path) -> Result<ManagedWorkspace> {
    let workspace = open_workspace(folder, true).await?;
    let source_root = canonical_plain(source_root)?;
    let native_manifest = workspace
        .manifest
        .baseline
        .files
        .iter()
        .find(|file| file.path == "nomos_training_manifest.json")
        .context(
            "This baseline has no Nomos training manifest. Use explicit dataset import instead.",
        )?;
    let manifest: Value = json(&contained(
        Path::new(&workspace.folder),
        &format!("{BASELINE}/nomos_training_manifest.json"),
    )?)?;
    let inputs = manifest
        .get("inputs")
        .and_then(Value::as_array)
        .context("Training manifest has no input list.")?;
    let counts = manifest
        .get("input_state_counts")
        .and_then(Value::as_object)
        .context("Training manifest has no input counts.")?;
    ensure!(
        !inputs.is_empty() && inputs.len() <= 32,
        "Unsupported final-stage input list."
    );
    let mut prepared = vec![];
    for input in inputs {
        let input = input
            .as_str()
            .context("Training input must be a relative path.")?;
        let relative = input.replace('\\', "/");
        validate_relative(&relative)?;
        let declared_rows = counts
            .get(input)
            .and_then(Value::as_u64)
            .context("Missing declared input row count.")?;
        let preview = inspect_dataset(
            &contained(&source_root, &relative)?,
            DatasetPurpose::Training,
        )?;
        ensure!(
            preview.rows == declared_rows && preview.native_nomos_training_rows == declared_rows,
            "Training input {relative} does not match the manifest count and accepted native training-row contract."
        );
        prepared.push((
            preview,
            TrainingSource {
                baseline_fingerprint: workspace.manifest.baseline.fingerprint.clone(),
                manifest_fingerprint: native_manifest.fingerprint.clone(),
                input: relative,
                declared_rows,
            },
        ));
    }
    for (preview, provenance) in prepared {
        let name = provenance
            .input
            .rsplit('/')
            .next()
            .context("Invalid training input path.")?
            .to_string();
        persist_import(
            &workspace,
            &preview,
            &name,
            DatasetPurpose::Training,
            Some(provenance),
        )
        .await?;
    }
    open_workspace(folder, true).await
}
