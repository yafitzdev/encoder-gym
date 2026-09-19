//! Immutable generation slots and outcomes. Native content is opaque here;
//! only the task adapter may admit it, and dataset management owns publication.

use generation_core::structured::StructuredGenerationRequest;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{OptimizationError, agent::AgentTokenUsage, fingerprint, require};

/// Frozen launch policy. V1 preserves the historical single structural
/// canary. V3 reserves an existing prefix for every repair combination and
/// requires semantic admission before any bulk slot may be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationCanaryPolicy {
    FirstBatchAllAdmittedV1,
    PerCombinationSemanticV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationPhase {
    Canary,
    Bulk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStrategy {
    LabelPreservingVariant,
    ExistingAnchorContrast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContrastSide {
    Left,
    Right,
}

/// Protocol-V3 execution identity. Optionality is solely for replaying V1/V2
/// task fixtures; a V3 launch requires this metadata on every generation task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationExecutionV3 {
    pub target_id: String,
    pub combination_id: String,
    pub strategy: GenerationStrategy,
    pub phase: GenerationPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_pair_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_side: Option<ContrastSide>,
}

impl GenerationExecutionV3 {
    fn validate(&self, task: &GenerationTask) -> Result<(), OptimizationError> {
        for value in [&self.target_id, &self.combination_id] {
            require(
                !value.trim().is_empty()
                    && value.len() <= 160
                    && !value.chars().any(char::is_whitespace),
                "Invalid protocol-V3 generation identity",
            )?;
        }
        match self.strategy {
            GenerationStrategy::LabelPreservingVariant => require(
                self.contrast_pair_id.is_none() && self.contrast_side.is_none(),
                "Variant generation cannot declare contrast coupling",
            )?,
            GenerationStrategy::ExistingAnchorContrast => require(
                self.contrast_pair_id
                    .as_ref()
                    .is_some_and(|value| !value.is_empty() && value.len() <= 160)
                    && self.contrast_side.is_some(),
                "Contrast generation requires a pair and side",
            )?,
        }
        match self.phase {
            GenerationPhase::Canary => require(
                task.first_row == 0 && (1..=2).contains(&task.requested_rows),
                "A protocol-V3 canary must be the first batch of at most two rows",
            ),
            GenerationPhase::Bulk => require(
                task.first_row > 0,
                "Protocol-V3 bulk generation cannot precede its canary",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationCanaryStatus {
    Disabled,
    NotRequired,
    Pending,
    Interrupted,
    Passed,
    Rejected,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationCanaryObservation {
    pub policy: Option<GenerationCanaryPolicy>,
    pub status: GenerationCanaryStatus,
    pub call_id: Option<Uuid>,
    pub requested: u32,
    pub admitted: u32,
    pub rejected: Vec<RejectedGenerationRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V3CanaryStatus {
    Passed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V3CanaryUnit {
    pub target_id: String,
    pub combination_id: String,
    pub strategy: GenerationStrategy,
    pub contrast_pair_id: Option<String>,
    pub requested: u32,
    pub structurally_admitted: u32,
    pub semantically_admitted: u32,
    pub rejected_row_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V3CanaryGate {
    pub status: V3CanaryStatus,
    pub units: Vec<V3CanaryUnit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairNotExecutedReason {
    CanaryRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairNotExecuted {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub iteration: u32,
    pub proposal_fingerprint: String,
    pub repair_plan_fingerprint: String,
    pub reason: RepairNotExecutedReason,
    pub canary: V3CanaryGate,
}

impl RepairNotExecuted {
    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            self.schema_version == 1
                && !self.run_id.is_nil()
                && (1..=10).contains(&self.iteration)
                && self.reason == RepairNotExecutedReason::CanaryRejected
                && self.canary.status == V3CanaryStatus::Rejected
                && !self.canary.units.is_empty(),
            "Invalid repair-not-executed receipt",
        )?;
        for value in [&self.proposal_fingerprint, &self.repair_plan_fingerprint] {
            require(
                value.strip_prefix("sha256:").is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }),
                "Invalid repair-not-executed fingerprint",
            )?;
        }
        Ok(())
    }
}

/// Evaluate every saved V3 canary as one immutable gate. `semantic_decisions`
/// must contain exactly one decision for each structurally admitted row.
/// Structural rejection, semantic rejection, or a partial contrast pair fails
/// the whole gate; callers must not dispatch bulk work after that result.
pub fn evaluate_v3_canaries(
    tasks: &[GenerationTask],
    outcomes: &[GenerationOutcome],
    semantic_decisions: &BTreeMap<String, bool>,
) -> Result<V3CanaryGate, OptimizationError> {
    require(
        !tasks.is_empty() && tasks.len() == outcomes.len(),
        "Protocol-V3 canary evidence is incomplete",
    )?;
    let mut units = BTreeMap::<String, V3CanaryUnit>::new();
    let mut contrast_sides = BTreeMap::<String, BTreeSet<ContrastSide>>::new();
    let mut consumed_decisions = BTreeSet::new();
    for (task, outcome) in tasks.iter().zip(outcomes) {
        task.validate()?;
        outcome.validate(task)?;
        let execution = task.execution_v3.as_ref().ok_or_else(|| {
            OptimizationError::Validation("Protocol-V3 canary metadata is missing".into())
        })?;
        require(
            execution.phase == GenerationPhase::Canary,
            "Bulk generation was included in the canary gate",
        )?;
        let admission = outcome.admission.as_ref().ok_or_else(|| {
            OptimizationError::Validation("Protocol-V3 canary outcome is interrupted".into())
        })?;
        let unit = units
            .entry(execution.combination_id.clone())
            .or_insert_with(|| V3CanaryUnit {
                target_id: execution.target_id.clone(),
                combination_id: execution.combination_id.clone(),
                strategy: execution.strategy,
                contrast_pair_id: execution.contrast_pair_id.clone(),
                requested: 0,
                structurally_admitted: 0,
                semantically_admitted: 0,
                rejected_row_ids: Vec::new(),
            });
        require(
            unit.target_id == execution.target_id
                && unit.strategy == execution.strategy
                && unit.contrast_pair_id == execution.contrast_pair_id,
            "Protocol-V3 canary combination identity conflicts",
        )?;
        unit.requested += task.requested_rows;
        unit.structurally_admitted += admission.accepted.len() as u32;
        for row in &admission.accepted {
            let row_id = generated_semantic_row_id(task, row.index);
            let admitted = semantic_decisions.get(&row_id).ok_or_else(|| {
                OptimizationError::Validation(
                    "Protocol-V3 canary row has no semantic admission".into(),
                )
            })?;
            require(
                consumed_decisions.insert(row_id.clone()),
                "Protocol-V3 canary semantic decision is repeated",
            )?;
            if *admitted {
                unit.semantically_admitted += 1;
            } else {
                unit.rejected_row_ids.push(row_id);
            }
        }
        unit.rejected_row_ids.extend(
            admission
                .rejected
                .iter()
                .map(|row| generated_semantic_row_id(task, row.index)),
        );
        if execution.strategy == GenerationStrategy::ExistingAnchorContrast {
            let side = execution.contrast_side.ok_or_else(|| {
                OptimizationError::Validation("Contrast canary side is missing".into())
            })?;
            require(
                contrast_sides
                    .entry(execution.combination_id.clone())
                    .or_default()
                    .insert(side),
                "Contrast canary repeats one side",
            )?;
        }
    }
    require(
        consumed_decisions.len() == semantic_decisions.len(),
        "Semantic canary evidence contains rows outside the canary task set",
    )?;
    for (combination, sides) in contrast_sides {
        require(
            sides == BTreeSet::from([ContrastSide::Left, ContrastSide::Right])
                && units[&combination].requested == 2,
            "Contrast canary is not a complete coupled pair",
        )?;
    }
    let units = units.into_values().collect::<Vec<_>>();
    let status = if units.iter().all(|unit| {
        unit.requested == unit.structurally_admitted
            && unit.requested == unit.semantically_admitted
            && unit.rejected_row_ids.is_empty()
    }) {
        V3CanaryStatus::Passed
    } else {
        V3CanaryStatus::Rejected
    };
    Ok(V3CanaryGate { status, units })
}

pub fn canary_passed(
    task: &GenerationTask,
    outcome: &GenerationOutcome,
) -> Result<bool, OptimizationError> {
    outcome.validate(task)?;
    Ok(outcome.admission.as_ref().is_some_and(|admission| {
        admission.accepted.len() == task.requested_rows as usize && admission.rejected.is_empty()
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationTask {
    pub id: Uuid,
    pub run_id: Uuid,
    pub iteration: u32,
    pub proposal_fingerprint: String,
    pub template_row_id: String,
    pub template_fingerprint: String,
    pub target_index: u32,
    pub first_row: u32,
    pub requested_rows: u32,
    pub request: StructuredGenerationRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_v3: Option<GenerationExecutionV3>,
}

impl GenerationTask {
    pub fn fingerprint(&self) -> Result<String, OptimizationError> {
        self.validate()?;
        fingerprint(self)
    }

    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            !self.id.is_nil() && !self.run_id.is_nil() && (1..=10).contains(&self.iteration),
            "Invalid generation task identity",
        )?;
        require(
            (1..=8).contains(&self.requested_rows)
                && self
                    .first_row
                    .checked_add(self.requested_rows)
                    .is_some_and(|v| v <= 5000),
            "Invalid generation task row range",
        )?;
        require(
            !self.template_row_id.is_empty() && self.template_row_id.len() <= 128,
            "Invalid generation template membership",
        )?;
        for value in [&self.proposal_fingerprint, &self.template_fingerprint] {
            require(
                value.strip_prefix("sha256:").is_some_and(|v| {
                    v.len() == 64
                        && v.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }),
                "Invalid generation input fingerprint",
            )?;
        }
        self.request.validate().map_err(|_| {
            OptimizationError::Validation("Invalid task generation prompt or output ceiling".into())
        })?;
        if let Some(execution) = &self.execution_v3 {
            execution.validate(self)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationReservation {
    pub id: Uuid,
    pub task_id: Uuid,
    pub task_fingerprint: String,
    pub attempt: u32,
    pub input_token_ceiling: u64,
    pub output_token_ceiling: u64,
    pub cost_ceiling_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectedGenerationRow {
    pub index: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdmittedGenerationRow {
    pub index: u32,
    pub fingerprint: String,
    /// Task-owned normalized input identity, independent of generated row IDs.
    pub deduplication_fingerprint: String,
    pub content: Value,
}

/// Stable cross-slice identity for semantic evidence about one structurally
/// admitted generated row. It contains no model-authored content.
pub fn generated_semantic_row_id(task: &GenerationTask, row_index: u32) -> String {
    format!("generation:{}:{row_index}", task.id)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationAdmission {
    pub accepted: Vec<AdmittedGenerationRow>,
    pub rejected: Vec<RejectedGenerationRow>,
}

impl GenerationAdmission {
    pub fn validate(&self, task: &GenerationTask) -> Result<(), OptimizationError> {
        let mut indexes = std::collections::BTreeSet::new();
        for row in &self.accepted {
            require(
                row.deduplication_fingerprint
                    .strip_prefix("sha256:")
                    .is_some_and(|v| {
                        v.len() == 64
                            && v.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    }),
                "Generated normalized input identity is invalid",
            )?;
            require(
                row.index < task.requested_rows && indexes.insert(row.index),
                "Generation admission repeats or invents a row",
            )?;
            require(
                fingerprint(&row.content)? == row.fingerprint && row.content.is_object(),
                "Generated row fingerprint is invalid",
            )?;
        }
        for row in &self.rejected {
            require(
                row.index < task.requested_rows && indexes.insert(row.index),
                "Generation rejection repeats or invents a row",
            )?;
            require(
                !row.reason.trim().is_empty() && row.reason.len() <= 400,
                "Invalid generated row rejection reason",
            )?;
        }
        require(
            indexes.len() == task.requested_rows as usize,
            "Generation admission is incomplete",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= 8_388_608,
            "Generation admission exceeds 8 MiB",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationOutcome {
    pub reservation: GenerationReservation,
    pub usage: AgentTokenUsage,
    pub admission: Option<GenerationAdmission>,
    /// Unknown remote outcome or interrupted local admission; never zero usage.
    pub interrupted: bool,
}

impl GenerationOutcome {
    pub fn validate(&self, task: &GenerationTask) -> Result<(), OptimizationError> {
        require(
            self.interrupted
                || !self.usage.exceeds(
                    self.reservation.input_token_ceiling,
                    self.reservation.output_token_ceiling,
                    self.reservation.cost_ceiling_microusd,
                ),
            "A generation overrun cannot admit rows",
        )?;
        require(
            self.reservation.task_id == task.id
                && self.reservation.task_fingerprint == task.fingerprint()?
                && !self.reservation.id.is_nil()
                && self.reservation.attempt > 0,
            "Generation outcome has another task reservation",
        )?;
        require(
            self.interrupted == self.admission.is_none(),
            "Generation outcome is neither admitted nor interrupted",
        )?;
        if let Some(admission) = &self.admission {
            admission.validate(task)?;
        }
        Ok(())
    }
}
