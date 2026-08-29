//! Explicit per-cell generation planning.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, DomainError, GenerationPlan, PlannedCell},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationNeed {
    pub planned: PlannedCell,
    pub accepted_count: u32,
    pub remaining_count: u32,
}

pub fn equal_target_plan(
    definition: &DatasetDefinition,
    target_per_cell: u32,
) -> Result<GenerationPlan, DomainError> {
    let cells = expand_generation_cells(definition)
        .into_iter()
        .map(|cell| PlannedCell {
            cell,
            target_count: target_per_cell,
        })
        .collect();
    GenerationPlan::new(definition.id, cells)
}

pub fn explicit_target_plan(
    definition: &DatasetDefinition,
    targets: Vec<PlannedCell>,
) -> Result<GenerationPlan, DomainError> {
    let valid_keys: BTreeSet<_> = expand_generation_cells(definition)
        .into_iter()
        .map(|cell| cell.key())
        .collect();

    for planned in &targets {
        let key = planned.cell.key();
        if !valid_keys.contains(&key) {
            return Err(DomainError::UnknownCell(key));
        }
    }

    GenerationPlan::new(definition.id, targets)
}

pub fn calculate_generation_needs(
    plan: &GenerationPlan,
    accepted_by_cell: &BTreeMap<String, u32>,
) -> Vec<GenerationNeed> {
    plan.cells
        .iter()
        .cloned()
        .map(|planned| {
            let accepted_count = accepted_by_cell
                .get(&planned.cell.key())
                .copied()
                .unwrap_or(0);
            let remaining_count = planned.target_count.saturating_sub(accepted_count);
            GenerationNeed {
                planned,
                accepted_count,
                remaining_count,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{calculate_generation_needs, equal_target_plan, explicit_target_plan};
    use crate::{
        dimensions::expand_generation_cells,
        domain::{DatasetDefinition, DimensionDefinition, PlannedCell},
    };

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("valid dimension"),
            ],
        )
        .expect("valid dataset")
    }

    #[test]
    fn equal_plan_is_only_a_convenience() {
        let plan = equal_target_plan(&dataset(), 100).expect("valid plan");
        assert_eq!(plan.cells.len(), 4);
        assert_eq!(plan.total_target_count(), 400);
    }

    #[test]
    fn explicit_plan_supports_unequal_targets() {
        let dataset = dataset();
        let cells = expand_generation_cells(&dataset);
        let plan = explicit_target_plan(
            &dataset,
            vec![
                PlannedCell {
                    cell: cells[0].clone(),
                    target_count: 100,
                },
                PlannedCell {
                    cell: cells[3].clone(),
                    target_count: 350,
                },
            ],
        )
        .expect("valid plan");

        assert_eq!(plan.total_target_count(), 450);
    }

    #[test]
    fn needs_use_accepted_coverage_and_saturate_at_zero() {
        let plan = equal_target_plan(&dataset(), 10).expect("valid plan");
        let accepted = BTreeMap::from([
            (plan.cells[0].cell.key(), 4),
            (plan.cells[1].cell.key(), 12),
        ]);
        let needs = calculate_generation_needs(&plan, &accepted);

        assert_eq!(needs[0].remaining_count, 6);
        assert_eq!(needs[1].remaining_count, 0);
        assert_eq!(needs[2].remaining_count, 10);
    }
}
