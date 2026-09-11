use crate::cli::WorkspaceOptimizationRunCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    domain::EvidenceRole,
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, BoundIdentity,
    ProjectOptimizationMaterialization, ProjectOptimizationPreparation,
    ProjectOptimizationRunState,
};
use project_workspace_local::{
    AppendActivity, append_activity, dataset_versions, initialize_activity, open_workspace,
    optimization_launch, optimization_runs, optimization_setup,
};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn execute(folder: &Path, command: WorkspaceOptimizationRunCommand) -> Result<()> {
    use WorkspaceOptimizationRunCommand::*;
    match command {
        List => super::print(&optimization_runs::list(folder).await?),
        Show { run_id } => super::print(&optimization_runs::show(folder, run_id).await?),
        Prepare { run_id } => prepare(folder, run_id).await,
        Materialize { run_id } => materialize(folder, run_id).await,
        Start {
            file,
            authorized_by,
        } => {
            ensure!(
                std::fs::metadata(&file)?.len() <= 32_768,
                "Optimization start request exceeds 32 KiB."
            );
            let request: optimization_launch::LaunchRequest =
                serde_json::from_slice(&std::fs::read(file)?)
                    .context("Invalid optimization start request.")?;
            initialize_activity(folder).await?;
            let action_id = Uuid::new_v4();
            let initial_references = vec![
                ActivityReference::new("launch", request.id.to_string())?,
                ActivityReference::new("setup", request.scope.setup.id.clone())?,
            ];
            let event = |state, references, failure| AppendActivity {
                action_id,
                operation: "optimization.start".into(),
                source: ActivitySource::Cli,
                state,
                stage: None,
                completed: None,
                total: None,
                references,
                failure,
                created_at: Utc::now(),
            };
            append_activity(
                folder,
                event(
                    ActivityEventState::Started,
                    initial_references.clone(),
                    None,
                ),
            )
            .await?;
            let result = optimization_runs::start(folder, request, &authorized_by).await;
            let terminal = match &result {
                Ok(run) => {
                    let mut references = initial_references;
                    references.push(ActivityReference::new("run", run.run.id.to_string())?);
                    event(ActivityEventState::Succeeded, references, None)
                }
                Err(_) => event(
                    ActivityEventState::Failed,
                    initial_references,
                    Some(ActivityFailure::new(
                        "optimization_start_failed",
                        "Optimize inputs, providers, or baseline changed. Review them and try again.",
                    )?),
                ),
            };
            append_activity(folder, terminal).await?;
            super::print(&serde_json::json!({"actionId":action_id,"run":result?}))
        }
    }
}

