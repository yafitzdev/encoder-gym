//! Finite run lifecycle, durable external-call intent, and budget accounting.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{SupervisorError, contract::GenerationQualityContract, fingerprint, required};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorRunState {
    Queued,
    Running,
    Generating,
    Assessing,
    Paused,
    Diagnosing,
    AwaitingReview,
    Canary,
    Completed,
    Failed,
    Cancelled,
}

impl SupervisorRunState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorRun {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_fingerprint: String,
    pub initial_prompt_version_id: Uuid,
    pub initial_prompt_version_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SupervisorRun {
    pub fn create(
        id: Uuid,
        contract: &GenerationQualityContract,
        initial_prompt_version_id: Uuid,
        initial_prompt_version_fingerprint: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        contract.validate()?;
        let initial_prompt_version_fingerprint = initial_prompt_version_fingerprint.into();
        if id.is_nil()
            || initial_prompt_version_id.is_nil()
            || initial_prompt_version_fingerprint.trim().is_empty()
        {
            return Err(SupervisorError::Validation(
                "run and initial prompt identities are required".into(),
            ));
        }
        let mut value = Self {
            id,
            contract_id: contract.id,
            contract_fingerprint: contract.fingerprint.clone(),
            initial_prompt_version_id,
            initial_prompt_version_fingerprint,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorRunEvent {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_event_fingerprint: Option<String>,
    pub from_state: SupervisorRunState,
    pub to_state: SupervisorRunState,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SupervisorRunEvent {
    pub fn transition(
        id: Uuid,
        run_id: Uuid,
        previous: Option<&Self>,
        from_state: SupervisorRunState,
        to_state: SupervisorRunState,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || run_id.is_nil() || from_state.is_terminal() {
            return Err(SupervisorError::InvalidTransition(
                "terminal runs cannot transition".into(),
            ));
        }
        if !allowed_transition(from_state, to_state) {
            return Err(SupervisorError::InvalidTransition(format!(
                "transition {from_state:?} -> {to_state:?} is not allowed"
            )));
        }
        let sequence = match previous {
            Some(previous) => {
                if previous.run_id != run_id
                    || previous.to_state != from_state
                    || previous.fingerprint.is_empty()
                    || previous.reproduce_fingerprint()? != previous.fingerprint
                {
                    return Err(SupervisorError::Integrity(
                        "run event does not extend the exact append-only chain".into(),
                    ));
                }
                previous.sequence.checked_add(1).ok_or_else(|| {
                    SupervisorError::Validation("run event sequence overflow".into())
                })?
            }
            None => {
                if from_state != SupervisorRunState::Queued {
                    return Err(SupervisorError::InvalidTransition(
                        "the first run transition must begin in queued state".into(),
                    ));
                }
                0
            }
        };
        let mut value = Self {
            id,
            run_id,
            sequence,
            previous_event_fingerprint: previous.map(|value| value.fingerprint.clone()),
            from_state,
            to_state,
            reason: required(reason, "run transition reason")?,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn verify_chain(
        run_id: Uuid,
        events: &[Self],
    ) -> Result<SupervisorRunState, SupervisorError> {
        if events.is_empty() {
            return Ok(SupervisorRunState::Queued);
        }
        let mut expected_state = SupervisorRunState::Queued;
        let mut previous: Option<&Self> = None;
        for (index, event) in events.iter().enumerate() {
            if event.run_id != run_id
                || event.sequence != u32::try_from(index).unwrap_or(u32::MAX)
                || event.from_state != expected_state
                || event.previous_event_fingerprint
                    != previous.map(|value| value.fingerprint.clone())
                || event.reproduce_fingerprint()? != event.fingerprint
                || !allowed_transition(event.from_state, event.to_state)
            {
                return Err(SupervisorError::Integrity(
                    "supervisor run event chain does not reproduce".into(),
                ));
            }
            expected_state = event.to_state;
            previous = Some(event);
        }
        Ok(expected_state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildKind {
    GenerationSegment,
    QualityAudit,
    PiModelTurn,
    PiToolCall,
    RevisionCanary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildOutcomeState {
    Succeeded,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildReservation {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: ChildKind,
    pub logical_input_key: String,
    pub child_id: Uuid,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_reservation_id: Option<Uuid>,
    pub input_fingerprint: String,
    pub reserved_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ChildReservation {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        run_id: Uuid,
        kind: ChildKind,
        logical_input_key: impl Into<String>,
        child_id: Uuid,
        attempt: u32,
        replaces_reservation_id: Option<Uuid>,
        input_fingerprint: impl Into<String>,
        reserved_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if [id, run_id, child_id].contains(&Uuid::nil()) || attempt == 0 {
            return Err(SupervisorError::Validation(
                "child reservation identities and attempt must be positive".into(),
            ));
        }
        let mut value = Self {
            id,
            run_id,
            kind,
            logical_input_key: required(logical_input_key, "child logical input key")?,
            child_id,
            attempt,
            replaces_reservation_id,
            input_fingerprint: required(input_fingerprint, "child input fingerprint")?,
            reserved_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildOutcome {
    pub id: Uuid,
    pub reservation_id: Uuid,
    pub reservation_fingerprint: String,
    pub state: ChildOutcomeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub finished_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ChildOutcome {
    pub fn record(
        id: Uuid,
        reservation: &ChildReservation,
        state: ChildOutcomeState,
        output: Option<(Uuid, String)>,
        error: Option<String>,
        finished_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || reservation.reproduce_fingerprint()? != reservation.fingerprint {
            return Err(SupervisorError::Integrity(
                "child outcome requires its exact durable reservation".into(),
            ));
        }
        if state == ChildOutcomeState::Succeeded && output.is_none() {
            return Err(SupervisorError::Validation(
                "successful child outcome requires output provenance".into(),
            ));
        }
        if state != ChildOutcomeState::Succeeded && error.as_deref().is_none_or(str::is_empty) {
            return Err(SupervisorError::Validation(
                "non-successful child outcome requires an explicit reason".into(),
            ));
        }
        let (output_id, output_fingerprint) = output.unzip();
        let mut value = Self {
            id,
            reservation_id: reservation.id,
            reservation_fingerprint: reservation.fingerprint.clone(),
            state,
            output_id,
            output_fingerprint,
            error,
            finished_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorUsage {
    pub generation_segments: u32,
    pub generated_rows: u64,
    pub quality_audits: u32,
    pub evaluator_requests: u32,
    pub evaluator_attempts: u32,
    pub prompt_revisions: u32,
    pub revision_canaries: u32,
    pub pi_model_turns: u32,
    pub pi_tool_calls: u32,
    pub pi_input_tokens: u64,
    pub pi_output_tokens: u64,
    pub external_retries: u32,
    pub elapsed_seconds: u64,
    pub cost_microunits: u64,
}

impl SupervisorUsage {
    pub fn checked_add(&self, delta: &Self) -> Result<Self, SupervisorError> {
        macro_rules! add {
            ($field:ident) => {
                self.$field.checked_add(delta.$field).ok_or_else(|| {
                    SupervisorError::BudgetExhausted(
                        concat!(stringify!($field), " counter overflow").into(),
                    )
                })?
            };
        }
        Ok(Self {
            generation_segments: add!(generation_segments),
            generated_rows: add!(generated_rows),
            quality_audits: add!(quality_audits),
            evaluator_requests: add!(evaluator_requests),
            evaluator_attempts: add!(evaluator_attempts),
            prompt_revisions: add!(prompt_revisions),
            revision_canaries: add!(revision_canaries),
            pi_model_turns: add!(pi_model_turns),
            pi_tool_calls: add!(pi_tool_calls),
            pi_input_tokens: add!(pi_input_tokens),
            pi_output_tokens: add!(pi_output_tokens),
            external_retries: add!(external_retries),
            elapsed_seconds: add!(elapsed_seconds),
            cost_microunits: add!(cost_microunits),
        })
    }

    pub fn reserve(
        &self,
        delta: &Self,
        contract: &GenerationQualityContract,
    ) -> Result<Self, SupervisorError> {
        let next = self.checked_add(delta)?;
        let budgets = &contract.budgets;
        let exceeded = next.generation_segments > budgets.maximum_generation_segments
            || next.generated_rows > budgets.maximum_generated_rows
            || next.quality_audits > budgets.maximum_quality_audits
            || next.evaluator_requests > budgets.maximum_evaluator_requests
            || next.evaluator_attempts > budgets.maximum_evaluator_attempts
            || next.prompt_revisions > budgets.maximum_prompt_revisions
            || next.revision_canaries > budgets.maximum_revision_canaries
            || next.pi_model_turns > budgets.maximum_pi_model_turns
            || next.pi_tool_calls > budgets.maximum_pi_tool_calls
            || next.pi_input_tokens > budgets.maximum_pi_input_tokens
            || next.pi_output_tokens > budgets.maximum_pi_output_tokens
            || next.external_retries > budgets.maximum_retries_per_external_call
            || next.elapsed_seconds > budgets.maximum_duration_seconds
            || budgets
                .maximum_cost_microunits
                .is_some_and(|maximum| next.cost_microunits > maximum);
        if exceeded {
            return Err(SupervisorError::BudgetExhausted(
                "reservation exceeds the immutable generation quality contract".into(),
            ));
        }
        Ok(next)
    }
}

fn allowed_transition(from: SupervisorRunState, to: SupervisorRunState) -> bool {
    use SupervisorRunState as State;
    matches!(
        (from, to),
        (State::Queued, State::Running)
            | (State::Queued, State::Cancelled)
            | (State::Running, State::Generating)
            | (State::Running, State::Completed)
            | (State::Running, State::Failed)
            | (State::Running, State::Cancelled)
            | (State::Generating, State::Assessing)
            | (State::Generating, State::Failed)
            | (State::Generating, State::Cancelled)
            | (State::Assessing, State::Running)
            | (State::Assessing, State::Paused)
            | (State::Assessing, State::Failed)
            | (State::Assessing, State::Cancelled)
            | (State::Paused, State::Diagnosing)
            | (State::Paused, State::Running)
            | (State::Paused, State::Failed)
            | (State::Paused, State::Cancelled)
            | (State::Diagnosing, State::AwaitingReview)
            | (State::Diagnosing, State::Paused)
            | (State::Diagnosing, State::Failed)
            | (State::Diagnosing, State::Cancelled)
            | (State::AwaitingReview, State::Canary)
            | (State::AwaitingReview, State::Paused)
            | (State::AwaitingReview, State::Cancelled)
            | (State::Canary, State::Running)
            | (State::Canary, State::Paused)
            | (State::Canary, State::Failed)
            | (State::Canary, State::Cancelled)
    )
}
