//! Project-bound generation reservations and native admission receipts. Budget
//! accounting includes pending/unknown calls across every iteration of the run.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{
    OptimizationError,
    agent::{AgentTokenUsage, conservative_charge},
    fingerprint,
    generation::{GenerationOutcome, GenerationReservation, GenerationTask},
    ports::{BoxFuture, OptimizationGenerationStore},
};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityNarrative, ActivityNarrativeKind,
    ActivityNarrativeOrigin, ActivityReference, ActivitySource, OptimizationLaunchAuthorization,
    ProviderConfiguration, ProviderLimits, ProviderRole,
};
use sqlx::{Connection, Row, SqliteConnection};
use sysinfo::{Pid, System};
use uuid::Uuid;

use crate::{
    AppendActivity, append_activity, connect, load_provider_catalog_history, open_workspace,
    optimization_agent, optimization_launch, optimization_runs,
};

pub struct ProjectGenerationStore {
    folder: PathBuf,
    run_id: Uuid,
    launch: OptimizationLaunchAuthorization,
    provider: ProviderConfiguration,
    record_activity: bool,
}

impl ProjectGenerationStore {
    pub async fn open(folder: &Path, run_id: Uuid) -> Result<Self> {
        let workspace = open_workspace(folder, false).await?;
        let run = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|v| v.id.to_string() == run.run.launch.id)
            .context("Run launch is missing")?;
        ensure!(
            launch.scope.agentic.is_some(),
            "Run has no Agent/generator execution authority"
        );
        let mut database = connect(Path::new(&workspace.folder), false, false).await?;
        sqlx::migrate!("./migrations").run(&mut database).await?;
        let catalogs = load_provider_catalog_history(&mut database, &workspace.manifest).await?;
        let catalog = catalogs
            .iter()
            .find(|v| {
                v.id.to_string() == launch.scope.provider_catalog.id
                    && v.fingerprint == launch.scope.provider_catalog.fingerprint
            })
            .context("Pinned generator connection revision is missing")?;
        let provider = catalog
            .provider(ProviderRole::Generation)
            .context("Pinned data generation model is missing")?
            .clone();
        database.close().await?;
        Ok(Self {
            folder: workspace.folder.into(),
            run_id,
            launch,
            provider,
            record_activity: false,
        })
    }

    /// The caller initializes project activity before opting into live events.
    pub fn with_activity(mut self) -> Self {
        self.record_activity = true;
        self
    }

    async fn activity(
        &self,
        task: &GenerationTask,
        call: &GenerationReservation,
        state: ActivityEventState,
    ) -> Result<()> {
        if !self.record_activity {
            return Ok(());
        }
        let progress = state == ActivityEventState::Progress;
        let failed = state == ActivityEventState::Failed;
        append_activity(
            &self.folder,
            AppendActivity {
                action_id: call.id,
                operation: "optimization.generation".into(),
                source: ActivitySource::Cli,
                state,
                stage: progress.then(|| "data_generation".into()),
                completed: None,
                total: None,
                narrative: if progress {
                    Some(ActivityNarrative::new(
                        ActivityNarrativeOrigin::Generation,
                        ActivityNarrativeKind::Intent,
                        "Generating questions for the Agent's selected training context.",
                    )?)
                } else {
                    None
                },
                references: vec![
                    ActivityReference::new("run", task.run_id.to_string())?,
                    ActivityReference::new("iteration", task.iteration.to_string())?,
                    ActivityReference::new("provider_call", call.id.to_string())?,
                    ActivityReference::new("generation_task", task.id.to_string())?,
                    ActivityReference::new("training_row", task.template_row_id.clone())?,
                ],
                failure: if failed {
                    Some(ActivityFailure::new(
                        "generation_interrupted",
                        "Generation did not finish; its reserved usage remains charged.",
                    )?)
                } else {
                    None
                },
                created_at: Utc::now(),
            },
        )
        .await?;
        Ok(())
    }

    pub fn provider(&self) -> &ProviderConfiguration {
        &self.provider
    }

    /// Recover only reservations whose exact owning process is no longer alive.
    /// Recovery does not dispatch a replacement or silently release its budget.
    pub async fn recover_interrupted(&self) -> Result<()> {
        let mut database = connect(&self.folder, false, false).await?;
        let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
        let rows = read_calls(&mut transaction, self.run_id).await?;
        let processes = System::new_all();
        for (task, call, outcome) in rows {
            if outcome.is_some() {
                continue;
            }
            let owner=sqlx::query("SELECT process_id,process_started_at FROM optimization_generation_calls WHERE id=?").bind(call.id.to_string()).fetch_one(&mut *transaction).await?;
            let pid = u32::try_from(owner.get::<i64, _>("process_id"))?;
            let started = u64::try_from(owner.get::<i64, _>("process_started_at"))?;
            ensure!(
                !processes
                    .process(Pid::from_u32(pid))
                    .is_some_and(|p| p.start_time() == started),
                "Generation request still belongs to a live process"
            );
            let outcome = GenerationOutcome {
                reservation: call,
                usage: AgentTokenUsage::default(),
                admission: None,
                interrupted: true,
            };
            insert_outcome(&mut transaction, &task, &outcome).await?;
        }
        transaction.commit().await?;
        database.close().await?;
        Ok(())
    }

    fn validate_task(&self, task: &GenerationTask) -> Result<()> {
        task.validate()?;
        let settings = self
            .launch
            .scope
            .agentic
            .as_ref()
            .context("Agent settings missing")?;
        ensure!(
            task.run_id == self.run_id && task.iteration <= settings.maximum_iterations,
            "Generation task belongs to another run or iteration"
        );
        Ok(())
    }
}

