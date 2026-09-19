//! Immutable generation slots and outcomes. Native content is opaque here;
//! only the task adapter may admit it, and dataset management owns publication.

use generation_core::structured::StructuredGenerationRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{OptimizationError, agent::AgentTokenUsage, fingerprint, require};

/// Frozen launch policy. The first existing slot is the canary, not an extra
/// provider request. It checks native admission, not semantic correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationCanaryPolicy {
    FirstBatchAllAdmittedV1,
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
        })
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
