//! Transfer a verified iteration output to ordinary project model/data custody.
use anyhow::{Context, Result, ensure};
use encoder_experiment_core::{
    domain::{ExternalProjectSnapshot, TrainingCandidate},
    journal::{ExperimentEvent, ExperimentEventKind},
    ports::TrainOutput,
};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    BoundIdentity, FileIdentity, ModelArtifact, ModelDatasetLink,
    optimization_iteration::ProjectOptimizationIteration,
    optimization_iteration_execution::IterationTrainingBinding,
};
use project_workspace_local::{
    CompletedModelRegistration,
    model_datasets::{self, CompletedTrainingData, RecordedTrainingInput},
    register_completed_model,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IterationCandidate {
    pub model: ModelArtifact,
    pub dataset: ModelDatasetLink,
}

pub(super) struct TrainedIteration<'a> {
    pub iteration: &'a ProjectOptimizationIteration,
    pub binding: &'a IterationTrainingBinding,
    pub project: &'a ExternalProjectSnapshot,
    pub candidate: &'a TrainingCandidate,
    pub output: &'a TrainOutput,
    pub events: &'a [ExperimentEvent],
}

pub(super) async fn register(
    folder: &Path,
    backend: &NomosBackend,
    completed: TrainedIteration<'_>,
) -> Result<IterationCandidate> {
    let TrainedIteration {
        iteration,
        binding,
        project,
        candidate,
        output,
        events,
    } = completed;
    let receipt = events
        .iter()
        .find(|event| {
            matches!(&event.event,
        ExperimentEventKind::CandidateTrainingCompleted { candidate_id, output: recorded }
        if *candidate_id == candidate.id && recorded == output)
        })
        .context("Iteration output has no training-completion receipt")?;
    ensure!(
        receipt.run_id == binding.experiment_run_id
            && receipt.protocol_id.to_string() == binding.protocol.id,
        "Training receipt belongs to another iteration"
    );
    let source = backend.verified_model_path(&output.model)?;
    let native = backend.verified_training_data(project, candidate, &output.model)?;
    ensure!(
        native.snapshot_id == binding.training_dataset.id
            && native.snapshot_fingerprint == binding.training_dataset.fingerprint,
        "Native checkpoint was trained on another dataset version"
    );
    let source_model = BoundIdentity {
        id: output.model.id.to_string(),
        fingerprint: output.model.fingerprint.clone(),
    };
    let producing_run = BoundIdentity {
        id: binding.experiment_run_id.to_string(),
        fingerprint: receipt.fingerprint.clone(),
    };
    let snapshot = BoundIdentity {
        id: native.snapshot_id.to_string(),
        fingerprint: native.snapshot_fingerprint,
    };
    let registered = register_completed_model(
        folder,
        &source,
        CompletedModelRegistration {
            parent_model_id: iteration.starting_model.id.parse()?,
            name: format!("Iteration {} candidate", iteration.scope.iteration),
            source_model: source_model.clone(),
            source_model_format: output.model.format.clone(),
            source_model_bytes: output.model.bytes,
            producing_run: producing_run.clone(),
            training_snapshot: snapshot.clone(),
            trainer: BoundIdentity {
                id: format!(
                    "{}:{}",
                    project.backend.name, project.backend.protocol_version
                ),
                fingerprint: project.backend.configuration_fingerprint.clone(),
            },
            effective_configuration_fingerprint: candidate.fingerprint.clone(),
            source_revision: project.source_revision.clone(),
        },
    )
    .await?;
    let model = registered
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog.artifacts.iter().find(|model| {
                model.source_model.as_ref() == Some(&source_model)
                    && model.producing_run.as_ref() == Some(&producing_run)
            })
        })
        .context("Registered iteration model is missing")?
        .clone();
    let dataset = model_datasets::adopt_materialized(
        folder,
        CompletedTrainingData {
            model_id: model.id,
            parent_version_id: Some(binding.training_dataset.id),
            snapshot,
            run: producing_run,
            manifest: FileIdentity {
                path: "nomos_training_manifest.json".into(),
                bytes: native.manifest_bytes,
                fingerprint: native.manifest_fingerprint,
            },
            inputs: native
                .inputs
                .into_iter()
                .map(|input| RecordedTrainingInput {
                    key: input.key,
                    path: input.path,
                    bytes: input.bytes,
                    fingerprint: input.fingerprint,
                    rows: input.rows,
                })
                .collect(),
        },
    )
    .await?;
    Ok(IterationCandidate { model, dataset })
}
