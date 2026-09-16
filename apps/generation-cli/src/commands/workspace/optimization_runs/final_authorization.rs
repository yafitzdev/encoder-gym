//! Final consent is a separate command, never an adaptive-loop side effect.
use anyhow::{Context, Result, ensure};
use project_workspace_local::{optimization_completions, optimization_final};
use std::path::Path;
use uuid::Uuid;

pub(super) async fn preview(folder: &Path, run_id: Uuid) -> Result<()> {
    let scientific = history(folder, run_id).await?;
    super::super::print(&optimization_final::preview(folder, run_id, &scientific).await?)
}

pub(super) async fn show(folder: &Path, run_id: Uuid) -> Result<()> {
    let scientific = history(folder, run_id).await?;
    super::super::print(&optimization_final::show(folder, run_id, &scientific).await?)
}

pub(super) async fn authorize(
    folder: &Path,
    run_id: Uuid,
    file: &Path,
    authorized_by: &str,
) -> Result<()> {
    ensure!(
        std::fs::metadata(file)?.len() <= 32_768,
        "Final authorization request exceeds 32 KiB"
    );
    let request: optimization_final::FinalAuthorizationRequest =
        serde_json::from_slice(&std::fs::read(file)?)
            .context("Invalid final authorization request")?;
    let scientific = history(folder, run_id).await?;
    super::super::print(
        &optimization_final::authorize(folder, run_id, request, authorized_by, &scientific).await?,
    )
}

async fn history(
    folder: &Path,
    run_id: Uuid,
) -> Result<Vec<optimization_completions::IterationScientificEvidence>> {
    let through = optimization_completions::list(folder, run_id)
        .await?
        .last()
        .map_or(0, |last| last.number);
    if through == 0 {
        return Ok(vec![]);
    }
    super::iteration_inputs::scientific_history(folder, run_id, through).await
}
