//! Durable project Agent calls. Native inspection payloads stay in this private
//! execution journal; only bounded public summaries enter the activity stream.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{
    OptimizationError,
    agent::{
        AgentAnalysisScope, AgentCallReservation, AgentTokenUsage, AgentTurnRecord,
        conservative_charge,
    },
    fingerprint,
    ports::{BoxFuture, OptimizationAgentStore},
};
use project_workspace_core::{
    ActivityEventState, ActivityNarrative, ActivityNarrativeKind, ActivityNarrativeOrigin,
    ActivityReference, ActivitySource, OptimizationLaunchAuthorization, ProviderConfiguration,
    ProviderLimits, ProviderRole, optimization_iteration::ProjectOptimizationIteration,
};
use sqlx::{Connection, Row, SqliteConnection};
use sysinfo::{Pid, System};
use uuid::Uuid;

use crate::{
    AppendActivity, append_activity, connect, load_provider_catalog_history, open_workspace,
    optimization_iterations, optimization_launch, optimization_runs,
};

pub struct ProjectAgentStore {
    folder: PathBuf,
    run_id: Uuid,
    action_id: Uuid,
    launch: OptimizationLaunchAuthorization,
    iterations: Vec<ProjectOptimizationIteration>,
    provider: ProviderConfiguration,
}

impl ProjectAgentStore {
    pub async fn open(folder: &Path, run_id: Uuid, action_id: Uuid) -> Result<Self> {
        let workspace = open_workspace(folder, false).await?;
        let run = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|launch| launch.id.to_string() == run.run.launch.id)
            .context("Run launch was not found")?;
        ensure!(
            launch.scope.agentic.is_some(),
            "This historical fixed-recipe run has no Agent execution authorization"
        );
        let iterations = optimization_iterations::list(folder, run_id).await?;
        ensure!(
            !iterations.is_empty(),
            "Bind the iteration's exact development evidence before Agent execution"
        );
        let mut database = connect(Path::new(&workspace.folder), false, false).await?;
        sqlx::migrate!("./migrations").run(&mut database).await?;
        let catalogs = load_provider_catalog_history(&mut database, &workspace.manifest).await?;
        let catalog = catalogs
            .iter()
            .find(|catalog| {
                catalog.id.to_string() == launch.scope.provider_catalog.id
                    && catalog.fingerprint == launch.scope.provider_catalog.fingerprint
            })
            .context("Pinned Agent connection revision is missing")?;
        let provider = catalog
            .provider(ProviderRole::Advisor)
            .context("Pinned Agent model is missing")?
            .clone();
        database.close().await?;
        Ok(Self {
            folder: workspace.folder.into(),
            run_id,
            action_id,
            launch,
            iterations,
            provider,
        })
    }

    pub fn provider(&self) -> &ProviderConfiguration {
        &self.provider
    }

    fn validate_scope(&self, scope: &AgentAnalysisScope) -> Result<()> {
        scope.validate()?;
        let settings = self
            .launch
            .scope
            .agentic
            .as_ref()
            .context("Agent settings missing")?;
        ensure!(
            scope.run_id == self.run_id && scope.launch_fingerprint == self.launch.fingerprint,
            "Agent scope belongs to another run authorization"
        );
        ensure!(
            scope.iteration <= settings.maximum_iterations
                && scope.maximum_turns == settings.maximum_agent_turns_per_iteration
                && scope.maximum_row_changes <= settings.maximum_row_changes
                && scope.objective == settings.objective,
            "Agent scope exceeds its pinned settings"
        );
        ensure!(
            self.iterations
                .iter()
                .any(|iteration| iteration.scope == *scope),
            "Agent dataset requires an exact authorized iteration binding"
        );
        Ok(())
    }

    /// Only an absent exact PID/start-time owner permits recovery. A timeout or
    /// pending row by itself never establishes that a provider call has stopped.
    pub async fn recover_interrupted(&self, scope: &AgentAnalysisScope) -> Result<()> {
        self.validate_scope(scope)?;
        let mut database = connect(&self.folder, false, false).await?;
        let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
        let rows = sqlx::query("SELECT c.metadata_json, c.fingerprint, c.process_id, c.process_started_at FROM optimization_agent_calls c LEFT JOIN optimization_agent_outcomes o ON o.call_id=c.id WHERE c.run_id=? AND c.iteration=? AND o.call_id IS NULL")
            .bind(self.run_id.to_string()).bind(i64::from(scope.iteration)).fetch_all(&mut *transaction).await?;
        let processes = System::new_all();
        for row in rows {
            let pid = u32::try_from(row.get::<i64, _>("process_id"))?;
            let start = u64::try_from(row.get::<i64, _>("process_started_at"))?;
            ensure!(
                !processes
                    .process(Pid::from_u32(pid))
                    .is_some_and(|process| process.start_time() == start),
                "The reserved Agent call still belongs to a live process"
            );
            let call: AgentCallReservation =
                serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
            ensure!(
                fingerprint(&call)? == row.get::<String, _>("fingerprint"),
                "Agent reservation fingerprint changed"
            );
            let record = AgentTurnRecord {
                call,
                explanations: Vec::new(),
                tools: Vec::new(),
                usage: AgentTokenUsage::default(),
                proposal: None,
                interrupted: true,
            };
            insert_outcome(&mut transaction, &record).await?;
        }
        transaction.commit().await?;
        database.close().await?;
        Ok(())
    }
}