async fn prepare(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let references = vec![ActivityReference::new("run", run_id.to_string())?];
    let activity = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.prepare".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        references: references.clone(),
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, activity(ActivityEventState::Started, None)).await?;
    let outcome: Result<_> = async {
        let started = optimization_runs::begin_preparation(folder, run_id).await?;
        if started.state.has_preparation() {
            return Ok(started);
        }
        ensure!(
            started.state == ProjectOptimizationRunState::Preparing,
            "Optimization run could not enter input verification."
        );
        match verify_preparation(folder, &started).await {
            Ok(preparation) => {
                optimization_runs::finish_preparation(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    preparation,
                )
                .await
            }
            Err(error) => {
                optimization_runs::fail_preparation(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    "input_verification_failed",
                )
                .await
                .context("Input verification failed and its run outcome could not be recorded.")?;
                Err(error)
            }
        }
    }
    .await;
    let terminal = if outcome.is_ok() {
        activity(ActivityEventState::Succeeded, None)
    } else {
        activity(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "input_verification_failed",
                "The selected model, data, benchmark, or runtime did not verify. No optimization work was executed.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"run":outcome?}))
}

async fn materialize(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let references = vec![ActivityReference::new("run", run_id.to_string())?];
    let activity = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.materialize".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        references: references.clone(),
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, activity(ActivityEventState::Started, None)).await?;
    let outcome: Result<_> = async {
        let started = optimization_runs::begin_materialization(folder, run_id).await?;
        if started.state.has_materialization() {
            return Ok(started);
        }
        ensure!(
            started.state == ProjectOptimizationRunState::Materializing,
            "Optimization run could not enter dataset materialization."
        );
        match materialize_training_project(folder, &started).await {
            Ok(receipt) => {
                optimization_runs::finish_materialization(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    receipt,
                )
                .await
            }
            Err(error) => {
                optimization_runs::fail_materialization(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    "dataset_materialization_failed",
                )
                .await
                .context("Dataset materialization failed and its outcome could not be recorded.")?;
                Err(error)
            }
        }
    }
    .await;
    let terminal = if outcome.is_ok() {
        activity(ActivityEventState::Succeeded, None)
    } else {
        activity(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "dataset_materialization_failed",
                "The selected rows do not form a valid native training dataset. No model was trained.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"run":outcome?}))
}

async fn materialize_training_project(
    folder: &Path,
    view: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<ProjectOptimizationMaterialization> {
    let workspace = open_workspace(folder, true).await?;
    let preparation = view
        .preparation
        .as_ref()
        .context("Verify optimization inputs before materializing data.")?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native evaluation and training runtime.")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after input verification."
    );
    let dataset = dataset_versions::verify(folder, preparation.dataset.id).await?;
    ensure!(
        dataset.reference() == preparation.dataset
            && dataset.members.len() as u64 == preparation.dataset_rows,
        "Selected training dataset changed."
    );
    let rows = dataset_versions::materialization_rows(folder, dataset.id).await?;
    ensure!(
        rows.len() == dataset.members.len(),
        "Selected training rows are incomplete."
    );

    let store = super::open_bound_store(&workspace.folder, binding).await?;
    let source_project = super::load_bound_project(&store, binding).await?;
    let backend = super::open_nomos_binding(binding, &source_project)?;
    let mut writer = backend.materialize_training_dataset(
        view.run.id,
        dataset.id,
        dataset.fingerprint.clone(),
        preparation.dataset_rows,
    )?;
    for row in rows {
        writer.append(&row.member.id, &row.member.content_fingerprint, &row.value)?;
    }
    let native = writer.finish()?;
    let backend = backend.with_training_dataset(native.clone())?;
    let fresh_project = backend.project_snapshot()?;
    ensure!(
        fresh_project.baseline_model.fingerprint == preparation.model.fingerprint,
        "Materialized project uses another baseline model."
    );
    let benchmark_id: Uuid = preparation.benchmark.id.parse()?;
    let benchmark = super::benchmarks::inspect(folder, benchmark_id).await?;
    NomosBackend::verify_benchmark_definition(&fresh_project, &benchmark.definition)?;
    backend.inspect(fresh_project.clone()).await?;
    let project = if let Some(existing) = store
        .find_project_by_source_fingerprint(fresh_project.source_fingerprint.clone())
        .await?
    {
        backend.verify_current_snapshot(existing.clone()).await?;
        existing
    } else {
        store.create_project(fresh_project.clone()).await?;
        fresh_project
    };
    store.pool().close().await;
    ProjectOptimizationMaterialization::create(
        &view.run,
        &launch,
        preparation,
        BoundIdentity {
            id: format!("{}:{}", native.run_id, native.dataset_version_id),
            fingerprint: native.fingerprint,
        },
        native.artifact,
        BoundIdentity {
            id: project.id.to_string(),
            fingerprint: project.fingerprint,
        },
        Utc::now(),
    )
    .map_err(Into::into)
}

async fn verify_preparation(
    folder: &Path,
    view: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<ProjectOptimizationPreparation> {
    let workspace = open_workspace(folder, true).await?;
    let setup = optimization_setup::list(folder)
        .await?
        .into_iter()
        .find(|value| value.id.to_string() == view.run.setup.id)
        .context("Optimization setup is missing.")?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let dataset = dataset_versions::verify(folder, setup.inputs.dataset.id).await?;
    ensure!(
        dataset.reference() == setup.inputs.dataset,
        "Selected training dataset changed."
    );
    let benchmark_id: Uuid = setup.inputs.benchmark.id.parse()?;
    let benchmark = super::benchmarks::inspect(folder, benchmark_id).await?;
    ensure!(
        benchmark.fingerprint == setup.inputs.benchmark.fingerprint,
        "Selected benchmark changed."
    );
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native evaluation and training runtime.")?;
    ensure!(
        binding.baseline_revision_id.to_string() == setup.inputs.baseline_revision.id,
        "Native runtime is bound to another baseline."
    );
    let store = super::open_bound_store(&workspace.folder, binding).await?;
    let project = super::load_bound_project(&store, binding).await?;
    let backend = super::open_nomos_binding(binding, &project)?;
    backend.inspect(project.clone()).await?;
    NomosBackend::verify_benchmark_definition(&project, &benchmark.definition)?;
    store.pool().close().await;
    let identity = backend.identity();
    let development_suites = benchmark
        .definition
        .suites
        .iter()
        .filter(|suite| suite.role == EvidenceRole::Development)
        .map(|suite| suite.key.clone())
        .collect();
    let final_suite = benchmark
        .definition
        .suites
        .iter()
        .find(|suite| suite.role == EvidenceRole::SealedAcceptance)
        .context("Shared benchmark has no final holdout.")?
        .key
        .clone();
    ProjectOptimizationPreparation::create(
        &view.run,
        &launch,
        &setup,
        BoundIdentity {
            id: binding.id.to_string(),
            fingerprint: binding.fingerprint.clone(),
        },
        binding.runtime.project_snapshot.clone(),
        BoundIdentity {
            id: format!("{}:{}", identity.name, identity.protocol_version),
            fingerprint: identity.configuration_fingerprint,
        },
        u64::try_from(dataset.members.len())?,
        development_suites,
        final_suite,
        Utc::now(),
    )
    .map_err(Into::into)
}
