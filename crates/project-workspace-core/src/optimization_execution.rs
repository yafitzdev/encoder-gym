//! Execution-attempt lifecycle for the existing bounded Agent coordinator.
//! This journal owns no iteration, provider, training, or acceptance policy.
use crate::{BoundIdentity, Invalid, optimization_loop::IterationCompletion, require};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionState {
    Running,
    StopRequested,
    Paused,
    Interrupted,
    Failed,
    BudgetExhausted,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentExecutionChange {
    Started,
    StopRequested,
    Paused,
    Interrupted,
    Failed,
    BudgetExhausted,
    Completed { completion: BoundIdentity },
}

impl AgentExecutionChange {
    pub fn storage_key(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::StopRequested => "stop_requested",
            Self::Paused => "paused",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Completed { .. } => "completed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExecutionEvent {
    pub id: Uuid,
    pub run: BoundIdentity,
    pub sequence: u64,
    pub previous: Option<String>,
    pub attempt_id: Uuid,
    pub change: AgentExecutionChange,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AgentExecutionEvent {
    pub fn create(
        run: BoundIdentity,
        previous: Option<&Self>,
        attempt_id: Uuid,
        change: AgentExecutionChange,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::create_with_id(
            Uuid::new_v4(),
            run,
            previous,
            attempt_id,
            change,
            created_at,
        )
    }

    /// A control command retains this identity when its response is lost.
    pub fn create_with_id(
        id: Uuid,
        run: BoundIdentity,
        previous: Option<&Self>,
        attempt_id: Uuid,
        change: AgentExecutionChange,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        require(!id.is_nil(), "Execution event needs a non-nil identity")?;
        let mut value = Self {
            id,
            run,
            sequence: previous.map_or(Ok(1), |event| {
                event
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| Invalid("Execution sequence overflow".into()))
            })?,
            previous: previous.map(|event| event.fingerprint.clone()),
            attempt_id,
            change,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        let mut value = serde_json::to_value(self).map_err(invalid)?;
        value.as_object_mut().expect("event").remove("fingerprint");
        artifact_core::fingerprint(&value).map_err(invalid)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExecutionView {
    pub state: AgentExecutionState,
    pub attempt_id: Uuid,
    pub attempts: u32,
    pub completion: Option<BoundIdentity>,
    pub last_sequence: u64,
    pub head_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

/// Completion identities must come from the verified iteration store. Mere
/// execution success cannot manufacture an adaptive outcome or authorize holdout.
pub fn replay(
    run: &BoundIdentity,
    events: &[AgentExecutionEvent],
    completions: &[IterationCompletion],
) -> Result<Option<AgentExecutionView>, Invalid> {
    run.validate("Agent execution run")?;
    require(
        Uuid::parse_str(&run.id).is_ok_and(|id| !id.is_nil()),
        "Execution needs a run UUID",
    )?;
    let mut view: Option<AgentExecutionView> = None;
    let mut event_ids = BTreeSet::new();
    let mut attempts = BTreeSet::new();
    for (index, event) in events.iter().enumerate() {
        require(
            !event.id.is_nil()
                && event_ids.insert(event.id)
                && !event.attempt_id.is_nil()
                && event.run == *run
                && event.sequence == index as u64 + 1
                && event.previous == view.as_ref().map(|v| v.head_fingerprint.clone())
                && view
                    .as_ref()
                    .is_none_or(|v| event.created_at >= v.updated_at)
                && event.fingerprint == event.reproduce()?,
            "Agent execution journal identity, ordering or fingerprint changed",
        )?;
        let prior = view.as_ref();
        let mut completion = None;
        let state = match &event.change {
            AgentExecutionChange::Started => {
                require(
                    prior.is_none_or(|v| {
                        matches!(
                            v.state,
                            AgentExecutionState::Interrupted
                                | AgentExecutionState::Failed
                                | AgentExecutionState::Paused
                        )
                    }) && attempts.insert(event.attempt_id),
                    "Agent execution cannot start while running or after completion",
                )?;
                AgentExecutionState::Running
            }
            AgentExecutionChange::StopRequested => {
                require(
                    prior.is_none_or(|v| {
                        !matches!(
                            v.state,
                            AgentExecutionState::Completed | AgentExecutionState::BudgetExhausted
                        ) && v.attempt_id == event.attempt_id
                    }),
                    "Stop request has no matching unfinished attempt",
                )?;
                AgentExecutionState::StopRequested
            }
            AgentExecutionChange::Paused => {
                require(
                    prior.is_some_and(|v| {
                        v.state == AgentExecutionState::StopRequested
                            && v.attempt_id == event.attempt_id
                    }),
                    "Pause needs the exact stop request",
                )?;
                AgentExecutionState::Paused
            }
            change => {
                require(
                    prior.is_some_and(|v| {
                        v.state == AgentExecutionState::Running && v.attempt_id == event.attempt_id
                    }),
                    "Agent outcome has no matching active attempt",
                )?;
                match change {
                    AgentExecutionChange::Interrupted => AgentExecutionState::Interrupted,
                    AgentExecutionChange::Failed => AgentExecutionState::Failed,
                    AgentExecutionChange::BudgetExhausted => AgentExecutionState::BudgetExhausted,
                    AgentExecutionChange::Completed {
                        completion: selected,
                    } => {
                        let last = completions.last().ok_or_else(|| {
                            Invalid("Agent completion needs a recorded iteration outcome".into())
                        })?;
                        last.validate_identity()?;
                        require(
                            last.run == *run
                                && last.end.is_some()
                                && last.identity() == *selected
                                && event.created_at >= last.created_at,
                            "Agent completion does not match the verified terminal iteration",
                        )?;
                        completion = Some(selected.clone());
                        AgentExecutionState::Completed
                    }
                    AgentExecutionChange::Started
                    | AgentExecutionChange::StopRequested
                    | AgentExecutionChange::Paused => unreachable!(),
                }
            }
        };
        view = Some(AgentExecutionView {
            state,
            attempt_id: event.attempt_id,
            attempts: attempts.len().try_into().map_err(invalid)?,
            completion,
            last_sequence: event.sequence,
            head_fingerprint: event.fingerprint.clone(),
            updated_at: event.created_at,
        });
    }
    Ok(view)
}

fn invalid(error: impl std::fmt::Display) -> Invalid {
    Invalid(error.to_string())
}

#[cfg(test)]
mod tests;
