//! Deterministic categorical-dimension combination logic.

use std::collections::BTreeMap;

use crate::domain::{DatasetDefinition, GenerationCell};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DimensionCardinality {
    pub name: String,
    pub value_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenerationSpaceSummary {
    pub dataset_id: uuid::Uuid,
    pub label_count: u64,
    pub dimensions: Vec<DimensionCardinality>,
    pub generation_cell_count: u64,
}

pub fn summarize_generation_space(definition: &DatasetDefinition) -> GenerationSpaceSummary {
    GenerationSpaceSummary {
        dataset_id: definition.id,
        label_count: definition.labels.len() as u64,
        dimensions: definition
            .dimensions
            .iter()
            .map(|dimension| DimensionCardinality {
                name: dimension.name.clone(),
                value_count: dimension.values.len() as u64,
            })
            .collect(),
        generation_cell_count: definition.generation_cell_count(),
    }
}

pub fn expand_generation_cells(definition: &DatasetDefinition) -> Vec<GenerationCell> {
    let mut combinations = vec![BTreeMap::new()];

    for dimension in &definition.dimensions {
        let mut expanded = Vec::with_capacity(combinations.len() * dimension.values.len());
        for existing in &combinations {
            for value in &dimension.values {
                let mut next = existing.clone();
                next.insert(dimension.name.clone(), value.clone());
                expanded.push(next);
            }
        }
        combinations = expanded;
    }

    definition
        .labels
        .iter()
        .flat_map(|label| {
            combinations
                .iter()
                .cloned()
                .map(|dimensions| GenerationCell {
                    label: label.clone(),
                    dimensions,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{expand_generation_cells, summarize_generation_space};
    use crate::domain::{DatasetDefinition, DimensionDefinition};

    #[test]
    fn expands_in_definition_order_deterministically() {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("valid dimension"),
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("valid dimension"),
            ],
        )
        .expect("valid dataset");

        let cells = expand_generation_cells(&dataset);
        assert_eq!(cells.len(), 8);
        assert_eq!(cells[0].label, "billing");
        assert_eq!(cells[0].dimensions["difficulty"], "easy");
        assert_eq!(cells[0].dimensions["style"], "clean");
        assert_eq!(cells[3].dimensions["difficulty"], "hard");
        assert_eq!(cells[3].dimensions["style"], "messy");
        assert_eq!(cells[4].label, "fraud");
    }

    #[test]
    fn a_dataset_without_extra_dimensions_has_one_cell_per_label() {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![],
        )
        .expect("valid dataset");

        let cells = expand_generation_cells(&dataset);
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().all(|cell| cell.dimensions.is_empty()));
    }

    #[test]
    fn summarizes_the_cartesian_space_without_materializing_it() {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
                DimensionDefinition::new(
                    "style",
                    vec!["clean".into(), "messy".into(), "formal".into()],
                )
                .expect("dimension"),
            ],
        )
        .expect("dataset");
        let summary = summarize_generation_space(&dataset);
        assert_eq!(summary.label_count, 2);
        assert_eq!(summary.generation_cell_count, 12);
        assert_eq!(summary.dimensions[1].value_count, 3);
    }
}
