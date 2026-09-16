//! Separate post-loop consent. No provider, training or evaluation dispatch.
//! Heap-backed entry points keep deep journal replay out of callers' async frames.
use crate::{
    connect,
    optimization_completions::{self, IterationScientificEvidence},
    optimization_iteration_execution, optimization_iterations, optimization_launch,
    optimization_runs,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::optimization_final::{
    AgentFinalAuthorization, AgentFinalScope, SelectedFinalEvidence,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalAuthorizationRequest {
    pub id: Uuid,
    pub scope: AgentFinalScope,
}

/// Read-only, including on project databases predating final consent.
/// Callers supply every completed iteration's original scientific journal from
/// the pinned store, after checking its unchanged native benchmark definition.
pub fn preview<'a>(
    folder: &'a Path,
    run_id: Uuid,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<AgentFinalScope>> + 'a {
    Box::pin(async move {
        let view = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|launch| launch.id.to_string() == view.run.launch.id)
            .context("Launch missing")?;
        let latest = optimization_completions::list(folder, run_id)
            .await?
            .pop()
            .context("No completed iteration is available for final holdout")?;
        let completion =
            optimization_completions::verify_completed(folder, run_id, latest.number, scientific)
                .await?;
        let selected = completion
            .selected
            .as_ref()
            .context("No development-eligible candidate was selected")?;
        let iteration = optimization_iterations::list(folder, run_id)
            .await?
            .into_iter()
            .find(|iteration| iteration.id.to_string() == selected.iteration.id)
            .context("Selected iteration missing")?;
        let training = optimization_iteration_execution::training(folder, run_id, iteration.id)
            .await?
            .context("Selected training receipt missing")?;
        let source = scientific
            .iter()
            .find(|source| source.iteration_id == iteration.id)
            .context("Selected scientific evidence missing")?;
        let scope = AgentFinalScope::bind(SelectedFinalEvidence {
            run: &view,
            launch: &launch,
            completion: &completion,
            iteration: &iteration,
            training: &training,
            project: &source.project,
            protocol: &source.protocol,
            events: &source.events,
        })?;
        ensure!(
            optimization_runs::show(folder, run_id).await? == view,
            "Run changed while previewing final holdout"
        );
        Ok(scope)
    })
}

/// Read and deeply revalidate an existing grant. Absence remains read-only.
pub fn show<'a>(
    folder: &'a Path,
    run_id: Uuid,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<Option<AgentFinalAuthorization>>> + 'a {
    Box::pin(async move {
        optimization_runs::show(folder, run_id).await?;
        let mut db = connect(folder, true, false).await?;
        let value = read(&mut db, run_id).await?;
        db.close().await?;
        if let Some(value) = &value {
            ensure!(
                value.scope == preview(folder, run_id, scientific).await?,
                "Final authority differs from the original completed run"
            );
        }
        Ok(value)
    })
}

/// Persist one explicit grant, without using it. Exact retries return the same
/// grant; even a new UUID cannot authorize a second holdout for this run.
pub fn authorize<'a>(
    folder: &'a Path,
    run_id: Uuid,
    request: FinalAuthorizationRequest,
    authorized_by: &'a str,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<AgentFinalAuthorization>> + 'a {
    Box::pin(async move {
        ensure!(
            !request.id.is_nil(),
            "Final authorization retry ID cannot be nil"
        );
        request.scope.validate_identity()?;
        project_workspace_core::validate_name(authorized_by)?;
        let scope = preview(folder, run_id, scientific).await?;
        ensure!(
            request.scope == scope,
            "Final holdout scope changed since preview"
        );
        let mut db = connect(folder, false, false).await?;
        // SQLite's migrator lock is a no-op. Serialize schema inspection too:
        // two first consent requests may both arrive on the pre-consent schema.
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::migrate!("./migrations").run(&mut *tx).await?;
        let head: String = sqlx::query_scalar("SELECT fingerprint FROM optimization_agent_execution_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1")
        .bind(run_id.to_string()).fetch_one(&mut *tx).await?;
        let completions = optimization_completions::read(&mut tx, run_id).await?;
        ensure!(
            head == scope.execution_head
                && completions
                    .last()
                    .is_some_and(|last| last.identity() == scope.completion),
            "Adaptive work changed before final authorization"
        );
        let value = if let Some(existing) = read(&mut tx, run_id).await? {
            ensure!(
                existing.id == request.id
                    && existing.scope == scope
                    && existing.authorized_by == authorized_by,
                "This run already has a different final authorization"
            );
            existing
        } else {
            let value = AgentFinalAuthorization::create(
                request.id,
                scope,
                authorized_by.into(),
                Utc::now(),
            )?;
            sqlx::query("INSERT INTO optimization_agent_final_authorizations(id,run_id,launch_id,completion_id,iteration_id,scope_fingerprint,fingerprint,metadata_json) VALUES(?,?,?,?,?,?,?,?)")
            .bind(value.id.to_string()).bind(run_id.to_string()).bind(&value.scope.launch.id)
            .bind(&value.scope.completion.id).bind(&value.scope.iteration.id)
            .bind(&value.scope.fingerprint).bind(&value.fingerprint)
            .bind(serde_json::to_string(&value)?).execute(&mut *tx).await?;
            value
        };
        tx.commit().await?;
        db.close().await?;
        Ok(value)
    })
}

async fn read(db: &mut SqliteConnection, run_id: Uuid) -> Result<Option<AgentFinalAuthorization>> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_agent_final_authorizations'")
        .fetch_one(&mut *db).await?;
    if exists == 0 {
        return Ok(None);
    }
    let row = sqlx::query("SELECT id,launch_id,completion_id,iteration_id,scope_fingerprint,fingerprint,metadata_json FROM optimization_agent_final_authorizations WHERE run_id=?")
        .bind(run_id.to_string()).fetch_optional(&mut *db).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let value: AgentFinalAuthorization =
        serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
    value.validate_identity()?;
    ensure!(
        value.id.to_string() == row.get::<String, _>("id")
            && value.scope.run.id == run_id.to_string()
            && value.scope.launch.id == row.get::<String, _>("launch_id")
            && value.scope.completion.id == row.get::<String, _>("completion_id")
            && value.scope.iteration.id == row.get::<String, _>("iteration_id")
            && value.scope.fingerprint == row.get::<String, _>("scope_fingerprint")
            && value.fingerprint == row.get::<String, _>("fingerprint"),
        "Final authorization storage changed"
    );
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_composition_keeps_bounded_async_frames() {
        fn size<Argument, Future>(_: impl FnOnce(Argument) -> Future) -> usize {
            std::mem::size_of::<Future>()
        }
        let folder = Path::new("unused");
        for (name, bytes) in [
            ("preview", size(|()| preview(folder, Uuid::nil(), &[]))),
            ("show", size(|()| show(folder, Uuid::nil(), &[]))),
            (
                "authorize",
                size(|request| authorize(folder, Uuid::nil(), request, "operator", &[])),
            ),
        ] {
            assert!(bytes < 128, "{name} composes a {bytes}-byte async frame");
        }
    }
}
