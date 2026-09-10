use super::*;
use anyhow::ensure;
use project_workspace_local::{CompletedModelRegistration, register_completed_model};

pub(super) async fn register(folder: &std::path::Path, run_id: Uuid) -> anyhow::Result<()> {
    use project_workspace_core::{
        ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    };
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| AppendActivity {
        action_id,
        operation: "models.register_run".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        references: vec![
            ActivityReference::new("run", run_id.to_string()).expect("UUID reference"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result = register_outputs(folder, run_id, false).await;
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "model_registration_failed",
                "Completed artifacts could not all be registered. Existing models, dataset versions, and the original run are retained; retry reuses them.",
            )?),
        )
    };
    append_activity(folder, event(state, failure)).await?;
    result.map(|_| ())
}

async fn register_outputs(
    folder: &std::path::Path,
    run_id: Uuid,
    datasets_only: bool,
) -> anyhow::Result<Vec<project_workspace_core::ModelDatasetLink>> {
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model history is not initialized.")?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the scientific runtime first.")?;
    let parent = catalog
        .baseline_revisions
        .iter()
        .find(|revision| revision.id == binding.baseline_revision_id)
        .context("The bound baseline revision is missing.")?;
    let store = open_bound_store(&workspace.folder, binding).await?;
    let project = load_bound_project(&store, binding).await?;
    let backend = open_nomos_binding(binding, &project)?;
    let outputs =
        super::super::encoder_optimize::completed_model_evidence(&store, run_id, &project).await?;
    store.pool().close().await;
    ensure!(
        !datasets_only || !outputs.is_empty(),
        "This run has no completed model outputs to link."
    );
    let mut links = Vec::new();
    for evidence in outputs {
        if !datasets_only {
            let source = backend.verified_model_path(&evidence.model)?;
            register_completed_model(
                folder,
                &source,
                CompletedModelRegistration {
                    parent_model_id: parent.model_artifact_id,
                    name: format!("Candidate {:02}", evidence.candidate.sequence),
                    source_model: BoundIdentity {
                        id: evidence.model.id.to_string(),
                        fingerprint: evidence.model.fingerprint.clone(),
                    },
                    source_model_format: evidence.model.format.clone(),
                    source_model_bytes: evidence.model.bytes,
                    producing_run: BoundIdentity {
                        id: evidence.experiment_run_id.to_string(),
                        fingerprint: evidence.training_receipt_fingerprint.clone(),
                    },
                    training_snapshot: BoundIdentity {
                        id: evidence.training_snapshot_id.to_string(),
                        fingerprint: evidence.training_snapshot_fingerprint.clone(),
                    },
                    trainer: BoundIdentity {
                        id: format!("{}:{}", binding.adapter.key, binding.adapter.protocol),
                        fingerprint: binding.adapter.configuration_fingerprint.clone(),
                    },
                    effective_configuration_fingerprint: evidence.candidate.fingerprint.clone(),
                    source_revision: evidence.project.source_revision.clone(),
                },
            )
            .await?;
        }
        links.push(link_dataset(folder, &backend, &evidence).await?);
    }
    Ok(links)
}

async fn link_dataset(
    folder: &std::path::Path,
    backend: &encoder_experiment_nomos::NomosBackend,
    evidence: &super::super::encoder_optimize::CompletedModelEvidence,
) -> anyhow::Result<project_workspace_core::ModelDatasetLink> {
    use project_workspace_local::model_datasets::{
        self, CompletedTrainingData, RecordedTrainingInput,
    };
    let native =
        backend.verified_training_data(&evidence.project, &evidence.candidate, &evidence.model)?;
    ensure!(
        native.snapshot_id == evidence.training_snapshot_id
            && native.snapshot_fingerprint == evidence.training_snapshot_fingerprint,
        "The native training snapshot differs from the completed run."
    );
    let workspace = open_workspace(folder, false).await?;
    let source = BoundIdentity {
        id: evidence.model.id.to_string(),
        fingerprint: evidence.model.fingerprint.clone(),
    };
    let run = BoundIdentity {
        id: evidence.experiment_run_id.to_string(),
        fingerprint: evidence.training_receipt_fingerprint.clone(),
    };
    let model = workspace
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog.artifacts.iter().find(|model| {
                model.source_model.as_ref() == Some(&source)
                    && model.producing_run.as_ref() == Some(&run)
            })
        })
        .context("Register the completed run's model before adopting its dataset.")?;
    if !workspace
        .model_dataset_links
        .iter()
        .any(|link| Some(link.model_id) == model.parent_model_id)
    {
        model_datasets::adopt_baseline(folder).await?;
    }
    model_datasets::adopt_completed(
        folder,
        CompletedTrainingData {
            model_id: model.id,
            snapshot: BoundIdentity {
                id: native.snapshot_id.to_string(),
                fingerprint: native.snapshot_fingerprint,
            },
            run,
            manifest: project_workspace_core::FileIdentity {
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
    .await
}

pub(super) async fn adopt_datasets(folder: &std::path::Path, run_id: Uuid) -> anyhow::Result<()> {
    use project_workspace_core::{
        ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    };
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let initial_refs = vec![ActivityReference::new("run", run_id.to_string())?];
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "dataset.adopt_run".into(),
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
        event(ActivityEventState::Started, initial_refs.clone(), None),
    )
    .await?;
    let result = register_outputs(folder, run_id, true).await;
    let mut references = initial_refs;
    if let Ok(links) = &result {
        for link in links {
            references.push(ActivityReference::new("model", link.model_id.to_string())?);
            references.push(ActivityReference::new(
                "dataset_version",
                link.version.id.to_string(),
            )?);
        }
    }
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "training_data_adoption_failed",
                "Run datasets could not all be linked. Retry reuses recorded imports, versions and links; existing model and run history are unchanged.",
            )?),
        )
    };
    append_activity(folder, event(state, references, failure)).await?;
    let links = result?;
    super::print(&serde_json::json!({"actionId":action_id,"runId":run_id,"links":links}))
}
