use crate::cli::WorkspaceOptimizationSetupCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
};
use project_workspace_local::{
    AppendActivity, append_activity, initialize_activity, optimization_setup,
};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn execute(
    folder: &Path,
    command: WorkspaceOptimizationSetupCommand,
) -> Result<()> {
    use WorkspaceOptimizationSetupCommand::*;
    match command {
        List => super::print(&optimization_setup::list(folder).await?),
        Preview {
            model,
            dataset_version,
            benchmark_version,
        } => super::print(
            &optimization_setup::preview(folder, model, dataset_version, benchmark_version).await?,
        ),
        Save { file } => {
            ensure!(
                std::fs::metadata(&file)?.len() <= 16_384,
                "Optimization setup request exceeds 16 KiB."
            );
            let request: optimization_setup::SetupRequest =
                serde_json::from_slice(&std::fs::read(file)?)
                    .context("Invalid optimization setup request.")?;
            request.inputs.validate()?;
            initialize_activity(folder).await?;
            let action_id = Uuid::new_v4();
            let references = vec![
                ActivityReference::new("setup", request.id.to_string())?,
                ActivityReference::new("model", request.inputs.model.id.clone())?,
                ActivityReference::new("dataset_version", request.inputs.dataset.id.to_string())?,
                ActivityReference::new("benchmark_version", request.inputs.benchmark.id.clone())?,
            ];
            let event = |state, failure| -> Result<AppendActivity> {
                Ok(AppendActivity {
                    action_id,
                    operation: "optimization.setup".into(),
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
            let result = optimization_setup::save(folder, request).await;
            let terminal = if result.is_ok() {
                event(ActivityEventState::Succeeded, None)?
            } else {
                event(
                    ActivityEventState::Failed,
                    Some(ActivityFailure::new(
                        "optimization_setup_failed",
                        "Setup could not be saved. Refresh the selected inputs; no run was started.",
                    )?),
                )?
            };
            append_activity(folder, terminal).await?;
            super::print(&serde_json::json!({"actionId":action_id,"setup":result?}))
        }
    }
}
