//! Production pre-training composition: exact inputs -> Agent -> generation ->
//! immutable dataset. The returned version is NOT permission to train: complete
//! task-owned benchmark isolation must still precede the training handoff.
pub(super) mod control;
mod inspection;
mod iteration_loop;
mod progress;
mod providers;
mod qualification;
mod registration;
mod training;
pub(super) use iteration_loop::execute as drive_loop;

use std::{collections::BTreeSet, path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{
    agent::{DatasetEditProposal, InspectionPage},
    ports::OptimizationAgentStore,
};
use encoder_optimization_runner::{OptimizationAgent, generation::OptimizationGenerator};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    optimization_iteration::ProjectOptimizationIteration,
};
use project_workspace_local::{
    AppendActivity, append_activity, initialize_activity, optimization_agent::ProjectAgentStore,
    optimization_dataset, optimization_generation::ProjectGenerationStore, optimization_launch,
    optimization_runs,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetStepResult {
    iteration_id: Uuid,
    proposal: DatasetEditProposal,
    publication: Option<optimization_dataset::OptimizationDatasetPublication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qualification: Option<qualification::QualifiedDataset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    development: Option<
        project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult,
    >,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<registration::IterationCandidate>,
}

pub(super) async fn execute(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, false, false).await
}

pub(super) async fn prepare_candidate(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, true, false).await
}

pub(super) async fn complete_iteration(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, true, true).await
}

async fn execute_step(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
    qualify: bool,
    train: bool,
) -> Result<()> {
    let database_url = super::super::sqlite_file_url(&folder.join("project.sqlite"));
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )
    .await?;
    if optimization_runs::show(folder, run_id)
        .await?
        .preparation
        .is_none()
    {
        super::prepare_inputs(folder, run_id).await?;
    }
    let iteration = super::iteration_inputs::bind_inputs(folder, run_id).await?;
    let (action_id, result) = run_step(folder, &iteration, runtime, qualify, train).await?;
    super::super::print(&serde_json::json!({"actionId": action_id, "datasetStep": result}))
}

async fn run_step(
    folder: &Path,
    iteration: &ProjectOptimizationIteration,
    runtime: crate::cli::ResearchRuntimeArgs,
    qualify: bool,
    train: bool,
) -> Result<(Uuid, DatasetStepResult)> {
    let run_id = iteration.scope.run_id;
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.agent".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references: vec![
            ActivityReference::new("run", run_id.to_string()).expect("UUID"),
            ActivityReference::new("iteration", iteration.scope.iteration.to_string())
                .expect("iteration"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result: Result<_> = async {
        if qualify {
            qualification::preflight(folder, run_id, iteration).await?;
        }
        let mut result = drive(folder, iteration, action_id, runtime).await?;
        ensure!(
            !project_workspace_local::optimization_execution::stopped(folder, run_id).await?,
            "Run stopped before qualification"
        );
        if qualify {
            if let Some(publication) = &result.publication {
                result.qualification =
                    Some(qualification::prepare(folder, run_id, publication).await?);
            }
        }
        if train {
            ensure!(
                !project_workspace_local::optimization_execution::stopped(folder, run_id).await?,
                "Run stopped before training"
            );
            if let Some(qualified) = &result.qualification {
                let (development, candidate) =
                    training::complete(folder, run_id, qualified).await?;
                result.development = Some(development);
                result.candidate = Some(candidate);
            }
        }
        Ok(result)
    }
    .await;
    let mut terminal = if result.is_ok() {
        event(ActivityEventState::Succeeded, None)
    } else {
        event(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "agent_dataset_failed",
                "Agent iteration stopped; inspect its recorded calls, dataset and scientific journal before resuming.",
            )?),
        )
    };
    if let Ok(step) = &result {
        terminal.references.push(ActivityReference::new(
            "iteration_id",
            step.iteration_id.to_string(),
        )?);
        if let Some(candidate) = &step.candidate {
            terminal.references.extend([
                ActivityReference::new("model", candidate.model.id.to_string())?,
                ActivityReference::new(
                    "dataset_version",
                    candidate.dataset.version.id.to_string(),
                )?,
            ]);
        }
        if let Some(development) = &step.development {
            terminal.references.push(ActivityReference::new(
                "experiment_run",
                development.experiment_run_id.to_string(),
            )?);
            for report in development.reports.values() {
                terminal.references.push(ActivityReference::new(
                    "evaluation_report",
                    report.id.to_string(),
                )?);
            }
        }
    }
    append_activity(folder, terminal).await?;
    Ok((action_id, result?))
}

