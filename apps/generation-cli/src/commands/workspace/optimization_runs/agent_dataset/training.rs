//! Same production composition continues from qualified edits to development.
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{domain::OptimizationBudget, ports::ExperimentStore};
use encoder_experiment_nomos::{NomosBackend, NomosFineTuneSettings, NomosTrainingDataset};
use encoder_experiment_runner::ExperimentRunner;
use project_workspace_core::{
    BoundIdentity, OptimizationDevice,
    optimization_iteration_execution::{IterationDevelopmentResult, IterationTrainingBinding},
};
use project_workspace_local::{
    dataset_versions, optimization_iteration_execution as custody, optimization_iterations,
    optimization_launch, optimization_runs,
};
use std::{collections::BTreeSet, path::Path};
use uuid::Uuid;

pub(super) async fn complete(
    folder: &Path,
    run_id: Uuid,
    qualified: &super::qualification::QualifiedDataset,
) -> Result<(
    IterationDevelopmentResult,
    super::registration::IterationCandidate,
)> {
    let run = optimization_runs::show(folder, run_id).await?;
    ensure!(!run.state.is_terminal(), "Run stopped before training");
    let iteration = optimization_iterations::list(folder, run_id)
        .await?
        .into_iter()
        .find(|item| item.id == qualified.native_dataset.run_id)
        .context("Qualified dataset has no iteration")?;
    let launch = optimization_launch::list(folder)
        .await?
        .into_iter()
        .find(|item| item.id.to_string() == run.run.launch.id)
        .context("Launch is missing")?;
    let settings = launch
        .scope
        .agentic
        .as_ref()
        .context("Agent settings are missing")?;
    ensure!(
        qualified.clearance.training_allowed(),
        "Training requires clean full-population clearance"
    );
    let full =
        dataset_versions::inspect(folder, qualified.native_dataset.dataset_version_id).await?;
    ensure!(
        full.fingerprint == qualified.native_dataset.dataset_version_fingerprint
            && full.members.len() as u64 == qualified.clearance.training_rows,
        "Clearance population changed"
    );
    let training = sampled_version(
        folder,
        &run.run,
        iteration.scope.iteration,
        &full,
        settings.training.maximum_training_rows,
    )
    .await?;
    let preparation = run.preparation.as_ref().context("Preparation is missing")?;
    let workspace = project_workspace_local::open_workspace(folder, false).await?;
    let runtime = project_workspace_local::scientific_binding_history(folder)
        .await?
        .into_iter()
        .find(|item| {
            item.id.to_string() == preparation.execution_binding.id
                && item.fingerprint == preparation.execution_binding.fingerprint
        })
        .context("Pinned execution runtime is missing")?;
    let (benchmark, source_binding) =
        project_workspace_local::benchmarks::inspect(folder, iteration.benchmark.id.parse()?)
            .await?;
    iteration.validate_benchmark(&benchmark)?;
    let source_store =
        super::super::super::open_bound_store(&workspace.folder, &source_binding).await?;
    let sources: Result<_> = async {
        let project =
            super::super::super::load_bound_project(&source_store, &source_binding).await?;
        let protocol = source_store
            .get_protocol(benchmark.source.protocol.id.parse()?)
            .await?
            .context("Original benchmark protocol missing")?;
        ensure!(
            protocol.fingerprint == benchmark.source.protocol.fingerprint
                && NomosBackend::recorded_benchmark(&project, &protocol)? == benchmark.definition,
            "Original benchmark evidence changed"
        );
        Ok((project, protocol))
    }
    .await;
    source_store.pool().close().await;
    let (source_project, source_protocol) = sources?;
    let database_url = super::super::super::bound_store_url(folder, &runtime)?;
    let _scientific_lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &database_url,
        run_id,
    )
    .await?;
    let store = super::super::super::open_bound_store_mutable(&workspace.folder, &runtime).await?;
    let result: Result<_> = async {
        let bound_project = super::super::super::load_bound_project(&store, &runtime).await?;
        let base = super::super::super::open_nomos_binding(folder, &runtime, &bound_project)?;
        let native = if training.id == full.id {
            qualified.native_dataset.clone()
        } else {
            materialize_sample(folder, &base, iteration.id, &training).await?
        };
        let backend = base
            .with_training_dataset(native.clone())?
            .with_progress_observer(std::sync::Arc::new(
                crate::commands::encoder_optimize::activity::IterationProgressOutput(
                    iteration.scope.iteration,
                ),
            ));
        let fresh = backend.project_snapshot()?;
        super::super::verify_runtime_baseline(
            workspace
                .model_catalog
                .as_ref()
                .context("Model catalog missing")?,
            &preparation.model,
            &backend,
            &fresh,
        )?;
        NomosBackend::verify_benchmark_definition(&fresh, &benchmark.definition)?;
        let project = match store
            .find_project_by_source_fingerprint(fresh.source_fingerprint.clone())
            .await?
        {
            Some(existing) => {
                backend.verify_current_snapshot(existing.clone()).await?;
                existing
            }
            None => {
                store.create_project(fresh.clone()).await?;
                fresh
            }
        };
        match store.get_project(source_project.id).await? {
            Some(existing) => ensure!(
                existing == source_project,
                "Conflicting benchmark source project"
            ),
            None => store.create_project(source_project.clone()).await?,
        }
        match store.get_protocol(source_protocol.id).await? {
            Some(existing) => ensure!(
                existing == source_protocol,
                "Conflicting benchmark source protocol"
            ),
            None => store.create_protocol(source_protocol.clone()).await?,
        }
        let number = u64::from(iteration.scope.iteration);
        let protocol_id = run.run.child_id("iteration-protocol", number)?;
        let candidate_id = run.run.child_id("iteration-candidate", number)?;
        let experiment_id = run.run.child_id("iteration-experiment", number)?;
        let existing_protocol = store.get_protocol(protocol_id).await?;
        let requested_device = match settings.training.device {
            OptimizationDevice::Auto => "auto",
            OptimizationDevice::Cpu => "cpu",
            OptimizationDevice::Cuda => "cuda",
        };
        let device = match existing_protocol
            .as_ref()
            .and_then(|p| p.candidates.first())
            .and_then(|c| c.parameters.get("device"))
        {
            Some(encoder_experiment_core::domain::ParameterValue::Text(device)) => device.clone(),
            _ => backend.resolve_training_device(requested_device).await?,
        };
        ensure!(
            requested_device == "auto" || requested_device == device,
            "Reserved training device differs from settings"
        );
        let candidate = NomosBackend::configured_training_candidate(
            candidate_id,
            &project,
            u64::from(settings.training.maximum_seconds_per_iteration),
            NomosFineTuneSettings {
                epochs: settings.training.maximum_epochs,
                batch_size: settings.training.batch_size,
                learning_rate_nanos: settings.training.learning_rate_nanos,
                device: device.clone(),
            },
        )?;
        let runner = ExperimentRunner::new(&store, &backend);
        let protocol = runner
            .prepare_multi_protocol_from_benchmark_identified(
                protocol_id,
                project.id,
                source_protocol.id,
                benchmark.definition,
                OptimizationBudget {
                    maximum_candidates: 1,
                    maximum_training_seconds: candidate.maximum_training_seconds,
                    maximum_development_evaluations: preparation
                        .development_suites
                        .len()
                        .try_into()?,
                    // Adaptive iterations never own final-holdout authority.
                    maximum_sealed_evaluations: 0,
                },
                source_protocol.maximum_evaluation_seconds,
                vec![candidate.clone()],
            )
            .await?;
        let mut binding = IterationTrainingBinding {
            iteration: bound(iteration.id, &iteration.fingerprint),
            qualified_dataset: full.reference(),
            clearance_fingerprint: qualified.clearance.fingerprint.clone(),
            training_dataset: training.reference(),
            training_rows: training.members.len() as u64,
            native_materialization_fingerprint: native.fingerprint,
            training_artifact: native.artifact,
            scientific_project: bound(project.id, &project.fingerprint),
            protocol: bound(protocol.id, &protocol.fingerprint),
            candidate: bound(candidate.id, &candidate.fingerprint),
            experiment_run_id: experiment_id,
            resolved_device: device,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        binding.fingerprint = binding.reproduce()?;
        let binding = custody::record_training(folder, run_id, binding).await?;
        let events = store.load_events(experiment_id).await?;
        let prior_training_started =
            events.iter().any(|event| {
                matches!(event.event,
            encoder_experiment_core::journal::ExperimentEventKind::CandidateTrainingStarted { .. })
            });
        let accounting =
            project_workspace_local::optimization_training_time::ProjectTrainingAccounting::open(
                folder,
                run_id,
                iteration.id,
                prior_training_started,
            )
            .await?;
        let backend = backend.with_training_accounting(std::sync::Arc::new(accounting));
        let runner = ExperimentRunner::new(&store, &backend);
        ensure!(
            !optimization_runs::show(folder, run_id)
                .await?
                .state
                .is_terminal(),
            "Run stopped before trainer dispatch"
        );
        runner
            .create_run_identified(protocol.id, experiment_id)
            .await?;
        let development = runner.run_development(experiment_id).await;
        // Stop must unwind without starting another custody operation. Other
        // evaluation/persistence failures must not hide an already trained model.
        if let Err(encoder_experiment_runner::ExperimentRunnerError::Stopped) = &development {
            return Err(encoder_experiment_runner::ExperimentRunnerError::Stopped.into());
        }
        let completed = runner.status(experiment_id).await?;
        let events = store.load_events(experiment_id).await?;
        let output = completed
            .candidates
            .get(&candidate.id)
            .and_then(|execution| execution.train_output.as_ref());
        let Some(output) = output else {
            development?;
            anyhow::bail!("Iteration did not produce a trained model");
        };
        // Preserve every trained output, even if a later evaluation failed.
        super::progress::finalizing(
            iteration.scope.iteration,
            "Verifying and saving the trained candidate and its dataset to Models.",
        );
        let registered = super::registration::register(
            folder,
            &backend,
            super::registration::TrainedIteration {
                iteration: &iteration,
                binding: &binding,
                project: &project,
                candidate: &candidate,
                output,
                events: &events,
            },
        )
        .await?;
        development?;
        super::progress::finalizing(
            iteration.scope.iteration,
            "Recording completed development evaluation results.",
        );
        let result =
            custody::record_result(folder, run_id, iteration.id, &project, &protocol, &events)
                .await?;
        Ok((result, registered))
    }
    .await;
    store.pool().close().await;
    // All work in this scientific stage has settled. Drain owned processes
    // before dropping its lease; otherwise a still-unwinding child keeps that
    // lease alive and the next iteration conflicts with its own coordinator.
    crate::process_ownership::quiesce().await?;
    result
}

