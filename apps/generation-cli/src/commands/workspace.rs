use crate::{cli::WorkspaceCommand, presentation::print};
use project_workspace_local::{
    backfill_nomos, create_workspace, import_dataset, inspect_dataset, inspect_model,
    open_workspace,
};

pub async fn execute(command: WorkspaceCommand) -> anyhow::Result<()> {
    match command {
        WorkspaceCommand::InspectModel { source } => print(&inspect_model(&source)?),
        WorkspaceCommand::Create {
            destination,
            name,
            model,
            expected_fingerprint,
            task,
        } => {
            eprintln!("Copying and verifying the local baseline; no training will run.");
            print(
                &create_workspace(&destination, &name, &model, &expected_fingerprint, task).await?,
            )
        }
        WorkspaceCommand::Open { folder } => print(&open_workspace(&folder, false).await?),
        WorkspaceCommand::Verify { folder } => print(&open_workspace(&folder, true).await?),
        WorkspaceCommand::InspectDataset { source, purpose } => {
            print(&inspect_dataset(&source, purpose.into())?)
        }
        WorkspaceCommand::ImportDataset {
            folder,
            source,
            name,
            purpose,
            expected_fingerprint,
        } => {
            eprintln!(
                "Copying and verifying JSONL data; no training membership or evaluation is created."
            );
            print(
                &import_dataset(
                    &folder,
                    &source,
                    &name,
                    purpose.into(),
                    &expected_fingerprint,
                )
                .await?,
            )
        }
        WorkspaceCommand::BackfillNomos {
            folder,
            source_root,
        } => {
            eprintln!("Backfilling final-stage inputs named by this baseline's training manifest.");
            print(&backfill_nomos(&folder, &source_root).await?)
        }
    }
}