async fn drive(
    folder: &Path,
    iteration: &ProjectOptimizationIteration,
    action_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<DatasetStepResult> {
    let run_id = iteration.scope.run_id;
    let run = optimization_runs::show(folder, run_id).await?;
    ensure!(
        !run.state.is_terminal() && run.materialization.is_none() && run.experiment.is_none(),
        "Agent edits cannot run after a fixed candidate was materialized or the run ended"
    );
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|launch| {
            launch.id.to_string() == run.run.launch.id
                && launch.fingerprint == run.run.launch.fingerprint
        })
        .context("Pinned launch is missing")?;
    let settings = launch
        .scope
        .agentic
        .as_ref()
        .context("Run has no Agent execution authorization")?;
    let inspection = Arc::new(inspection::IterationInspection::load(folder, iteration).await?);
    let agent_store = Arc::new(ProjectAgentStore::open(folder, run_id, action_id).await?);
    let generation_store = Arc::new(
        ProjectGenerationStore::open(folder, run_id)
            .await?
            .with_activity(),
    );
    // Exact PID/start-time checks reject an invocation while its predecessor is
    // still alive. A missing response does not itself authorize a replacement.
    agent_store.recover_interrupted(&iteration.scope).await?;
    generation_store.recover_interrupted().await?;
    let selected = providers::agent(
        run.run.project_id,
        agent_store.provider(),
        &launch.scope.advisor,
        runtime,
    )?;
    let agent = OptimizationAgent::new(
        selected.runtime,
        agent_store.clone(),
        inspection.clone(),
        selected.selection,
    );
    let proposal = agent.analyze(iteration.scope.clone()).await?;
    if proposal.stop {
        return Ok(DatasetStepResult {
            iteration_id: iteration.id,
            proposal,
            publication: None,
            qualification: None,
            development: None,
            candidate: None,
        });
    }
    let history = agent_store.history(iteration.scope.clone()).await?;
    let mut evidence = BTreeSet::new();
    for tool in history.iter().flat_map(|turn| &turn.tools).filter(|tool| {
        !tool.failed
            && matches!(
                tool.name.as_str(),
                "inspect_development_failures" | "inspect_dataset_landscape"
            )
    }) {
        let page: InspectionPage = serde_json::from_value(tool.result.clone())?;
        page.validate(20)?;
        evidence.extend(page.items.into_iter().map(|item| item.id));
    }
    let templates = Arc::new(inspection.templates(&proposal)?);
    let tasks = templates.tasks(
        &iteration.scope,
        &proposal,
        &evidence,
        providers::output_limit(&launch.scope.generation),
    )?;
    if !tasks.is_empty() {
        let backend = providers::generation(run.run.project_id, generation_store.provider())?;
        let generator = OptimizationGenerator {
            backend,
            admission: templates,
            store: generation_store,
            concurrency: settings.generation_concurrency,
            maximum_cost_microusd_per_call: providers::cost_limit(&launch.scope.generation),
        };
        generator.generate(tasks).await?;
    }
    ensure!(
        !agent_store.stopped(run_id).await?,
        "Run stopped before dataset publication"
    );
    let publication =
        optimization_dataset::publish(folder, run_id, iteration.scope.iteration).await?;
    ensure!(
        publication.parent == iteration.dataset,
        "Published dataset has another parent"
    );
    Ok(DatasetStepResult {
        iteration_id: iteration.id,
        proposal,
        publication: Some(publication),
        qualification: None,
        development: None,
        candidate: None,
    })
}