async fn sampled_version(
    folder: &Path,
    run: &project_workspace_core::ProjectOptimizationRun,
    number: u32,
    full: &dataset_core::versions::DatasetVersion,
    limit: Option<u32>,
) -> Result<dataset_core::versions::DatasetVersion> {
    let Some(limit) = limit.filter(|limit| full.members.len() > *limit as usize) else {
        return Ok(full.clone());
    };
    let mut ids: Vec<_> = full.members.iter().map(|row| &row.id).collect();
    ids.sort();
    ids.truncate(limit as usize);
    let chosen: BTreeSet<_> = ids.into_iter().collect();
    let fork = dataset_versions::fork(
        folder,
        run.child_id("training-sample-dataset", u64::from(number))?,
        run.child_id("training-sample-fork", u64::from(number))?,
        &format!("Iteration {number} diagnostic training sample"),
        full.id,
    )
    .await?;
    dataset_versions::revise(
        folder,
        dataset_versions::DatasetVersionUpdate {
            version_id: run.child_id("training-sample-version", u64::from(number))?,
            dataset_id: fork.dataset_id,
            parent_id: fork.id,
            added: vec![],
            replaced: vec![],
            removed: full
                .members
                .iter()
                .filter(|row| !chosen.contains(&row.id))
                .map(|row| row.id.clone())
                .collect(),
        },
    )
    .await
}

async fn materialize_sample(
    folder: &Path,
    backend: &NomosBackend,
    iteration_id: Uuid,
    training: &dataset_core::versions::DatasetVersion,
) -> Result<NomosTrainingDataset> {
    let rows = dataset_versions::materialization_rows(folder, training.id).await?;
    let mut writer = backend.materialize_training_dataset(
        iteration_id,
        training.id,
        &training.fingerprint,
        rows.len() as u64,
    )?;
    for row in rows {
        writer.append(&row.member.id, &row.member.content_fingerprint, &row.value)?;
    }
    Ok(writer.finish()?)
}
fn bound(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}
