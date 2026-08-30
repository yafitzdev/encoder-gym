use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

impl ArchitectUsage {
    pub fn checked_add(self, other: Self) -> Result<Self, ArchitectError> {
        Ok(Self {
            model_turns: self
                .model_turns
                .checked_add(other.model_turns)
                .ok_or_else(|| ArchitectError::BudgetExhausted("model turns overflowed".into()))?,
            tool_calls: self
                .tool_calls
                .checked_add(other.tool_calls)
                .ok_or_else(|| ArchitectError::BudgetExhausted("tool calls overflowed".into()))?,
            allocation_previews: self
                .allocation_previews
                .checked_add(other.allocation_previews)
                .ok_or_else(|| {
                    ArchitectError::BudgetExhausted("allocation previews overflowed".into())
                })?,
            input_tokens: self
                .input_tokens
                .checked_add(other.input_tokens)
                .ok_or_else(|| ArchitectError::BudgetExhausted("input tokens overflowed".into()))?,
            output_tokens: self
                .output_tokens
                .checked_add(other.output_tokens)
                .ok_or_else(|| {
                    ArchitectError::BudgetExhausted("output tokens overflowed".into())
                })?,
            cost_microusd: self
                .cost_microusd
                .checked_add(other.cost_microusd)
                .ok_or_else(|| ArchitectError::BudgetExhausted("cost overflowed".into()))?,
        })
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
    pub plan: Vec<String>,
    pub usage: ArchitectUsage,
    pub cancel_requested: bool,
    pub stop_reason: Option<ArchitectStopReason>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub specification_fingerprint: String,
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
        let mut value = Self {
            id: Uuid::new_v4(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            protocol_version,
            protocol_fingerprint: required(protocol_fingerprint, "protocol fingerprint")?,
            state: ArchitectRunState::Queued,
            plan: vec![],
            usage: ArchitectUsage::default(),
            cancel_requested: false,
            stop_reason: None,
            error_message: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            specification_fingerprint: String::new(),
        };
        value.specification_fingerprint = value.reproduce_specification_fingerprint()?;
        Ok(value)
    }

    pub fn start(&mut self) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Queued || self.cancel_requested {
            return Err(ArchitectError::Validation(
                "only a non-cancelled queued architect run may start".into(),
            ));
        }
        self.state = ArchitectRunState::Running;
        self.started_at = Some(Utc::now());
        Ok(())
    }

    pub fn set_plan(&mut self, plan: Vec<String>) -> Result<(), ArchitectError> {
        if self.state != ArchitectRunState::Running || !self.plan.is_empty() {
            return Err(ArchitectError::Validation(
                "architect plan can be recorded once on a running run".into(),
            ));
        }
        let plan = plan
            .into_iter()
            .map(|step| required(step, "architect plan step"))
            .collect::<Result<Vec<_>, _>>()?;
        if plan.is_empty() {
            return Err(ArchitectError::Validation(
                "architect plan requires at least one step".into(),
            ));
        }
        self.plan = plan;
        Ok(())
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
        let candidate = self.usage.checked_add(delta)?;
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
        Ok(())
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
        Ok(())
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
        self.finished_at = Some(Utc::now());
        Ok(())
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
        self.finished_at = Some(Utc::now());
        Ok(())
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
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, ArchitectError> {
        fingerprint(&(
            self.id,
            self.brief_id,
            &self.brief_fingerprint,
            self.protocol_version,
            &self.protocol_fingerprint,
            self.created_at,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectToolKind {
    InspectDataset,
    InspectSemantics,
    InspectAuthenticity,
    InspectCoverage,
    InspectDevelopmentEvidence,
    PreviewAllocation,
    EstimateCost,
    SubmitProposal,
    FinishArchitecture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectToolCallState {
    Started,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectToolCall {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u32,
    pub kind: ArchitectToolKind,
    pub state: ArchitectToolCallState,
    pub request: Value,
    pub response: Option<Value>,
    pub usage: ArchitectUsage,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ArchitectToolCall {
    pub fn start(
        run: &ArchitectRun,
        sequence: u32,
        kind: ArchitectToolKind,
        request: Value,
    ) -> Result<Self, ArchitectError> {
        if run.state != ArchitectRunState::Running || run.cancel_requested || sequence == 0 {
            return Err(ArchitectError::Validation(
                "new tool calls require a running, non-cancelled run and positive sequence".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            run_id: run.id,
            sequence,
            kind,
            state: ArchitectToolCallState::Started,
            request,
            response: None,
            usage: ArchitectUsage::default(),
            error_message: None,
            started_at: Utc::now(),
            finished_at: None,
        })
    }

    pub fn succeed(
        &mut self,
        response: Value,
        usage: ArchitectUsage,
    ) -> Result<(), ArchitectError> {
        self.finish(
            ArchitectToolCallState::Succeeded,
            Some(response),
            usage,
            None,
        )
    }

    pub fn fail(&mut self, message: String, usage: ArchitectUsage) -> Result<(), ArchitectError> {
        self.finish(
            ArchitectToolCallState::Failed,
            None,
            usage,
            Some(required(message, "tool error")?),
        )
    }

    pub fn interrupt(&mut self) -> Result<(), ArchitectError> {
        self.finish(
            ArchitectToolCallState::Interrupted,
            None,
            ArchitectUsage::default(),
            None,
        )
    }

    fn finish(
        &mut self,
        state: ArchitectToolCallState,
        response: Option<Value>,
        usage: ArchitectUsage,
        error_message: Option<String>,
    ) -> Result<(), ArchitectError> {
        if self.state != ArchitectToolCallState::Started {
            return Err(ArchitectError::Validation(
                "a terminal architect tool call cannot change".into(),
            ));
        }
        self.state = state;
        self.response = response;
        self.usage = usage;
        self.error_message = error_message;
        self.finished_at = Some(Utc::now());
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
            let before = run.usage;
            assert!(matches!(
                run.record_usage(&brief, usage),
                Err(ArchitectError::BudgetExhausted(_))
            ));
            assert_eq!(run.usage, before);
        }
    }
}
