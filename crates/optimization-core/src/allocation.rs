use std::collections::{BTreeMap, BTreeSet};

use generation_core::domain::GenerationCell;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    evidence::CurrentCellCoverage,
    protocol::{LabelAllocationBounds, OptimizationProtocol},
    scoring::ScoredCell,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationFeasibility {
    Feasible,
    Infeasible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppliedConstraint {
    CellMinimum,
    CellMaximum,
    LabelMinimum,
    LabelMaximum,
    GlobalCellMaximumShare,
    AllocationIncrement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellAllocation {
    pub cell: GenerationCell,
    pub current_accepted: u32,
    pub additional_count: u32,
    pub proposed_target: u32,
    pub score: f64,
    pub constraints: Vec<AppliedConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConstraintIssue {
    NoEligibleCells,
    BudgetNotDivisibleByIncrement {
        budget: u32,
        increment: u32,
        remainder: u32,
    },
    CellMinimumUnavailable {
        cell_key: String,
        minimum: u32,
        available: u32,
    },
    LabelMinimumUnavailable {
        label: String,
        minimum: u32,
        available: u32,
    },
    CapacityExhausted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllocationResult {
    pub requested_budget: u32,
    pub allocated_budget: u32,
    pub unallocated_budget: u32,
    pub feasibility: AllocationFeasibility,
    pub issues: Vec<ConstraintIssue>,
    pub cells: Vec<CellAllocation>,
}

pub fn allocate_budget(
    scored_cells: &[ScoredCell],
    current_coverage: &[CurrentCellCoverage],
    protocol: &OptimizationProtocol,
) -> Result<AllocationResult, AllocationError> {
    protocol.validate()?;
    let increment = u64::from(protocol.minimum_allocation_increment);
    let budget = u64::from(protocol.additional_example_budget);
    let coverage = current_coverage
        .iter()
        .map(|coverage| (coverage.cell.key(), coverage.accepted))
        .collect::<BTreeMap<_, _>>();
    if coverage.len() != current_coverage.len() {
        return Err(AllocationError::DuplicateCoverage);
    }
    let label_limits = label_limits(protocol, increment)?;
    let cell_bounds = protocol
        .cell_bounds
        .iter()
        .map(|bounds| (bounds.cell.key(), bounds))
        .collect::<BTreeMap<_, _>>();
    let global_cell_cap = protocol.maximum_cell_budget_share.map_or(budget, |share| {
        align_down(share_floor(budget, share), increment)
    });

    let mut states = Vec::new();
    let mut seen = BTreeSet::new();
    for scored in scored_cells
        .iter()
        .filter(|scored| scored.eligibility.eligible)
    {
        let key = scored.cell.key();
        if !seen.insert(key.clone()) {
            return Err(AllocationError::DuplicateScoredCell(key));
        }
        if !scored.score.final_score.is_finite() || scored.score.final_score <= 0.0 {
            return Err(AllocationError::InvalidScore(key));
        }
        let current_accepted = coverage
            .get(&key)
            .copied()
            .ok_or_else(|| AllocationError::MissingCoverage(key.clone()))?;
        let explicit = cell_bounds.get(&key).copied();
        let explicit_cap = explicit
            .and_then(|bounds| bounds.maximum_addition)
            .map_or(budget, u64::from);
        let cap = align_down(global_cell_cap.min(explicit_cap).min(budget), increment);
        let minimum = explicit.map_or(0, |bounds| u64::from(bounds.minimum_addition));
        let mut constraints = vec![AppliedConstraint::AllocationIncrement];
        if explicit.is_some_and(|bounds| bounds.minimum_addition > 0) {
            constraints.push(AppliedConstraint::CellMinimum);
        }
        if explicit.is_some_and(|bounds| bounds.maximum_addition.is_some()) {
            constraints.push(AppliedConstraint::CellMaximum);
        }
        if protocol.maximum_cell_budget_share.is_some() {
            constraints.push(AppliedConstraint::GlobalCellMaximumShare);
        }
        if let Some(label_limit) = label_limits.get(&scored.cell.label) {
            if label_limit.minimum > 0 {
                constraints.push(AppliedConstraint::LabelMinimum);
            }
            if label_limit.maximum < budget {
                constraints.push(AppliedConstraint::LabelMaximum);
            }
        }
        constraints.sort_unstable_by_key(|constraint| *constraint as u8);
        states.push(State {
            cell: scored.cell.clone(),
            current_accepted,
            score: scored.score.final_score,
            minimum: align_up(minimum, increment)?,
            cap,
            allocated: 0,
            constraints,
        });
    }
    states.sort_by_key(|state| state.cell.key());

    let mut issues = Vec::new();
    if states.is_empty() {
        issues.push(ConstraintIssue::NoEligibleCells);
    }
    let indivisible = budget % increment;
    if indivisible > 0 {
        issues.push(ConstraintIssue::BudgetNotDivisibleByIncrement {
            budget: protocol.additional_example_budget,
            increment: protocol.minimum_allocation_increment,
            remainder: u32::try_from(indivisible).map_err(|_| AllocationError::CountOverflow)?,
        });
    }
    let allocatable_budget = budget - indivisible;

    for index in 0..states.len() {
        let label_allocated = allocated_for_label(&states, &states[index].cell.label);
        let label_cap = label_limits
            .get(&states[index].cell.label)
            .map_or(allocatable_budget, |limit| limit.maximum);
        let total_allocated = total_allocated(&states)?;
        let available = states[index]
            .cap
            .saturating_sub(states[index].allocated)
            .min(label_cap.saturating_sub(label_allocated))
            .min(allocatable_budget.saturating_sub(total_allocated));
        let requested = states[index].minimum;
        let addition = align_down(requested.min(available), increment);
        states[index].allocated += addition;
        if addition < requested {
            issues.push(ConstraintIssue::CellMinimumUnavailable {
                cell_key: states[index].cell.key(),
                minimum: to_u32(requested)?,
                available: to_u32(addition)?,
            });
        }
    }

    for (label, limit) in &label_limits {
        let allocated = allocated_for_label(&states, label);
        if allocated >= limit.minimum {
            continue;
        }
        let required = limit.minimum - allocated;
        let distributed = distribute(&mut states, required, increment, &label_limits, Some(label))?;
        if distributed < required {
            issues.push(ConstraintIssue::LabelMinimumUnavailable {
                label: label.clone(),
                minimum: to_u32(limit.minimum)?,
                available: to_u32(allocated + distributed)?,
            });
        }
    }

    let allocated = total_allocated(&states)?;
    let remaining = allocatable_budget.saturating_sub(allocated);
    let distributed = distribute(&mut states, remaining, increment, &label_limits, None)?;
    let allocated = allocated + distributed;
    if allocated < allocatable_budget && !states.is_empty() {
        issues.push(ConstraintIssue::CapacityExhausted);
    }
    let unallocated = budget - allocated;
    let cells = states
        .into_iter()
        .filter(|state| state.allocated > 0)
        .map(|state| {
            let additional_count = to_u32(state.allocated)?;
            let proposed_target = state
                .current_accepted
                .checked_add(additional_count)
                .ok_or(AllocationError::CountOverflow)?;
            Ok(CellAllocation {
                cell: state.cell,
                current_accepted: state.current_accepted,
                additional_count,
                proposed_target,
                score: state.score,
                constraints: state.constraints,
            })
        })
        .collect::<Result<Vec<_>, AllocationError>>()?;
    Ok(AllocationResult {
        requested_budget: protocol.additional_example_budget,
        allocated_budget: to_u32(allocated)?,
        unallocated_budget: to_u32(unallocated)?,
        feasibility: if issues.is_empty() && unallocated == 0 {
            AllocationFeasibility::Feasible
        } else {
            AllocationFeasibility::Infeasible
        },
        issues,
        cells,
    })
}

fn distribute(
    states: &mut [State],
    requested: u64,
    increment: u64,
    label_limits: &BTreeMap<String, LabelLimit>,
    selected_label: Option<&String>,
) -> Result<u64, AllocationError> {
    let mut remaining_chunks = requested / increment;
    let mut distributed_chunks = 0_u64;
    while remaining_chunks > 0 {
        let label_allocations = states.iter().fold(BTreeMap::new(), |mut values, state| {
            *values.entry(state.cell.label.clone()).or_insert(0_u64) += state.allocated;
            values
        });
        let active = states
            .iter()
            .enumerate()
            .filter(|(_, state)| {
                selected_label.is_none_or(|label| &state.cell.label == label)
                    && available_chunks(state, &label_allocations, label_limits, increment) > 0
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if active.is_empty() {
            break;
        }
        let total_score = active.iter().map(|index| states[*index].score).sum::<f64>();
        if !total_score.is_finite() || total_score <= 0.0 {
            break;
        }
        let mut proposals = active
            .iter()
            .map(|index| {
                let ideal = remaining_chunks as f64 * states[*index].score / total_score;
                let capacity =
                    available_chunks(&states[*index], &label_allocations, label_limits, increment);
                let base = (ideal.floor() as u64).min(capacity);
                (*index, base, ideal - ideal.floor())
            })
            .collect::<Vec<_>>();
        let mut reserved_by_label = BTreeMap::<String, u64>::new();
        for (index, base, _) in &mut proposals {
            let label = &states[*index].cell.label;
            let label_allocated = label_allocations.get(label).copied().unwrap_or(0);
            let label_cap = label_limits
                .get(label)
                .map_or(u64::MAX, |limit| limit.maximum);
            let reserved = reserved_by_label.get(label).copied().unwrap_or(0);
            let available = label_cap
                .saturating_sub(label_allocated)
                .saturating_sub(reserved)
                / increment;
            *base = (*base).min(available);
            *reserved_by_label.entry(label.clone()).or_insert(0) += *base * increment;
        }
        let base_total = proposals.iter().map(|(_, base, _)| *base).sum::<u64>();
        for (index, base, _) in &proposals {
            states[*index].allocated = states[*index]
                .allocated
                .checked_add(base * increment)
                .ok_or(AllocationError::CountOverflow)?;
        }
        remaining_chunks -= base_total;
        distributed_chunks += base_total;
        if remaining_chunks == 0 {
            break;
        }
        proposals.sort_by(
            |(left_index, _, left_remainder), (right_index, _, right_remainder)| {
                right_remainder
                    .total_cmp(left_remainder)
                    .then_with(|| {
                        states[*right_index]
                            .score
                            .total_cmp(&states[*left_index].score)
                    })
                    .then_with(|| {
                        states[*left_index]
                            .cell
                            .key()
                            .cmp(&states[*right_index].cell.key())
                    })
            },
        );
        let mut progress = 0_u64;
        for (index, _, _) in proposals {
            if remaining_chunks == 0 {
                break;
            }
            let refreshed_labels = states.iter().fold(BTreeMap::new(), |mut values, state| {
                *values.entry(state.cell.label.clone()).or_insert(0_u64) += state.allocated;
                values
            });
            if available_chunks(&states[index], &refreshed_labels, label_limits, increment) > 0 {
                states[index].allocated += increment;
                remaining_chunks -= 1;
                distributed_chunks += 1;
                progress += 1;
            }
        }
        if base_total == 0 && progress == 0 {
            break;
        }
    }
    Ok(distributed_chunks * increment)
}

fn available_chunks(
    state: &State,
    label_allocations: &BTreeMap<String, u64>,
    label_limits: &BTreeMap<String, LabelLimit>,
    increment: u64,
) -> u64 {
    let label_allocated = label_allocations
        .get(&state.cell.label)
        .copied()
        .unwrap_or(0);
    let label_cap = label_limits
        .get(&state.cell.label)
        .map_or(u64::MAX, |limit| limit.maximum);
    state
        .cap
        .saturating_sub(state.allocated)
        .min(label_cap.saturating_sub(label_allocated))
        / increment
}

fn label_limits(
    protocol: &OptimizationProtocol,
    increment: u64,
) -> Result<BTreeMap<String, LabelLimit>, AllocationError> {
    let budget = u64::from(protocol.additional_example_budget);
    protocol
        .label_bounds
        .iter()
        .map(|bounds| {
            let minimum = label_minimum(bounds, budget, increment)?;
            let maximum = label_maximum(bounds, budget, increment);
            Ok((bounds.label.clone(), LabelLimit { minimum, maximum }))
        })
        .collect()
}

fn label_minimum(
    bounds: &LabelAllocationBounds,
    budget: u64,
    increment: u64,
) -> Result<u64, AllocationError> {
    let share = bounds
        .minimum_budget_share
        .map_or(0, |share| share_ceil(budget, share));
    align_up(u64::from(bounds.minimum_addition).max(share), increment)
}

fn label_maximum(bounds: &LabelAllocationBounds, budget: u64, increment: u64) -> u64 {
    let count = bounds.maximum_addition.map_or(budget, u64::from);
    let share = bounds
        .maximum_budget_share
        .map_or(budget, |share| share_floor(budget, share));
    align_down(count.min(share), increment)
}

fn allocated_for_label(states: &[State], label: &str) -> u64 {
    states
        .iter()
        .filter(|state| state.cell.label == label)
        .map(|state| state.allocated)
        .sum()
}

fn total_allocated(states: &[State]) -> Result<u64, AllocationError> {
    states.iter().try_fold(0_u64, |total, state| {
        total
            .checked_add(state.allocated)
            .ok_or(AllocationError::CountOverflow)
    })
}

fn share_floor(budget: u64, share: f64) -> u64 {
    (budget as f64 * share).floor() as u64
}

fn share_ceil(budget: u64, share: f64) -> u64 {
    (budget as f64 * share).ceil() as u64
}

fn align_down(value: u64, increment: u64) -> u64 {
    value - value % increment
}

fn align_up(value: u64, increment: u64) -> Result<u64, AllocationError> {
    if value == 0 {
        return Ok(0);
    }
    value
        .checked_add(increment - 1)
        .map(|value| align_down(value, increment))
        .ok_or(AllocationError::CountOverflow)
}

fn to_u32(value: u64) -> Result<u32, AllocationError> {
    u32::try_from(value).map_err(|_| AllocationError::CountOverflow)
}

struct State {
    cell: GenerationCell,
    current_accepted: u32,
    score: f64,
    minimum: u64,
    cap: u64,
    allocated: u64,
    constraints: Vec<AppliedConstraint>,
}

#[derive(Clone, Copy)]
struct LabelLimit {
    minimum: u64,
    maximum: u64,
}

#[derive(Debug, Error)]
pub enum AllocationError {
    #[error(transparent)]
    Protocol(#[from] crate::protocol::OptimizationProtocolError),
    #[error("current coverage repeats a generation cell")]
    DuplicateCoverage,
    #[error("eligible scored cell is duplicated: {0}")]
    DuplicateScoredCell(String),
    #[error("eligible cell has a non-finite or non-positive score: {0}")]
    InvalidScore(String),
    #[error("eligible cell has no current coverage entry: {0}")]
    MissingCoverage(String),
    #[error("allocation count exceeds the supported range")]
    CountOverflow,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use generation_core::domain::GenerationCell;

    use crate::{
        evidence::CurrentCellCoverage,
        protocol::{
            CellAllocationBounds, InfeasibleAllocationBehavior, LabelAllocationBounds,
            OptimizationProtocol,
        },
        scoring::{EligibilityDecision, ScoreBreakdown, ScoredCell},
    };

    use super::{AllocationFeasibility, ConstraintIssue, allocate_budget};

    #[test]
    fn conserves_budget_with_stable_largest_remainder_ties() {
        let protocol = OptimizationProtocol::legacy(10, 1);
        let scored = vec![scored("a", 3.0), scored("b", 1.0)];
        let result = allocate_budget(&scored, &coverage(&scored), &protocol).expect("allocation");
        assert_eq!(result.allocated_budget, 10);
        assert_eq!(result.unallocated_budget, 0);
        assert_eq!(result.feasibility, AllocationFeasibility::Feasible);
        assert_eq!(
            result
                .cells
                .iter()
                .map(|cell| (cell.cell.label.as_str(), cell.additional_count))
                .collect::<Vec<_>>(),
            vec![("a", 8), ("b", 2)]
        );
    }

    #[test]
    fn obeys_cell_and_label_caps_then_redistributes() {
        let protocol = OptimizationProtocol {
            cell_bounds: vec![CellAllocationBounds {
                cell: cell("a"),
                minimum_addition: 0,
                maximum_addition: Some(4),
            }],
            label_bounds: vec![LabelAllocationBounds {
                label: "b".into(),
                minimum_addition: 6,
                maximum_addition: Some(6),
                minimum_budget_share: None,
                maximum_budget_share: None,
            }],
            ..OptimizationProtocol::legacy(10, 1)
        };
        let scored = vec![scored("a", 9.0), scored("b", 1.0)];
        let result = allocate_budget(&scored, &coverage(&scored), &protocol).expect("allocation");
        assert_eq!(result.feasibility, AllocationFeasibility::Feasible);
        assert_eq!(result.cells[0].additional_count, 4);
        assert_eq!(result.cells[1].additional_count, 6);
    }

    #[test]
    fn reports_structured_unallocated_budget_when_capacity_is_infeasible() {
        let protocol = OptimizationProtocol {
            maximum_cell_budget_share: Some(0.2),
            minimum_allocation_increment: 2,
            infeasible_allocation_behavior: InfeasibleAllocationBehavior::AllowUnallocated,
            ..OptimizationProtocol::legacy(10, 1)
        };
        let scored = vec![scored("a", 1.0), scored("b", 1.0)];
        let result = allocate_budget(&scored, &coverage(&scored), &protocol).expect("preview");
        assert_eq!(result.allocated_budget, 4);
        assert_eq!(result.unallocated_budget, 6);
        assert_eq!(result.feasibility, AllocationFeasibility::Infeasible);
        assert!(result.issues.contains(&ConstraintIssue::CapacityExhausted));
    }

    #[test]
    fn aggregate_label_cap_cannot_be_exceeded_by_multiple_cells() {
        let protocol = OptimizationProtocol {
            label_bounds: vec![LabelAllocationBounds {
                label: "a".into(),
                minimum_addition: 0,
                maximum_addition: Some(4),
                minimum_budget_share: None,
                maximum_budget_share: None,
            }],
            ..OptimizationProtocol::legacy(10, 1)
        };
        let mut first = scored("a", 10.0);
        first.cell.dimensions.insert("style".into(), "clean".into());
        let mut second = scored("a", 9.0);
        second
            .cell
            .dimensions
            .insert("style".into(), "messy".into());
        let scored = vec![first, second, scored("b", 1.0)];
        let result = allocate_budget(&scored, &coverage(&scored), &protocol).expect("allocation");
        assert_eq!(
            result
                .cells
                .iter()
                .filter(|cell| cell.cell.label == "a")
                .map(|cell| cell.additional_count)
                .sum::<u32>(),
            4
        );
        assert_eq!(result.allocated_budget, 10);
    }

    fn scored(label: &str, score: f64) -> ScoredCell {
        ScoredCell {
            cell: cell(label),
            finding_key: label.into(),
            finding_fingerprint: "sha256:finding".into(),
            evidence_fingerprint: "sha256:evidence".into(),
            eligibility: EligibilityDecision {
                eligible: true,
                reasons: Vec::new(),
            },
            score: ScoreBreakdown {
                policy: crate::protocol::ScoringPolicy::ErrorCount,
                risk_policy: crate::protocol::RiskPolicy::None,
                evidence_support: 10,
                evidence_errors: 2,
                error_rate: 0.2,
                baseline_error_rate: 0.1,
                error_rate_lift: 0.1,
                error_share: 0.5,
                high_confidence_error_severity: 0.2,
                marginal_error_count: 1,
                comparison_analyzed_only_errors: None,
                comparison_persistent_errors: None,
                adjusted_error_rate: 0.2,
                uncertainty_factor: 1.0,
                support_factor: 1.0,
                raw_policy_score: score,
                comparison_boost: 1.0,
                review_penalty: 1.0,
                final_score: score,
            },
        }
    }

    fn cell(label: &str) -> GenerationCell {
        GenerationCell {
            label: label.into(),
            dimensions: BTreeMap::new(),
        }
    }

    fn coverage(scored: &[ScoredCell]) -> Vec<CurrentCellCoverage> {
        scored
            .iter()
            .map(|scored| CurrentCellCoverage {
                cell: scored.cell.clone(),
                accepted: 10,
            })
            .collect()
    }
}