impl OptimizationAgentStore for ProjectAgentStore {
    fn history(&self, scope: AgentAnalysisScope) -> BoxFuture<'_, Vec<AgentTurnRecord>> {
        Box::pin(async move {
            self.validate_scope(&scope).map_err(adapter)?;
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let result = read_history(&mut database, self.run_id, Some(scope.iteration))
                .await
                .map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(result
                .into_iter()
                .filter_map(|(_, outcome)| outcome)
                .collect())
        })
    }

    fn reserve(&self, scope: AgentAnalysisScope, call: AgentCallReservation) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.validate_scope(&scope).map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            reserve(&mut transaction, &scope, &call, &self.launch.scope.advisor)
                .await
                .map_err(adapter)?;
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(())
        })
    }

    fn finish(&self, scope: AgentAnalysisScope, record: AgentTurnRecord) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.validate_scope(&scope).map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            let rows = read_history(&mut transaction, scope.run_id, Some(scope.iteration))
                .await
                .map_err(adapter)?;
            let (call, existing) = rows
                .iter()
                .find(|(call, _)| call.id == record.call.id)
                .ok_or_else(|| adapter("Agent call was never reserved"))?;
            if call != &record.call
                || existing
                    .as_ref()
                    .is_some_and(|existing| existing != &record)
            {
                return Err(adapter(
                    "Agent outcome differs from its immutable reservation or previous outcome",
                ));
            }
            if existing.is_none() {
                insert_outcome(&mut transaction, &record)
                    .await
                    .map_err(adapter)?;
            }
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(())
        })
    }

    fn stopped(&self, run_id: Uuid) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            if run_id != self.run_id {
                return Err(adapter("Agent stop query belongs to another run"));
            }
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let stopped = crate::optimization_execution::dispatch_stopped(&mut database, run_id)
                .await
                .map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(stopped)
        })
    }

    fn public_explanation(
        &self,
        scope: AgentAnalysisScope,
        call_id: Uuid,
        text: String,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.validate_scope(&scope).map_err(adapter)?;
            let printable = text.split_whitespace().collect::<Vec<_>>().join(" ");
            // Keep the full response in the private turn record. The existing
            // activity schema carries concise, real model-produced summaries.
            let summary: String = printable.chars().take(400).collect();
            let narrative = ActivityNarrative::new(
                ActivityNarrativeOrigin::Agent,
                ActivityNarrativeKind::Reasoning,
                summary,
            )
            .map_err(adapter)?;
            append_activity(
                &self.folder,
                AppendActivity {
                    action_id: self.action_id,
                    operation: "optimization.agent".into(),
                    source: ActivitySource::Cli,
                    state: ActivityEventState::Progress,
                    stage: Some("agent_analysis".into()),
                    completed: None,
                    total: None,
                    narrative: Some(narrative),
                    references: vec![
                        ActivityReference::new("run", self.run_id.to_string()).map_err(adapter)?,
                        ActivityReference::new("iteration", scope.iteration.to_string())
                            .map_err(adapter)?,
                        ActivityReference::new("provider_call", call_id.to_string())
                            .map_err(adapter)?,
                    ],
                    failure: None,
                    created_at: Utc::now(),
                },
            )
            .await
            .map_err(adapter)?;
            Ok(())
        })
    }
}

