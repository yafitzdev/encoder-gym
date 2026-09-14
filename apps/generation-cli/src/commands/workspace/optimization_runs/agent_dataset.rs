//! Production pre-training composition: exact inputs -> Agent -> generation ->
//! immutable dataset. The returned version is NOT permission to train: complete
//! task-owned benchmark isolation must still precede the training handoff.
mod inspection;
mod providers;
mod qualification;

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
}

pub(super) async fn execute(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, false).await
}

pub(super) async fn prepare_candidate(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    execute_step(folder, run_id, runtime, true).await
}

async fn execute_step(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
    qualify: bool,
) -> Result<()> {
    let database_url = format!("sqlite://{}", folder.join("project.sqlite").display());
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )?;
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
            ActivityReference::new("iteration", "1").expect("iteration"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result: Result<_> = async {
        let mut result = drive(folder, run_id, action_id, runtime).await?;
        if qualify {
            if let Some(publication) = &result.publication {
                result.qualification =
                    Some(qualification::prepare(folder, run_id, publication).await?);
            }
        }
        Ok(result)
    }
    .await;
    let terminal = if result.is_ok() {
        event(ActivityEventState::Succeeded, None)
    } else {
        event(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "agent_dataset_failed",
                "Agent dataset preparation stopped before training; recorded calls and completed edits are retained.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::super::print(&serde_json::json!({"actionId": action_id, "datasetStep": result?}))
}

async fn drive(
    folder: &Path,
    run_id: Uuid,
    action_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<DatasetStepResult> {
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
    let iteration = super::iteration_inputs::bind_inputs(folder, run_id).await?;
    let inspection = Arc::new(inspection::IterationInspection::load(folder, &iteration).await?);
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
        });
    }
    let history = agent_store.history(iteration.scope.clone()).await?;
    let mut evidence = BTreeSet::new();
    for tool in history
        .iter()
        .flat_map(|turn| &turn.tools)
        .filter(|tool| !tool.failed && tool.name == "inspect_development_failures")
    {
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
    })
}
