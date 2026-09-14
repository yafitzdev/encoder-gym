//! Project-owned optimization children can live in a materialized scientific
//! project, not the runtime's original snapshot. Follow exact receipts only.
use std::path::Path;

use anyhow::{Context, Result, ensure};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    ModelCatalog,
    benchmark_results::{BenchmarkRunEvidence, OptimizationResultLineage, ProjectBenchmarkResults},
};
use project_workspace_local::{optimization_launch, optimization_runs, optimization_setup};

pub(super) async fn include(
    folder: &Path,
    catalog: &ModelCatalog,
    results: &mut ProjectBenchmarkResults,
) -> Result<()> {
    let runs = optimization_runs::list(folder).await?;
    let relevant: Vec<_> = runs
        .iter()
        .filter(|view| {
            view.experiment.is_some()
                && view.preparation.as_ref().is_some_and(|preparation| {
                    preparation.benchmark.id == results.version.id.to_string()
                })
        })
        .collect();
    if relevant.is_empty() {
        return Ok(());
    }
    let setups = optimization_setup::list(folder).await?;
    let launches = optimization_launch::list(folder).await?;
    let bindings = project_workspace_local::scientific_binding_history(folder).await?;
    let workspace = folder.to_str().context("Project path is not valid UTF-8")?;
    for view in relevant {
        let preparation = view
            .preparation
            .as_ref()
            .context("Run preparation is missing")?;
        let materialization = view
            .materialization
            .as_ref()
            .context("Run materialization is missing")?;
        let experiment = view
            .experiment
            .as_ref()
            .context("Run experiment is missing")?;
        let setup = setups
            .iter()
            .find(|value| value.id.to_string() == view.run.setup.id)
            .context("Run setup is missing")?;
        let launch = launches
            .iter()
            .find(|value| value.id.to_string() == view.run.launch.id)
            .context("Run launch is missing")?;
        let binding = bindings
            .iter()
            .find(|value| value.id.to_string() == preparation.execution_binding.id)
            .context("Run execution binding is missing")?;
        let store = super::super::open_bound_store(workspace, binding).await?;
        let runtime_project = super::super::load_bound_project(&store, binding).await?;
        let project = store
            .get_project(materialization.scientific_project.id.parse()?)
            .await?
            .context("Optimization's materialized scientific project is missing")?;
        let protocol = store
            .get_protocol(experiment.protocol.id.parse()?)
            .await?
            .context("Optimization's scientific protocol is missing")?;
        let events = store
            .load_events(experiment.experiment_run.id.parse()?)
            .await?;
        let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
        ensure!(
            definition == results.version.definition,
            "Optimization's recorded benchmark changed"
        );
        results
            .include_optimization_run(
                catalog,
                BenchmarkRunEvidence {
                    binding,
                    project: &project,
                    protocol: &protocol,
                    definition: &definition,
                    events: &events,
                },
                OptimizationResultLineage {
                    run: &view.run,
                    launch,
                    setup,
                    preparation,
                    materialization,
                    experiment,
                    runtime_project: &runtime_project,
                },
            )
            .with_context(|| {
                format!(
                    "Optimization {} has inconsistent result lineage",
                    view.run.id
                )
            })?;
        store.pool().close().await;
    }
    Ok(())
}
