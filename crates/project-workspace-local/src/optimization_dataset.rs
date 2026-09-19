//! Compose recorded Agent edits and completed native generation admission with
//! ordinary managed imports and Dataset Management. This publishes a dataset,
//! not training/benchmark qualification or permission to train.

use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use dataset_core::versions::DatasetVersionRef;
use encoder_optimization_core::{agent::AgentAnalysisScope, fingerprint};
use project_workspace_core::DatasetPurpose;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{
    connect,
    dataset_versions::{self, DatasetVersionUpdate, RecordSelection},
    datasets, files, open_workspace, optimization_agent, optimization_generation,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedRowLink {
    pub call_id: Uuid,
    pub task_id: Uuid,
    pub row_index: u32,
    pub content_fingerprint: String,
    /// Empty only for a rejected cross-batch duplicate, never an admitted row.
    pub source_record: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationDatasetPublication {
    pub run_id: Uuid,
    pub iteration: u32,
    pub proposal_fingerprint: String,
    pub parent: DatasetVersionRef,
    pub version: DatasetVersionRef,
    pub import_id: Option<Uuid>,
    pub removed: Vec<String>,
    pub generated: Vec<GeneratedRowLink>,
    pub generation_rejections: u64,
    pub cross_batch_duplicates: u64,
}

/// Read the exact published edit result without generating or publishing work.
pub async fn publication(
    folder: &Path,
    run_id: Uuid,
    iteration: u32,
) -> Result<OptimizationDatasetPublication> {
    let mut database = connect(folder, true, false).await?;
    let value = read_publication(&mut database, run_id, iteration).await?;
    database.close().await?;
    value.context("Published dataset record is missing")
}

pub(crate) async fn read_publication(
    database: &mut sqlx::SqliteConnection,
    run_id: Uuid,
    iteration: u32,
) -> Result<Option<OptimizationDatasetPublication>> {
    let row = sqlx::query("SELECT metadata_json,fingerprint,version_id FROM optimization_dataset_publications WHERE run_id=? AND iteration=?")
        .bind(run_id.to_string()).bind(i64::from(iteration)).fetch_optional(database).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let value: OptimizationDatasetPublication =
        serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
    ensure!(
        value.run_id == run_id
            && value.iteration == iteration
            && value.version.id.to_string() == row.get::<String, _>("version_id")
            && fingerprint(&value)? == row.get::<String, _>("fingerprint"),
        "Published dataset record changed"
    );
    Ok(Some(value))
}

pub async fn publish(
    folder: &Path,
    run_id: Uuid,
    iteration: u32,
) -> Result<OptimizationDatasetPublication> {
    let workspace = open_workspace(folder, false).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let raw=sqlx::query("SELECT metadata_json,fingerprint FROM optimization_agent_scopes WHERE run_id=? AND iteration=?").bind(run_id.to_string()).bind(i64::from(iteration)).fetch_one(&mut database).await?;
    let scope: AgentAnalysisScope = serde_json::from_str(&raw.get::<String, _>("metadata_json"))?;
    ensure!(
        scope.run_id == run_id
            && scope.iteration == iteration
            && scope.fingerprint()? == raw.get::<String, _>("fingerprint"),
        "Dataset publication scope changed"
    );
    let calls = optimization_agent::read_history(&mut database, run_id, Some(iteration)).await?;
    let proposals: Vec<_> = calls
        .iter()
        .filter_map(|(_, record)| record.as_ref())
        .filter(|record| !record.interrupted)
        .filter_map(|record| record.proposal.as_ref())
        .collect();
    ensure!(
        proposals.len() == 1,
        "Dataset publication requires one accepted Agent proposal"
    );
    let proposal = proposals[0];
    ensure!(
        !proposal.stop,
        "A no-change decision has no derived dataset to publish"
    );
    let proposal_fingerprint = fingerprint(proposal)?;
    let generated_calls = optimization_generation::read_calls(&mut database, run_id).await?;
    database.close().await?;
    let run = crate::optimization_runs::show(folder, run_id).await?;
    let launches = crate::optimization_launch::list(folder).await?;
    let launch = launches
        .iter()
        .find(|launch| launch.id.to_string() == run.run.launch.id)
        .context("Dataset publication launch is missing")?;
    run.run.validate(launch)?;
    ensure!(
        scope.launch_fingerprint == launch.fingerprint,
        "Dataset publication launch changed"
    );
    if launch
        .scope
        .agentic
        .as_ref()
        .and_then(|settings| settings.generation_canary)
        .is_some()
        && !proposal.additions.is_empty()
    {
        let (task, _, outcome) = generated_calls
            .iter()
            .filter(|(task, _, _)| {
                task.iteration == iteration
                    && task.target_index == 0
                    && task.first_row == 0
                    && task.proposal_fingerprint == proposal_fingerprint
            })
            .max_by_key(|(_, call, _)| call.attempt)
            .context("Generation canary has no recorded result")?;
        ensure!(
            outcome.as_ref().is_some_and(|outcome| {
                encoder_optimization_core::generation::canary_passed(task, outcome).unwrap_or(false)
            }),
            "Dataset publication requires a passed generation canary"
        );
    }
    let parent = dataset_versions::inspect(folder, scope.dataset_version_id).await?;
    ensure!(
        parent.fingerprint == scope.dataset_fingerprint,
        "Dataset publication parent changed"
    );
    let mut completed = Vec::new();
    for (target_index, target) in proposal.additions.iter().enumerate() {
        for first_row in (0..target.count).step_by(8) {
            let matching: Vec<_> = generated_calls
                .iter()
                .filter(|(task, _, _)| {
                    task.iteration == iteration
                        && task.proposal_fingerprint == proposal_fingerprint
                        && task.target_index == target_index as u32
                        && task.first_row == first_row
                })
                .collect();
            ensure!(
                !matching.is_empty(),
                "Generation is incomplete; a requested slot has no reservation"
            );
            let task_id = matching[0].0.id;
            ensure!(
                matching.iter().all(|(task, _, _)| task.id == task_id
                    && task.template_row_id == target.template_row_id
                    && task.requested_rows == (target.count - first_row).min(8)),
                "Generation slot does not match its proposal"
            );
            let admitted: Vec<_> = matching
                .iter()
                .filter_map(|(task, call, outcome)| {
                    outcome
                        .as_ref()
                        .filter(|v| !v.interrupted)
                        .map(|outcome| (task, call, outcome))
                })
                .collect();
            ensure!(
                admitted.len() == 1,
                "Generation is incomplete or the same slot completed twice"
            );
            completed.push(admitted[0]);
        }
    }
    let mut rows = Vec::new();
    let mut generated = Vec::new();
    let mut seen = BTreeSet::new();
    let mut generation_rejections = 0;
    let mut cross_batch_duplicates = 0;
    // Proposal/slot order, not response completion order, selects the survivor.
    for (task, call, outcome) in completed {
        let admission = outcome
            .admission
            .as_ref()
            .context("Generation admission is missing")?;
        generation_rejections += admission.rejected.len() as u64;
        for row in &admission.accepted {
            let record = if seen.insert(row.deduplication_fingerprint.clone()) {
                rows.push(row.content.clone());
                Some(rows.len() as u64)
            } else {
                cross_batch_duplicates += 1;
                None
            };
            generated.push(GeneratedRowLink {
                call_id: call.id,
                task_id: task.id,
                row_index: row.index,
                content_fingerprint: row.fingerprint.clone(),
                source_record: record,
            });
        }
    }
    ensure!(
        !rows.is_empty() || !proposal.removals.is_empty(),
        "Every generated row was rejected; no dataset change can be published"
    );
    let removed = proposal
        .removals
        .iter()
        .map(|row| row.row_id.clone())
        .collect::<Vec<_>>();
    let (version, import_id) =
        publish_version(folder, &scope, &proposal_fingerprint, &rows, &removed).await?;
    let publication = OptimizationDatasetPublication {
        run_id,
        iteration,
        proposal_fingerprint,
        parent: parent.reference(),
        version,
        import_id,
        removed,
        generated,
        generation_rejections,
        cross_batch_duplicates,
    };
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let stored=sqlx::query("SELECT metadata_json,fingerprint,version_id FROM optimization_dataset_publications WHERE run_id=? AND iteration=?").bind(run_id.to_string()).bind(i64::from(iteration)).fetch_optional(&mut *transaction).await?;
    if let Some(stored) = stored {
        ensure!(
            serde_json::from_str::<OptimizationDatasetPublication>(
                &stored.get::<String, _>("metadata_json")
            )? == publication
                && stored.get::<String, _>("fingerprint") == fingerprint(&publication)?
                && stored.get::<String, _>("version_id") == publication.version.id.to_string(),
            "Published dataset receipt changed"
        );
    } else {
        sqlx::query("INSERT INTO optimization_dataset_publications(run_id,iteration,version_id,fingerprint,metadata_json) VALUES(?,?,?,?,?)").bind(run_id.to_string()).bind(i64::from(iteration)).bind(publication.version.id.to_string()).bind(fingerprint(&publication)?).bind(serde_json::to_string(&publication)?).execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    database.close().await?;
    Ok(publication)
}

async fn publish_version(
    folder: &Path,
    scope: &AgentAnalysisScope,
    proposal_fingerprint: &str,
    rows: &[serde_json::Value],
    removed: &[String],
) -> Result<(DatasetVersionRef, Option<Uuid>)> {
    ensure_running(folder, scope.run_id).await?;
    let workspace = open_workspace(folder, false).await?;
    let parent = dataset_versions::inspect(folder, scope.dataset_version_id).await?;
    ensure!(
        parent.fingerprint == scope.dataset_fingerprint,
        "Dataset parent changed before publication"
    );
    let run_id = scope.run_id;
    let iteration = scope.iteration;
    let seed = (run_id, iteration, proposal_fingerprint);
    let identity = |name: &str| -> Result<Uuid> {
        Ok(encoder_optimization_core::child_id(&(
            "agent-dataset-publication-v1",
            &seed,
            name,
        ))?)
    };
    let branch_id = identity("branch")?;
    let fork_id = identity("fork")?;
    let version_id = identity("version")?;
    let import_id = if rows.is_empty() {
        None
    } else {
        let imports = files::contained(Path::new(&workspace.folder), "datasets/imports")?;
        let scratch = tempfile::Builder::new()
            .prefix(".agent-rows-")
            .tempdir_in(imports)?;
        let source = scratch.path().join("data.jsonl");
        let mut writer = BufWriter::new(File::create(&source)?);
        for row in rows {
            serde_json::to_writer(&mut writer, row)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        let preview = datasets::inspect_dataset(&source, DatasetPurpose::Training)?;
        datasets::persist_import(
            &workspace,
            &preview,
            &format!("Agent {} iteration {iteration}", &run_id.to_string()[..8]),
            DatasetPurpose::Training,
            None,
        )
        .await?;
        let current = open_workspace(folder, false).await?;
        Some(
            current
                .datasets
                .iter()
                .find(|d| {
                    d.artifact.fingerprint == preview.artifact.fingerprint
                        && d.purpose == DatasetPurpose::Training
                })
                .context("Generated import was not recorded")?
                .id,
        )
    };
    ensure_running(folder, run_id).await?;
    dataset_versions::fork(
        folder,
        branch_id,
        fork_id,
        &format!("Run {} · iteration {iteration}", &run_id.to_string()[..8]),
        parent.id,
    )
    .await?;
    let added = import_id
        .map(|import_id| {
            (1..=rows.len() as u64)
                .map(|record| RecordSelection { import_id, record })
                .collect()
        })
        .unwrap_or_default();
    ensure_running(folder, run_id).await?;
    let version = dataset_versions::revise(
        folder,
        DatasetVersionUpdate {
            version_id,
            dataset_id: branch_id,
            parent_id: fork_id,
            added,
            removed: removed.to_vec(),
            replaced: vec![],
        },
    )
    .await?;
    Ok((version.reference(), import_id))
}

async fn ensure_running(folder: &Path, run_id: Uuid) -> Result<()> {
    let mut db = connect(folder, true, false).await?;
    let kind:Option<String>=sqlx::query_scalar("SELECT kind FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1").bind(run_id.to_string()).fetch_optional(&mut db).await?;
    db.close().await?;
    ensure!(
        kind.as_deref() != Some("cancelled"),
        "Run stopped before dataset publication; completed generation is retained"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
