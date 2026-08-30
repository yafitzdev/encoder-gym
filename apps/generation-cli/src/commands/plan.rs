use anyhow::{Context, bail};
use generation_core::{
    dimensions::{expand_generation_cells, summarize_generation_space},
    domain::PlannedCell,
    planning::{equal_target_plan, explicit_target_plan},
    ports::{DatasetStore, PlanStore},
};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::PlanCommand;

pub async fn execute(command: PlanCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        PlanCommand::Describe { dataset_id } => {
            let dataset = store
                .get_dataset(dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {dataset_id}"))?;
            crate::presentation::print(&summarize_generation_space(&dataset))?;
            Ok(())
        }
        PlanCommand::Preview { dataset_id } => {
            let dataset = store
                .get_dataset(dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {dataset_id}"))?;
            crate::presentation::print(&expand_generation_cells(&dataset))?;
            Ok(())
        }
        PlanCommand::Create {
            dataset_id,
            per_cell,
            targets,
        } => {
            let dataset = store
                .get_dataset(dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {dataset_id}"))?;
            let plan = if let Some(path) = targets {
                let raw = std::fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?;
                let targets: Vec<PlannedCell> = serde_json::from_str(&raw)
                    .with_context(|| format!("invalid target JSON in {}", path.display()))?;
                explicit_target_plan(&dataset, targets)?
            } else if let Some(target) = per_cell {
                equal_target_plan(&dataset, target)?
            } else {
                bail!("provide either --per-cell or --targets");
            };
            store.create_plan(&plan).await?;
            crate::presentation::print(&plan)?;
            Ok(())
        }
        PlanCommand::Show { id } => {
            let plan = store
                .get_plan(id)
                .await?
                .with_context(|| format!("plan not found: {id}"))?;
            crate::presentation::print(&plan)?;
            Ok(())
        }
    }
}
