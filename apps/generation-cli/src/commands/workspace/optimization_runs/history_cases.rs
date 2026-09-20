//! On-demand, passive case diagnostics for one completed iteration. Routine
//! history polling stays independent of native diagnostic file availability.
use std::path::Path;

use anyhow::{Context, Result, ensure};
use encoder_experiment_core::ports::ExperimentStore;
use project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult;
use project_workspace_local::{
    open_workspace, optimization_iteration_execution, optimization_iterations, optimization_runs,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DevelopmentCases {
    project_id: Uuid,
    run_id: Uuid,
    iteration_id: Uuid,
    iteration: u32,
    baseline_model_id: Uuid,
    candidate_model_id: Uuid,
    comparisons: Vec<encoder_experiment_nomos::NomosDevelopmentComparison>,
}

pub(super) async fn read(folder: &Path, run_id: Uuid, number: u32) -> Result<DevelopmentCases> {
    ensure!(
        (1..=10).contains(&number),
        "Case comparison iteration must be 1–10"
    );
    // Reuse the normal lineage/assessment validator. This never executes or
    // reconciles a run, nor grants permission to access sealed evidence.
    super::history::read(folder, run_id).await?;
    let iterations = optimization_iterations::list(folder, run_id).await?;
    let iteration = iterations
        .iter()
        .find(|value| value.scope.iteration == number)
        .context("Case comparison iteration is missing")?;
    let training = optimization_iteration_execution::training(folder, run_id, iteration.id)
        .await?
        .context("Case comparison training custody is missing")?;
    let run = optimization_runs::show(folder, run_id).await?;
    let preparation = run
        .preparation
        .as_ref()
        .context("Case comparison preparation is missing")?;
    let bindings = project_workspace_local::scientific_binding_history(folder).await?;
    let binding = bindings
        .iter()
        .find(|value| {
            value.id.to_string() == preparation.execution_binding.id
                && value.fingerprint == preparation.execution_binding.fingerprint
        })
        .context("Case comparison runtime binding is missing")?;
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Case comparison model catalog is missing")?;
    let baseline_model_id = catalog
        .baseline_revisions
        .iter()
        .find(|value| value.id.to_string() == iteration.comparison_baseline_revision.id)
        .context("Original baseline revision is missing")?
        .model_artifact_id;
    let candidate_model_id = catalog
        .artifacts
        .iter()
        .find(|value| {
            value
                .producing_run
                .as_ref()
                .is_some_and(|source| source.id == training.experiment_run_id.to_string())
        })
        .context("Exact iteration model is missing")?
        .id;
    let store = super::super::open_bound_store(&folder.to_string_lossy(), binding).await?;
    let result: Result<_> = async {
        let runtime_project = super::super::load_bound_project(&store, binding).await?;
        let project = store
            .get_project(training.scientific_project.id.parse()?)
            .await?
            .context("Scientific project is missing")?;
        let protocol = store
            .get_protocol(training.protocol.id.parse()?)
            .await?
            .context("Scientific protocol is missing")?;
        let events = store.load_events(training.experiment_run_id).await?;
        let result =
            IterationDevelopmentResult::from_journal(&training, &project, &protocol, &events)?;
        let backend = super::super::open_nomos_binding(folder, binding, &runtime_project)?;
        let baseline = protocol.baseline_development_reports();
        let comparisons = result
            .reports
            .values()
            .map(|candidate| {
                let original = baseline
                    .iter()
                    .find(|report| report.suite_key == candidate.suite_key)
                    .context("Original development baseline report is missing")?;
                Ok(backend.read_development_comparison(
                    &project,
                    &protocol.metric_contract,
                    original,
                    candidate,
                )?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DevelopmentCases {
            project_id: run.run.project_id,
            run_id,
            iteration_id: iteration.id,
            iteration: number,
            baseline_model_id,
            candidate_model_id,
            comparisons,
        })
    }
    .await;
    store.pool().close().await;
    result
}
