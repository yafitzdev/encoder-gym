//! Project-bound persistence for native dataset variants. Sources remain owned
//! immutable imports; version creation is not scientific training approval.
pub(crate) mod rows;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use dataset_core::versions::{
    DatasetBranch, DatasetChanges, DatasetMember, DatasetVersion, DatasetVersionRef,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{ManagedWorkspace, connect, open_workspace};
pub use rows::{DatasetRowPage, InspectedDatasetRow, read_rows};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedDatasetChange {
    pub id: String,
    pub kind: &'static str,
    pub before: Option<InspectedDatasetRow>,
    pub after: Option<InspectedDatasetRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetChangePage {
    pub version_id: Uuid,
    pub offset: u64,
    pub total: u64,
    pub changes: Vec<InspectedDatasetChange>,
}

pub async fn read_changes(
    folder: &Path,
    version_id: Uuid,
    offset: u64,
    limit: u32,
) -> Result<DatasetChangePage> {
    ensure!((1..=50).contains(&limit), "Change page size must be 1–50.");
    let workspace = open_workspace(folder, false).await?;
    let version = inspect_workspace(&workspace, version_id).await?;
    let parent = match &version.parent {
        Some(parent) => Some(inspect_workspace(&workspace, parent.id).await?),
        None => None,
    };
    let previous: BTreeMap<_, _> = parent
        .as_ref()
        .map(|v| v.members.iter().map(|m| (&m.id, m)).collect())
        .unwrap_or_default();
    let changes: Vec<_> = version
        .changes
        .added
        .iter()
        .map(|member| ("added", member.id.clone(), None, Some(member)))
        .chain(
            version
                .changes
                .removed
                .iter()
                .map(|id| ("removed", id.clone(), previous.get(id).copied(), None)),
        )
        .chain(version.changes.replaced.iter().map(|member| {
            (
                "replaced",
                member.id.clone(),
                previous.get(&member.id).copied(),
                Some(member),
            )
        }))
        .collect();
    let mut inspected = Vec::new();
    let selected: Vec<_> = changes
        .iter()
        .skip(usize::try_from(offset)?)
        .take(limit as usize)
        .collect();
    let members: Vec<_> = selected
        .iter()
        .flat_map(|(_, _, before, after)| before.iter().chain(after.iter()).map(|m| (*m).clone()))
        .collect();
    let mut rows = rows::inspect_members(&workspace, &members)?.into_iter();
    for (kind, id, before, after) in selected {
        inspected.push(InspectedDatasetChange {
            id: id.clone(),
            kind,
            before: before.map(|_| rows.next().expect("one inspected row per member")),
            after: after.map(|_| rows.next().expect("one inspected row per member")),
        });
    }
    Ok(DatasetChangePage {
        version_id,
        offset,
        total: changes.len() as u64,
        changes: inspected,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetVersionSummary {
    pub version: DatasetVersionRef,
    pub parent_id: Option<Uuid>,
    pub created_at: String,
    pub rows: u64,
    pub added: u64,
    pub removed: u64,
    pub replaced: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetEntry {
    pub dataset: DatasetBranch,
    pub versions: Vec<DatasetVersionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordSelection {
    pub import_id: Uuid,
    pub record: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowReplacement {
    pub id: String,
    pub source: RecordSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetVersionUpdate {
    pub version_id: Uuid,
    pub dataset_id: Uuid,
    pub parent_id: Uuid,
    pub added: Vec<RecordSelection>,
    pub removed: Vec<String>,
    pub replaced: Vec<RowReplacement>,
}

async fn writable(workspace: &ManagedWorkspace) -> Result<SqliteConnection> {
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    Ok(database)
}

async fn schema_exists(database: &mut SqliteConnection) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dataset_branches'",
    )
    .fetch_one(database)
    .await?
        == 1)
}

/// A metadata-only collection. Row counts are derived from immutable membership,
/// not separately maintained counters or renderer state.
pub async fn list(folder: &Path) -> Result<Vec<DatasetEntry>> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    if !schema_exists(&mut database).await? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let branches = sqlx::query("SELECT id FROM dataset_branches ORDER BY rowid")
        .fetch_all(&mut database)
        .await?;
    for record in branches {
        let id: Uuid = record.get::<String, _>("id").parse()?;
        let dataset = load_branch(&mut database, workspace.manifest.id, id).await?;
        let summaries = sqlx::query(
            "SELECT id, number, parent_version_id, fingerprint,
            json_extract(metadata_json, '$.createdAt') AS created_at,
            json_array_length(metadata_json, '$.members') AS rows,
            json_array_length(metadata_json, '$.changes.added') AS added,
            json_array_length(metadata_json, '$.changes.removed') AS removed,
            json_array_length(metadata_json, '$.changes.replaced') AS replaced
            FROM dataset_versions WHERE dataset_id=? ORDER BY number DESC",
        )
        .bind(id.to_string())
        .fetch_all(&mut database)
        .await?;
        let versions = summaries
            .into_iter()
            .map(|row| {
                let version = DatasetVersionRef {
                    id: row.get::<String, _>("id").parse()?,
                    dataset_id: id,
                    project_id: dataset.project_id,
                    number: u64::try_from(row.get::<i64, _>("number"))?,
                    fingerprint: row.get("fingerprint"),
                };
                version.validate()?;
                Ok(DatasetVersionSummary {
                    version,
                    parent_id: row
                        .get::<Option<String>, _>("parent_version_id")
                        .map(|id| id.parse())
                        .transpose()?,
                    created_at: row.get("created_at"),
                    rows: u64::try_from(row.get::<i64, _>("rows"))?,
                    added: u64::try_from(row.get::<i64, _>("added"))?,
                    removed: u64::try_from(row.get::<i64, _>("removed"))?,
                    replaced: u64::try_from(row.get::<i64, _>("replaced"))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(!versions.is_empty(), "Dataset has no initial version.");
        entries.push(DatasetEntry { dataset, versions });
    }
    database.close().await?;
    Ok(entries)
}

pub async fn inspect(folder: &Path, version_id: Uuid) -> Result<DatasetVersion> {
    let workspace = open_workspace(folder, false).await?;
    inspect_workspace(&workspace, version_id).await
}

/// Deep preparation boundary: reproduce version history and re-read every
/// selected source row without returning row payloads to the caller.
pub async fn verify(folder: &Path, version_id: Uuid) -> Result<DatasetVersion> {
    let workspace = open_workspace(folder, true).await?;
    let version = inspect_workspace(&workspace, version_id).await?;
    rows::verify_members(&workspace, &version.members)?;
    Ok(version)
}

async fn inspect_workspace(
    workspace: &ManagedWorkspace,
    version_id: Uuid,
) -> Result<DatasetVersion> {
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let version = load_version(&mut database, workspace.manifest.id, version_id).await?;
    database.close().await?;
    Ok(version)
}

pub async fn create_base(
    folder: &Path,
    dataset_id: Uuid,
    version_id: Uuid,
    name: &str,
    imports: &[Uuid],
) -> Result<DatasetVersion> {
    let workspace = open_workspace(folder, false).await?;
    ensure!(!imports.is_empty(), "Choose at least one training source.");
    ensure!(
        imports.iter().collect::<BTreeSet<_>>().len() == imports.len(),
        "A source was selected more than once."
    );
    let mut members = Vec::new();
    for id in imports {
        members.extend(rows::source_members(&workspace, *id)?);
    }
    let at = Utc::now();
    let branch = DatasetBranch::new(dataset_id, workspace.manifest.id, name.into(), None, at)?;
    let version = DatasetVersion::initial(version_id, &branch, members, at)?;
    persist_new_branch(&workspace, branch, version).await
}

pub async fn fork(
    folder: &Path,
    dataset_id: Uuid,
    version_id: Uuid,
    name: &str,
    parent_id: Uuid,
) -> Result<DatasetVersion> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let parent = load_version(&mut database, workspace.manifest.id, parent_id).await?;
    database.close().await?;
    rows::verify_members(&workspace, &parent.members)?;
    let at = Utc::now();
    let branch = DatasetBranch::new(
        dataset_id,
        workspace.manifest.id,
        name.into(),
        Some(parent.reference()),
        at,
    )?;
    let version = DatasetVersion::fork(version_id, &branch, &parent, at)?;
    persist_new_branch(&workspace, branch, version).await
}

async fn persist_new_branch(
    workspace: &ManagedWorkspace,
    mut branch: DatasetBranch,
    mut version: DatasetVersion,
) -> Result<DatasetVersion> {
    let mut database = writable(workspace).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(existing) =
        optional_version(&mut transaction, workspace.manifest.id, version.id).await?
    {
        let existing_branch =
            load_branch(&mut transaction, workspace.manifest.id, existing.dataset_id).await?;
        branch.created_at = existing_branch.created_at;
        version.created_at = existing.created_at;
        version.fingerprint.clone_from(&existing.fingerprint);
        ensure!(
            branch == existing_branch && version == existing,
            "The dataset creation identity was already used for different contents."
        );
        transaction.rollback().await?;
        database.close().await?;
        return Ok(existing);
    }
    sqlx::query("INSERT INTO dataset_branches (id, project_id, origin_version_id, metadata_json) VALUES (?, ?, ?, ?)")
        .bind(branch.id.to_string()).bind(branch.project_id.to_string()).bind(branch.origin.as_ref().map(|v| v.id.to_string()))
        .bind(serde_json::to_string(&branch)?).execute(&mut *transaction).await?;
    save_version(&mut transaction, &version).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(version)
}

pub async fn revise(folder: &Path, update: DatasetVersionUpdate) -> Result<DatasetVersion> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = writable(&workspace).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let branch = load_branch(&mut transaction, workspace.manifest.id, update.dataset_id).await?;
    let parent = load_version(&mut transaction, workspace.manifest.id, update.parent_id).await?;
    let mut sources = BTreeMap::new();
    for selection in update
        .added
        .iter()
        .chain(update.replaced.iter().map(|r| &r.source))
    {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            sources.entry(selection.import_id)
        {
            entry.insert(rows::source_members(&workspace, selection.import_id)?);
        }
    }
    let resolve = |selection: &RecordSelection| -> Result<DatasetMember> {
        sources
            .get(&selection.import_id)
            .and_then(|members| members.iter().find(|m| m.source.record == selection.record))
            .cloned()
            .context("Selected source record was not found.")
    };
    let changes = DatasetChanges {
        added: update.added.iter().map(&resolve).collect::<Result<_>>()?,
        removed: update.removed,
        replaced: update
            .replaced
            .iter()
            .map(|replacement| {
                let mut member = resolve(&replacement.source)?;
                member.id.clone_from(&replacement.id);
                Ok(member)
            })
            .collect::<Result<_>>()?,
    };
    let previous =
        optional_version(&mut transaction, workspace.manifest.id, update.version_id).await?;
    let created = previous.as_ref().map_or_else(Utc::now, |v| v.created_at);
    let version = DatasetVersion::revise(update.version_id, &branch, &parent, changes, created)?;
    if let Some(existing) = previous {
        ensure!(
            existing == version,
            "The version identity was already used for different changes."
        );
        transaction.rollback().await?;
        database.close().await?;
        return Ok(existing);
    }
    let head: String = sqlx::query_scalar(
        "SELECT id FROM dataset_versions WHERE dataset_id=? ORDER BY number DESC LIMIT 1",
    )
    .bind(branch.id.to_string())
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        head == parent.id.to_string(),
        "Dataset has changed. Reload its latest version before saving, or create a variant from the older version."
    );
    rows::verify_members(&workspace, &version.members)?;
    save_version(&mut transaction, &version).await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(version)
}

async fn save_version(database: &mut SqliteConnection, version: &DatasetVersion) -> Result<()> {
    version.validate_integrity()?;
    sqlx::query("INSERT INTO dataset_versions (id, dataset_id, number, parent_version_id, fingerprint, metadata_json) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(version.id.to_string()).bind(version.dataset_id.to_string()).bind(i64::try_from(version.number)?)
        .bind(version.parent.as_ref().map(|v| v.id.to_string())).bind(&version.fingerprint).bind(serde_json::to_string(version)?)
        .execute(database).await?;
    Ok(())
}

async fn load_branch(
    database: &mut SqliteConnection,
    project_id: Uuid,
    id: Uuid,
) -> Result<DatasetBranch> {
    let row = sqlx::query(
        "SELECT project_id, origin_version_id, metadata_json FROM dataset_branches WHERE id=?",
    )
    .bind(id.to_string())
    .fetch_optional(database)
    .await?
    .context("Dataset not found in this project.")?;
    let branch: DatasetBranch = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
    branch.validate()?;
    ensure!(
        branch.id == id
            && branch.project_id == project_id
            && row.get::<String, _>("project_id") == project_id.to_string()
            && row.get::<Option<String>, _>("origin_version_id")
                == branch.origin.as_ref().map(|v| v.id.to_string()),
        "Dataset identity does not match its project storage."
    );
    Ok(branch)
}

async fn optional_version(
    database: &mut SqliteConnection,
    project_id: Uuid,
    id: Uuid,
) -> Result<Option<DatasetVersion>> {
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_versions WHERE id=?")
        .bind(id.to_string())
        .fetch_one(&mut *database)
        .await?
        > 0;
    if exists {
        Ok(Some(load_version(database, project_id, id).await?))
    } else {
        Ok(None)
    }
}

/// Lightweight metadata for project navigation. Full row reads and mutations
/// still use `load_version`, which reconstructs the entire version ancestry.
pub(crate) async fn load_reference(
    database: &mut SqliteConnection,
    project_id: Uuid,
    id: Uuid,
) -> Result<DatasetVersionRef> {
    let row = sqlx::query("SELECT v.dataset_id, v.number, v.fingerprint, b.project_id, json_extract(v.metadata_json, '$.id') AS json_id, json_extract(v.metadata_json, '$.datasetId') AS json_dataset, json_extract(v.metadata_json, '$.projectId') AS json_project, json_extract(v.metadata_json, '$.number') AS json_number, json_extract(v.metadata_json, '$.fingerprint') AS json_fingerprint FROM dataset_versions v JOIN dataset_branches b ON b.id=v.dataset_id WHERE v.id=?")
        .bind(id.to_string()).fetch_optional(database).await?.context("Linked dataset version is missing.")?;
    let version = DatasetVersionRef {
        id,
        dataset_id: row.try_get::<String, _>("dataset_id")?.parse()?,
        project_id,
        number: u64::try_from(row.try_get::<i64, _>("number")?)?,
        fingerprint: row.try_get("fingerprint")?,
    };
    version.validate()?;
    ensure!(
        row.try_get::<String, _>("project_id")? == project_id.to_string()
            && row.try_get::<String, _>("json_project")? == project_id.to_string()
            && row.try_get::<String, _>("json_id")? == id.to_string()
            && row.try_get::<String, _>("json_dataset")? == version.dataset_id.to_string()
            && row.try_get::<i64, _>("json_number")? == i64::try_from(version.number)?
            && row.try_get::<String, _>("json_fingerprint")? == version.fingerprint,
        "Linked dataset version metadata changed."
    );
    Ok(version)
}

pub(crate) async fn load_version(
    database: &mut SqliteConnection,
    project_id: Uuid,
    id: Uuid,
) -> Result<DatasetVersion> {
    let mut chain = Vec::new();
    let mut visited = BTreeSet::new();
    let mut current = Some(id);
    while let Some(id) = current {
        ensure!(
            visited.insert(id) && visited.len() <= 10_000,
            "Dataset ancestry contains a cycle or exceeds the inspection limit."
        );
        let row = sqlx::query("SELECT dataset_id, number, parent_version_id, fingerprint, metadata_json FROM dataset_versions WHERE id=?")
            .bind(id.to_string()).fetch_optional(&mut *database).await?.context("Dataset version not found in this project.")?;
        let version: DatasetVersion = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        // The bottom-up replay below validates and reconstructs each complete
        // version. Do not repeat its expensive membership validation here.
        ensure!(
            version.id == id
                && version.project_id == project_id
                && row.get::<String, _>("dataset_id") == version.dataset_id.to_string()
                && row.get::<i64, _>("number") == i64::try_from(version.number)?
                && row.get::<String, _>("fingerprint") == version.fingerprint
                && row.get::<Option<String>, _>("parent_version_id")
                    == version.parent.as_ref().map(|v| v.id.to_string()),
            "Dataset version storage does not match its immutable identity."
        );
        let branch = load_branch(database, project_id, version.dataset_id).await?;
        current = version.parent.as_ref().map(|v| v.id);
        chain.push((branch, version));
    }
    let mut parent = None;
    for (branch, version) in chain.into_iter().rev() {
        version.verify(&branch, parent.as_ref())?;
        parent = Some(version);
    }
    parent.context("Dataset version was not found.")
}