impl OptimizationGenerationStore for ProjectGenerationStore {
    fn history(&self, task: GenerationTask) -> BoxFuture<'_, Vec<GenerationOutcome>> {
        Box::pin(async move {
            self.validate_task(&task).map_err(adapter)?;
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let rows = read_calls(&mut database, self.run_id)
                .await
                .map_err(adapter)?;
            let mut history = Vec::new();
            for (stored, _, outcome) in rows
                .into_iter()
                .filter(|(stored, _, _)| stored.id == task.id)
            {
                if stored != task {
                    return Err(adapter("Generation task changed on resume"));
                }
                if let Some(outcome) = outcome {
                    history.push(outcome)
                }
            }
            database.close().await.map_err(adapter)?;
            Ok(history)
        })
    }

    fn reserve(&self, task: GenerationTask, call: GenerationReservation) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.validate_task(&task).map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            let concurrency = self
                .launch
                .scope
                .agentic
                .as_ref()
                .ok_or_else(|| adapter("Agent settings missing"))?
                .generation_concurrency;
            reserve(
                &mut transaction,
                &task,
                &call,
                &self.launch.scope.generation,
                concurrency,
            )
            .await
            .map_err(adapter)?;
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            self.activity(&task, &call, ActivityEventState::Started)
                .await
                .map_err(adapter)?;
            self.activity(&task, &call, ActivityEventState::Progress)
                .await
                .map_err(adapter)?;
            Ok(())
        })
    }

    fn finish(&self, task: GenerationTask, outcome: GenerationOutcome) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.validate_task(&task).map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            insert_outcome(&mut transaction, &task, &outcome)
                .await
                .map_err(adapter)?;
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            self.activity(
                &task,
                &outcome.reservation,
                if outcome.interrupted {
                    ActivityEventState::Failed
                } else {
                    ActivityEventState::Succeeded
                },
            )
            .await
            .map_err(adapter)?;
            Ok(())
        })
    }

    fn stopped(&self, run_id: Uuid) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            if run_id != self.run_id {
                return Err(adapter("Generation stop check belongs to another run"));
            }
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let stopped = is_stopped(&mut database, run_id).await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(stopped)
        })
    }
}

async fn is_stopped(database: &mut SqliteConnection, run: Uuid) -> Result<bool> {
    let latest:Option<String>=sqlx::query_scalar("SELECT kind FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1").bind(run.to_string()).fetch_optional(database).await?;
    Ok(latest.as_deref() == Some("cancelled"))
}

