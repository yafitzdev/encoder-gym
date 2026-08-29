//! Coverage calculations derived from persisted generation facts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::{GenerationCell, GenerationPlan};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellCounts {
    pub attempted: u32,
    pub accepted: u32,
    pub rejected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellCoverage {
    pub cell: GenerationCell,
    pub target: u32,
    pub attempted: u32,
    pub accepted: u32,
    pub rejected: u32,
    pub remaining: u32,
}

pub fn calculate_coverage(
    plan: &GenerationPlan,
    counts_by_cell: &BTreeMap<String, CellCounts>,
) -> Vec<CellCoverage> {
    plan.cells
        .iter()
        .map(|planned| {
            let counts = counts_by_cell
                .get(&planned.cell.key())
                .cloned()
                .unwrap_or_default();
            CellCoverage {
                cell: planned.cell.clone(),
                target: planned.target_count,
                attempted: counts.attempted,
                accepted: counts.accepted,
                rejected: counts.rejected,
                remaining: planned.target_count.saturating_sub(counts.accepted),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{CellCounts, calculate_coverage};
    use crate::{
        domain::{DatasetDefinition, DimensionDefinition},
        planning::equal_target_plan,
    };

    #[test]
    fn coverage_includes_empty_cells_and_uses_accepted_for_remaining() {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into()],
            vec![
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("valid dimension"),
            ],
        )
        .expect("valid dataset");
        let plan = equal_target_plan(&dataset, 100).expect("valid plan");
        let counts = BTreeMap::from([(
            plan.cells[0].cell.key(),
            CellCounts {
                attempted: 100,
                accepted: 73,
                rejected: 27,
            },
        )]);

        let coverage = calculate_coverage(&plan, &counts);
        assert_eq!(coverage[0].remaining, 27);
        assert_eq!(coverage[1].attempted, 0);
        assert_eq!(coverage[1].remaining, 100);
    }
}
