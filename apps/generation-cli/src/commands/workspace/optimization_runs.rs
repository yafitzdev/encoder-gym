use crate::cli::WorkspaceOptimizationRunCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
};
use project_workspace_local::{
    AppendActivity, append_activity, initialize_activity, optimization_launch, optimization_runs,
};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn execute(folder: &Path, command: WorkspaceOptimizationRunCommand) -> Result<()> {
    use WorkspaceOptimizationRunCommand::*;
    match command {
        List => super::print(&optimization_runs::list(folder).await?),
        Show { run_id } => super::print(&optimization_runs::show(folder, run_id).await?),
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
