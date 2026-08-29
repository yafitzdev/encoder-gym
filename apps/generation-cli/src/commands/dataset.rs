use anyhow::{Context, bail};
use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition},
    ports::{DatasetQuery, DatasetStore},
};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::DatasetCommand;

use super::ingestion;

pub async fn execute(command: DatasetCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        DatasetCommand::Create {
            name,
            task,
            labels,
            dimensions,
        } => {
            let dimensions = dimensions
                .into_iter()
                .map(|raw| parse_dimension(&raw))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let dataset = DatasetDefinition::new(name, task, labels, dimensions)?;
            store.create_dataset(&dataset).await?;
            print_json(&dataset)
        }
        DatasetCommand::List { name, page } => {
            let datasets = store
                .query_datasets(DatasetQuery {
                    name_contains: name,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&datasets, datasets.len(), page)
        }
        DatasetCommand::Show { id } => {
            let dataset = store
                .get_dataset(id)
                .await?
                .with_context(|| format!("dataset not found: {id}"))?;
            print_json(&dataset)
        }
        DatasetCommand::Import(args) => ingestion::run(args, store).await,
        DatasetCommand::Imports {
            dataset_id,
            limit,
            offset,
            summary,
        } => ingestion::list(dataset_id, limit, offset, summary, store).await,
        DatasetCommand::ImportShow { id } => ingestion::show(id, store).await,
        DatasetCommand::ImportRows {
            id,
            limit,
            offset,
            summary,
        } => ingestion::rows(id, limit, offset, summary, store).await,
    }
}

fn parse_dimension(raw: &str) -> anyhow::Result<DimensionDefinition> {
    let Some((name, values)) = raw.split_once('=') else {
        bail!("dimension must use name=value1,value2 syntax: {raw}");
    };
    DimensionDefinition::new(name, values.split(',').map(str::to_owned).collect())
        .map_err(Into::into)
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}

#[cfg(test)]
mod tests {
    use super::parse_dimension;

    #[test]
    fn parses_arbitrary_categorical_dimension() {
        let dimension = parse_dimension("tone=formal,casual").expect("valid dimension");
        assert_eq!(dimension.name, "tone");
        assert_eq!(dimension.values, vec!["formal", "casual"]);
    }
}
