//! Deterministic allocation of one exact initial dataset budget into Slice 1 cells.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, GenerationCell, GenerationPlan, PlannedCell},
    planning::explicit_target_plan,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InitialAllocationPolicy {
    Balanced,
    Weighted {
        #[serde(default)]
        weights: AllocationWeights,
    },
    MinimumThenWeighted {
        minimum_per_cell: u32,
        #[serde(default)]
        weights: AllocationWeights,
    },
    Explicit {
        targets: Vec<ExplicitCellTarget>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AllocationWeights {
    #[serde(default)]
    pub labels: BTreeMap<String, f64>,
    #[serde(default)]
    pub dimension_values: Vec<DimensionValueWeight>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionValueWeight {
    pub dimension: String,
    pub value: String,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplicitCellTarget {
    pub cell: GenerationCell,
    pub target: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialCellCoverage {
    pub cell: GenerationCell,
    pub accepted: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialCellConstraint {
    pub cell: GenerationCell,
    #[serde(default)]
    pub minimum_target: u32,
    pub maximum_target: Option<u32>,
    #[serde(default)]
    pub excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialAllocationRequest {
    pub total_rows: u32,
    #[serde(default)]
    pub reserved_rows: u32,
    pub policy: InitialAllocationPolicy,
    #[serde(default)]
    pub current_coverage: Vec<InitialCellCoverage>,
    #[serde(default)]
    pub constraints: Vec<InitialCellConstraint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitialAllocationFeasibility {
    Feasible,
    Infeasible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InitialAllocationIssue {
    ExistingCoverageExceedsInitialTarget {
        existing: u64,
        initial_target: u32,
    },
    MinimumsExceedInitialTarget {
        minimum: u64,
        initial_target: u32,
    },
    CapacityBelowInitialTarget {
        capacity: u64,
        initial_target: u32,
    },
    CurrentCoverageExceedsMaximum {
        cell_key: String,
        current: u32,
        maximum: u32,
    },
    MinimumExceedsMaximum {
        cell_key: String,
        minimum: u32,
        maximum: u32,
    },
    ExcludedCellRequiresGrowth {
        cell_key: String,
        current: u32,
        minimum: u32,
    },
    ExplicitTargetBelowCurrent {
        cell_key: String,
        target: u32,
        current: u32,
    },
    ExplicitTargetBelowMinimum {
        cell_key: String,
        target: u32,
        minimum: u32,
    },
    ExplicitTargetAboveMaximum {
        cell_key: String,
        target: u32,
        maximum: u32,
    },
    ExplicitTargetChangesExcludedCell {
        cell_key: String,
        target: u32,
        current: u32,
    },
    ExplicitTotalMismatch {
        actual: u64,
        expected: u32,
    },
    NoAllocatableCells,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialCellAllocation {
    pub cell: GenerationCell,
    pub current_accepted: u32,
    pub minimum_target: u32,
    pub maximum_target: Option<u32>,
    pub target: u32,
    pub additional_required: u32,
    pub effective_weight: f64,
    pub excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialAllocationResult {
    pub dataset_id: uuid::Uuid,
    pub requested_total_rows: u32,
    pub initial_target_rows: u32,
    pub reserved_rows: u32,
    pub allocated_target_rows: u64,
    pub unallocated_rows: u32,
    pub overallocated_rows: u64,
    pub feasibility: InitialAllocationFeasibility,
    pub issues: Vec<InitialAllocationIssue>,
    pub cells: Vec<InitialCellAllocation>,
    pub policy: InitialAllocationPolicy,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialAllocationRecord {
    pub id: uuid::Uuid,
    pub result: InitialAllocationResult,
    pub generation_plan_id: uuid::Uuid,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl InitialAllocationRecord {
    pub fn new(
        result: InitialAllocationResult,
        generation_plan_id: uuid::Uuid,
    ) -> Result<Self, InitialAllocationError> {
        if result.feasibility != InitialAllocationFeasibility::Feasible {
            return Err(InitialAllocationError::Infeasible);
        }
        if result.reproduce_fingerprint()? != result.fingerprint {
            return Err(InitialAllocationError::FingerprintMismatch);
        }
        let mut record = Self {
            id: uuid::Uuid::new_v4(),
            result,
            generation_plan_id,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        record.fingerprint = allocation_record_fingerprint(&record)?;
        Ok(record)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, InitialAllocationError> {
        allocation_record_fingerprint(self)
    }
}

impl InitialAllocationResult {
    pub fn reproduce_fingerprint(&self) -> Result<String, InitialAllocationError> {
        allocation_fingerprint(self)
    }

    pub fn to_generation_plan(
        &self,
        dataset: &DatasetDefinition,
    ) -> Result<GenerationPlan, InitialAllocationError> {
        if self.feasibility != InitialAllocationFeasibility::Feasible {
            return Err(InitialAllocationError::Infeasible);
        }
        if self.dataset_id != dataset.id {
            return Err(InitialAllocationError::DatasetMismatch);
        }
        explicit_target_plan(
            dataset,
            self.cells
                .iter()
                .map(|allocation| PlannedCell {
                    cell: allocation.cell.clone(),
                    target_count: allocation.target,
                })
                .collect(),
        )
        .map_err(|error| InitialAllocationError::Plan(error.to_string()))
    }
}

pub fn allocate_initial_budget(
    dataset: &DatasetDefinition,
    request: InitialAllocationRequest,
) -> Result<InitialAllocationResult, InitialAllocationError> {
    if request.reserved_rows > request.total_rows {
        return Err(InitialAllocationError::ReserveExceedsTotal {
            reserve: request.reserved_rows,
            total: request.total_rows,
        });
    }
    let initial_target_rows = request.total_rows - request.reserved_rows;
    let mut cells = expand_generation_cells(dataset);
    cells.sort_by_key(GenerationCell::key);
    let valid_cells = cells
        .iter()
        .map(|cell| (cell.key(), cell.clone()))
        .collect::<BTreeMap<_, _>>();
    let coverage = normalize_coverage(&request.current_coverage, &valid_cells)?;
    let constraints = normalize_constraints(&request.constraints, &valid_cells)?;
    let weights = validate_policy(&request.policy, dataset, &valid_cells)?;

    let mut issues = Vec::new();
    let policy_minimum = match &request.policy {
        InitialAllocationPolicy::MinimumThenWeighted {
            minimum_per_cell, ..
        } => *minimum_per_cell,
        _ => 0,
    };
    let explicit_targets = match &request.policy {
        InitialAllocationPolicy::Explicit { targets } => Some(
            targets
                .iter()
                .map(|target| (target.cell.key(), target.target))
                .collect::<BTreeMap<_, _>>(),
        ),
        _ => None,
    };

    let mut states = Vec::with_capacity(cells.len());
    for cell in cells {
        let key = cell.key();
        let current = coverage.get(&key).copied().unwrap_or(0);
        let constraint = constraints.get(&key);
        let excluded = constraint.is_some_and(|constraint| constraint.excluded);
        let configured_minimum = constraint.map_or(0, |constraint| constraint.minimum_target);
        let minimum = if excluded {
            configured_minimum
        } else {
            configured_minimum.max(policy_minimum)
        };
        let maximum = constraint.and_then(|constraint| constraint.maximum_target);

        if let Some(maximum) = maximum {
            if current > maximum {
                issues.push(InitialAllocationIssue::CurrentCoverageExceedsMaximum {
                    cell_key: key.clone(),
                    current,
                    maximum,
                });
            }
            if minimum > maximum {
                issues.push(InitialAllocationIssue::MinimumExceedsMaximum {
                    cell_key: key.clone(),
                    minimum,
                    maximum,
                });
            }
        }
        if excluded && minimum > current {
            issues.push(InitialAllocationIssue::ExcludedCellRequiresGrowth {
                cell_key: key.clone(),
                current,
                minimum,
            });
        }

        let baseline = current.max(minimum);
        let cap = maximum.unwrap_or(u32::MAX);
        let weight = weights.get(&key).copied().unwrap_or(1.0);
        states.push(CellState {
            cell,
            current,
            minimum,
            maximum,
            excluded,
            weight,
            target: baseline,
            cap,
        });
    }

    if let Some(explicit_targets) = explicit_targets {
        apply_explicit_targets(
            &mut states,
            &explicit_targets,
            initial_target_rows,
            &mut issues,
        );
    } else {
        distribute_weighted(&mut states, initial_target_rows, &mut issues)?;
    }

    let allocated_target_rows = states.iter().map(|state| u64::from(state.target)).sum();
    let unallocated_rows = u64::from(initial_target_rows)
        .saturating_sub(allocated_target_rows)
        .try_into()
        .map_err(|_| InitialAllocationError::CountOverflow)?;
    let overallocated_rows = allocated_target_rows.saturating_sub(u64::from(initial_target_rows));
    let feasibility = if issues.is_empty()
        && unallocated_rows == 0
        && overallocated_rows == 0
        && allocated_target_rows == u64::from(initial_target_rows)
    {
        InitialAllocationFeasibility::Feasible
    } else {
        InitialAllocationFeasibility::Infeasible
    };
    let cells = states
        .into_iter()
        .map(|state| InitialCellAllocation {
            additional_required: state.target.saturating_sub(state.current),
            cell: state.cell,
            current_accepted: state.current,
            minimum_target: state.minimum,
            maximum_target: state.maximum,
            target: state.target,
            effective_weight: state.weight,
            excluded: state.excluded,
        })
        .collect();
    let mut result = InitialAllocationResult {
        dataset_id: dataset.id,
        requested_total_rows: request.total_rows,
        initial_target_rows,
        reserved_rows: request.reserved_rows,
        allocated_target_rows,
        unallocated_rows,
        overallocated_rows,
        feasibility,
        issues,
        cells,
        policy: request.policy,
        fingerprint: String::new(),
    };
    result.fingerprint = allocation_fingerprint(&result)?;
    Ok(result)
}

fn normalize_coverage(
    coverage: &[InitialCellCoverage],
    valid_cells: &BTreeMap<String, GenerationCell>,
) -> Result<BTreeMap<String, u32>, InitialAllocationError> {
    let mut normalized = BTreeMap::new();
    for coverage in coverage {
        let key = coverage.cell.key();
        if !valid_cells.contains_key(&key) {
            return Err(InitialAllocationError::UnknownCoverageCell(key));
        }
        if normalized.insert(key.clone(), coverage.accepted).is_some() {
            return Err(InitialAllocationError::DuplicateCoverageCell(key));
        }
    }
    Ok(normalized)
}

fn normalize_constraints<'a>(
    constraints: &'a [InitialCellConstraint],
    valid_cells: &BTreeMap<String, GenerationCell>,
) -> Result<BTreeMap<String, &'a InitialCellConstraint>, InitialAllocationError> {
    let mut normalized = BTreeMap::new();
    for constraint in constraints {
        let key = constraint.cell.key();
        if !valid_cells.contains_key(&key) {
            return Err(InitialAllocationError::UnknownConstraintCell(key));
        }
        if normalized.insert(key.clone(), constraint).is_some() {
            return Err(InitialAllocationError::DuplicateConstraintCell(key));
        }
    }
    Ok(normalized)
}

fn validate_policy(
    policy: &InitialAllocationPolicy,
    dataset: &DatasetDefinition,
    valid_cells: &BTreeMap<String, GenerationCell>,
) -> Result<BTreeMap<String, f64>, InitialAllocationError> {
    match policy {
        InitialAllocationPolicy::Balanced | InitialAllocationPolicy::Explicit { .. } => {
            if let InitialAllocationPolicy::Explicit { targets } = policy {
                validate_explicit_targets(targets, valid_cells)?;
            }
            Ok(valid_cells.keys().map(|key| (key.clone(), 1.0)).collect())
        }
        InitialAllocationPolicy::Weighted { weights }
        | InitialAllocationPolicy::MinimumThenWeighted { weights, .. } => {
            validate_weights(weights, dataset, valid_cells)
        }
    }
}

fn validate_explicit_targets(
    targets: &[ExplicitCellTarget],
    valid_cells: &BTreeMap<String, GenerationCell>,
) -> Result<(), InitialAllocationError> {
    let mut seen = BTreeSet::new();
    for target in targets {
        let key = target.cell.key();
        if !valid_cells.contains_key(&key) {
            return Err(InitialAllocationError::UnknownExplicitCell(key));
        }
        if !seen.insert(key.clone()) {
            return Err(InitialAllocationError::DuplicateExplicitCell(key));
        }
    }
    if seen.len() != valid_cells.len() {
        let missing = valid_cells
            .keys()
            .find(|key| !seen.contains(*key))
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        return Err(InitialAllocationError::MissingExplicitCell(missing));
    }
    Ok(())
}

fn validate_weights(
    weights: &AllocationWeights,
    dataset: &DatasetDefinition,
    valid_cells: &BTreeMap<String, GenerationCell>,
) -> Result<BTreeMap<String, f64>, InitialAllocationError> {
    for (label, weight) in &weights.labels {
        if !dataset.labels.contains(label) {
            return Err(InitialAllocationError::UnknownWeightLabel(label.clone()));
        }
        validate_weight(*weight, format!("label {label}"))?;
    }
    let dimensions = dataset
        .dimensions
        .iter()
        .map(|dimension| (dimension.name.as_str(), &dimension.values))
        .collect::<BTreeMap<_, _>>();
    let mut dimension_weights = BTreeMap::new();
    for weighted in &weights.dimension_values {
        let values = dimensions.get(weighted.dimension.as_str()).ok_or_else(|| {
            InitialAllocationError::UnknownWeightDimension(weighted.dimension.clone())
        })?;
        if !values.contains(&weighted.value) {
            return Err(InitialAllocationError::UnknownWeightDimensionValue {
                dimension: weighted.dimension.clone(),
                value: weighted.value.clone(),
            });
        }
        validate_weight(
            weighted.weight,
            format!("dimension {}={}", weighted.dimension, weighted.value),
        )?;
        let identity = (weighted.dimension.clone(), weighted.value.clone());
        if dimension_weights
            .insert(identity.clone(), weighted.weight)
            .is_some()
        {
            return Err(InitialAllocationError::DuplicateDimensionWeight {
                dimension: identity.0,
                value: identity.1,
            });
        }
    }

    valid_cells
        .iter()
        .map(|(key, cell)| {
            let mut weight = weights.labels.get(&cell.label).copied().unwrap_or(1.0);
            for (dimension, value) in &cell.dimensions {
                weight *= dimension_weights
                    .get(&(dimension.clone(), value.clone()))
                    .copied()
                    .unwrap_or(1.0);
            }
            if !weight.is_finite() || weight <= 0.0 {
                return Err(InitialAllocationError::InvalidEffectiveWeight(key.clone()));
            }
            Ok((key.clone(), weight))
        })
        .collect()
}

fn validate_weight(weight: f64, subject: String) -> Result<(), InitialAllocationError> {
    if !weight.is_finite() || weight <= 0.0 {
        Err(InitialAllocationError::InvalidWeight { subject })
    } else {
        Ok(())
    }
}

fn apply_explicit_targets(
    states: &mut [CellState],
    targets: &BTreeMap<String, u32>,
    initial_target_rows: u32,
    issues: &mut Vec<InitialAllocationIssue>,
) {
    for state in states.iter_mut() {
        let key = state.cell.key();
        let requested = targets[&key];
        if requested < state.current {
            issues.push(InitialAllocationIssue::ExplicitTargetBelowCurrent {
                cell_key: key.clone(),
                target: requested,
                current: state.current,
            });
        }
        if requested < state.minimum {
            issues.push(InitialAllocationIssue::ExplicitTargetBelowMinimum {
                cell_key: key.clone(),
                target: requested,
                minimum: state.minimum,
            });
        }
        if let Some(maximum) = state.maximum
            && requested > maximum
        {
            issues.push(InitialAllocationIssue::ExplicitTargetAboveMaximum {
                cell_key: key.clone(),
                target: requested,
                maximum,
            });
        }
        if state.excluded && requested != state.current {
            issues.push(InitialAllocationIssue::ExplicitTargetChangesExcludedCell {
                cell_key: key,
                target: requested,
                current: state.current,
            });
        }
        state.target = requested.max(state.current).max(state.minimum);
    }
    let actual = states.iter().map(|state| u64::from(state.target)).sum();
    if actual != u64::from(initial_target_rows) {
        issues.push(InitialAllocationIssue::ExplicitTotalMismatch {
            actual,
            expected: initial_target_rows,
        });
    }
}

fn distribute_weighted(
    states: &mut [CellState],
    initial_target_rows: u32,
    issues: &mut Vec<InitialAllocationIssue>,
) -> Result<(), InitialAllocationError> {
    let existing: u64 = states.iter().map(|state| u64::from(state.current)).sum();
    let baseline: u64 = states.iter().map(|state| u64::from(state.target)).sum();
    let capacity: u64 = states
        .iter()
        .map(|state| {
            if state.excluded {
                u64::from(state.current)
            } else {
                u64::from(state.cap.max(state.current))
            }
        })
        .sum();
    if existing > u64::from(initial_target_rows) {
        issues.push(
            InitialAllocationIssue::ExistingCoverageExceedsInitialTarget {
                existing,
                initial_target: initial_target_rows,
            },
        );
    }
    if baseline > u64::from(initial_target_rows) {
        issues.push(InitialAllocationIssue::MinimumsExceedInitialTarget {
            minimum: baseline,
            initial_target: initial_target_rows,
        });
        return Ok(());
    }
    if capacity < u64::from(initial_target_rows) {
        issues.push(InitialAllocationIssue::CapacityBelowInitialTarget {
            capacity,
            initial_target: initial_target_rows,
        });
    }
    let mut remaining = u64::from(initial_target_rows) - baseline;
    while remaining > 0 {
        let active = states
            .iter()
            .enumerate()
            .filter(|(_, state)| !state.excluded && state.target < state.cap)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if active.is_empty() {
            issues.push(InitialAllocationIssue::NoAllocatableCells);
            break;
        }
        let total_weight = active
            .iter()
            .map(|index| states[*index].weight)
            .sum::<f64>();
        if !total_weight.is_finite() || total_weight <= 0.0 {
            return Err(InitialAllocationError::InvalidTotalWeight);
        }
        let mut proposals = active
            .into_iter()
            .map(|index| {
                let ideal = remaining as f64 * states[index].weight / total_weight;
                let capacity = u64::from(states[index].cap - states[index].target);
                let base = (ideal.floor() as u64).min(capacity);
                (index, base, ideal - ideal.floor())
            })
            .collect::<Vec<_>>();
        let base_total = proposals.iter().map(|(_, base, _)| *base).sum::<u64>();
        for (index, base, _) in &proposals {
            states[*index].target = states[*index]
                .target
                .checked_add(
                    (*base)
                        .try_into()
                        .map_err(|_| InitialAllocationError::CountOverflow)?,
                )
                .ok_or(InitialAllocationError::CountOverflow)?;
        }
        remaining -= base_total;
        if remaining == 0 {
            break;
        }
        proposals.sort_by(
            |(left_index, _, left_remainder), (right_index, _, right_remainder)| {
                right_remainder
                    .total_cmp(left_remainder)
                    .then_with(|| {
                        states[*right_index]
                            .weight
                            .total_cmp(&states[*left_index].weight)
                    })
                    .then_with(|| {
                        states[*left_index]
                            .cell
                            .key()
                            .cmp(&states[*right_index].cell.key())
                    })
            },
        );
        let mut progress = base_total;
        for (index, _, _) in proposals {
            if remaining == 0 {
                break;
            }
            if states[index].target < states[index].cap {
                states[index].target += 1;
                remaining -= 1;
                progress += 1;
            }
        }
        if progress == 0 {
            issues.push(InitialAllocationIssue::NoAllocatableCells);
            break;
        }
    }
    Ok(())
}

fn allocation_fingerprint(
    result: &InitialAllocationResult,
) -> Result<String, InitialAllocationError> {
    #[derive(Serialize)]
    struct FingerprintInput<'a> {
        dataset_id: uuid::Uuid,
        requested_total_rows: u32,
        initial_target_rows: u32,
        reserved_rows: u32,
        allocated_target_rows: u64,
        unallocated_rows: u32,
        overallocated_rows: u64,
        feasibility: InitialAllocationFeasibility,
        issues: &'a [InitialAllocationIssue],
        cells: &'a [InitialCellAllocation],
        policy: &'a InitialAllocationPolicy,
    }

    artifact_core::fingerprint(&FingerprintInput {
        dataset_id: result.dataset_id,
        requested_total_rows: result.requested_total_rows,
        initial_target_rows: result.initial_target_rows,
        reserved_rows: result.reserved_rows,
        allocated_target_rows: result.allocated_target_rows,
        unallocated_rows: result.unallocated_rows,
        overallocated_rows: result.overallocated_rows,
        feasibility: result.feasibility,
        issues: &result.issues,
        cells: &result.cells,
        policy: &result.policy,
    })
    .map_err(|error| InitialAllocationError::Fingerprint(error.to_string()))
}

fn allocation_record_fingerprint(
    record: &InitialAllocationRecord,
) -> Result<String, InitialAllocationError> {
    #[derive(Serialize)]
    struct FingerprintInput<'a> {
        id: uuid::Uuid,
        result: &'a InitialAllocationResult,
        generation_plan_id: uuid::Uuid,
        created_at: DateTime<Utc>,
    }

    artifact_core::fingerprint(&FingerprintInput {
        id: record.id,
        result: &record.result,
        generation_plan_id: record.generation_plan_id,
        created_at: record.created_at,
    })
    .map_err(|error| InitialAllocationError::Fingerprint(error.to_string()))
}

struct CellState {
    cell: GenerationCell,
    current: u32,
    minimum: u32,
    maximum: Option<u32>,
    excluded: bool,
    weight: f64,
    target: u32,
    cap: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InitialAllocationError {
    #[error("reserved rows {reserve} exceed requested total rows {total}")]
    ReserveExceedsTotal { reserve: u32, total: u32 },
    #[error("current coverage references an unknown cell: {0}")]
    UnknownCoverageCell(String),
    #[error("current coverage repeats a cell: {0}")]
    DuplicateCoverageCell(String),
    #[error("a cell constraint references an unknown cell: {0}")]
    UnknownConstraintCell(String),
    #[error("cell constraints repeat a cell: {0}")]
    DuplicateConstraintCell(String),
    #[error("an explicit target references an unknown cell: {0}")]
    UnknownExplicitCell(String),
    #[error("explicit targets repeat a cell: {0}")]
    DuplicateExplicitCell(String),
    #[error("explicit targets omit a dataset cell: {0}")]
    MissingExplicitCell(String),
    #[error("a label weight references an unknown label: {0}")]
    UnknownWeightLabel(String),
    #[error("a dimension weight references an unknown dimension: {0}")]
    UnknownWeightDimension(String),
    #[error("a dimension weight references an unknown value: {dimension}={value}")]
    UnknownWeightDimensionValue { dimension: String, value: String },
    #[error("a dimension value is weighted more than once: {dimension}={value}")]
    DuplicateDimensionWeight { dimension: String, value: String },
    #[error("allocation weight must be finite and positive for {subject}")]
    InvalidWeight { subject: String },
    #[error("combined allocation weight is invalid for cell: {0}")]
    InvalidEffectiveWeight(String),
    #[error("the total active allocation weight is invalid")]
    InvalidTotalWeight,
    #[error("allocation count exceeds the supported range")]
    CountOverflow,
    #[error("an infeasible allocation cannot become a generation plan")]
    Infeasible,
    #[error("allocation and dataset identities differ")]
    DatasetMismatch,
    #[error("Slice 1 rejected the derived explicit plan: {0}")]
    Plan(String),
    #[error("allocation fingerprint failed: {0}")]
    Fingerprint(String),
    #[error("allocation fingerprint does not reproduce")]
    FingerprintMismatch,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use generation_core::{
        dimensions::expand_generation_cells,
        domain::{DatasetDefinition, DimensionDefinition},
    };

    use super::{
        AllocationWeights, DimensionValueWeight, ExplicitCellTarget, InitialAllocationError,
        InitialAllocationFeasibility, InitialAllocationIssue, InitialAllocationPolicy,
        InitialAllocationRequest, InitialCellConstraint, InitialCellCoverage,
        allocate_initial_budget,
    };

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::new(
            "support",
            "classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
        )
        .expect("dataset")
    }

    fn request(total_rows: u32, policy: InitialAllocationPolicy) -> InitialAllocationRequest {
        InitialAllocationRequest {
            total_rows,
            reserved_rows: 0,
            policy,
            current_coverage: Vec::new(),
            constraints: Vec::new(),
        }
    }

    #[test]
    fn balanced_allocation_conserves_exact_total_with_stable_ties() {
        let result =
            allocate_initial_budget(&dataset(), request(10, InitialAllocationPolicy::Balanced))
                .expect("allocation");

        assert_eq!(result.feasibility, InitialAllocationFeasibility::Feasible);
        assert_eq!(result.allocated_target_rows, 10);
        assert_eq!(result.unallocated_rows, 0);
        let keys_and_targets = result
            .cells
            .iter()
            .map(|cell| (cell.cell.key(), cell.target))
            .collect::<Vec<_>>();
        assert!(
            keys_and_targets
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        assert_eq!(
            keys_and_targets
                .iter()
                .map(|(_, target)| *target)
                .collect::<Vec<_>>(),
            vec![3, 3, 2, 2]
        );
        assert_eq!(
            result.fingerprint,
            result.reproduce_fingerprint().expect("fingerprint")
        );
    }

    #[test]
    fn label_and_dimension_weights_multiply_deterministically() {
        let result = allocate_initial_budget(
            &dataset(),
            request(
                20,
                InitialAllocationPolicy::Weighted {
                    weights: AllocationWeights {
                        labels: BTreeMap::from([("billing".into(), 3.0)]),
                        dimension_values: vec![DimensionValueWeight {
                            dimension: "difficulty".into(),
                            value: "hard".into(),
                            weight: 2.0,
                        }],
                    },
                },
            ),
        )
        .expect("allocation");

        let targets = result
            .cells
            .iter()
            .map(|cell| {
                (
                    format!("{}/{}", cell.cell.label, cell.cell.dimensions["difficulty"]),
                    cell.target,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            targets,
            BTreeMap::from([
                ("billing/easy".into(), 5),
                ("billing/hard".into(), 10),
                ("fraud/easy".into(), 2),
                ("fraud/hard".into(), 3),
            ])
        );
    }

    #[test]
    fn minimum_then_weighted_honors_floor_before_remainder() {
        let result = allocate_initial_budget(
            &dataset(),
            request(
                12,
                InitialAllocationPolicy::MinimumThenWeighted {
                    minimum_per_cell: 1,
                    weights: AllocationWeights {
                        labels: BTreeMap::from([("billing".into(), 3.0)]),
                        dimension_values: Vec::new(),
                    },
                },
            ),
        )
        .expect("allocation");

        let by_label = result
            .cells
            .iter()
            .fold(BTreeMap::new(), |mut counts, cell| {
                *counts.entry(cell.cell.label.clone()).or_insert(0_u32) += cell.target;
                counts
            });
        assert_eq!(
            by_label,
            BTreeMap::from([("billing".into(), 8), ("fraud".into(), 4)])
        );
        assert!(result.cells.iter().all(|cell| cell.target >= 1));
    }

    #[test]
    fn current_coverage_and_reserved_budget_are_accounted_for() {
        let cells = expand_generation_cells(&dataset());
        let mut allocation = request(20, InitialAllocationPolicy::Balanced);
        allocation.reserved_rows = 4;
        allocation.current_coverage = vec![InitialCellCoverage {
            cell: cells[0].clone(),
            accepted: 10,
        }];

        let result = allocate_initial_budget(&dataset(), allocation).expect("allocation");
        assert_eq!(result.initial_target_rows, 16);
        assert_eq!(result.reserved_rows, 4);
        assert_eq!(result.cells.iter().map(|cell| cell.target).sum::<u32>(), 16);
        let covered = result
            .cells
            .iter()
            .find(|cell| cell.cell == cells[0])
            .expect("covered cell");
        assert!(covered.target >= 10);
        assert_eq!(covered.additional_required, covered.target - 10);
    }

    #[test]
    fn explicit_complete_targets_create_an_ordinary_unequal_plan() {
        let dataset = dataset();
        let cells = expand_generation_cells(&dataset);
        let targets = cells
            .iter()
            .cloned()
            .zip([1, 2, 3, 4])
            .map(|(cell, target)| ExplicitCellTarget { cell, target })
            .collect();
        let result = allocate_initial_budget(
            &dataset,
            request(10, InitialAllocationPolicy::Explicit { targets }),
        )
        .expect("allocation");
        let plan = result.to_generation_plan(&dataset).expect("plan");

        assert_eq!(plan.total_target_count(), 10);
        let expected = cells
            .iter()
            .cloned()
            .zip([1, 2, 3, 4])
            .map(|(cell, target)| (cell.key(), target))
            .collect::<BTreeMap<_, _>>();
        let actual = plan
            .cells
            .iter()
            .map(|cell| (cell.cell.key(), cell.target_count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn infeasible_bounds_report_overallocation_and_reject_plan_creation() {
        let cells = expand_generation_cells(&dataset());
        let mut allocation = request(2, InitialAllocationPolicy::Balanced);
        allocation.constraints = vec![InitialCellConstraint {
            cell: cells[0].clone(),
            minimum_target: 5,
            maximum_target: Some(5),
            excluded: false,
        }];

        let result = allocate_initial_budget(&dataset(), allocation).expect("result");
        assert_eq!(result.feasibility, InitialAllocationFeasibility::Infeasible);
        assert_eq!(result.overallocated_rows, 3);
        assert!(matches!(
            result.issues.as_slice(),
            [InitialAllocationIssue::MinimumsExceedInitialTarget { .. }]
        ));
        assert_eq!(
            result.to_generation_plan(&dataset()),
            Err(InitialAllocationError::Infeasible)
        );
    }

    #[test]
    fn capacity_exhaustion_is_structured_and_preserves_unallocated_rows() {
        let cells = expand_generation_cells(&dataset());
        let mut allocation = request(10, InitialAllocationPolicy::Balanced);
        allocation.constraints = cells
            .into_iter()
            .map(|cell| InitialCellConstraint {
                cell,
                minimum_target: 0,
                maximum_target: Some(2),
                excluded: false,
            })
            .collect();

        let result = allocate_initial_budget(&dataset(), allocation).expect("result");
        assert_eq!(result.allocated_target_rows, 8);
        assert_eq!(result.unallocated_rows, 2);
        assert!(result.issues.iter().any(|issue| matches!(
            issue,
            InitialAllocationIssue::CapacityBelowInitialTarget { .. }
        )));
    }

    #[test]
    fn invalid_weight_and_unknown_cells_are_rejected() {
        let invalid_weight = allocate_initial_budget(
            &dataset(),
            request(
                10,
                InitialAllocationPolicy::Weighted {
                    weights: AllocationWeights {
                        labels: BTreeMap::from([("other".into(), 1.0)]),
                        dimension_values: Vec::new(),
                    },
                },
            ),
        );
        assert_eq!(
            invalid_weight,
            Err(InitialAllocationError::UnknownWeightLabel("other".into()))
        );

        let other = DatasetDefinition::new("other", "classify", vec!["x".into()], Vec::new())
            .expect("other dataset");
        let mut unknown = request(10, InitialAllocationPolicy::Balanced);
        unknown.current_coverage = vec![InitialCellCoverage {
            cell: expand_generation_cells(&other)[0].clone(),
            accepted: 1,
        }];
        assert!(matches!(
            allocate_initial_budget(&dataset(), unknown),
            Err(InitialAllocationError::UnknownCoverageCell(_))
        ));
    }
}
