use recovery_core::{RecoveryState, RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{RecoveryCommand, WorkflowKindArg};

use super::generation;

pub async fn execute(command: RecoveryCommand, store: SqliteStore) -> anyhow::Result<()> {
    match command {
        RecoveryCommand::Scan => {
            print_json(&store.detect_interrupted_workflows().await?)?;
        }
        RecoveryCommand::List { all } => {
            print_json(&store.list_recovery_records(all).await?)?;
        }
        RecoveryCommand::ResumeGeneration(args) => {
            generation::resume(args, store).await?;
        }
        RecoveryCommand::Dismiss { kind, id } => {
            store
                .resolve_recovery(workflow_kind(kind), id, RecoveryState::Dismissed, None)
                .await?;
            print_json(&serde_json::json!({
                "workflow_kind": workflow_kind(kind),
                "workflow_id": id,
                "recovery_state": RecoveryState::Dismissed,
            }))?;
        }
    }
    Ok(())
}

const fn workflow_kind(kind: WorkflowKindArg) -> WorkflowKind {
    match kind {
        WorkflowKindArg::Generation => WorkflowKind::Generation,
        WorkflowKindArg::Training => WorkflowKind::Training,
        WorkflowKindArg::Evaluation => WorkflowKind::Evaluation,
        WorkflowKindArg::EncoderWorkflow => WorkflowKind::EncoderWorkflow,
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}