async fn reserve(
    database: &mut SqliteConnection,
    scope: &AgentAnalysisScope,
    call: &AgentCallReservation,
    limits: &ProviderLimits,
) -> Result<()> {
    scope.validate()?;
    ensure!(
        !crate::optimization_execution::dispatch_stopped(database, scope.run_id).await?,
        "The optimization run was cancelled or its execution attempt is closed before Agent dispatch"
    );
    ensure!(
        call.scope_fingerprint == scope.fingerprint()?
            && !call.id.is_nil()
            && call.sequence <= scope.maximum_turns
            && call.input_token_ceiling > 0
            && call.output_token_ceiling > 0,
        "Invalid Agent call reservation"
    );
    let history = read_history(database, scope.run_id, None).await?;
    ensure!(
        history.iter().all(|(_, outcome)| outcome.is_some()),
        "Another Agent call is still pending; inspect its process before recovering"
    );
    let scope_history: Vec<_> = history
        .iter()
        .filter(|(previous, _)| previous.scope_fingerprint == call.scope_fingerprint)
        .collect();
    ensure!(
        call.sequence as usize == scope_history.len() + 1
            && scope_history.iter().all(|(_, outcome)| outcome
                .as_ref()
                .is_none_or(|record| record.proposal.is_none())),
        "Agent sequence is stale or the iteration already has a proposal"
    );
    ensure!(
        history.len() < limits.maximum_requests as usize,
        "Agent request budget exhausted"
    );
    let mut input = call.input_token_ceiling;
    let mut output = call.output_token_ceiling;
    let mut cost = call.cost_ceiling_microusd;
    for (previous, outcome) in &history {
        let known = outcome.as_ref();
        let interrupted = known.is_none_or(|record| record.interrupted);
        input = input
            .checked_add(conservative_charge(
                known.and_then(|r| r.usage.input_tokens),
                previous.input_token_ceiling,
                interrupted,
            ))
            .context("Agent input accounting overflow")?;
        output = output
            .checked_add(conservative_charge(
                known.and_then(|r| r.usage.output_tokens),
                previous.output_token_ceiling,
                interrupted,
            ))
            .context("Agent output accounting overflow")?;
        cost = cost
            .checked_add(conservative_charge(
                known.and_then(|r| r.usage.cost_microusd),
                previous.cost_ceiling_microusd,
                interrupted,
            ))
            .context("Agent cost accounting overflow")?;
    }
    ensure!(
        input <= limits.maximum_input_tokens
            && output <= limits.maximum_output_tokens
            && cost <= limits.maximum_cost_microusd,
        "Agent token or spend budget exhausted"
    );
    let scope_fingerprint = scope.fingerprint()?;
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT metadata_json FROM optimization_agent_scopes WHERE run_id=? AND iteration=?",
    )
    .bind(scope.run_id.to_string())
    .bind(i64::from(scope.iteration))
    .fetch_optional(&mut *database)
    .await?;
    if let Some(existing) = existing {
        ensure!(
            serde_json::from_str::<AgentAnalysisScope>(&existing)? == *scope,
            "Agent iteration inputs changed"
        );
    } else {
        sqlx::query("INSERT INTO optimization_agent_scopes (run_id,iteration,fingerprint,metadata_json) VALUES (?,?,?,?)").bind(scope.run_id.to_string()).bind(i64::from(scope.iteration)).bind(&scope_fingerprint).bind(serde_json::to_string(scope)?).execute(&mut *database).await?;
    }
    let pid = std::process::id();
    let system = System::new_all();
    let started = system
        .process(Pid::from_u32(pid))
        .context("Agent worker process identity is unavailable")?
        .start_time();
    sqlx::query("INSERT INTO optimization_agent_calls (id,run_id,iteration,sequence,scope_fingerprint,fingerprint,metadata_json,process_id,process_started_at,created_at) VALUES (?,?,?,?,?,?,?,?,?,?)")
        .bind(call.id.to_string()).bind(scope.run_id.to_string()).bind(i64::from(scope.iteration)).bind(i64::from(call.sequence)).bind(scope_fingerprint).bind(fingerprint(call)?).bind(serde_json::to_string(call)?).bind(i64::from(pid)).bind(i64::try_from(started)?).bind(Utc::now().to_rfc3339()).execute(&mut *database).await?;
    Ok(())
}