async fn reserve(
    database: &mut SqliteConnection,
    task: &GenerationTask,
    call: &GenerationReservation,
    limits: &ProviderLimits,
    concurrency: u32,
) -> Result<()> {
    task.validate()?;
    ensure!(
        !is_stopped(database, task.run_id).await?,
        "Run was stopped before generation dispatch"
    );
    ensure!(
        !call.id.is_nil()
            && call.task_id == task.id
            && call.task_fingerprint == task.fingerprint()?
            && call.attempt > 0
            && call.input_token_ceiling > 0
            && call.output_token_ceiling == u64::from(task.request.maximum_output_tokens),
        "Invalid generation reservation"
    );
    ensure!(
        (1..=16).contains(&concurrency),
        "Invalid generation concurrency"
    );
    let agent =
        optimization_agent::read_history(database, task.run_id, Some(task.iteration)).await?;
    let proposal = agent
        .iter()
        .filter_map(|(_, record)| record.as_ref())
        .filter(|record| !record.interrupted)
        .filter_map(|record| record.proposal.as_ref())
        .find(|proposal| fingerprint(proposal).ok().as_ref() == Some(&task.proposal_fingerprint))
        .context("Generation has no accepted, recorded Agent proposal")?;
    let target = proposal
        .additions
        .get(task.target_index as usize)
        .context("Generation target is not in the Agent proposal")?;
    ensure!(
        task.template_row_id == target.template_row_id
            && task.first_row % 8 == 0
            && task.first_row < target.count
            && task.requested_rows == (target.count - task.first_row).min(8),
        "Generation range does not match the Agent proposal"
    );
    let history = read_calls(database, task.run_id).await?;
    let task_history: Vec<_> = history
        .iter()
        .filter(|(stored, _, _)| stored.id == task.id)
        .collect();
    ensure!(
        task_history
            .iter()
            .all(|(stored, _, outcome)| stored == task
                && outcome.as_ref().is_some_and(|v| v.interrupted))
            && call.attempt as usize == task_history.len() + 1,
        "Generation slot is pending, completed, changed or stale"
    );
    ensure!(
        history
            .iter()
            .all(|(stored, _, _)| stored.iteration != task.iteration
                || stored.target_index != task.target_index
                || stored.first_row != task.first_row
                || stored.id == task.id),
        "Generation slot already has another identity"
    );
    ensure!(
        history
            .iter()
            .filter(|(_, _, outcome)| outcome.is_none())
            .count()
            < (concurrency as usize),
        "Generation concurrency limit reached"
    );
    ensure!(
        history.len() < (limits.maximum_requests as usize),
        "Generation request budget exhausted"
    );
    let mut input = call.input_token_ceiling;
    let mut output = call.output_token_ceiling;
    let mut cost = call.cost_ceiling_microusd;
    for (_, previous, outcome) in &history {
        let known = outcome.as_ref();
        let interrupted = known.is_none_or(|record| record.interrupted);
        input = input
            .checked_add(conservative_charge(
                known.and_then(|v| v.usage.input_tokens),
                previous.input_token_ceiling,
                interrupted,
            ))
            .context("Generation input accounting overflow")?;
        output = output
            .checked_add(conservative_charge(
                known.and_then(|v| v.usage.output_tokens),
                previous.output_token_ceiling,
                interrupted,
            ))
            .context("Generation output accounting overflow")?;
        cost = cost
            .checked_add(conservative_charge(
                known.and_then(|v| v.usage.cost_microusd),
                previous.cost_ceiling_microusd,
                interrupted,
            ))
            .context("Generation spend accounting overflow")?;
    }
    ensure!(
        input <= limits.maximum_input_tokens
            && output <= limits.maximum_output_tokens
            && cost <= limits.maximum_cost_microusd,
        "Generation token or spend budget exhausted"
    );
    if task_history.is_empty() {
        sqlx::query("INSERT INTO optimization_generation_tasks(id,run_id,iteration,fingerprint,metadata_json) VALUES(?,?,?,?,?)").bind(task.id.to_string()).bind(task.run_id.to_string()).bind(i64::from(task.iteration)).bind(task.fingerprint()?).bind(serde_json::to_string(task)?).execute(&mut *database).await?;
    }
    let pid = std::process::id();
    let processes = System::new_all();
    let start = processes
        .process(Pid::from_u32(pid))
        .context("Generator process identity unavailable")?
        .start_time();
    sqlx::query("INSERT INTO optimization_generation_calls(id,task_id,attempt,fingerprint,metadata_json,process_id,process_started_at,created_at) VALUES(?,?,?,?,?,?,?,?)").bind(call.id.to_string()).bind(task.id.to_string()).bind(i64::from(call.attempt)).bind(fingerprint(call)?).bind(serde_json::to_string(call)?).bind(i64::from(pid)).bind(i64::try_from(start)?).bind(Utc::now().to_rfc3339()).execute(database).await?;
    Ok(())
}

