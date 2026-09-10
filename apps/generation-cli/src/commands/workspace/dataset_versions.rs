use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
};
use project_workspace_local::{
    AppendActivity, append_activity, dataset_versions as datasets, initialize_activity,
};
use uuid::Uuid;

use crate::cli::WorkspaceDatasetCommand;

pub(super) async fn execute(folder: &Path, command: WorkspaceDatasetCommand) -> Result<()> {
    use WorkspaceDatasetCommand::*;
    match command {
        List => super::print(&datasets::list(folder).await?),
        AdoptBaseline => adopt_baseline(folder).await,
        Inspect { version_id } => super::print(&datasets::inspect(folder, version_id).await?),
        Rows {
            version_id,
            offset,
            limit,
        } => super::print(&datasets::read_rows(folder, version_id, offset, limit).await?),
        Changes {
            version_id,
            offset,
            limit,
        } => super::print(&datasets::read_changes(folder, version_id, offset, limit).await?),
        Create {
            name,
            sources,
            dataset_id,
            version_id,
        } => {
            let dataset_id = dataset_id.unwrap_or_else(Uuid::new_v4);
            let version_id = version_id.unwrap_or_else(Uuid::new_v4);
            record(
                folder,
                "dataset.create",
                dataset_id,
                version_id,
                datasets::create_base(folder, dataset_id, version_id, &name, &sources),
            )
            .await
        }
        Fork {
            parent_id,
            name,
            dataset_id,
            version_id,
        } => {
            let dataset_id = dataset_id.unwrap_or_else(Uuid::new_v4);
            let version_id = version_id.unwrap_or_else(Uuid::new_v4);
            record(
                folder,
                "dataset.fork",
                dataset_id,
                version_id,
                datasets::fork(folder, dataset_id, version_id, &name, parent_id),
            )
            .await
        }
        Revise { file } => {
            ensure!(
                std::fs::metadata(&file)?.len() <= 16 * 1_048_576,
                "Dataset change request exceeds 16 MiB."
            );
            let request: datasets::DatasetVersionUpdate =
                serde_json::from_slice(&std::fs::read(file)?)
                    .context("Invalid dataset change request.")?;
            record(
                folder,
                "dataset.revise",
                request.dataset_id,
                request.version_id,
                datasets::revise(folder, request),
            )
            .await
        }
    }
}

async fn adopt_baseline(folder: &Path) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "dataset.adopt_baseline".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        references,
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, vec![], None)).await?;
    let result = project_workspace_local::model_datasets::adopt_baseline(folder).await;
    let terminal = match &result {
        Ok(link) => event(
            ActivityEventState::Succeeded,
            vec![
                ActivityReference::new("model", link.model_id.to_string())?,
                ActivityReference::new("dataset_version", link.version.id.to_string())?,
            ],
            None,
        ),
        Err(_) => event(
            ActivityEventState::Failed,
            vec![],
            Some(ActivityFailure::new(
                "training_data_adoption_failed",
                "Training data could not be linked. Retry reuses any recorded dataset version; model files and history are unchanged.",
            )?),
        ),
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId": action_id, "link": result?}))
}

async fn record(
    folder: &Path,
    operation: &str,
    dataset_id: Uuid,
    version_id: Uuid,
    work: impl std::future::Future<Output = Result<dataset_core::versions::DatasetVersion>>,
) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| -> Result<AppendActivity> {
        Ok(AppendActivity {
            action_id,
            operation: operation.into(),
            source: ActivitySource::Cli,
            state,
            stage: None,
            completed: None,
            total: None,
            references: vec![
                ActivityReference::new("dataset", dataset_id.to_string())?,
                ActivityReference::new("dataset_version", version_id.to_string())?,
            ],
            failure,
            created_at: Utc::now(),
        })
    };
    append_activity(folder, event(ActivityEventState::Started, None)?).await?;
    let result = work.await;
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "dataset_change_failed",
                "The dataset change could not be saved. Existing versions are unchanged.",
            )?),
        )
    };
    append_activity(folder, event(state, failure)?).await?;
    let version = result?;
    super::print(
        &serde_json::json!({"actionId": action_id, "version": version.reference(), "rows": version.members.len()}),
    )
}
