use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use dataset_core::domain::SnapshotSplit;
use generation_core::{domain::GenerationCell, prompting::PromptBuilder};
use project_config::{ProjectConfig, ProjectInitializer, ProjectOverrides, ResolvedProjectConfig};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{ConfigCommand, ConfigResolveArgs, ConstructionPreviewArgs, SnapshotSplitArg};

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
        ConfigCommand::ConstructionPreview(args) => construction_preview(args),
    }
}

fn construction_preview(args: ConstructionPreviewArgs) -> anyhow::Result<()> {
    let resolved = load(&args.file)?.resolve(ProjectOverrides::default())?;
    let dataset = resolved.dataset_definition()?;
    anyhow::ensure!(
        dataset.labels.contains(&args.label),
        "unknown label {:?}; expected one of {:?}",
        args.label,
        dataset.labels
    );
    let dimensions = parse_dimensions(args.dimensions)?;
    let expected = dataset
        .dimensions
        .iter()
        .map(|dimension| dimension.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let actual = dimensions
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    anyhow::ensure!(
        actual == expected,
        "dimensions must select exactly {expected:?}"
    );
    for dimension in &dataset.dimensions {
        anyhow::ensure!(
            dimension
                .values
                .contains(dimensions.get(&dimension.name).expect("dimension exists")),
            "unknown value for dimension {}",
            dimension.name
        );
    }
    let construction = resolved.row_construction_plan()?.compile()?;
    let prepared = construction.prepare(
        GenerationCell {
            label: args.label,
            dimensions,
        },
        args.start_index,
        args.count,
    )?;
    let provider_required = prepared.requires_llm();
    let llm_field_count = prepared.llm_fields.len();
    let deterministic_field_count = construction
        .plan()
        .fields
        .len()
        .saturating_sub(llm_field_count);
    let (request, completed_rows) = if provider_required {
        (
            Some(PromptBuilder::default().build_hybrid(
                &dataset,
                prepared.clone(),
                resolved.generation_parameters(),
                &[],
            )),
            None,
        )
    } else {
        (None, Some(construction.complete(&prepared, Vec::new())?))
    };
    print_json(&serde_json::json!({
        "construction_plan": construction.plan(),
        "provider_required": provider_required,
        "deterministic_field_count": deterministic_field_count,
        "llm_field_count": llm_field_count,
        "prepared_rows": prepared.rows,
        "request": request,
        "completed_rows": completed_rows,
    }))
}

fn parse_dimensions(values: Vec<String>) -> anyhow::Result<BTreeMap<String, String>> {
    values
        .into_iter()
        .map(|raw| {
            let (name, value) = raw
                .split_once('=')
                .context("dimensions must use NAME=VALUE")?;
            anyhow::ensure!(
                !name.trim().is_empty() && !value.trim().is_empty(),
                "dimensions must use non-empty NAME=VALUE"
            );
            Ok((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
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
