//! Finite composition of the existing per-iteration execution. No nested CLI
//! processes or alternate trainer/generator path.
use super::*;
use encoder_experiment_core::ports::EncoderTaskAdapterError;
use encoder_experiment_runner::ExperimentRunnerError;
use project_workspace_core::optimization_loop::IterationCompletion;
use project_workspace_local::{
    optimization_completions, optimization_execution, optimization_iterations,
};

pub(in crate::commands::workspace::optimization_runs) async fn execute(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
    resume: Option<String>,
) -> Result<()> {
    let database_url = super::super::super::sqlite_file_url(&folder.join("project.sqlite"));
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )
    .await?;
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
    let attempt = optimization_execution::begin_attempt(folder, run_id, resume.as_deref()).await?;
    if attempt.is_none()
        && view.agent_execution.as_ref().is_some_and(|execution| {
            execution.state
            == project_workspace_core::optimization_execution::AgentExecutionState::BudgetExhausted
        })
    {
        anyhow::bail!("Training time budget exhausted; completed work is preserved");
    }
    let mut watcher = attempt.map(|_| control::StopWatcher::start(folder, run_id));
    let signal = watcher
        .as_ref()
        .map(|w| w.signal.clone())
        .unwrap_or_default();
    let probe: std::sync::Arc<dyn Fn() -> bool + Send + Sync> =
        std::sync::Arc::new(move || signal.load(std::sync::atomic::Ordering::Acquire));
    let mut result = project_workspace_local::progress::with_stop_probe(
        probe.clone(),
        encoder_experiment_nomos::with_stop_probe(
            probe,
            Box::pin(run(folder, run_id, runtime, settings.maximum_iterations)),
        ),
    )
    .await;
    if let Some(watcher) = &mut watcher {
        if let Err(error) = watcher.finish().await {
            result = Err(error.context("Could not observe the run's durable Stop state"));
        }
    }
    if let Some(attempt) = attempt {
        if acknowledge_pending_stop(folder, run_id, attempt).await? {
            return paused(run_id);
        }
    }
    match result {
        Ok((verified, completed)) => {
            if let Some(attempt) = attempt {
                if let Err(error) =
                    optimization_execution::complete_attempt(folder, run_id, attempt, &verified)
                        .await
                {
                    if acknowledge_pending_stop(folder, run_id, attempt).await? {
                        return paused(run_id);
                    }
                    return Err(error);
                }
            }
            super::super::super::print(&serde_json::json!({
                "runId":run_id, "completion":verified, "iterations":completed,
            }))
        }
        Err(error) => {
            if let Some(attempt) = attempt {
                // Keep the original execution error, even if recording its
                // failure is itself interrupted. The next lease owner recovers it.
                let record = if is_training_budget_stop(&error) {
                    optimization_execution::exhaust_budget(folder, run_id, attempt).await
                } else {
                    optimization_execution::fail_attempt(folder, run_id, attempt).await
                };
                if let Err(record_error) = record {
                    if acknowledge_pending_stop(folder, run_id, attempt).await? {
                        return paused(run_id);
                    }
                    return Err(error.context(format!(
                        "Could not persist the execution failure: {record_error}"
                    )));
                }
            }
            Err(error)
        }
    }
}

fn is_training_budget_stop(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let exhausted = |error: &EncoderTaskAdapterError| {
            matches!(
                error,
                EncoderTaskAdapterError::TrainingBudgetExhausted
                    | EncoderTaskAdapterError::TimeLimitExceeded
            )
        };
        cause
            .downcast_ref::<EncoderTaskAdapterError>()
            .is_some_and(exhausted)
            || cause
                .downcast_ref::<ExperimentRunnerError>()
                .is_some_and(|error| {
                    matches!(error, ExperimentRunnerError::Adapter(adapter) if exhausted(adapter))
                })
    })
}

fn paused(run_id: Uuid) -> Result<()> {
    super::super::super::print(&serde_json::json!({"runId":run_id, "state":"paused"}))
}

async fn acknowledge_pending_stop(folder: &Path, run_id: Uuid, attempt: Uuid) -> Result<bool> {
    let current = optimization_runs::show(folder, run_id).await?;
    if current.agent_execution.as_ref().is_some_and(|v| {
        v.state
            == project_workspace_core::optimization_execution::AgentExecutionState::StopRequested
            && v.attempt_id == attempt
    }) {
        optimization_execution::acknowledge_stop(folder, run_id, attempt).await?;
        return Ok(true);
    }
    Ok(false)
}

async fn run(
    folder: &Path,
    run_id: Uuid,
    runtime: crate::cli::ResearchRuntimeArgs,
    maximum_iterations: u32,
) -> Result<(IterationCompletion, Vec<IterationCompletion>)> {
    let view = optimization_runs::show(folder, run_id).await?;
    if view.preparation.is_none() {
        super::super::prepare_inputs(folder, run_id).await?;
    }
    // The bound is immutable. Completion, not an in-memory counter, decides
    // whether to resume an existing iteration, advance, or return a final result.
    for _ in 0..=maximum_iterations {
        if !optimization_runs::show(folder, run_id)
            .await?
            .state
            .is_terminal()
        {
            ensure!(
                !optimization_execution::stopped(folder, run_id).await?,
                "Run stopped before the next iteration step"
            );
        }
        let completed = optimization_completions::list(folder, run_id).await?;
        let iteration = if let Some(last) = completed.last() {
            let sources =
                super::super::iteration_inputs::scientific_history(folder, run_id, last.number)
                    .await?;
            let verified =
                optimization_completions::finish(folder, run_id, last.number, &sources).await?;
            if verified.end.is_some() {
                return Ok((verified, completed));
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
