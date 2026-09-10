use super::*;
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
    let result = register_outputs(folder, run_id).await;
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "model_registration_failed",
                "The model could not be registered. Its original run and checkpoint are retained.",
            )?),
        )
    };
    append_activity(folder, event(state, failure)).await?;
    result
}

async fn register_outputs(folder: &std::path::Path, run_id: Uuid) -> anyhow::Result<()> {
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
    for evidence in outputs {
        let source = backend.verified_model_path(&evidence.model)?;
        register_completed_model(
            folder,
            &source,
            CompletedModelRegistration {
                parent_model_id: parent.model_artifact_id,
                name: format!("Candidate {:02}", evidence.candidate.sequence),
                source_model: BoundIdentity {
                    id: evidence.model.id.to_string(),
                    fingerprint: evidence.model.fingerprint,
                },
                source_model_format: evidence.model.format,
                source_model_bytes: evidence.model.bytes,
                producing_run: BoundIdentity {
                    id: evidence.experiment_run_id.to_string(),
                    fingerprint: evidence.training_receipt_fingerprint,
                },
                training_snapshot: BoundIdentity {
                    id: evidence.training_snapshot_id.to_string(),
                    fingerprint: evidence.training_snapshot_fingerprint,
                },
                trainer: BoundIdentity {
                    id: format!("{}:{}", binding.adapter.key, binding.adapter.protocol),
                    fingerprint: binding.adapter.configuration_fingerprint.clone(),
                },
                effective_configuration_fingerprint: evidence.candidate.fingerprint,
                source_revision: evidence.project.source_revision,
            },
        )
        .await?;
    }
    Ok(())
}
