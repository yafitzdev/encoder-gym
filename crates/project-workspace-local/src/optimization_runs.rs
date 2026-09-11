//! Project-owned run roots for input-first optimization.
//!
//! This module reserves and verifies the root journal only. Slice execution is
//! attached later through its own adapters and stores.

use crate::{
    connect, load_provider_catalog_history, open_workspace, optimization_launch, optimization_setup,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    OptimizationLaunchAuthorization, OptimizationSetup, ProjectOptimizationEvent,
    ProjectOptimizationExperiment, ProjectOptimizationFinalResult,
    ProjectOptimizationMaterialization, ProjectOptimizationOutcome, ProjectOptimizationPreparation,
    ProjectOptimizationRun, ProjectOptimizationRunState, ProjectOptimizationRunView,
    replay_project_optimization,
};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

pub async fn start(
    folder: &Path,
    request: optimization_launch::LaunchRequest,
    authorized_by: &str,
) -> Result<ProjectOptimizationRunView> {
    // Authorization and reservation are separately idempotent. If the process
    // stops between them, the same request finishes the missing reservation.
    let authorization = optimization_launch::authorize(folder, request, authorized_by).await?;
    let verified = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&verified.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &verified).await?;
    let providers = load_provider_catalog_history(&mut transaction, &verified.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &verified, &setups, &providers).await?;
    let persisted = launches
        .iter()
        .find(|value| value.id == authorization.id)
        .context("Optimization launch authorization is missing.")?;
    ensure!(
        persisted == &authorization,
        "Optimization launch authorization changed."
    );
    let existing = load(&mut transaction, verified.manifest.id, &launches, &setups).await?;
    if let Some(run) = existing
        .into_iter()
        .find(|run| run.run.launch.id == authorization.id.to_string())
    {
        transaction.commit().await?;
        database.close().await?;
        return Ok(run);
    }

    // A launch authorized earlier but not yet reserved must still be current at
    // the moment it becomes executable work.
    let setup = setups
        .iter()
        .find(|value| value.id.to_string() == authorization.scope.setup.id)
        .context("Optimization setup is missing.")?;
    ensure!(
        setups.last().is_some_and(|current| current.id == setup.id),
        "Optimization inputs changed. Review them again."
    );
    let provider = providers
        .iter()
        .find(|value| value.id.to_string() == authorization.scope.provider_catalog.id)
        .context("Optimization provider settings are missing.")?;
    ensure!(
        providers
            .last()
            .is_some_and(|current| current.id == provider.id),
        "Provider settings changed. Review Optimize again."
    );
    let benchmark = verified
        .benchmark_versions
        .iter()
        .find(|value| value.id.to_string() == setup.inputs.benchmark.id)
        .context("Optimization benchmark is missing.")?;
    authorization.validate(setup, provider, benchmark)?;
    let active: String = sqlx::query_scalar(
        "SELECT active_baseline_revision_id FROM model_catalog_state WHERE singleton=1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    ensure!(
        active == setup.inputs.baseline_revision.id,
        "Baseline changed. Review Optimize inputs again."
    );

    let created_at = Utc::now();
    let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &authorization, created_at)?;
    let event = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, created_at)?;
    sqlx::query("INSERT INTO project_optimization_runs (id, project_id, launch_id, launch_fingerprint, setup_id, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(run.id.to_string()).bind(run.project_id.to_string()).bind(&run.launch.id)
        .bind(&run.launch.fingerprint).bind(&run.setup.id).bind(&run.fingerprint)
        .bind(serde_json::to_string(&run)?).bind(run.created_at.to_rfc3339())
        .execute(&mut *transaction).await?;
    sqlx::query("INSERT INTO project_optimization_events (id, run_id, sequence, previous_event_fingerprint, kind, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(event.id.to_string()).bind(event.run_id.to_string()).bind(i64::try_from(event.sequence)?)
        .bind(&event.previous_event_fingerprint).bind("reserved").bind(&event.fingerprint)
        .bind(serde_json::to_string(&event)?).bind(event.created_at.to_rfc3339())
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    replay_project_optimization(&run, &authorization, &[event]).map_err(Into::into)
}

pub async fn list(folder: &Path) -> Result<Vec<ProjectOptimizationRunView>> {
    let workspace = open_workspace(folder, false).await?;
    let launches = optimization_launch::list(folder).await?;
    let setups = optimization_setup::list(folder).await?;
    let mut database = connect(Path::new(&workspace.folder), true, false).await?;
    let runs = load(&mut database, workspace.manifest.id, &launches, &setups).await?;
    database.close().await?;
    Ok(runs)
}

pub async fn show(folder: &Path, run_id: Uuid) -> Result<ProjectOptimizationRunView> {
    list(folder)
        .await?
        .into_iter()
        .find(|run| run.run.id == run_id)
        .context("Project optimization run was not found.")
}

/// Permanently stop a project optimization root. The append is atomic with
/// respect to every stage completion, so an in-flight worker holding an older
/// journal head cannot publish a result after cancellation wins the race.
pub async fn cancel(folder: &Path, run_id: Uuid) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::Cancelled {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        !view.state.is_terminal(),
        "A completed optimization run cannot be cancelled."
    );
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::cancelled(
        Uuid::new_v4(),
        &view.run,
        &previous,
        "user_requested",
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted cancellation belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

/// Start or recover the no-provider preparation attempt. Duplicate verification
/// is harmless; completion still uses the exact returned journal head.
pub async fn begin_preparation(folder: &Path, run_id: Uuid) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let views = load(&mut transaction, workspace.manifest.id, &launches, &setups).await?;
    let view = views
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::Preparing || view.state.has_preparation() {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::preparation_started(
        Uuid::new_v4(),
        &view.run,
        &previous,
        view.attempt + 1,
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted preparation belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

pub async fn finish_preparation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    preparation: ProjectOptimizationPreparation,
) -> Result<ProjectOptimizationRunView> {
    record_preparation(folder, run_id, expected_head, Some(preparation), None).await
}

pub async fn fail_preparation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    failure_code: &str,
) -> Result<ProjectOptimizationRunView> {
    record_preparation(folder, run_id, expected_head, None, Some(failure_code)).await
}

async fn record_preparation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    preparation: Option<ProjectOptimizationPreparation>,
    failure_code: Option<&str>,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let views = load(&mut transaction, workspace.manifest.id, &launches, &setups).await?;
    let view = views
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state.has_preparation() {
        ensure!(
            preparation.as_ref() == view.preparation.as_ref(),
            "Optimization preparation already completed with another receipt."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    if view.state == ProjectOptimizationRunState::PreparationFailed {
        ensure!(
            preparation.is_none() && failure_code == view.failure_code.as_deref(),
            "Optimization preparation attempt already recorded another outcome."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        view.state == ProjectOptimizationRunState::Preparing
            && view.head_fingerprint == expected_head,
        "Optimization preparation journal changed; reload it before recording an outcome."
    );
    let launch = launches
        .iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let setup = setups
        .iter()
        .find(|value| value.id.to_string() == view.run.setup.id)
        .context("Optimization setup is missing.")?;
    if let Some(receipt) = &preparation {
        receipt.validate_for(&view.run, launch, setup)?;
        let catalog = workspace
            .model_catalog
            .as_ref()
            .context("Model catalog is missing.")?;
        let model = catalog.active_model();
        let binding = workspace
            .scientific_binding
            .as_ref()
            .context("Scientific runtime binding is missing.")?;
        let dataset = crate::dataset_versions::load_reference(
            &mut transaction,
            workspace.manifest.id,
            receipt.dataset.id,
        )
        .await?;
        let benchmark = workspace
            .benchmark_versions
            .iter()
            .find(|value| value.id.to_string() == receipt.benchmark.id)
            .context("Shared benchmark version is missing.")?;
        ensure!(
            catalog.active_baseline_revision_id.to_string() == setup.inputs.baseline_revision.id
                && model.id.to_string() == receipt.model.id
                && model.fingerprint == receipt.model.fingerprint
                && dataset == receipt.dataset
                && benchmark.fingerprint == receipt.benchmark.fingerprint
                && binding.id.to_string() == receipt.execution_binding.id
                && binding.fingerprint == receipt.execution_binding.fingerprint
                && binding.runtime.project_snapshot == receipt.runtime_project
                && receipt.adapter.id
                    == format!("{}:{}", binding.adapter.key, binding.adapter.protocol)
                && receipt.adapter.fingerprint == binding.adapter.configuration_fingerprint,
            "Project inputs or execution runtime changed before preparation completed."
        );
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = match (preparation, failure_code) {
        (Some(receipt), None) => ProjectOptimizationEvent::preparation_completed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.attempt,
            receipt,
            Utc::now(),
        )?,
        (None, Some(code)) => ProjectOptimizationEvent::preparation_failed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.attempt,
            code,
            Utc::now(),
        )?,
        _ => anyhow::bail!("Preparation must record exactly one outcome."),
    };
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted outcome belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

/// Start or recover adapter-owned rendering of the already verified dataset.
pub async fn begin_materialization(
    folder: &Path,
    run_id: Uuid,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::Materializing || view.state.has_materialization()
    {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        matches!(
            view.state,
            ProjectOptimizationRunState::Ready | ProjectOptimizationRunState::MaterializationFailed
        ),
        "Verify optimization inputs before materializing training data."
    );
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::materialization_started(
        Uuid::new_v4(),
        &view.run,
        &previous,
        view.materialization_attempt + 1,
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted materialization belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

pub async fn finish_materialization(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    materialization: ProjectOptimizationMaterialization,
) -> Result<ProjectOptimizationRunView> {
    record_materialization(folder, run_id, expected_head, Some(materialization), None).await
}

pub async fn fail_materialization(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    failure_code: &str,
) -> Result<ProjectOptimizationRunView> {
    record_materialization(folder, run_id, expected_head, None, Some(failure_code)).await
}

async fn record_materialization(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    materialization: Option<ProjectOptimizationMaterialization>,
    failure_code: Option<&str>,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state.has_materialization() {
        ensure!(
            materialization.as_ref() == view.materialization.as_ref(),
            "Optimization training data already materialized with another receipt."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    if view.state == ProjectOptimizationRunState::MaterializationFailed {
        ensure!(
            materialization.is_none() && failure_code == view.failure_code.as_deref(),
            "Optimization materialization attempt already recorded another outcome."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        view.state == ProjectOptimizationRunState::Materializing
            && view.head_fingerprint == expected_head,
        "Optimization materialization journal changed; reload it before recording an outcome."
    );
    let launch = launches
        .iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let preparation = view
        .preparation
        .as_ref()
        .context("Optimization preparation is missing.")?;
    if let Some(receipt) = &materialization {
        receipt.validate_for(&view.run, launch, preparation)?;
        let dataset = crate::dataset_versions::load_reference(
            &mut transaction,
            workspace.manifest.id,
            receipt.dataset.id,
        )
        .await?;
        let binding = workspace
            .scientific_binding
            .as_ref()
            .context("Scientific runtime binding is missing.")?;
        ensure!(
            dataset == receipt.dataset
                && binding.id.to_string() == preparation.execution_binding.id
                && binding.fingerprint == preparation.execution_binding.fingerprint
                && receipt.adapter.id
                    == format!("{}:{}", binding.adapter.key, binding.adapter.protocol)
                && receipt.adapter.fingerprint == binding.adapter.configuration_fingerprint,
            "Dataset or execution runtime changed before materialization completed."
        );
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = match (materialization, failure_code) {
        (Some(receipt), None) => ProjectOptimizationEvent::materialization_completed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.materialization_attempt,
            receipt,
            Utc::now(),
        )?,
        (None, Some(code)) => ProjectOptimizationEvent::materialization_failed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.materialization_attempt,
            code,
            Utc::now(),
        )?,
        _ => anyhow::bail!("Materialization must record exactly one outcome."),
    };
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted materialization outcome belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

/// Start or recover creation of the first finite experiment child. No trainer
/// or evaluator is invoked by this custody transition.
pub async fn begin_experiment_attachment(
    folder: &Path,
    run_id: Uuid,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::AttachingExperiment || view.state.has_experiment()
    {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        matches!(
            view.state,
            ProjectOptimizationRunState::Materialized
                | ProjectOptimizationRunState::ExperimentAttachmentFailed
        ),
        "Materialize training data before attaching an experiment."
    );
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::experiment_attachment_started(
        Uuid::new_v4(),
        &view.run,
        &previous,
        view.experiment_attempt + 1,
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted attachment belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

pub async fn finish_experiment_attachment(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    experiment: ProjectOptimizationExperiment,
) -> Result<ProjectOptimizationRunView> {
    record_experiment_attachment(folder, run_id, expected_head, Some(experiment), None).await
}

pub async fn fail_experiment_attachment(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    failure_code: &str,
) -> Result<ProjectOptimizationRunView> {
    record_experiment_attachment(folder, run_id, expected_head, None, Some(failure_code)).await
}

async fn record_experiment_attachment(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    experiment: Option<ProjectOptimizationExperiment>,
    failure_code: Option<&str>,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state.has_experiment() {
        ensure!(
            experiment.as_ref() == view.experiment.as_ref(),
            "Optimization run already attached another experiment."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    if view.state == ProjectOptimizationRunState::ExperimentAttachmentFailed {
        ensure!(
            experiment.is_none() && failure_code == view.failure_code.as_deref(),
            "Experiment attachment attempt already recorded another outcome."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        view.state == ProjectOptimizationRunState::AttachingExperiment
            && view.head_fingerprint == expected_head,
        "Optimization journal changed; reload it before attaching the experiment."
    );
    let launch = launches
        .iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let preparation = view
        .preparation
        .as_ref()
        .context("Optimization preparation is missing.")?;
    let materialization = view
        .materialization
        .as_ref()
        .context("Optimization materialization is missing.")?;
    if let Some(receipt) = &experiment {
        receipt.validate_for(&view.run, launch, preparation, materialization)?;
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = match (experiment, failure_code) {
        (Some(receipt), None) => ProjectOptimizationEvent::experiment_attached(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.experiment_attempt,
            receipt,
            Utc::now(),
        )?,
        (None, Some(code)) => ProjectOptimizationEvent::experiment_attachment_failed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.experiment_attempt,
            code,
            Utc::now(),
        )?,
        _ => anyhow::bail!("Experiment attachment must record exactly one outcome."),
    };
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted attachment outcome belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

/// Start or recover candidate training and shared-benchmark comparison. The
/// scientific child journal owns the detailed progress and is safe to resume.
pub async fn begin_execution(folder: &Path, run_id: Uuid) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::Optimizing || view.state.has_outcome() {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        matches!(
            view.state,
            ProjectOptimizationRunState::ReadyToRun | ProjectOptimizationRunState::ExecutionFailed
        ),
        "Attach an optimization experiment before executing it."
    );
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::execution_started(
        Uuid::new_v4(),
        &view.run,
        &previous,
        view.execution_attempt + 1,
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted execution belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

pub async fn finish_execution(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    outcome: ProjectOptimizationOutcome,
) -> Result<ProjectOptimizationRunView> {
    record_execution(folder, run_id, expected_head, Some(outcome), None).await
}

pub async fn fail_execution(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    failure_code: &str,
) -> Result<ProjectOptimizationRunView> {
    record_execution(folder, run_id, expected_head, None, Some(failure_code)).await
}

async fn record_execution(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    outcome: Option<ProjectOptimizationOutcome>,
    failure_code: Option<&str>,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state.has_outcome() {
        ensure!(
            outcome.as_ref() == view.outcome.as_ref(),
            "Optimization already completed with another outcome."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    if view.state == ProjectOptimizationRunState::ExecutionFailed {
        ensure!(
            outcome.is_none() && failure_code == view.failure_code.as_deref(),
            "Optimization attempt already recorded another failure."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        view.state == ProjectOptimizationRunState::Optimizing
            && view.head_fingerprint == expected_head,
        "Optimization journal changed; reload it before recording the outcome."
    );
    let launch = launches
        .iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    if let Some(receipt) = &outcome {
        receipt.validate_for(
            &view.run,
            launch,
            view.preparation
                .as_ref()
                .context("Optimization preparation is missing.")?,
            view.materialization
                .as_ref()
                .context("Optimization materialization is missing.")?,
            view.experiment
                .as_ref()
                .context("Optimization experiment is missing.")?,
        )?;
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = match (outcome, failure_code) {
        (Some(receipt), None) => ProjectOptimizationEvent::execution_completed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.execution_attempt,
            receipt,
            Utc::now(),
        )?,
        (None, Some(code)) => ProjectOptimizationEvent::execution_failed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.execution_attempt,
            code,
            Utc::now(),
        )?,
        _ => anyhow::bail!("Optimization execution must record exactly one outcome."),
    };
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted execution outcome belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

/// Start or recover the one final evaluation explicitly authorized by this
/// run's immutable launch scope.
pub async fn begin_final_evaluation(
    folder: &Path,
    run_id: Uuid,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state == ProjectOptimizationRunState::EvaluatingFinal || view.state.has_final_result() {
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        matches!(
            view.state,
            ProjectOptimizationRunState::ReadyForFinalEvaluation
                | ProjectOptimizationRunState::FinalEvaluationFailed
        ),
        "A candidate must pass the shared benchmark before final evaluation."
    );
    let previous = last_event(&mut transaction, run_id).await?;
    let event = ProjectOptimizationEvent::final_evaluation_started(
        Uuid::new_v4(),
        &view.run,
        &previous,
        view.final_attempt + 1,
        Utc::now(),
    )?;
    insert_event(&mut transaction, &event).await?;
    let result = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted final evaluation belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(result)
}

pub async fn finish_final_evaluation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    result: ProjectOptimizationFinalResult,
) -> Result<ProjectOptimizationRunView> {
    record_final_evaluation(folder, run_id, expected_head, Some(result), None).await
}

pub async fn fail_final_evaluation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    failure_code: &str,
) -> Result<ProjectOptimizationRunView> {
    record_final_evaluation(folder, run_id, expected_head, None, Some(failure_code)).await
}

async fn record_final_evaluation(
    folder: &Path,
    run_id: Uuid,
    expected_head: &str,
    result: Option<ProjectOptimizationFinalResult>,
    failure_code: Option<&str>,
) -> Result<ProjectOptimizationRunView> {
    let workspace = open_workspace(folder, true).await?;
    let mut database = connect(Path::new(&workspace.folder), false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let setups = optimization_setup::load(&mut transaction, &workspace).await?;
    let providers = load_provider_catalog_history(&mut transaction, &workspace.manifest).await?;
    let launches =
        optimization_launch::load(&mut transaction, &workspace, &setups, &providers).await?;
    let view = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .context("Project optimization run was not found.")?;
    if view.state.has_final_result() {
        ensure!(
            result.as_ref() == view.final_result.as_ref(),
            "Final evaluation already completed with another result."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    if view.state == ProjectOptimizationRunState::FinalEvaluationFailed {
        ensure!(
            result.is_none() && failure_code == view.failure_code.as_deref(),
            "Final evaluation attempt already recorded another failure."
        );
        transaction.commit().await?;
        database.close().await?;
        return Ok(view);
    }
    ensure!(
        view.state == ProjectOptimizationRunState::EvaluatingFinal
            && view.head_fingerprint == expected_head,
        "Optimization journal changed; reload it before recording final evaluation."
    );
    let launch = launches
        .iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    if let Some(receipt) = &result {
        receipt.validate_for(
            &view.run,
            launch,
            view.preparation
                .as_ref()
                .context("Optimization preparation is missing.")?,
            view.materialization
                .as_ref()
                .context("Optimization materialization is missing.")?,
            view.experiment
                .as_ref()
                .context("Optimization experiment is missing.")?,
            view.outcome
                .as_ref()
                .context("Optimization candidate outcome is missing.")?,
        )?;
    }
    let previous = last_event(&mut transaction, run_id).await?;
    let event = match (result, failure_code) {
        (Some(receipt), None) => ProjectOptimizationEvent::final_evaluation_completed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.final_attempt,
            receipt,
            Utc::now(),
        )?,
        (None, Some(code)) => ProjectOptimizationEvent::final_evaluation_failed(
            Uuid::new_v4(),
            &view.run,
            &previous,
            view.final_attempt,
            code,
            Utc::now(),
        )?,
        _ => anyhow::bail!("Final evaluation must record exactly one result."),
    };
    insert_event(&mut transaction, &event).await?;
    let loaded = load(&mut transaction, workspace.manifest.id, &launches, &setups)
        .await?
        .into_iter()
        .find(|value| value.run.id == run_id)
        .expect("inserted final evaluation result belongs to loaded run");
    transaction.commit().await?;
    database.close().await?;
    Ok(loaded)
}

async fn last_event(
    database: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<ProjectOptimizationEvent> {
    let json: String = sqlx::query_scalar(
        "SELECT metadata_json FROM project_optimization_events WHERE run_id=? ORDER BY sequence DESC LIMIT 1",
    )
    .bind(run_id.to_string())
    .fetch_one(database)
    .await?;
    Ok(serde_json::from_str(&json)?)
}

async fn insert_event(
    database: &mut SqliteConnection,
    event: &ProjectOptimizationEvent,
) -> Result<()> {
    sqlx::query("INSERT INTO project_optimization_events (id, run_id, sequence, previous_event_fingerprint, kind, fingerprint, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(event.id.to_string()).bind(event.run_id.to_string()).bind(i64::try_from(event.sequence)?)
        .bind(&event.previous_event_fingerprint).bind(event.kind.storage_key()).bind(&event.fingerprint)
        .bind(serde_json::to_string(event)?).bind(event.created_at.to_rfc3339())
        .execute(database).await?;
    Ok(())
}

async fn load(
    database: &mut SqliteConnection,
    project_id: Uuid,
    launches: &[OptimizationLaunchAuthorization],
    setups: &[OptimizationSetup],
) -> Result<Vec<ProjectOptimizationRunView>> {
    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_optimization_runs'",
    )
    .fetch_one(&mut *database)
    .await?
        == 0
    {
        return Ok(vec![]);
    }
    let records = sqlx::query("SELECT * FROM project_optimization_runs ORDER BY created_at, id")
        .fetch_all(&mut *database)
        .await?;
    let mut result = Vec::new();
    for row in records {
        let run: ProjectOptimizationRun =
            serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?;
        ensure!(
            run.project_id == project_id
                && row.try_get::<String, _>("id")? == run.id.to_string()
                && row.try_get::<String, _>("project_id")? == project_id.to_string()
                && row.try_get::<String, _>("launch_id")? == run.launch.id
                && row.try_get::<String, _>("launch_fingerprint")? == run.launch.fingerprint
                && row.try_get::<String, _>("setup_id")? == run.setup.id
                && row.try_get::<String, _>("fingerprint")? == run.fingerprint
                && row.try_get::<String, _>("created_at")? == run.created_at.to_rfc3339(),
            "Project optimization run storage changed."
        );
        let launch = launches
            .iter()
            .find(|launch| launch.id.to_string() == run.launch.id)
            .context("Project optimization launch is missing.")?;
        let event_rows = sqlx::query(
            "SELECT * FROM project_optimization_events WHERE run_id=? ORDER BY sequence",
        )
        .bind(run.id.to_string())
        .fetch_all(&mut *database)
        .await?;
        let mut events = Vec::new();
        for event_row in event_rows {
            let event: ProjectOptimizationEvent =
                serde_json::from_str(&event_row.try_get::<String, _>("metadata_json")?)?;
            ensure!(
                event.run_id == run.id
                    && event_row.try_get::<String, _>("id")? == event.id.to_string()
                    && event_row.try_get::<String, _>("run_id")? == run.id.to_string()
                    && u64::try_from(event_row.try_get::<i64, _>("sequence")?)? == event.sequence
                    && event_row.try_get::<Option<String>, _>("previous_event_fingerprint")?
                        == event.previous_event_fingerprint
                    && event_row.try_get::<String, _>("kind")? == event.kind.storage_key()
                    && event_row.try_get::<String, _>("fingerprint")? == event.fingerprint
                    && event_row.try_get::<String, _>("created_at")?
                        == event.created_at.to_rfc3339(),
                "Project optimization event storage changed."
            );
            events.push(event);
        }
        let view = replay_project_optimization(&run, launch, &events)?;
        if let Some(preparation) = &view.preparation {
            let setup = setups
                .iter()
                .find(|value| value.id.to_string() == run.setup.id)
                .context("Optimization setup is missing.")?;
            preparation.validate_for(&run, launch, setup)?;
            if let Some(materialization) = &view.materialization {
                materialization.validate_for(&run, launch, preparation)?;
                if let Some(experiment) = &view.experiment {
                    experiment.validate_for(&run, launch, preparation, materialization)?;
                    if let Some(outcome) = &view.outcome {
                        outcome.validate_for(
                            &run,
                            launch,
                            preparation,
                            materialization,
                            experiment,
                        )?;
                        if let Some(result) = &view.final_result {
                            result.validate_for(
                                &run,
                                launch,
                                preparation,
                                materialization,
                                experiment,
                                outcome,
                            )?;
                        }
                    }
                }
            }
        }
        result.push(view);
    }
    Ok(result)
}