pub(crate) async fn read_history(
    database: &mut SqliteConnection,
    run_id: Uuid,
    iteration: Option<u32>,
) -> Result<Vec<(AgentCallReservation, Option<AgentTurnRecord>)>> {
    let rows = sqlx::query("SELECT c.id, c.iteration, c.sequence, c.scope_fingerprint, s.metadata_json AS scope_json, s.fingerprint AS stored_scope_fingerprint, c.metadata_json AS call_json, c.fingerprint AS call_fingerprint, o.metadata_json AS outcome_json, o.fingerprint AS outcome_fingerprint FROM optimization_agent_calls c JOIN optimization_agent_scopes s ON s.run_id=c.run_id AND s.iteration=c.iteration LEFT JOIN optimization_agent_outcomes o ON o.call_id=c.id WHERE c.run_id=? AND (? IS NULL OR c.iteration=?) ORDER BY c.iteration,c.sequence")
        .bind(run_id.to_string()).bind(iteration.map(i64::from)).bind(iteration.map(i64::from)).fetch_all(database).await?;
    rows.into_iter()
        .map(|row| {
            let scope: AgentAnalysisScope =
                serde_json::from_str(&row.get::<String, _>("scope_json"))?;
            scope.validate()?;
            let scope_fingerprint = scope.fingerprint()?;
            ensure!(
                scope.run_id == run_id
                    && i64::from(scope.iteration) == row.get::<i64, _>("iteration")
                    && scope_fingerprint == row.get::<String, _>("stored_scope_fingerprint")
                    && scope_fingerprint == row.get::<String, _>("scope_fingerprint"),
                "Agent scope binding was modified"
            );
            let call: AgentCallReservation =
                serde_json::from_str(&row.get::<String, _>("call_json"))?;
            ensure!(
                fingerprint(&call)? == row.get::<String, _>("call_fingerprint")
                    && call.id.to_string() == row.get::<String, _>("id")
                    && i64::from(call.sequence) == row.get::<i64, _>("sequence")
                    && call.sequence > 0
                    && call.sequence <= scope.maximum_turns
                    && call.scope_fingerprint == scope_fingerprint,
                "Agent reservation was modified"
            );
            let outcome = row
                .get::<Option<String>, _>("outcome_json")
                .map(|raw| serde_json::from_str::<AgentTurnRecord>(&raw))
                .transpose()?;
            if let Some(record) = &outcome {
                record.validate()?;
                ensure!(
                    record.call == call
                        && Some(fingerprint(record)?)
                            == row.get::<Option<String>, _>("outcome_fingerprint"),
                    "Agent outcome was modified"
                );
            }
            Ok((call, outcome))
        })
        .collect()
}

async fn insert_outcome(database: &mut SqliteConnection, record: &AgentTurnRecord) -> Result<()> {
    record.validate()?;
    sqlx::query("INSERT INTO optimization_agent_outcomes (call_id,fingerprint,metadata_json,created_at) VALUES (?,?,?,?)").bind(record.call.id.to_string()).bind(fingerprint(record)?).bind(serde_json::to_string(record)?).bind(Utc::now().to_rfc3339()).execute(database).await?;
    Ok(())
}

fn adapter(error: impl std::fmt::Display) -> OptimizationError {
    OptimizationError::Adapter(error.to_string())
}

#[cfg(test)]
mod tests;
