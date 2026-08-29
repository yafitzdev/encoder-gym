use std::path::Path;

use anyhow::Context;
use serde::de::DeserializeOwned;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    ports::{WorkflowDefinitionQuery, WorkflowRunQuery, WorkflowRunStore},
    workflow::{
        WorkflowDefinition, WorkflowDefinitionRequest, WorkflowRun, WorkflowRunState,
        WorkflowStage, WorkflowStageAttempt,
    },
};

use crate::cli::{WorkflowCommand, WorkflowRunStateArg};

pub async fn execute(command: WorkflowCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        WorkflowCommand::Define { definition } => {
            let request: WorkflowDefinitionRequest = read_document(&definition)?;
            let definition = WorkflowDefinition::new(request)?;
            store.create_workflow_definition(&definition).await?;
            crate::presentation::print(&definition)
        }
        WorkflowCommand::DefinitionShow { id } => {
            crate::presentation::print(&require_definition(store, id).await?)
        }
        WorkflowCommand::DefinitionList { dataset_id, page } => {
            let definitions = store
                .query_workflow_definitions(WorkflowDefinitionQuery {
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&definitions, definitions.len(), page)
        }
        WorkflowCommand::Start { definition_id } => {
            let definition = require_definition(store, definition_id).await?;
            let mut run = WorkflowRun::queued(&definition)?;
            let attempt = WorkflowStageAttempt::start(
                &definition,
                &mut run,
                WorkflowStage::InitialAllocation,
                None,
                1,
            )?;
            store.create_workflow_run(&run, &attempt).await?;
            crate::presentation::print(&serde_json::json!({
                "run": run,
                "attempt": attempt,
            }))
        }
        WorkflowCommand::Status { id } => {
            let run = require_run(store, id).await?;
            let attempts = store.list_workflow_attempts(id).await?;
            crate::presentation::print(&serde_json::json!({
                "run": run,
                "latest_attempt": attempts.last(),
                "attempt_count": attempts.len(),
            }))
        }
        WorkflowCommand::List {
            definition_id,
            state,
            page,
        } => {
            let runs = store
                .query_workflow_runs(WorkflowRunQuery {
                    definition_id,
                    state: state.map(run_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&runs, runs.len(), page)
        }
        WorkflowCommand::History { id } => {
            require_run(store, id).await?;
            crate::presentation::print(&store.list_workflow_attempts(id).await?)
        }
        WorkflowCommand::Cancel { id } => {
            let mut run = require_run(store, id).await?;
            let expected = run.latest_attempt_id;
            run.request_cancel()?;
            store.save_workflow_run(&run, expected).await?;
            crate::presentation::print(&run)
        }
    }
}

async fn require_definition(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<WorkflowDefinition> {
    store
        .get_workflow_definition(id)
        .await?
        .with_context(|| format!("workflow definition not found: {id}"))
}

async fn require_run(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<WorkflowRun> {
    store
        .get_workflow_run(id)
        .await?
        .with_context(|| format!("workflow run not found: {id}"))
}

fn read_document<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        _ => {
            toml::from_str(&contents).with_context(|| format!("invalid TOML in {}", path.display()))
        }
    }
}

const fn run_state(value: WorkflowRunStateArg) -> WorkflowRunState {
    match value {
        WorkflowRunStateArg::Queued => WorkflowRunState::Queued,
        WorkflowRunStateArg::Running => WorkflowRunState::Running,
        WorkflowRunStateArg::AwaitingApproval => WorkflowRunState::AwaitingApproval,
        WorkflowRunStateArg::AwaitingUser => WorkflowRunState::AwaitingUser,
        WorkflowRunStateArg::DevelopmentComplete => WorkflowRunState::DevelopmentComplete,
        WorkflowRunStateArg::Completed => WorkflowRunState::Completed,
        WorkflowRunStateArg::Failed => WorkflowRunState::Failed,
        WorkflowRunStateArg::Cancelled => WorkflowRunState::Cancelled,
        WorkflowRunStateArg::Exhausted => WorkflowRunState::Exhausted,
        WorkflowRunStateArg::Inconclusive => WorkflowRunState::Inconclusive,
    }
}
