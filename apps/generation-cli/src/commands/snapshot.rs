use anyhow::Context;
use dataset_core::{
    domain::{SnapshotSplit, SplitConfiguration, SplitRatios},
    export::{to_csv, to_jsonl},
    ports::{AcceptedRowSource, SnapshotQuery, SnapshotStore},
    splitting::build_snapshot,
    statistics::calculate_statistics,
};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{ExportFormat, SnapshotCommand, SnapshotSplitArg};

use super::config;

pub async fn execute(command: SnapshotCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        SnapshotCommand::Create {
            dataset_id,
            name,
            description,
            train_ratio,
            validation_ratio,
            test_ratio,
            seed,
            group_dimension,
            config: config_path,
        } => {
            let configured = config_path
                .as_deref()
                .map(config::resolve_path)
                .transpose()?;
            let defaults = SplitRatios::default();
            let ratios = SplitRatios::new(
                train_ratio.unwrap_or_else(|| {
                    configured
                        .as_ref()
                        .map_or(defaults.train, |config| config.snapshot.train_ratio)
                }),
                validation_ratio.unwrap_or_else(|| {
                    configured.as_ref().map_or(defaults.validation, |config| {
                        config.snapshot.validation_ratio
                    })
                }),
                test_ratio.unwrap_or_else(|| {
                    configured
                        .as_ref()
                        .map_or(defaults.test, |config| config.snapshot.test_ratio)
                }),
            )?;
            let name = name
                .or_else(|| {
                    configured
                        .as_ref()
                        .map(|config| config.snapshot.name.clone())
                })
                .context("provide --name or a project --config")?;
            let description = description.or_else(|| {
                configured
                    .as_ref()
                    .and_then(|config| config.snapshot.description.clone())
            });
            let seed = seed.unwrap_or_else(|| {
                configured
                    .as_ref()
                    .map_or(42, |config| config.snapshot.seed)
            });
            let source_rows = store.list_accepted_source_rows(dataset_id).await?;
            let (snapshot, members) = build_snapshot(
                dataset_id,
                name,
                description,
                SplitConfiguration::new(ratios, seed).with_group_dimension(group_dimension)?,
                source_rows,
            )?;
            store.create_snapshot(&snapshot, &members).await?;
            print_json(&serde_json::json!({
                "snapshot": snapshot,
                "statistics": calculate_statistics(&members),
            }))?;
        }
        SnapshotCommand::List { dataset_id, page } => {
            let snapshots = store
                .query_snapshots(SnapshotQuery {
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&snapshots, snapshots.len(), page)?;
        }
        SnapshotCommand::Show { id } => {
            let snapshot = require_snapshot(store, id).await?;
            let members = store.list_snapshot_members(id).await?;
            print_json(&serde_json::json!({
                "snapshot": snapshot,
                "statistics": calculate_statistics(&members),
            }))?;
        }
        SnapshotCommand::Members { id, split } => {
            require_snapshot(store, id).await?;
            let expected = split.map(snapshot_split);
            let members = store
                .list_snapshot_members(id)
                .await?
                .into_iter()
                .filter(|member| expected.is_none_or(|split| member.split == split))
                .collect::<Vec<_>>();
            print_json(&members)?;
        }
        SnapshotCommand::Stats { id } => {
            require_snapshot(store, id).await?;
            print_json(&calculate_statistics(
                &store.list_snapshot_members(id).await?,
            ))?;
        }
        SnapshotCommand::Export { id, format, output } => {
            require_snapshot(store, id).await?;
            let members = store.list_snapshot_members(id).await?;
            let contents = match format {
                ExportFormat::Jsonl => to_jsonl(&members)?,
                ExportFormat::Csv => to_csv(&members)?,
            };
            std::fs::write(&output, contents)
                .with_context(|| format!("could not write {}", output.display()))?;
            crate::presentation::print(&serde_json::json!({
                "exported": true,
                "artifact": "snapshot",
                "output": output,
            }))?;
        }
    }
    Ok(())
}

async fn require_snapshot(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<dataset_core::domain::DatasetSnapshot> {
    store
        .get_snapshot(id)
        .await?
        .with_context(|| format!("snapshot not found: {id}"))
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
