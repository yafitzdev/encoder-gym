use crate::cli::WorkspaceOptimizationRunCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    domain::{EvidenceRole, OptimizationBudget},
    journal::{ExperimentRunState, FinalDecision},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, BoundIdentity,
    ProjectOptimizationExperiment, ProjectOptimizationFinalResult,
    ProjectOptimizationFinalResultKind, ProjectOptimizationMaterialization,
    ProjectOptimizationOutcome, ProjectOptimizationOutcomeKind, ProjectOptimizationPreparation,
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
        Attach { run_id } => attach(folder, run_id).await,
        Execute { run_id } => execute_candidate(folder, run_id).await,
        Register { run_id } => super::completed_models::register_project(folder, run_id).await,
        Finalize { run_id } => finalize_candidate(folder, run_id).await,
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

async fn finalize_candidate(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let references = vec![ActivityReference::new("run", run_id.to_string())?];
    let activity = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.final_evaluation".into(),
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
    let result: Result<_> = async {
        let started = optimization_runs::begin_final_evaluation(folder, run_id).await?;
        if started.state.has_final_result() {
            return Ok(started);
        }
        ensure!(
            started.state == ProjectOptimizationRunState::EvaluatingFinal,
            "Optimization run could not start final evaluation."
        );
        match run_final_evaluation(folder, &started).await {
            Ok(receipt) => {
                optimization_runs::finish_final_evaluation(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    receipt,
                )
                .await
            }
            Err(error) => {
                optimization_runs::fail_final_evaluation(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    "final_evaluation_failed",
                )
                .await
                .context("Final evaluation failed and its outcome could not be recorded.")?;
                Err(error)
            }
        }
    }
    .await;
    let terminal = if result.is_ok() {
        activity(ActivityEventState::Succeeded, None)
    } else {
        activity(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "final_evaluation_failed",
                "The final evaluation did not complete. No baseline was changed.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"run":result?}))
}

async fn run_final_evaluation(
    folder: &Path,
    view: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<ProjectOptimizationFinalResult> {
    let workspace = open_workspace(folder, true).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
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
    let experiment = view
        .experiment
        .as_ref()
        .context("Optimization experiment is missing.")?;
    let outcome = view
        .outcome
        .as_ref()
        .context("Candidate comparison outcome is missing.")?;
    ensure!(
        outcome.kind == ProjectOptimizationOutcomeKind::CandidateReady,
        "No candidate is ready for final evaluation."
    );
    outcome.validate_for(&view.run, &launch, preparation, materialization, experiment)?;

    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native evaluation runtime.")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after input verification."
    );
    let database_url = super::bound_store_url(Path::new(&workspace.folder), binding)?;
    let _execution_lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        view.run.id,
    )?;
    let store = super::open_bound_store_mutable(&workspace.folder, binding).await?;
    let bound_project = super::load_bound_project(&store, binding).await?;
    let backend = super::open_nomos_binding(binding, &bound_project)?.with_progress_observer(
        std::sync::Arc::new(crate::commands::encoder_optimize::activity::ProgressOutput),
    );
    let project_id: Uuid = experiment.scientific_project.id.parse()?;
    let project = store
        .get_project(project_id)
        .await?
        .context("The optimization project snapshot is missing.")?;
    ensure!(
        project.fingerprint == experiment.scientific_project.fingerprint,
        "Optimization project snapshot changed."
    );
    backend.verify_current_snapshot(project.clone()).await?;
    let protocol_id: Uuid = experiment.protocol.id.parse()?;
    let protocol = store
        .get_protocol(protocol_id)
        .await?
        .context("The optimization protocol is missing.")?;
    ensure!(
        protocol.fingerprint == experiment.protocol.fingerprint
            && protocol.project_snapshot_id == project.id
            && protocol.sealed_suite_key == preparation.final_suite,
        "Final evaluation protocol changed."
    );
    let run_id: Uuid = experiment.experiment_run.id.parse()?;
    let runner = ExperimentRunner::new(&store, &backend);
    let current = runner.status(run_id).await?;
    ensure!(
        current.protocol_id == protocol.id,
        "Final evaluation run belongs to another protocol."
    );
    let authorized = match current.state {
        ExperimentRunState::AwaitingSealedAuthorization => {
            runner
                .authorize_sealed(run_id, launch.authorized_by.clone())
                .await?
        }
        _ => current,
    };
    let completed = match authorized.state {
        ExperimentRunState::SealedAuthorized
        | ExperimentRunState::SealedEvaluating
        | ExperimentRunState::SealedEvaluated => runner.run_sealed(run_id).await?,
        ExperimentRunState::Completed => authorized,
        _ => anyhow::bail!("Final evaluation did not enter its authorized state."),
    };
    ensure!(
        completed.state == ExperimentRunState::Completed,
        "Final evaluation did not complete."
    );
    let model = completed
        .selected_model()
        .context("Final evaluation has no selected model.")?;
    ensure!(
        outcome.selected_model.as_ref().is_some_and(|selected| {
            selected.id == model.id.to_string() && selected.fingerprint == model.fingerprint
        }),
        "Final evaluation selected another model."
    );
    let report = completed
        .sealed_report
        .as_ref()
        .context("Final evaluation report is missing.")?;
    ensure!(
        report.evidence_role == EvidenceRole::SealedAcceptance
            && report.suite_key == preparation.final_suite
            && report.model == *model,
        "Final evaluation report changed or belongs to another model."
    );
    let kind = match completed.final_decision {
        Some(FinalDecision::PromoteCandidate) => {
            ProjectOptimizationFinalResultKind::CandidateAccepted
        }
        Some(FinalDecision::RetainBaseline) => {
            ProjectOptimizationFinalResultKind::CandidateRejected
        }
        None => anyhow::bail!("Final evaluation has no deterministic decision."),
    };
    let result = ProjectOptimizationFinalResult::create(
        &view.run,
        &launch,
        preparation,
        materialization,
        experiment,
        outcome,
        BoundIdentity {
            id: completed.run_id.to_string(),
            fingerprint: completed.last_event_fingerprint,
        },
        kind,
        BoundIdentity {
            id: report.id.to_string(),
            fingerprint: report.fingerprint.clone(),
        },
        Utc::now(),
    )?;
    store.pool().close().await;
    Ok(result)
}

async fn execute_candidate(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let references = vec![ActivityReference::new("run", run_id.to_string())?];
    let activity = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.execute".into(),
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
        let started = optimization_runs::begin_execution(folder, run_id).await?;
        if started.state.has_outcome() {
            return Ok(started);
        }
        ensure!(
            started.state == ProjectOptimizationRunState::Optimizing,
            "Optimization run could not start candidate execution."
        );
        match run_attached_candidate(folder, &started).await {
            Ok(receipt) => {
                optimization_runs::finish_execution(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    receipt,
                )
                .await
            }
            Err(error) => {
                optimization_runs::fail_execution(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    "candidate_execution_failed",
                )
                .await
                .context("Optimization failed and its outcome could not be recorded.")?;
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
                "candidate_execution_failed",
                "The candidate could not be trained or compared. The same run can be retried.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"run":outcome?}))
}

async fn run_attached_candidate(
    folder: &Path,
    view: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<ProjectOptimizationOutcome> {
    let workspace = open_workspace(folder, true).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
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
    let experiment = view
        .experiment
        .as_ref()
        .context("Optimization experiment is missing.")?;
    experiment.validate_for(&view.run, &launch, preparation, materialization)?;

    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native evaluation and training runtime.")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after input verification."
    );
    let database_url = super::bound_store_url(Path::new(&workspace.folder), binding)?;
    let _execution_lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        view.run.id,
    )?;
    let store = super::open_bound_store_mutable(&workspace.folder, binding).await?;
    let bound_project = super::load_bound_project(&store, binding).await?;
    let base_backend = super::open_nomos_binding(binding, &bound_project)?;
    let project_id: Uuid = experiment.scientific_project.id.parse()?;
    let project = store
        .get_project(project_id)
        .await?
        .context("The optimization project snapshot is missing.")?;
    ensure!(
        project.fingerprint == experiment.scientific_project.fingerprint,
        "Optimization project snapshot changed."
    );
    let native = base_backend.load_training_dataset(
        view.run.id,
        preparation.dataset.id,
        &preparation.dataset.fingerprint,
    )?;
    ensure!(
        materialization.native_materialization.id
            == format!("{}:{}", native.run_id, native.dataset_version_id)
            && materialization.native_materialization.fingerprint == native.fingerprint
            && materialization.training_artifact == native.artifact,
        "Native training materialization changed."
    );
    let backend = base_backend
        .with_training_dataset(native)?
        .with_progress_observer(std::sync::Arc::new(
            crate::commands::encoder_optimize::activity::ProgressOutput,
        ));
    backend.verify_current_snapshot(project.clone()).await?;

    let protocol_id: Uuid = experiment.protocol.id.parse()?;
    let protocol = store
        .get_protocol(protocol_id)
        .await?
        .context("The optimization protocol is missing.")?;
    ensure!(
        protocol.fingerprint == experiment.protocol.fingerprint
            && protocol.project_snapshot_id == project.id
            && protocol.candidates.len() == 1
            && protocol.candidates[0].id.to_string() == experiment.candidate.id
            && protocol.candidates[0].fingerprint == experiment.candidate.fingerprint,
        "Optimization protocol or candidate changed."
    );
    let experiment_run_id: Uuid = experiment.experiment_run.id.parse()?;
    let runner = ExperimentRunner::new(&store, &backend);
    let current = runner.status(experiment_run_id).await?;
    ensure!(
        current.protocol_id == protocol.id,
        "Optimization run belongs to another protocol."
    );
    let completed = match current.state {
        ExperimentRunState::Ready | ExperimentRunState::Running => {
            runner.run_development(experiment_run_id).await?
        }
        ExperimentRunState::AwaitingSealedAuthorization | ExperimentRunState::Completed => current,
        _ => anyhow::bail!(
            "Optimization run advanced beyond candidate comparison without a project outcome."
        ),
    };
    let (kind, selected_model) = match completed.state {
        ExperimentRunState::AwaitingSealedAuthorization => {
            ensure!(
                completed.selected_candidate_id == Some(protocol.candidates[0].id),
                "Another candidate was selected."
            );
            let model = completed
                .selected_model()
                .context("Selected candidate has no saved model.")?;
            (
                ProjectOptimizationOutcomeKind::CandidateReady,
                Some(BoundIdentity {
                    id: model.id.to_string(),
                    fingerprint: model.fingerprint.clone(),
                }),
            )
        }
        ExperimentRunState::Completed => {
            ensure!(
                completed.final_decision == Some(FinalDecision::RetainBaseline)
                    && completed.selected_candidate_id.is_none(),
                "Final evaluation was executed outside this project transition."
            );
            (ProjectOptimizationOutcomeKind::BaselineRetained, None)
        }
        _ => anyhow::bail!("Candidate comparison did not reach an outcome."),
    };
    let receipt = ProjectOptimizationOutcome::create(
        &view.run,
        &launch,
        preparation,
        materialization,
        experiment,
        BoundIdentity {
            id: completed.run_id.to_string(),
            fingerprint: completed.last_event_fingerprint,
        },
        kind,
        selected_model,
        Utc::now(),
    )?;
    store.pool().close().await;
    Ok(receipt)
}

async fn attach(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let references = vec![ActivityReference::new("run", run_id.to_string())?];
    let activity = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.attach_experiment".into(),
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
        let started = optimization_runs::begin_experiment_attachment(folder, run_id).await?;
        if started.state.has_experiment() {
            return Ok(started);
        }
        ensure!(
            started.state == ProjectOptimizationRunState::AttachingExperiment,
            "Optimization run could not enter experiment attachment."
        );
        match attach_experiment(folder, &started).await {
            Ok(receipt) => {
                optimization_runs::finish_experiment_attachment(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    receipt,
                )
                .await
            }
            Err(error) => {
                optimization_runs::fail_experiment_attachment(
                    folder,
                    run_id,
                    &started.head_fingerprint,
                    "experiment_attachment_failed",
                )
                .await
                .context("Experiment attachment failed and its outcome could not be recorded.")?;
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
                "experiment_attachment_failed",
                "The finite candidate experiment was not attached. No model was trained.",
            )?),
        )
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"run":outcome?}))
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

