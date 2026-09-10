use std::{collections::BTreeMap, fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, ProjectAction,
    ProjectActivityEvent, ProjectActivityLog,
};
use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{MANIFEST, canonical_plain, connect, contained, hash, json};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityInitialization {
    pub project_id: Uuid,
    pub created: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendActivity {
    pub action_id: Uuid,
    pub operation: String,
    pub source: ActivitySource,
    pub state: ActivityEventState,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub completed: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub references: Vec<ActivityReference>,
    #[serde(default)]
    pub failure: Option<ActivityFailure>,
    pub created_at: DateTime<Utc>,
}

/// Add only missing project-database migrations after verifying that the
/// immutable manifest still belongs to this database. This deliberately avoids
/// the deep artifact rehash performed by the explicit workspace upgrade.
pub async fn initialize_activity(folder: &Path) -> Result<ActivityInitialization> {
    let root = canonical_plain(folder)?;
    let manifest_path = contained(&root, MANIFEST)?;
    let manifest: project_workspace_core::ProjectManifest = json(&manifest_path)?;
    manifest.validate()?;
    let mut database = connect(&root, false, false).await?;
    verify_workspace_binding(&manifest_path, manifest.id, &mut database).await?;
    let created = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_activity_events'",
    )
    .fetch_one(&mut database)
    .await?
        == 0;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    database.close().await?;
    Ok(ActivityInitialization {
        project_id: manifest.id,
        created,
    })
}

pub async fn append_activity(
    folder: &Path,
    request: AppendActivity,
) -> Result<ProjectActivityEvent> {
    let root = canonical_plain(folder)?;
    let manifest: project_workspace_core::ProjectManifest = json(&contained(&root, MANIFEST)?)?;
    manifest.validate()?;
    let mut database = connect(&root, false, false).await?;
    verify_workspace_binding(&contained(&root, MANIFEST)?, manifest.id, &mut database).await?;
    ensure_activity_schema(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let rows = sqlx::query(
        "SELECT artifact_json FROM project_activity_events WHERE action_id=? ORDER BY sequence",
    )
    .bind(request.action_id.to_string())
    .fetch_all(&mut *transaction)
    .await?;
    let existing = rows
        .into_iter()
        .map(|row| {
            serde_json::from_str::<ProjectActivityEvent>(&row.get::<String, _>("artifact_json"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (sequence, previous) = if existing.is_empty() {
        ensure!(
            request.state == ActivityEventState::Started,
            "An activity action must start before it can advance."
        );
        (1, None)
    } else {
        let action = ProjectAction::replay(existing)?;
        ensure!(
            !action
                .events
                .last()
                .is_some_and(ProjectActivityEvent::is_terminal),
            "This activity action is already complete."
        );
        ensure!(
            request.state != ActivityEventState::Started
                && action.operation == request.operation
                && action.source == request.source
                && action.project_id == manifest.id,
            "Activity continuation does not match its start event."
        );
        let last = action.events.last().expect("replayed nonempty action");
        (last.sequence + 1, Some(last.fingerprint.clone()))
    };
    let event = ProjectActivityEvent::create(
        request.action_id,
        manifest.id,
        sequence,
        request.operation,
        request.source,
        request.state,
        request.stage,
        request.completed,
        request.total,
        request.references,
        request.failure,
        request.created_at,
        previous,
    )?;
    sqlx::query(
        "INSERT INTO project_activity_events (id, action_id, project_id, sequence, previous_event_fingerprint, fingerprint, operation, state, artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(event.id.to_string())
    .bind(event.action_id.to_string())
    .bind(event.project_id.to_string())
    .bind(i64::from(event.sequence))
    .bind(event.previous_event_fingerprint.as_deref())
    .bind(&event.fingerprint)
    .bind(&event.operation)
    .bind(state_name(event.state))
    .bind(serde_json::to_string(&event)?)
    .bind(event.created_at.to_rfc3339())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    database.close().await?;
    Ok(event)
}

pub async fn read_activity(folder: &Path, limit: usize) -> Result<ProjectActivityLog> {
    ensure!(
        (1..=10_000).contains(&limit),
        "Activity limit must be between 1 and 10000."
    );
    let root = canonical_plain(folder)?;
    let manifest: project_workspace_core::ProjectManifest = json(&contained(&root, MANIFEST)?)?;
    manifest.validate()?;
    let mut database = connect(&root, true, false).await?;
    verify_workspace_binding(&contained(&root, MANIFEST)?, manifest.id, &mut database).await?;
    ensure_activity_schema(&mut database).await?;
    let rows = sqlx::query("SELECT artifact_json FROM project_activity_events ORDER BY rowid")
        .fetch_all(&mut database)
        .await?;
    database.close().await?;
    let mut grouped: BTreeMap<Uuid, Vec<ProjectActivityEvent>> = BTreeMap::new();
    let mut order = vec![];
    for row in rows {
        let event: ProjectActivityEvent =
            serde_json::from_str(&row.get::<String, _>("artifact_json"))?;
        if !grouped.contains_key(&event.action_id) {
            order.push(event.action_id);
        }
        grouped.entry(event.action_id).or_default().push(event);
    }
    let mut actions = order
        .into_iter()
        .map(|id| ProjectAction::replay(grouped.remove(&id).expect("known action")))
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        actions
            .iter()
            .all(|action| action.project_id == manifest.id),
        "Activity log contains another project's action."
    );
    actions.reverse();
    actions.truncate(limit);
    Ok(ProjectActivityLog {
        project_id: manifest.id,
        actions,
    })
}

pub async fn read_action(folder: &Path, action_id: Uuid) -> Result<ProjectAction> {
    read_activity(folder, 10_000)
        .await?
        .actions
        .into_iter()
        .find(|action| action.action_id == action_id)
        .with_context(|| format!("Activity action not found: {action_id}"))
}

pub async fn export_activity(folder: &Path, output: &Path) -> Result<u64> {
    let log = read_activity(folder, 10_000).await?;
    let output = std::path::absolute(output)?;
    let parent = output
        .parent()
        .context("Activity export requires an existing destination folder.")?;
    canonical_plain(parent)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&output)?;
    let mut count = 0;
    for action in log.actions.iter().rev() {
        for event in &action.events {
            serde_json::to_writer(&mut file, event)?;
            file.write_all(b"\n")?;
            count += 1;
        }
    }
    file.sync_all()?;
    Ok(count)
}

async fn ensure_activity_schema(database: &mut sqlx::SqliteConnection) -> Result<()> {
    ensure!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_activity_events'").fetch_one(database).await? == 1, "Upgrade this managed workspace before recording project activity.");
    Ok(())
}

async fn verify_workspace_binding(
    manifest_path: &Path,
    project_id: Uuid,
    database: &mut sqlx::SqliteConnection,
) -> Result<()> {
    let binding =
        sqlx::query("SELECT project_id, manifest_sha256 FROM workspace_identity WHERE singleton=1")
            .fetch_one(database)
            .await
            .context("Invalid workspace database.")?;
    ensure!(
        binding.get::<String, _>("project_id") == project_id.to_string()
            && binding.get::<String, _>("manifest_sha256")
                == hash(manifest_path, MANIFEST)?.fingerprint,
        "Project manifest and database do not match. Restore the matching project files."
    );
    Ok(())
}

fn state_name(state: ActivityEventState) -> &'static str {
    match state {
        ActivityEventState::Started => "started",
        ActivityEventState::Progress => "progress",
        ActivityEventState::Succeeded => "succeeded",
        ActivityEventState::Failed => "failed",
    }
}
