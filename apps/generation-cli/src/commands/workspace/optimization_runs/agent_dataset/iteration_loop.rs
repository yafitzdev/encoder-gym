//! Finite composition of the existing per-iteration execution. No nested CLI
//! processes or alternate trainer/generator path.
use super::*;
use project_workspace_local::{optimization_completions, optimization_iterations};

pub(in crate::commands::workspace::optimization_runs) async fn execute(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
) -> Result<()> {
    let database_url = format!("sqlite://{}", folder.join("project.sqlite").display());
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )?;
    let view = optimization_runs::show(folder, run_id).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Launch missing")?;
    let settings = launch
        .scope
        .agentic
        .as_ref()
        .context("Run has no Agent authorization")?;
    settings.validate()?;
    if view.preparation.is_none() {
        super::super::prepare_inputs(folder, run_id).await?;
    }
    // The bound is immutable. Completion, not an in-memory counter, decides
    // whether to resume an existing iteration, advance, or return a final result.
    for _ in 0..=settings.maximum_iterations {
        let completed = optimization_completions::list(folder, run_id).await?;
        let iteration = if let Some(last) = completed.last() {
            let sources =
                super::super::iteration_inputs::scientific_history(folder, run_id, last.number)
                    .await?;
            let verified =
                optimization_completions::finish(folder, run_id, last.number, &sources).await?;
            if verified.end.is_some() {
                return super::super::super::print(&serde_json::json!({
                    "runId":run_id, "completion":verified, "iterations":completed,
                }));
            }
            optimization_iterations::begin_next(folder, run_id, &sources).await?
        } else {
            super::super::iteration_inputs::bind_inputs(folder, run_id).await?
        };
        run_step(folder, &iteration, runtime.clone(), true, true).await?;
        let sources = super::super::iteration_inputs::scientific_history(
            folder,
            run_id,
            iteration.scope.iteration,
        )
        .await?;
        optimization_completions::finish(folder, run_id, iteration.scope.iteration, &sources)
            .await?;
    }
    anyhow::bail!("Agent loop did not reach its persisted finite completion")
}