async fn attach_experiment(
    folder: &Path,
    view: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<ProjectOptimizationExperiment> {
    let workspace = open_workspace(folder, true).await?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|value| value.id.to_string() == view.run.launch.id)
        .context("Optimization launch authorization is missing.")?;
    let preparation = view
        .preparation
        .as_ref()
        .context("Verify optimization inputs before attaching an experiment.")?;
    let materialization = view
        .materialization
        .as_ref()
        .context("Materialize training data before attaching an experiment.")?;
    materialization.validate_for(&view.run, &launch, preparation)?;
    let benchmark_id: Uuid = preparation.benchmark.id.parse()?;
    let benchmark = super::benchmarks::inspect(folder, benchmark_id).await?;
    ensure!(
        benchmark.fingerprint == preparation.benchmark.fingerprint,
        "Selected benchmark changed."
    );

    let source_binding = project_workspace_local::scientific_binding_history(folder)
        .await?
        .into_iter()
        .find(|binding| {
            binding.id.to_string() == benchmark.source.scientific_binding.id
                && binding.fingerprint == benchmark.source.scientific_binding.fingerprint
        })
        .context("The benchmark's scientific source binding is missing.")?;
    ensure!(
        source_binding.runtime.project_snapshot == benchmark.source.project_snapshot,
        "Benchmark source project changed."
    );
    let source_store = super::open_bound_store(&workspace.folder, &source_binding).await?;
    let source_project_id: Uuid = benchmark.source.project_snapshot.id.parse()?;
    let source_project = source_store
        .get_project(source_project_id)
        .await?
        .context("The benchmark's source project is missing.")?;
    ensure!(
        source_project.fingerprint == benchmark.source.project_snapshot.fingerprint,
        "Benchmark source project fingerprint changed."
    );
    let source_protocol_id: Uuid = benchmark.source.protocol.id.parse()?;
    let source_protocol = source_store
        .get_protocol(source_protocol_id)
        .await?
        .context("The benchmark's source protocol is missing.")?;
    ensure!(
        source_protocol.fingerprint == benchmark.source.protocol.fingerprint
            && NomosBackend::recorded_benchmark(&source_project, &source_protocol)?
                == benchmark.definition,
        "Benchmark source protocol changed."
    );
    source_store.pool().close().await;

    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native evaluation and training runtime.")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after input verification."
    );
    let store = super::open_bound_store_mutable(&workspace.folder, binding).await?;
    let bound_project = super::load_bound_project(&store, binding).await?;
    let base_backend = super::open_nomos_binding(binding, &bound_project)?;
    let native = base_backend.load_training_dataset(
        view.run.id,
        preparation.dataset.id,
        &preparation.dataset.fingerprint,
    )?;
    ensure!(
        materialization.native_materialization.id
            == format!("{}:{}", native.run_id, native.dataset_version_id)
            && materialization.native_materialization.fingerprint == native.fingerprint
            && materialization.training_artifact == native.artifact
            && preparation.dataset_rows == native.rows,
        "Native training materialization changed."
    );
    let backend = base_backend.with_training_dataset(native)?;
    let target_project_id: Uuid = materialization.scientific_project.id.parse()?;
    let target_project = store
        .get_project(target_project_id)
        .await?
        .context("The materialized scientific project is missing.")?;
    ensure!(
        target_project.fingerprint == materialization.scientific_project.fingerprint,
        "Materialized scientific project changed."
    );
    backend
        .verify_current_snapshot(target_project.clone())
        .await?;
    NomosBackend::verify_benchmark_definition(&target_project, &benchmark.definition)?;

    match store.get_project(source_project.id).await? {
        Some(existing) => ensure!(
            existing == source_project,
            "Benchmark source project identity has conflicting contents."
        ),
        None => store.create_project(source_project.clone()).await?,
    }
    match store.get_protocol(source_protocol.id).await? {
        Some(existing) => ensure!(
            existing == source_protocol,
            "Benchmark source protocol identity has conflicting contents."
        ),
        None => store.create_protocol(source_protocol.clone()).await?,
    }

    let maximum_training_seconds = launch.scope.limits.maximum_training_seconds
        / u64::from(launch.scope.limits.maximum_models);
    ensure!(
        maximum_training_seconds > 0,
        "Optimization training budget cannot fund one candidate."
    );
    let candidate = backend
        .initial_training_candidate(
            view.run.child_id("candidate", 1)?,
            &target_project,
            maximum_training_seconds,
        )
        .await?;
    let runner = ExperimentRunner::new(&store, &backend);
    let protocol = runner
        .prepare_multi_protocol_from_benchmark_identified(
            view.run.child_id("experiment-protocol", 1)?,
            target_project.id,
            source_protocol.id,
            benchmark.definition,
            OptimizationBudget {
                maximum_candidates: 1,
                maximum_training_seconds,
                maximum_development_evaluations: u32::try_from(
                    preparation.development_suites.len(),
                )?,
                maximum_sealed_evaluations: launch.scope.limits.maximum_final_evaluations,
            },
            source_protocol.maximum_evaluation_seconds,
            vec![candidate.clone()],
        )
        .await?;
    let experiment = runner
        .create_run_identified(protocol.id, view.run.child_id("experiment-run", 1)?)
        .await?;
    let receipt = ProjectOptimizationExperiment::create(
        &view.run,
        &launch,
        preparation,
        materialization,
        BoundIdentity {
            id: source_protocol.id.to_string(),
            fingerprint: source_protocol.fingerprint,
        },
        BoundIdentity {
            id: candidate.id.to_string(),
            fingerprint: candidate.fingerprint,
        },
        BoundIdentity {
            id: protocol.id.to_string(),
            fingerprint: protocol.fingerprint,
        },
        BoundIdentity {
            id: experiment.run_id.to_string(),
            fingerprint: experiment.last_event_fingerprint,
        },
        Utc::now(),
    )?;
    store.pool().close().await;
    Ok(receipt)
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

    let store = super::open_bound_store_mutable(&workspace.folder, binding).await?;
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
