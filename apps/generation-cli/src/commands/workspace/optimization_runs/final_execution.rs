//! Separate final dispatch, never a continuation of the adaptive experiment.
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{domain::ModelArtifactIdentity, ports::EncoderTaskBackend};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    optimization_final_execution::AgentFinalResult,
    optimization_iteration_execution::IterationDevelopmentResult,
};
use project_workspace_local::{
    optimization_completions::IterationScientificEvidence,
    optimization_final_execution::{self as custody, AgentFinalExecution},
    optimization_runs,
};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn show(folder: &Path, run_id: Uuid) -> Result<()> {
    let scientific = super::final_authorization::history(folder, run_id).await?;
    let Some(view) = custody::show(folder, run_id, &scientific).await? else {
        return super::super::print(&serde_json::Value::Null);
    };
    if let Some(result) = &view.result {
        let (backend, model) = backend(folder, run_id, &view, &scientific).await?;
        let source = selected(&view, &scientific)?;
        let report = backend
            .recover_evaluation(
                source.project.clone(),
                model,
                source.protocol.metric_contract.clone(),
                view.authorization.scope.final_suite.clone(),
            )
            .await?
            .context(
                "Completed final native evidence is missing; evaluation will not be repeated",
            )?;
        result.recover(
            &view.authorization,
            view.dispatch.as_ref().context("Final dispatch missing")?,
            &source.project,
            &source.protocol,
            report,
        )?;
    }
    print(&view)
}

pub(super) async fn execute(folder: &Path, run_id: Uuid, authorization_id: Uuid) -> Result<()> {
    // Reject absent/incorrect consent before acquiring a lease or touching runtime state.
    let scientific = super::final_authorization::history(folder, run_id).await?;
    let view = custody::show(folder, run_id, &scientific)
        .await?
        .context("Explicit final consent is required")?;
    ensure!(
        view.authorization.id == authorization_id,
        "Final consent differs from the requested authorization"
    );
    let project_url = super::super::sqlite_file_url(&folder.join("project.sqlite"));
    let _root_lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &project_url,
        run_id,
    )
    .await?;
    let binding = runtime_binding(folder, run_id).await?;
    let scientific_url = super::super::bound_store_url(folder, &binding)?;
    let _scientific_lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &scientific_url,
        run_id,
    )
    .await?;
    let result = execute_owned(folder, run_id, authorization_id, scientific).await;
    // A final dispatch never leaves descendants alive while returning an outcome.
    crate::process_ownership::quiesce().await?;
    print(&result?)
}

async fn execute_owned(
    folder: &Path,
    run_id: Uuid,
    authorization_id: Uuid,
    scientific: Vec<IterationScientificEvidence>,
) -> Result<AgentFinalExecution> {
    let mut view = custody::show(folder, run_id, &scientific)
        .await?
        .context("Final consent is missing")?;
    ensure!(
        view.authorization.id == authorization_id,
        "Final consent changed before execution"
    );
    let source = selected(&view, &scientific)?;
    let (backend, model) = backend(folder, run_id, &view, &scientific).await?;
    if let Some(result) = &view.result {
        let report = backend
            .recover_evaluation(
                source.project.clone(),
                model,
                source.protocol.metric_contract.clone(),
                view.authorization.scope.final_suite.clone(),
            )
            .await?
            .context(
                "Completed final native evidence is missing; evaluation will not be repeated",
            )?;
        result.recover(
            &view.authorization,
            view.dispatch.as_ref().context("Final dispatch missing")?,
            &source.project,
            &source.protocol,
            report,
        )?;
        return Ok(view);
    }
    let (dispatch, fresh) = custody::reserve(folder, run_id, authorization_id, &scientific).await?;
    let scope = &view.authorization.scope;
    let report = if fresh {
        backend.evaluate(source.project.clone(), model, source.protocol.metric_contract.clone(),
            scope.final_suite.clone(), scope.maximum_evaluation_seconds).await
            .context("Final evaluation did not complete; its allowance remains reserved and retries only recover saved evidence")?
    } else {
        backend.recover_evaluation(source.project.clone(), model, source.protocol.metric_contract.clone(), scope.final_suite.clone())
            .await?.context("Final outcome is unknown: completed native evidence is missing; reserved holdout allowance cannot be reused")?
    };
    crate::process_ownership::quiesce().await?;
    let result = AgentFinalResult::create(
        &view.authorization,
        &dispatch,
        &source.project,
        &source.protocol,
        report,
        Utc::now(),
    )?;
    let result = custody::record_result(folder, run_id, result, &scientific)
        .await
        .context(
            "Final result could not be saved; retry will only recover completed native evidence",
        )?;
    view.dispatch = Some(dispatch);
    view.result = Some(result);
    Ok(view)
}

fn print(view: &AgentFinalExecution) -> Result<()> {
    super::super::print(&serde_json::json!({"state":view.state(), "execution":view}))
}

pub(super) fn selected<'a>(
    view: &AgentFinalExecution,
    scientific: &'a [IterationScientificEvidence],
) -> Result<&'a IterationScientificEvidence> {
    scientific
        .iter()
        .find(|source| source.iteration_id.to_string() == view.authorization.scope.iteration.id)
        .context("Selected final scientific evidence is missing")
}

pub(super) async fn runtime_binding(
    folder: &Path,
    run_id: Uuid,
) -> Result<project_workspace_core::ScientificBinding> {
    let run = optimization_runs::show(folder, run_id).await?;
    let preparation = run.preparation.context("Run preparation missing")?;
    project_workspace_local::scientific_binding_history(folder)
        .await?
        .into_iter()
        .find(|binding| {
            binding.id.to_string() == preparation.execution_binding.id
                && binding.fingerprint == preparation.execution_binding.fingerprint
        })
        .context("Pinned scientific runtime binding missing")
}

pub(super) async fn backend(
    folder: &Path,
    run_id: Uuid,
    view: &AgentFinalExecution,
    scientific: &[IterationScientificEvidence],
) -> Result<(NomosBackend, ModelArtifactIdentity)> {
    let binding = runtime_binding(folder, run_id).await?;
    let store = super::super::open_bound_store(&folder.to_string_lossy(), &binding).await?;
    let bound_project = super::super::load_bound_project(&store, &binding).await;
    store.pool().close().await;
    let base = super::super::open_nomos_binding(folder, &binding, &bound_project?)?;
    let scope = &view.authorization.scope;
    let training = project_workspace_local::optimization_iteration_execution::training(
        folder,
        run_id,
        scope.iteration.id.parse()?,
    )
    .await?
    .context("Selected training custody missing")?;
    let native = base.load_training_dataset(
        scope.iteration.id.parse()?,
        scope.dataset.id,
        &scope.dataset.fingerprint,
    )?;
    ensure!(
        native.fingerprint == training.native_materialization_fingerprint
            && native.artifact == training.training_artifact,
        "Selected native training custody changed"
    );
    let backend = base
        .with_training_dataset(native)?
        .with_progress_observer(std::sync::Arc::new(
            crate::commands::encoder_optimize::activity::ProgressOutput,
        ));
    let source = selected(view, scientific)?;
    backend
        .verify_current_snapshot(source.project.clone())
        .await?;
    let development = IterationDevelopmentResult::from_journal(
        &training,
        &source.project,
        &source.protocol,
        &source.events,
    )?;
    // Missing/altered checkpoint bytes fail before the only dispatch is reserved.
    backend.verified_model_path(&development.output.model)?;
    Ok((backend, development.output.model))
}
