use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ArchitectError, brief::ResolvedArchitectBrief, fingerprint, required};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectUsage {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub allocation_previews: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microusd: u64,
}

impl std::ops::Add for ArchitectUsage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            model_turns: self.model_turns.saturating_add(other.model_turns),
            tool_calls: self.tool_calls.saturating_add(other.tool_calls),
            allocation_previews: self
                .allocation_previews
                .saturating_add(other.allocation_previews),
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cost_microusd: self.cost_microusd.saturating_add(other.cost_microusd),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectRunState {
    Queued,
    Running,
    AwaitingReview,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectStopReason {
    ProposalSubmitted,
    BudgetExhausted,
    Cancelled,
    ProviderFailure,
    ValidationFailure,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectRun {
    pub id: Uuid,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub protocol_version: u32,
    pub protocol_fingerprint: String,
    pub state: ArchitectRunState,
    pub usage: ArchitectUsage,
    pub cancel_requested: bool,
    pub stop_reason: Option<ArchitectStopReason>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ArchitectRun {
    pub fn queue(
        brief: &ResolvedArchitectBrief,
        protocol_version: u32,
        protocol_fingerprint: String,
    ) -> Result<Self, ArchitectError> {
        if brief.reproduce_fingerprint()? != brief.fingerprint || protocol_version == 0 {
            return Err(ArchitectError::Integrity(
                "cannot queue from an invalid brief or protocol".into(),
            ));
        }
        let now = Utc::now();
        let mut value = Self {
            id: Uuid::new_v4(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            protocol_version,
            protocol_fingerprint: required(protocol_fingerprint, "protocol fingerprint")?,
            state: ArchitectRunState::Queued,
            usage: ArchitectUsage::default(),
            cancel_requested: false,
            stop_reason: None,
            error_message: None,
            created_at: now,
            updated_at: now,
            fingerprint: String::new(),
        };
        value.refresh_fingerprint()?;
        Ok(value)
    }

    pub fn start(&mut self) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Queued || self.cancel_requested {
            return Err(ArchitectError::Validation(
                "only a non-cancelled queued architect run may start".into(),
            ));
        }
        self.state = ArchitectRunState::Running;
        self.touch()
    }

    pub fn record_usage(
        &mut self,
        brief: &ResolvedArchitectBrief,
        delta: ArchitectUsage,
    ) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Running || self.cancel_requested {
            return Err(ArchitectError::Validation(
                "usage can be recorded only for a live architect run".into(),
            ));
        }
        let candidate = self.usage + delta;
        let budget = &brief.budgets;
        let exceeded = candidate.model_turns > budget.max_model_turns
            || candidate.tool_calls > budget.max_tool_calls
            || candidate.allocation_previews > budget.max_allocation_previews
            || candidate.input_tokens > budget.max_input_tokens
            || candidate.output_tokens > budget.max_output_tokens
            || candidate.cost_microusd > budget.max_cost_microusd;
        if exceeded {
            return Err(ArchitectError::BudgetExhausted(
                "architect usage would exceed a persisted hard limit".into(),
            ));
        }
        self.usage = candidate;
        self.touch()
    }

    pub fn request_cancel(&mut self) -> Result<(), ArchitectError> {
        if !matches!(
            self.state,
            ArchitectRunState::Queued | ArchitectRunState::Running
        ) {
            return Err(ArchitectError::Validation(
                "terminal architect run cannot be cancelled".into(),
            ));
        }
        self.cancel_requested = true;
        self.touch()
    }

    pub fn await_review(&mut self, reason: ArchitectStopReason) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Running
            || !matches!(
                reason,
                ArchitectStopReason::ProposalSubmitted | ArchitectStopReason::BudgetExhausted
            )
        {
            return Err(ArchitectError::Validation(
                "awaiting review requires a running run and a reviewable stop reason".into(),
            ));
        }
        self.state = ArchitectRunState::AwaitingReview;
        self.stop_reason = Some(reason);
        self.touch()
    }

    pub fn cancel(&mut self) -> Result<(), ArchitectError> {
        if !matches!(
            self.state,
            ArchitectRunState::Queued | ArchitectRunState::Running
        ) {
            return Err(ArchitectError::Validation(
                "only queued or running architect runs can be cancelled".into(),
            ));
        }
        self.cancel_requested = true;
        self.state = ArchitectRunState::Cancelled;
        self.stop_reason = Some(ArchitectStopReason::Cancelled);
        self.touch()
    }

    pub fn fail(
        &mut self,
        reason: ArchitectStopReason,
        message: String,
    ) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Running
            || matches!(
                reason,
                ArchitectStopReason::ProposalSubmitted | ArchitectStopReason::Cancelled
            )
        {
            return Err(ArchitectError::Validation(
                "invalid architect failure transition".into(),
            ));
        }
        self.state = ArchitectRunState::Failed;
        self.stop_reason = Some(reason);
        self.error_message = Some(required(message, "failure message")?);
        self.touch()
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn touch(&mut self) -> Result<(), ArchitectError> {
        self.updated_at = Utc::now();
        self.refresh_fingerprint()
    }

    fn refresh_fingerprint(&mut self) -> Result<(), ArchitectError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::brief::tests::brief;

    use super::*;

    #[test]
    fn every_hard_usage_limit_is_checked_before_commit() {
        let brief = brief();
        let limits = [
            ArchitectUsage {
                model_turns: brief.budgets.max_model_turns + 1,
                ..ArchitectUsage::default()
            },
            ArchitectUsage {
                tool_calls: brief.budgets.max_tool_calls + 1,
                ..ArchitectUsage::default()
            },
            ArchitectUsage {
                allocation_previews: brief.budgets.max_allocation_previews + 1,
                ..ArchitectUsage::default()
            },
            ArchitectUsage {
                input_tokens: brief.budgets.max_input_tokens + 1,
                ..ArchitectUsage::default()
            },
            ArchitectUsage {
                output_tokens: brief.budgets.max_output_tokens + 1,
                ..ArchitectUsage::default()
            },
            ArchitectUsage {
                cost_microusd: brief.budgets.max_cost_microusd + 1,
                ..ArchitectUsage::default()
            },
        ];
        for usage in limits {
            let mut run = ArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
            run.start().unwrap();
            let before = run.clone();
            assert!(matches!(
                run.record_usage(&brief, usage),
                Err(ArchitectError::BudgetExhausted(_))
            ));
            assert_eq!(run, before);
        }
    }
}
