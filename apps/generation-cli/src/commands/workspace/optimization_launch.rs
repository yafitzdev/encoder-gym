use crate::cli::WorkspaceOptimizationLaunchCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
};
use project_workspace_local::{
    AppendActivity, append_activity, initialize_activity, optimization_launch,
};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn execute(
    folder: &Path,
    command: WorkspaceOptimizationLaunchCommand,
) -> Result<()> {
    use WorkspaceOptimizationLaunchCommand::*;
    match command {
        List => super::print(&optimization_launch::list(folder).await?),
        Preview { setup } => super::print(&optimization_launch::preview(folder, setup).await?),
        Authorize {
            file,
            authorized_by,
        } => {
            ensure!(
                std::fs::metadata(&file)?.len() <= 32_768,
                "Optimization launch request exceeds 32 KiB."
            );
            let request: optimization_launch::LaunchRequest =
                serde_json::from_slice(&std::fs::read(file)?)
                    .context("Invalid optimization launch request.")?;
            initialize_activity(folder).await?;
            let action_id = Uuid::new_v4();
            let references = vec![
                ActivityReference::new("launch", request.id.to_string())?,
                ActivityReference::new("setup", request.scope.setup.id.clone())?,
                ActivityReference::new(
                    "provider_catalog",
                    request.scope.provider_catalog.id.clone(),
                )?,
            ];
            let event = |state, failure| -> Result<AppendActivity> {
                Ok(AppendActivity {
                    action_id,
                    operation: "optimization.launch".into(),
                    source: ActivitySource::Cli,
                    state,
                    stage: None,
                    completed: None,
                    total: None,
                    narrative: None,
                    references: references.clone(),
                    failure,
                    created_at: Utc::now(),
                })
            };
            append_activity(folder, event(ActivityEventState::Started, None)?).await?;
            let result = optimization_launch::authorize(folder, request, &authorized_by).await;
            let terminal = if result.is_ok() {
                event(ActivityEventState::Succeeded, None)?
            } else {
                event(
                    ActivityEventState::Failed,
                    Some(ActivityFailure::new(
                        "optimization_launch_failed",
                        "Optimize inputs or provider settings changed. Review them and try again.",
                    )?),
                )?
            };
            append_activity(folder, terminal).await?;
            super::print(&serde_json::json!({"actionId":action_id,"authorization":result?}))
        }
    }
}
