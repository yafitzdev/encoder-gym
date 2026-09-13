use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use dataset_core::{
    domain::SnapshotSplit,
    versions::{DatasetMember, SourceRecord},
};
use project_workspace_core::DatasetPurpose;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    ManagedWorkspace,
    files::{contained, hash},
    inspect_dataset, open_workspace,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedDatasetRow {
    pub member: DatasetMember,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetRowPage {
    pub version_id: Uuid,
    pub offset: u64,
    pub total: u64,
    pub rows: Vec<InspectedDatasetRow>,
}

struct SourceContents {
    members: BTreeMap<u64, DatasetMember>,
    selected: BTreeMap<u64, Value>,
}

fn scan(
    workspace: &ManagedWorkspace,
    import_id: Uuid,
    selected: Option<&BTreeSet<u64>>,
    selected_maximum_bytes: usize,
    selected_maximum_message: &str,
) -> Result<SourceContents> {
    let source = workspace
        .datasets
        .iter()
        .find(|source| source.id == import_id)
        .context("Source does not belong to this project.")?;
    ensure!(
        source.purpose == DatasetPurpose::Training,
        "Only training sources may be inspected or used in training datasets."
    );
    let path = contained(Path::new(&workspace.folder), &source.artifact.path)?;
    let preview = inspect_dataset(&path, DatasetPurpose::Training)?;
    ensure!(
        preview.artifact.fingerprint == source.artifact.fingerprint
            && preview.artifact.bytes == source.artifact.bytes
            && preview.rows == source.rows,
        "The dataset source no longer matches its immutable import."
    );
    let mut reader = BufReader::new(File::open(&path)?);
    let mut members = BTreeMap::new();
    let mut payloads = BTreeMap::new();
    let mut record = 0;
    let mut returned_bytes = 0;
    crate::progress::row_progress(&source.name, 0, source.rows);
    loop {
        let mut line = Vec::new();
        let count = (&mut reader)
            .take(8 * 1_048_576 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        ensure!(
            line.len() <= 8 * 1_048_576,
            "A source record exceeds the inspection limit."
        );
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        record += 1;
        if record % 100 == 0 || record == source.rows {
            crate::progress::row_progress(&source.name, record, source.rows);
        }
        // The complete source was already validated and checksum-bound above.
        // A page only needs content fingerprints for the requested records;
        // constructing membership for every large native row makes paging slow.
        if selected.is_some_and(|records| !records.contains(&record)) {
            continue;
        }
        let value: Value =
            serde_json::from_slice(&line).context("An imported row is no longer valid JSON.")?;
        ensure!(
            value.is_object(),
            "An imported record is no longer a JSON object."
        );
        members.insert(
            record,
            DatasetMember::imported(
                SourceRecord {
                    import_id,
                    artifact_fingerprint: source.artifact.fingerprint.clone(),
                    record,
                },
                artifact_core::fingerprint(&value)?,
                SnapshotSplit::Train,
            )?,
        );
        if selected.is_some() {
            returned_bytes += line.len();
            ensure!(
                returned_bytes <= selected_maximum_bytes,
                "{selected_maximum_message}"
            );
            payloads.insert(record, value);
        }
    }
    ensure!(
        record == source.rows && hash(&path, &source.artifact.path)? == source.artifact,
        "Dataset changed while reading its records."
    );
    Ok(SourceContents {
        members,
        selected: payloads,
    })
}

pub(crate) fn source_members(
    workspace: &ManagedWorkspace,
    import_id: Uuid,
) -> Result<Vec<DatasetMember>> {
    Ok(scan(workspace, import_id, None, 0, "")?
        .members
        .into_values()
        .collect())
}

pub(crate) fn verify_members(
    workspace: &ManagedWorkspace,
    members: &[DatasetMember],
) -> Result<()> {
    let mut sources = BTreeMap::new();
    for member in members {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            sources.entry(member.source.import_id)
        {
            entry.insert(source_members(workspace, member.source.import_id)?);
        }
        let stored = sources[&member.source.import_id]
            .get(usize::try_from(member.source.record - 1)?)
            .context("Source record is absent from the import.")?;
        ensure!(
            stored.source == member.source
                && stored.content_fingerprint == member.content_fingerprint
                && stored.split == member.split,
            "Version membership does not match its verified training source."
        );
    }
    Ok(())
}

pub async fn read_rows(
    folder: &Path,
    version_id: Uuid,
    offset: u64,
    limit: u32,
) -> Result<DatasetRowPage> {
    ensure!((1..=100).contains(&limit), "Row page size must be 1–100.");
    let workspace = open_workspace(folder, false).await?;
    let version = super::inspect_workspace(&workspace, version_id).await?;
    let selected: Vec<_> = version
        .members
        .iter()
        .skip(usize::try_from(offset)?)
        .take(limit as usize)
        .cloned()
        .collect();
    let rows = inspect_members(&workspace, &selected)?;
    Ok(DatasetRowPage {
        version_id,
        offset,
        total: version.members.len() as u64,
        rows,
    })
}

pub(super) fn inspect_members(
    workspace: &ManagedWorkspace,
    members: &[DatasetMember],
) -> Result<Vec<InspectedDatasetRow>> {
    inspect_members_bounded(
        workspace,
        members,
        16 * 1_048_576,
        "Selected rows exceed 16 MiB. Request a smaller page.",
    )
}

pub(super) fn inspect_materialization_members(
    workspace: &ManagedWorkspace,
    members: &[DatasetMember],
) -> Result<Vec<InspectedDatasetRow>> {
    // A task adapter consumes these values immediately. Keep one finite local
    // ceiling while allowing ordinary training snapshots larger than a UI page.
    inspect_members_bounded(
        workspace,
        members,
        512 * 1_048_576,
        "Selected training rows exceed the 512 MiB local materialization limit.",
    )
}

fn inspect_members_bounded(
    workspace: &ManagedWorkspace,
    members: &[DatasetMember],
    maximum_bytes: usize,
    maximum_message: &str,
) -> Result<Vec<InspectedDatasetRow>> {
    let mut requested: BTreeMap<Uuid, BTreeSet<u64>> = BTreeMap::new();
    for member in members {
        requested
            .entry(member.source.import_id)
            .or_default()
            .insert(member.source.record);
    }
    let mut sources = BTreeMap::new();
    for (id, records) in requested {
        sources.insert(
            id,
            scan(
                workspace,
                id,
                Some(&records),
                maximum_bytes,
                maximum_message,
            )?,
        );
    }
    let mut rows = Vec::new();
    let mut bytes = 0;
    for member in members {
        let source = &sources[&member.source.import_id];
        let original = source
            .members
            .get(&member.source.record)
            .context("Source record is absent from the import.")?;
        ensure!(
            original.source == member.source
                && original.content_fingerprint == member.content_fingerprint
                && original.split == member.split,
            "Version row does not match its verified training source."
        );
        let value = source
            .selected
            .get(&member.source.record)
            .context("Selected source row was not read.")?
            .clone();
        bytes += serde_json::to_vec(&value)?.len();
        ensure!(bytes <= maximum_bytes, "{maximum_message}");
        rows.push(InspectedDatasetRow {
            member: member.clone(),
            value,
        });
    }
    Ok(rows)
}
