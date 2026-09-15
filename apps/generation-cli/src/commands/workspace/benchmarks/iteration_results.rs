//! Discover adaptive results through project-owned iteration/training receipts,
//! not an unscoped search for models or scientific database rows.
use anyhow::{Context, Result, ensure};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    ModelCatalog,
    benchmark_results::{BenchmarkRunEvidence, IterationResultLineage, ProjectBenchmarkResults},
};
use project_workspace_local::{
    optimization_iteration_execution, optimization_iterations, optimization_launch,
    optimization_runs, optimization_setup, scientific_binding_history,
};
use std::path::Path;

pub(super) async fn include(
    folder: &Path,
    catalog: &ModelCatalog,
    results: &mut ProjectBenchmarkResults,
) -> Result<()> {
    let runs = optimization_runs::list(folder).await?;
    let relevant: Vec<_> = runs
        .iter()
        .filter(|view| {
            view.preparation.as_ref().is_some_and(|preparation| {
                preparation.benchmark.id == results.version.id.to_string()
            })
        })
        .collect();
    if relevant.is_empty() {
        return Ok(());
    }
    let launches = optimization_launch::list(folder).await?;
    let setups = optimization_setup::list(folder).await?;
    let bindings = scientific_binding_history(folder).await?;
    let workspace = folder.to_str().context("Project path is not valid UTF-8")?;
    for view in relevant {
        let launch = launches
            .iter()
            .find(|item| item.id.to_string() == view.run.launch.id)
            .context("Iteration launch is missing")?;
        if launch.scope.agentic.is_none() {
            continue;
        }
        let preparation = view
            .preparation
            .as_ref()
            .context("Iteration preparation is missing")?;
        let setup = setups
            .iter()
            .find(|item| item.id.to_string() == view.run.setup.id)
            .context("Iteration setup is missing")?;
        let binding = bindings
            .iter()
            .find(|item| item.id.to_string() == preparation.execution_binding.id)
            .context("Iteration runtime binding is missing")?;
        let store = super::super::open_bound_store(workspace, binding).await?;
        let projected: Result<()> = async {
            let runtime_project = super::super::load_bound_project(&store, binding).await?;
            for iteration in optimization_iterations::list(folder, view.run.id).await? {
                let Some(training) =
                    optimization_iteration_execution::training(folder, view.run.id, iteration.id)
                        .await?
                else {
                    continue;
                };
                let project = store
                    .get_project(training.scientific_project.id.parse()?)
                    .await?
                    .context("Iteration scientific project is missing")?;
                let protocol = store
                    .get_protocol(training.protocol.id.parse()?)
                    .await?
                    .context("Iteration protocol is missing")?;
                let events = store.load_events(training.experiment_run_id).await?;
                // Training can be reserved before the first scientific event.
                if events.is_empty() {
                    continue;
                }
                let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
                ensure!(
                    definition == results.version.definition,
                    "Iteration benchmark changed"
                );
                results
                    .include_agent_iteration(
                        catalog,
                        BenchmarkRunEvidence {
                            binding,
                            project: &project,
                            protocol: &protocol,
                            definition: &definition,
                            events: &events,
                        },
                        IterationResultLineage {
                            run: &view.run,
                            launch,
                            setup,
                            preparation,
                            iteration: &iteration,
                            training: &training,
                            runtime_project: &runtime_project,
                        },
                    )
                    .with_context(|| {
                        format!("Iteration {} has inconsistent result lineage", iteration.id)
                    })?;
            }
            Ok(())
        }
        .await;
        store.pool().close().await;
        projected?;
    }
    Ok(())
}
