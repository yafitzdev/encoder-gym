use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    BenchmarkArchitectError, brief::ResolvedBenchmarkArchitectBrief, fingerprint, required,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArchitectUsage {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub searches: u32,
    pub fetched_pages: u32,
    pub fetched_bytes: u64,
    pub blueprint_previews: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microusd: u64,
}

impl BenchmarkArchitectUsage {
    pub fn checked_add(self, other: Self) -> Result<Self, BenchmarkArchitectError> {
        macro_rules! add {
            ($field:ident) => {
                self.$field.checked_add(other.$field).ok_or_else(|| {
                    BenchmarkArchitectError::BudgetExhausted(format!(
                        "{} usage overflowed",
                        stringify!($field)
                    ))
                })?
            };
        }
        Ok(Self {
            model_turns: add!(model_turns),
            tool_calls: add!(tool_calls),
            searches: add!(searches),
            fetched_pages: add!(fetched_pages),
            fetched_bytes: add!(fetched_bytes),
            blueprint_previews: add!(blueprint_previews),
            input_tokens: add!(input_tokens),
            output_tokens: add!(output_tokens),
            cost_microusd: add!(cost_microusd),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkArchitectRunState {
    Queued,
    Running,
    AwaitingReview,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkArchitectStopReason {
    ProposalSubmitted,
    BudgetExhausted,
    Cancelled,
    ProviderFailure,
    ValidationFailure,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArchitectRun {
    pub id: Uuid,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub protocol_version: u32,
    pub protocol_fingerprint: String,
    pub state: BenchmarkArchitectRunState,
    pub plan: Vec<String>,
    pub usage: BenchmarkArchitectUsage,
    pub cancel_requested: bool,
    pub stop_reason: Option<BenchmarkArchitectStopReason>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub specification_fingerprint: String,
}

impl BenchmarkArchitectRun {
    pub fn queue(
        brief: &ResolvedBenchmarkArchitectBrief,
        protocol_version: u32,
        protocol_fingerprint: String,
    ) -> Result<Self, BenchmarkArchitectError> {
        if brief.reproduce_fingerprint()? != brief.fingerprint || protocol_version == 0 {
            return Err(BenchmarkArchitectError::Integrity(
                "cannot queue from an invalid brief or protocol".into(),
            ));
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            protocol_version,
            protocol_fingerprint: required(protocol_fingerprint, "protocol_fingerprint")?,
            state: BenchmarkArchitectRunState::Queued,
            plan: vec![],
            usage: BenchmarkArchitectUsage::default(),
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

    pub fn start(&mut self) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectRunState::Queued || self.cancel_requested {
            return Err(BenchmarkArchitectError::Validation(
                "only a non-cancelled queued run may start".into(),
            ));
        }
        self.state = BenchmarkArchitectRunState::Running;
        self.started_at = Some(Utc::now());
        Ok(())
    }

    pub fn set_plan(&mut self, plan: Vec<String>) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectRunState::Running || !self.plan.is_empty() {
            return Err(BenchmarkArchitectError::Validation(
                "a plan can be recorded once on a running run".into(),
            ));
        }
        let plan = plan
            .into_iter()
            .map(|step| required(step, "plan step"))
            .collect::<Result<Vec<_>, _>>()?;
        if plan.is_empty() {
            return Err(BenchmarkArchitectError::Validation(
                "a plan requires at least one step".into(),
            ));
        }
        self.plan = plan;
        Ok(())
    }

    pub fn record_usage(
        &mut self,
        brief: &ResolvedBenchmarkArchitectBrief,
        delta: BenchmarkArchitectUsage,
    ) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectRunState::Running || self.cancel_requested {
            return Err(BenchmarkArchitectError::Validation(
                "usage requires a live run".into(),
            ));
        }
        let candidate = self.usage.checked_add(delta)?;
        let budget = &brief.budgets;
        if candidate.model_turns > budget.max_model_turns
            || candidate.tool_calls > budget.max_tool_calls
            || candidate.searches > budget.max_searches
            || candidate.fetched_pages > budget.max_fetched_pages
            || candidate.fetched_bytes > budget.max_fetched_bytes
            || candidate.blueprint_previews > budget.max_blueprint_previews
            || candidate.input_tokens > budget.max_input_tokens
            || candidate.output_tokens > budget.max_output_tokens
            || candidate.cost_microusd > budget.max_cost_microusd
        {
            return Err(BenchmarkArchitectError::BudgetExhausted(
                "usage would exceed a persisted hard limit".into(),
            ));
        }
        self.usage = candidate;
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), BenchmarkArchitectError> {
        if !matches!(
            self.state,
            BenchmarkArchitectRunState::Queued | BenchmarkArchitectRunState::Running
        ) {
            return Err(BenchmarkArchitectError::Validation(
                "terminal run cannot request cancellation".into(),
            ));
        }
        self.cancel_requested = true;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), BenchmarkArchitectError> {
        if !matches!(
            self.state,
            BenchmarkArchitectRunState::Queued | BenchmarkArchitectRunState::Running
        ) {
            return Err(BenchmarkArchitectError::Validation(
                "only a queued or running run can be cancelled".into(),
            ));
        }
        self.cancel_requested = true;
        self.state = BenchmarkArchitectRunState::Cancelled;
        self.stop_reason = Some(BenchmarkArchitectStopReason::Cancelled);
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn await_review(
        &mut self,
        reason: BenchmarkArchitectStopReason,
    ) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectRunState::Running
            || !matches!(
                reason,
                BenchmarkArchitectStopReason::ProposalSubmitted
                    | BenchmarkArchitectStopReason::BudgetExhausted
            )
        {
            return Err(BenchmarkArchitectError::Validation(
                "awaiting review requires a live run and reviewable reason".into(),
            ));
        }
        self.state = BenchmarkArchitectRunState::AwaitingReview;
        self.stop_reason = Some(reason);
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn fail(
        &mut self,
        reason: BenchmarkArchitectStopReason,
        message: String,
    ) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectRunState::Running
            || matches!(
                reason,
                BenchmarkArchitectStopReason::ProposalSubmitted
                    | BenchmarkArchitectStopReason::Cancelled
            )
        {
            return Err(BenchmarkArchitectError::Validation(
                "invalid failure transition".into(),
            ));
        }
        self.state = BenchmarkArchitectRunState::Failed;
        self.stop_reason = Some(reason);
        self.error_message = Some(required(message, "failure message")?);
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        fingerprint(&(
            self.id,
            self.brief_id,
            self.brief_fingerprint.as_str(),
            self.protocol_version,
            self.protocol_fingerprint.as_str(),
            self.created_at,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkArchitectToolKind {
    InspectBrief,
    InspectExistingBenchmark,
    InspectExposureHistory,
    SearchWeb,
    FetchPage,
    RecordEvidence,
    InspectEvidence,
    PreviewBlueprint,
    SubmitProposal,
    FinishArchitecture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkArchitectToolCallState {
    Started,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArchitectToolCall {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u32,
    pub kind: BenchmarkArchitectToolKind,
    pub state: BenchmarkArchitectToolCallState,
    pub request: Value,
    pub response: Option<Value>,
    pub usage: BenchmarkArchitectUsage,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl BenchmarkArchitectToolCall {
    pub fn start(
        run: &BenchmarkArchitectRun,
        sequence: u32,
        kind: BenchmarkArchitectToolKind,
        request: Value,
    ) -> Result<Self, BenchmarkArchitectError> {
        if run.state != BenchmarkArchitectRunState::Running || run.cancel_requested || sequence == 0
        {
            return Err(BenchmarkArchitectError::Validation(
                "new tool calls require a live run and positive sequence".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            run_id: run.id,
            sequence,
            kind,
            state: BenchmarkArchitectToolCallState::Started,
            request,
            response: None,
            usage: BenchmarkArchitectUsage::default(),
            error_message: None,
            started_at: Utc::now(),
            finished_at: None,
        })
    }

    pub fn succeed(
        &mut self,
        response: Value,
        usage: BenchmarkArchitectUsage,
    ) -> Result<(), BenchmarkArchitectError> {
        self.finish(
            BenchmarkArchitectToolCallState::Succeeded,
            Some(response),
            usage,
            None,
        )
    }

    pub fn fail(
        &mut self,
        message: String,
        usage: BenchmarkArchitectUsage,
    ) -> Result<(), BenchmarkArchitectError> {
        self.finish(
            BenchmarkArchitectToolCallState::Failed,
            None,
            usage,
            Some(required(message, "tool error")?),
        )
    }

    pub fn interrupt(&mut self) -> Result<(), BenchmarkArchitectError> {
        self.finish(
            BenchmarkArchitectToolCallState::Interrupted,
            None,
            BenchmarkArchitectUsage::default(),
            None,
        )
    }

    fn finish(
        &mut self,
        state: BenchmarkArchitectToolCallState,
        response: Option<Value>,
        usage: BenchmarkArchitectUsage,
        error_message: Option<String>,
    ) -> Result<(), BenchmarkArchitectError> {
        if self.state != BenchmarkArchitectToolCallState::Started {
            return Err(BenchmarkArchitectError::Validation(
                "a terminal tool call cannot change".into(),
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
    fn usage_is_rejected_before_any_hard_limit_changes() {
        let brief = brief();
        let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let before = run.usage;
        let result = run.record_usage(
            &brief,
            BenchmarkArchitectUsage {
                searches: brief.budgets.max_searches + 1,
                ..BenchmarkArchitectUsage::default()
            },
        );
        assert!(matches!(
            result,
            Err(BenchmarkArchitectError::BudgetExhausted(_))
        ));
        assert_eq!(run.usage, before);
    }
}
