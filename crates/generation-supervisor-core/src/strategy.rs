//! Exact, deterministic compilation of approximate strategy shares.

use std::{cmp::Ordering, collections::BTreeMap};

use chrono::{DateTime, Utc};
use generation_core::{
    domain::{GenerationCell, GenerationPlan},
    strategy::{GenerationStrategyDirective, ResolvedGenerationStrategyContext},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    SupervisorError,
    contract::GenerationQualityContract,
    fingerprint,
    observation::{ContractRowVerdict, RowQualityObservation, StructuralOutcome},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyScope {
    pub cell_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<Uuid>,
}

impl StrategyScope {
    pub fn default_for(cell_key: impl Into<String>) -> Self {
        Self {
            cell_key: cell_key.into(),
            directive_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyAssignment {
    pub assignment_set_id: Uuid,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub strategy_context_id: Uuid,
    pub strategy_context_fingerprint: String,
    pub cell: GenerationCell,
    pub cell_key: String,
    pub row_sequence: u32,
    pub deterministic_seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_kind: Option<String>,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_criterion: Option<String>,
    pub fingerprint: String,
}

impl StrategyAssignment {
    pub fn scope(&self) -> StrategyScope {
        StrategyScope {
            cell_key: self.cell_key.clone(),
            directive_id: self.directive_id,
        }
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.assignment_set_id.is_nil()
            || self.plan_id.is_nil()
            || self.strategy_context_id.is_nil()
            || self.cell.key() != self.cell_key
            || self.plan_fingerprint.is_empty()
            || self.strategy_context_fingerprint.is_empty()
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "strategy assignment identity or fingerprint is invalid".into(),
            ));
        }
        match self.directive_id {
            Some(_) => {
                if self
                    .directive_kind
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                    || self.instructions.is_empty()
                    || self
                        .instructions
                        .iter()
                        .any(|value| value.trim().is_empty())
                {
                    return Err(SupervisorError::Validation(
                        "assigned directives require kind and instructions".into(),
                    ));
                }
            }
            None => {
                if self.directive_kind.is_some()
                    || !self.instructions.is_empty()
                    || self.expected_criterion.is_some()
                {
                    return Err(SupervisorError::Validation(
                        "default strategy assignment cannot carry directive guidance".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyAssignmentSet {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub strategy_context_id: Uuid,
    pub strategy_context_fingerprint: String,
    pub base_seed: u64,
    pub assignments: Vec<StrategyAssignment>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyCoverage {
    pub target: u32,
    pub attempted: u32,
    pub structurally_accepted: u32,
    pub assessed: u32,
    pub qualified: u32,
    pub borderline: u32,
    pub quarantined: u32,
    pub unassessed: u32,
    pub invalid: u32,
    pub remaining: u32,
}

impl StrategyAssignmentSet {
    pub fn compile(
        id: Uuid,
        plan: &GenerationPlan,
        context: &ResolvedGenerationStrategyContext,
        base_seed: u64,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || context.id.is_nil() {
            return Err(SupervisorError::Validation(
                "strategy assignment identities must not be nil".into(),
            ));
        }
        let plan_fingerprint = fingerprint(plan)?;
        if context.plan_id != plan.id
            || context.dataset_id != plan.dataset_id
            || context.plan_fingerprint != plan_fingerprint
            || context
                .reproduce_fingerprint()
                .map_err(|error| SupervisorError::Integrity(error.to_string()))?
                != context.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "strategy context is not bound to the exact generation plan".into(),
            ));
        }

        let mut assignments =
            Vec::with_capacity(usize::try_from(plan.total_target_count()).map_err(|_| {
                SupervisorError::Validation("generation target does not fit local memory".into())
            })?);
        for planned in &plan.cells {
            let directives = context.for_cell(&planned.cell);
            let buckets = allocate_counts(planned.target_count, directives)?;
            let schedule = fair_schedule(planned.target_count, &buckets);
            for (row_sequence, bucket_index) in schedule.into_iter().enumerate() {
                let bucket = &buckets[bucket_index];
                let directive = bucket.directive;
                let mut assignment = StrategyAssignment {
                    assignment_set_id: id,
                    plan_id: plan.id,
                    plan_fingerprint: plan_fingerprint.clone(),
                    strategy_context_id: context.id,
                    strategy_context_fingerprint: context.fingerprint.clone(),
                    cell: planned.cell.clone(),
                    cell_key: planned.cell.key(),
                    row_sequence: u32::try_from(row_sequence).map_err(|_| {
                        SupervisorError::Validation("row sequence exceeds u32".into())
                    })?,
                    deterministic_seed: mix_seed(
                        base_seed,
                        &planned.cell.key(),
                        u32::try_from(row_sequence).map_err(|_| {
                            SupervisorError::Validation("row sequence exceeds u32".into())
                        })?,
                    ),
                    directive_id: directive.map(|value| value.source_directive_id),
                    directive_kind: directive.map(|value| value.kind.clone()),
                    instructions: directive
                        .map(|value| value.instructions.clone())
                        .unwrap_or_default(),
                    expected_criterion: directive.map(|value| value.kind.clone()),
                    fingerprint: String::new(),
                };
                assignment.fingerprint = assignment.reproduce_fingerprint()?;
                assignment.validate()?;
                assignments.push(assignment);
            }
        }
        let mut value = Self {
            id,
            plan_id: plan.id,
            plan_fingerprint,
            strategy_context_id: context.id,
            strategy_context_fingerprint: context.fingerprint.clone(),
            base_seed,
            assignments,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate(plan)?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self, plan: &GenerationPlan) -> Result<(), SupervisorError> {
        if self.id.is_nil()
            || self.plan_id != plan.id
            || self.plan_fingerprint != fingerprint(plan)?
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "strategy assignment set does not reproduce".into(),
            ));
        }
        let mut actual = BTreeMap::<String, u32>::new();
        for assignment in &self.assignments {
            assignment.validate()?;
            if assignment.assignment_set_id != self.id
                || assignment.plan_id != self.plan_id
                || assignment.plan_fingerprint != self.plan_fingerprint
                || assignment.strategy_context_id != self.strategy_context_id
                || assignment.strategy_context_fingerprint != self.strategy_context_fingerprint
            {
                return Err(SupervisorError::Integrity(
                    "strategy assignment has mismatched parent binding".into(),
                ));
            }
            let count = actual.entry(assignment.cell_key.clone()).or_default();
            *count = count.checked_add(1).ok_or_else(|| {
                SupervisorError::Validation("strategy assignment count overflow".into())
            })?;
        }
        for planned in &plan.cells {
            if actual.remove(&planned.cell.key()).unwrap_or(0) != planned.target_count {
                return Err(SupervisorError::Integrity(format!(
                    "strategy assignments do not conserve target for {}",
                    planned.cell.key()
                )));
            }
        }
        if !actual.is_empty() {
            return Err(SupervisorError::Integrity(
                "strategy assignments reference cells outside the plan".into(),
            ));
        }
        Ok(())
    }

    pub fn target_counts(&self) -> BTreeMap<StrategyScope, u32> {
        let mut counts = BTreeMap::new();
        for assignment in &self.assignments {
            *counts.entry(assignment.scope()).or_default() += 1;
        }
        counts
    }

    /// Derives strategy coverage exclusively from immutable assignments and
    /// persisted row observations. `remaining` means rows still needed to
    /// reach the qualified target, not merely rows that have not been tried.
    pub fn coverage(
        &self,
        contract: &GenerationQualityContract,
        observations: &[RowQualityObservation],
    ) -> Result<BTreeMap<StrategyScope, StrategyCoverage>, SupervisorError> {
        contract.validate()?;
        if self.plan_id != contract.plan.id
            || self.plan_fingerprint != contract.plan.fingerprint
            || contract.strategy_context.as_ref().is_none_or(|binding| {
                binding.id != self.strategy_context_id
                    || binding.fingerprint != self.strategy_context_fingerprint
            })
        {
            return Err(SupervisorError::Integrity(
                "strategy coverage assignments are outside the quality contract".into(),
            ));
        }
        let assignments = self
            .assignments
            .iter()
            .map(|assignment| (assignment.fingerprint.as_str(), assignment))
            .collect::<BTreeMap<_, _>>();
        let mut coverage = self
            .target_counts()
            .into_iter()
            .map(|(scope, target)| {
                (
                    scope,
                    StrategyCoverage {
                        target,
                        ..StrategyCoverage::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for observation in observations {
            observation.validate(contract)?;
            let assignment_fingerprint = observation
                .strategy_assignment_fingerprint
                .as_deref()
                .ok_or_else(|| {
                    SupervisorError::Integrity(
                        "strategy coverage row has no exact assignment binding".into(),
                    )
                })?;
            let assignment = assignments.get(assignment_fingerprint).ok_or_else(|| {
                SupervisorError::Integrity(
                    "strategy coverage row references an unknown assignment".into(),
                )
            })?;
            if assignment.cell_key != observation.cell_key
                || assignment.directive_id != observation.strategy_directive_id
            {
                return Err(SupervisorError::Integrity(
                    "strategy coverage row does not match its assignment".into(),
                ));
            }
            let item = coverage.get_mut(&assignment.scope()).ok_or_else(|| {
                SupervisorError::Integrity("strategy coverage scope is missing".into())
            })?;
            item.attempted = checked_increment(item.attempted)?;
            if observation.structural_outcome == StructuralOutcome::Accepted {
                item.structurally_accepted = checked_increment(item.structurally_accepted)?;
            }
            if observation.assessment.is_some() {
                item.assessed = checked_increment(item.assessed)?;
            }
            match observation.contract_verdict(contract) {
                ContractRowVerdict::Qualified => {
                    item.qualified = checked_increment(item.qualified)?
                }
                ContractRowVerdict::Borderline => {
                    item.borderline = checked_increment(item.borderline)?;
                }
                ContractRowVerdict::Quarantined => {
                    item.quarantined = checked_increment(item.quarantined)?;
                }
                ContractRowVerdict::Unassessed => {
                    item.unassessed = checked_increment(item.unassessed)?;
                }
                ContractRowVerdict::Invalid => item.invalid = checked_increment(item.invalid)?,
            }
        }
        for item in coverage.values_mut() {
            item.remaining = item.target.saturating_sub(item.qualified);
        }
        Ok(coverage)
    }
}

fn checked_increment(value: u32) -> Result<u32, SupervisorError> {
    value
        .checked_add(1)
        .ok_or_else(|| SupervisorError::Validation("strategy coverage count overflow".into()))
}

#[derive(Debug)]
struct AllocationBucket<'a> {
    directive: Option<&'a GenerationStrategyDirective>,
    stable_key: String,
    target: u32,
    remainder: u64,
}

fn allocate_counts<'a>(
    target: u32,
    directives: &'a [GenerationStrategyDirective],
) -> Result<Vec<AllocationBucket<'a>>, SupervisorError> {
    let share_sum = directives.iter().try_fold(0_u32, |sum, directive| {
        if directive.share_basis_points == 0 || directive.share_basis_points > 10_000 {
            return Err(SupervisorError::Validation(
                "strategy shares must be within 1..=10000 basis points".into(),
            ));
        }
        sum.checked_add(u32::from(directive.share_basis_points))
            .ok_or_else(|| SupervisorError::Validation("strategy share sum overflow".into()))
    })?;
    if share_sum > 10_000 {
        return Err(SupervisorError::Validation(
            "strategy shares for one cell exceed 10000 basis points".into(),
        ));
    }

    let mut entries = directives
        .iter()
        .map(|directive| {
            (
                Some(directive),
                directive.source_directive_id.to_string(),
                u32::from(directive.share_basis_points),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.1.cmp(&right.1));
    if share_sum < 10_000 {
        entries.push((None, "~default".into(), 10_000 - share_sum));
    }

    let mut buckets = entries
        .into_iter()
        .map(|(directive, stable_key, share)| {
            let numerator = u64::from(target) * u64::from(share);
            AllocationBucket {
                directive,
                stable_key,
                target: u32::try_from(numerator / 10_000).expect("target floor fits u32"),
                remainder: numerator % 10_000,
            }
        })
        .collect::<Vec<_>>();
    let allocated = buckets.iter().map(|bucket| bucket.target).sum::<u32>();
    let mut remainder_indices = (0..buckets.len()).collect::<Vec<_>>();
    remainder_indices.sort_by(|left, right| {
        buckets[*right]
            .remainder
            .cmp(&buckets[*left].remainder)
            .then_with(|| buckets[*left].stable_key.cmp(&buckets[*right].stable_key))
    });
    for index in remainder_indices
        .into_iter()
        .take(usize::try_from(target - allocated).expect("remainder fits usize"))
    {
        buckets[index].target += 1;
    }
    Ok(buckets)
}

fn fair_schedule(target: u32, buckets: &[AllocationBucket<'_>]) -> Vec<usize> {
    let mut emitted = vec![0_u32; buckets.len()];
    let mut schedule = Vec::with_capacity(usize::try_from(target).expect("target fits usize"));
    for position in 0..target {
        let choice = (0..buckets.len())
            .filter(|index| emitted[*index] < buckets[*index].target)
            .max_by(|left, right| {
                let score = |index: usize| {
                    i128::from(buckets[index].target) * i128::from(position + 1)
                        - i128::from(emitted[index]) * i128::from(target)
                };
                score(*left).cmp(&score(*right)).then_with(|| {
                    match buckets[*right].stable_key.cmp(&buckets[*left].stable_key) {
                        Ordering::Equal => right.cmp(left),
                        ordering => ordering,
                    }
                })
            })
            .expect("allocated strategy counts conserve a positive target");
        emitted[choice] += 1;
        schedule.push(choice);
    }
    schedule
}

fn mix_seed(base_seed: u64, cell_key: &str, sequence: u32) -> u64 {
    // FNV-1a is used only for a stable scheduling seed, never for security.
    let mut value = 0xcbf2_9ce4_8422_2325_u64 ^ base_seed;
    for byte in cell_key.bytes().chain(sequence.to_le_bytes()) {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use generation_core::{
        domain::{DatasetDefinition, GenerationCell, GenerationPlan, PlannedCell},
        strategy::{GenerationStrategyDirective, ResolvedGenerationStrategyContext},
    };

    use super::*;

    fn fixture(target: u32) -> (GenerationPlan, ResolvedGenerationStrategyContext) {
        let now = Utc.with_ymd_and_hms(2026, 8, 31, 10, 0, 0).unwrap();
        let dataset = DatasetDefinition::with_identity(
            Uuid::new_v4(),
            "intent data",
            "Classify support requests",
            vec!["billing".into()],
            vec![],
            now,
        )
        .unwrap();
        let cell = GenerationCell {
            label: "billing".into(),
            dimensions: BTreeMap::new(),
        };
        let plan = GenerationPlan::with_identity(
            Uuid::new_v4(),
            dataset.id,
            vec![PlannedCell {
                cell: cell.clone(),
                target_count: target,
            }],
            now,
        )
        .unwrap();
        let directives = vec![
            GenerationStrategyDirective {
                source_directive_id: Uuid::from_u128(2),
                kind: "boundary".into(),
                share_basis_points: 3_333,
                instructions: vec!["Use a realistic boundary case.".into()],
                related_labels: vec!["billing".into()],
                rationale: "Boundary coverage".into(),
                confidence: "high".into(),
            },
            GenerationStrategyDirective {
                source_directive_id: Uuid::from_u128(1),
                kind: "noise".into(),
                share_basis_points: 1_667,
                instructions: vec!["Use plausible input noise.".into()],
                related_labels: vec!["billing".into()],
                rationale: "Robustness".into(),
                confidence: "high".into(),
            },
        ];
        let context = ResolvedGenerationStrategyContext::create(
            &dataset,
            &plan,
            Uuid::new_v4(),
            "proposal-fp".into(),
            Uuid::new_v4(),
            "approval-fp".into(),
            BTreeMap::from([(cell.key(), directives)]),
        )
        .unwrap();
        (plan, context)
    }

    #[test]
    fn largest_remainder_conserves_every_row_and_default_share() {
        let (plan, context) = fixture(7);
        let set = StrategyAssignmentSet::compile(
            Uuid::from_u128(10),
            &plan,
            &context,
            42,
            Utc.with_ymd_and_hms(2026, 8, 31, 10, 1, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(set.assignments.len(), 7);
        let counts = set.target_counts();
        assert_eq!(counts.values().sum::<u32>(), 7);
        assert_eq!(
            counts[&StrategyScope {
                cell_key: plan.cells[0].cell.key(),
                directive_id: Some(Uuid::from_u128(2)),
            }],
            2
        );
        assert_eq!(
            counts[&StrategyScope::default_for(plan.cells[0].cell.key())],
            4
        );
        set.validate(&plan).unwrap();
    }

    #[test]
    fn compilation_is_reproducible_with_fixed_identity_and_time() {
        let (plan, context) = fixture(25);
        let id = Uuid::from_u128(10);
        let now = Utc.with_ymd_and_hms(2026, 8, 31, 10, 1, 0).unwrap();
        let first = StrategyAssignmentSet::compile(id, &plan, &context, 42, now).unwrap();
        let second = StrategyAssignmentSet::compile(id, &plan, &context, 42, now).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn shares_over_one_hundred_percent_fail_closed() {
        let (plan, mut context) = fixture(10);
        context.per_cell.values_mut().next().unwrap()[0].share_basis_points = 9_000;
        context.fingerprint = context.reproduce_fingerprint().unwrap();
        assert!(matches!(
            StrategyAssignmentSet::compile(Uuid::new_v4(), &plan, &context, 1, Utc::now()),
            Err(SupervisorError::Validation(message)) if message.contains("exceed")
        ));
    }
}