pub(crate) async fn read_calls(
    database: &mut SqliteConnection,
    run: Uuid,
) -> Result<
    Vec<(
        GenerationTask,
        GenerationReservation,
        Option<GenerationOutcome>,
    )>,
> {
    let rows=sqlx::query("SELECT t.id AS task_id,t.iteration,t.fingerprint AS task_fingerprint,t.metadata_json AS task_json,c.id AS call_id,c.attempt,c.fingerprint AS call_fingerprint,c.metadata_json AS call_json,o.fingerprint AS outcome_fingerprint,o.metadata_json AS outcome_json FROM optimization_generation_tasks t JOIN optimization_generation_calls c ON c.task_id=t.id LEFT JOIN optimization_generation_outcomes o ON o.call_id=c.id WHERE t.run_id=? ORDER BY t.iteration,t.id,c.attempt").bind(run.to_string()).fetch_all(database).await?;
    rows.into_iter()
        .map(|row| {
            let task: GenerationTask = serde_json::from_str(&row.get::<String, _>("task_json"))?;
            ensure!(
                task.run_id == run
                    && task.id.to_string() == row.get::<String, _>("task_id")
                    && i64::from(task.iteration) == row.get::<i64, _>("iteration")
                    && task.fingerprint()? == row.get::<String, _>("task_fingerprint"),
                "Generation task binding changed"
            );
            let call: GenerationReservation =
                serde_json::from_str(&row.get::<String, _>("call_json"))?;
            ensure!(
                call.task_id == task.id
                    && call.task_fingerprint == task.fingerprint()?
                    && call.id.to_string() == row.get::<String, _>("call_id")
                    && i64::from(call.attempt) == row.get::<i64, _>("attempt")
                    && fingerprint(&call)? == row.get::<String, _>("call_fingerprint"),
                "Generation call binding changed"
            );
            let outcome = row
                .get::<Option<String>, _>("outcome_json")
                .map(|raw| serde_json::from_str::<GenerationOutcome>(&raw))
                .transpose()?;
            if let Some(outcome) = &outcome {
                outcome.validate(&task)?;
                ensure!(
                    outcome.reservation == call
                        && Some(fingerprint(outcome)?)
                            == row.get::<Option<String>, _>("outcome_fingerprint"),
                    "Generation outcome changed"
                );
            }
            Ok((task, call, outcome))
        })
        .collect()
}

async fn insert_outcome(
    database: &mut SqliteConnection,
    task: &GenerationTask,
    outcome: &GenerationOutcome,
) -> Result<()> {
    outcome.validate(task)?;
    let history = read_calls(database, task.run_id).await?;
    let (stored, call, previous) = history
        .iter()
        .find(|(_, call, _)| call.id == outcome.reservation.id)
        .context("Generation call was never reserved")?;
    ensure!(
        stored == task && call == &outcome.reservation,
        "Generation outcome belongs to another reservation"
    );
    if let Some(previous) = previous {
        ensure!(previous == outcome, "Generation outcome changed on retry");
        return Ok(());
    }
    sqlx::query("INSERT INTO optimization_generation_outcomes(call_id,fingerprint,metadata_json,created_at) VALUES(?,?,?,?)").bind(call.id.to_string()).bind(fingerprint(outcome)?).bind(serde_json::to_string(outcome)?).bind(Utc::now().to_rfc3339()).execute(database).await?;
    Ok(())
}

fn adapter(error: impl std::fmt::Display) -> OptimizationError {
    OptimizationError::Adapter(error.to_string())
}

#[cfg(test)]
mod tests;
