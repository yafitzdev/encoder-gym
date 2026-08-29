use std::path::Path;

use anyhow::Context;
use dataset_core::domain::SnapshotSplit;
use project_config::{ProjectConfig, ProjectInitializer, ProjectOverrides, ResolvedProjectConfig};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{ConfigCommand, ConfigResolveArgs, SnapshotSplitArg};

pub async fn execute(command: ConfigCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ConfigCommand::Validate { file } => {
            load(&file)?.resolve(ProjectOverrides::default())?;
            print_json(&serde_json::json!({
                "valid": true,
                "file": canonical_display(&file)?,
            }))
        }
        ConfigCommand::Show(args) => {
            let resolved = resolve(&args)?;
            print_json(&serde_json::json!({
                "fingerprint": resolved.fingerprint()?,
                "resolved_configuration": resolved,
            }))
        }
        ConfigCommand::Init(args) => {
            let resolved = resolve(&args)?;
            let dataset = resolved.dataset_definition()?;
            let plan = resolved.generation_plan(&dataset)?;
            let backend = resolved.backend_configuration();
            let configuration = resolved.persisted(dataset.id, plan.id)?;
            store
                .initialize_project(&dataset, &plan, backend.as_ref(), &configuration)
                .await?;
            print_json(&serde_json::json!({
                "dataset": dataset,
                "generation_plan": plan,
                "backend_configuration": backend,
                "project_configuration": configuration,
                "resolved_configuration": resolved,
            }))
        }
    }
}

pub fn load(path: &Path) -> anyhow::Result<ProjectConfig> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("could not read project configuration {}", path.display()))?;
    ProjectConfig::parse(&source).map_err(Into::into)
}

pub fn resolve_path(path: &Path) -> anyhow::Result<ResolvedProjectConfig> {
    load(path)?
        .resolve(ProjectOverrides::default())
        .map_err(Into::into)
}

pub fn resolve(args: &ConfigResolveArgs) -> anyhow::Result<ResolvedProjectConfig> {
    load(&args.file)?
        .resolve(ProjectOverrides {
            dataset_name: args.dataset_name.clone(),
            target_per_cell: args.target_per_cell,
            snapshot_seed: args.snapshot_seed,
            training_epochs: args.training_epochs,
            training_learning_rate: args.training_learning_rate,
            evaluation_split: args.evaluation_split.map(snapshot_split),
        })
        .map_err(Into::into)
}

fn canonical_display(path: &Path) -> anyhow::Result<String> {
    Ok(std::fs::canonicalize(path)
        .with_context(|| format!("could not resolve {}", path.display()))?
        .to_string_lossy()
        .into_owned())
}

const fn snapshot_split(split: SnapshotSplitArg) -> SnapshotSplit {
    match split {
        SnapshotSplitArg::Train => SnapshotSplit::Train,
        SnapshotSplitArg::Validation => SnapshotSplit::Validation,
        SnapshotSplitArg::Test => SnapshotSplit::Test,
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}
