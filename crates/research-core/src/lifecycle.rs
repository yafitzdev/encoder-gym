use artifact_core::fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{ResearchError, brief::ResolvedResearchBrief, nonempty};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchUsage {
    pub model_turns: u32,
    pub searches: u32,
    pub fetched_pages: u32,
    pub fetched_bytes: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microusd: u64,
}

impl ResearchUsage {
    pub fn checked_add(self, delta: Self) -> Result<Self, ResearchError> {
        Ok(Self {
            model_turns: self
                .model_turns
                .checked_add(delta.model_turns)
                .ok_or_else(|| ResearchError::BudgetExhausted("model_turns overflowed".into()))?,
            searches: self
                .searches
                .checked_add(delta.searches)
                .ok_or_else(|| ResearchError::BudgetExhausted("searches overflowed".into()))?,
            fetched_pages: self
                .fetched_pages
                .checked_add(delta.fetched_pages)
                .ok_or_else(|| ResearchError::BudgetExhausted("fetched_pages overflowed".into()))?,
            fetched_bytes: self
                .fetched_bytes
                .checked_add(delta.fetched_bytes)
                .ok_or_else(|| ResearchError::BudgetExhausted("fetched_bytes overflowed".into()))?,
            input_tokens: self
                .input_tokens
                .checked_add(delta.input_tokens)
                .ok_or_else(|| ResearchError::BudgetExhausted("input_tokens overflowed".into()))?,
            output_tokens: self
                .output_tokens
                .checked_add(delta.output_tokens)
                .ok_or_else(|| ResearchError::BudgetExhausted("output_tokens overflowed".into()))?,
            cost_microusd: self
                .cost_microusd
                .checked_add(delta.cost_microusd)
                .ok_or_else(|| ResearchError::BudgetExhausted("cost_microusd overflowed".into()))?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchRunState {
    Queued,
    Running,
    AwaitingReview,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStopReason {
    SufficientEvidence,
    BudgetExhausted,
    UserCancellation,
    ProviderFailure,
    ValidationFailure,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRun {
    pub id: Uuid,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub protocol_version: u32,
    pub protocol_fingerprint: String,
    pub state: ResearchRunState,
    pub plan: Vec<String>,
    pub usage: ResearchUsage,
    pub cancel_requested: bool,
    pub stop_reason: Option<ResearchStopReason>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub specification_fingerprint: String,
}

impl ResearchRun {
    pub fn queue(
        brief: &ResolvedResearchBrief,
        protocol_version: u32,
        protocol_fingerprint: String,
    ) -> Result<Self, ResearchError> {
        if brief.reproduce_fingerprint()? != brief.fingerprint {
            return Err(ResearchError::Integrity(
                "research brief fingerprint mismatch".into(),
            ));
        }
        if protocol_version == 0 {
            return Err(ResearchError::Validation(
                "protocol_version must be greater than zero".into(),
            ));
        }
        let protocol_fingerprint = nonempty(protocol_fingerprint, "protocol_fingerprint")?;
        let mut run = Self {
            id: Uuid::new_v4(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            protocol_version,
            protocol_fingerprint,
            state: ResearchRunState::Queued,
            plan: Vec::new(),
            usage: ResearchUsage::default(),
            cancel_requested: false,
            stop_reason: None,
            error_message: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            specification_fingerprint: String::new(),
        };
        run.specification_fingerprint = run.reproduce_specification_fingerprint()?;
        Ok(run)
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            self.id,
            self.brief_id,
            &self.brief_fingerprint,
            self.protocol_version,
            &self.protocol_fingerprint,
            self.created_at,
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }

    pub fn start(&mut self) -> Result<(), ResearchError> {
        self.require_state(ResearchRunState::Queued, "start")?;
        self.state = ResearchRunState::Running;
        self.started_at = Some(Utc::now());
        Ok(())
    }

    pub fn set_plan(&mut self, plan: Vec<String>) -> Result<(), ResearchError> {
        self.require_state(ResearchRunState::Running, "set research plan")?;
        let plan = crate::normalized_list(plan, "research plan")?;
        if plan.is_empty() {
            return Err(ResearchError::Validation(
                "research plan must contain at least one step".into(),
            ));
        }
        self.plan = plan;
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), ResearchError> {
        if !matches!(
            self.state,
            ResearchRunState::Queued | ResearchRunState::Running
        ) {
            return Err(ResearchError::InvalidTransition(format!(
                "cannot cancel a {:?} run",
                self.state
            )));
        }
        self.cancel_requested = true;
        Ok(())
    }

    pub fn record_usage(
        &mut self,
        brief: &ResolvedResearchBrief,
        delta: ResearchUsage,
    ) -> Result<(), ResearchError> {
        self.require_state(ResearchRunState::Running, "record usage")?;
        let next = self.usage.checked_add(delta)?;
        enforce_budget(&brief.budgets, next)?;
        self.usage = next;
        Ok(())
    }

    pub fn await_review(&mut self, reason: ResearchStopReason) -> Result<(), ResearchError> {
        self.require_state(ResearchRunState::Running, "complete research")?;
        if self.cancel_requested {
            return Err(ResearchError::InvalidTransition(
                "a cancellation-requested run cannot publish a profile".into(),
            ));
        }
        if !matches!(
            reason,
            ResearchStopReason::SufficientEvidence | ResearchStopReason::BudgetExhausted
        ) {
            return Err(ResearchError::InvalidTransition(
                "awaiting review requires a successful stop reason".into(),
            ));
        }
        self.finish(ResearchRunState::AwaitingReview, reason, None)
    }

    pub fn cancel(&mut self) -> Result<(), ResearchError> {
        if !matches!(
            self.state,
            ResearchRunState::Queued | ResearchRunState::Running
        ) {
            return Err(ResearchError::InvalidTransition(format!(
                "cannot finish cancellation from {:?}",
                self.state
            )));
        }
        self.cancel_requested = true;
        self.finish(
            ResearchRunState::Cancelled,
            ResearchStopReason::UserCancellation,
            None,
        )
    }

    pub fn fail(
        &mut self,
        reason: ResearchStopReason,
        message: String,
    ) -> Result<(), ResearchError> {
        if self.state != ResearchRunState::Running {
            return Err(ResearchError::InvalidTransition(format!(
                "cannot fail a {:?} run",
                self.state
            )));
        }
        if matches!(
            reason,
            ResearchStopReason::SufficientEvidence | ResearchStopReason::UserCancellation
        ) {
            return Err(ResearchError::InvalidTransition(
                "failure requires a failure stop reason".into(),
            ));
        }
        self.finish(
            ResearchRunState::Failed,
            reason,
            Some(nonempty(message, "failure message")?),
        )
    }

    fn finish(
        &mut self,
        state: ResearchRunState,
        reason: ResearchStopReason,
        error_message: Option<String>,
    ) -> Result<(), ResearchError> {
        self.state = state;
        self.stop_reason = Some(reason);
        self.error_message = error_message;
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    fn require_state(&self, expected: ResearchRunState, action: &str) -> Result<(), ResearchError> {
        if self.state != expected {
            return Err(ResearchError::InvalidTransition(format!(
                "cannot {action} from {:?}",
                self.state
            )));
        }
        Ok(())
    }
}

fn enforce_budget(
    budgets: &crate::brief::ResearchBudgets,
    usage: ResearchUsage,
) -> Result<(), ResearchError> {
    let checks = [
        (
            "model turns",
            u64::from(usage.model_turns),
            u64::from(budgets.max_model_turns),
        ),
        (
            "searches",
            u64::from(usage.searches),
            u64::from(budgets.max_searches),
        ),
        (
            "fetched pages",
            u64::from(usage.fetched_pages),
            u64::from(budgets.max_fetched_pages),
        ),
        (
            "fetched bytes",
            usage.fetched_bytes,
            budgets.max_fetched_bytes,
        ),
        ("input tokens", usage.input_tokens, budgets.max_input_tokens),
        (
            "output tokens",
            usage.output_tokens,
            budgets.max_output_tokens,
        ),
        ("cost", usage.cost_microusd, budgets.max_cost_microusd),
    ];
    if let Some((name, actual, maximum)) = checks
        .into_iter()
        .find(|(_, actual, maximum)| actual > maximum)
    {
        return Err(ResearchError::BudgetExhausted(format!(
            "{name} would become {actual}, above the persisted limit {maximum}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchToolKind {
    SearchWeb,
    FetchPage,
    RecordEvidence,
    InspectEvidence,
    DraftProfile,
    FinishResearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallState {
    Started,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchToolCall {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u32,
    pub kind: ResearchToolKind,
    pub state: ToolCallState,
    pub request: Value,
    pub response: Option<Value>,
    pub usage: ResearchUsage,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ResearchToolCall {
    pub fn start(
        run: &ResearchRun,
        sequence: u32,
        kind: ResearchToolKind,
        request: Value,
    ) -> Result<Self, ResearchError> {
        if run.state != ResearchRunState::Running || run.cancel_requested {
            return Err(ResearchError::InvalidTransition(
                "new tool calls require a running, non-cancelled run".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            run_id: run.id,
            sequence,
            kind,
            state: ToolCallState::Started,
            request,
            response: None,
            usage: ResearchUsage::default(),
            error_message: None,
            started_at: Utc::now(),
            finished_at: None,
        })
    }

    pub fn succeed(&mut self, response: Value, usage: ResearchUsage) -> Result<(), ResearchError> {
        self.finish(ToolCallState::Succeeded, Some(response), usage, None)
    }

    pub fn fail(&mut self, message: String) -> Result<(), ResearchError> {
        self.finish(
            ToolCallState::Failed,
            None,
            ResearchUsage::default(),
            Some(nonempty(message, "tool error")?),
        )
    }

    pub fn interrupt(&mut self) -> Result<(), ResearchError> {
        self.finish(
            ToolCallState::Interrupted,
            None,
            ResearchUsage::default(),
            None,
        )
    }

    fn finish(
        &mut self,
        state: ToolCallState,
        response: Option<Value>,
        usage: ResearchUsage,
        error_message: Option<String>,
    ) -> Result<(), ResearchError> {
        if self.state != ToolCallState::Started {
            return Err(ResearchError::InvalidTransition(
                "a terminal tool call cannot be changed".into(),
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
    use std::collections::BTreeMap;

    use crate::brief::*;

    use super::*;

    fn brief() -> ResolvedResearchBrief {
        ResolvedResearchBrief::create(ResearchBriefDraft {
            schema_version: 1,
            dataset: ArtifactReference {
                id: Uuid::new_v4(),
                fingerprint: "sha256:d".into(),
            },
            task: "task".into(),
            labels: vec!["a".into()],
            dimensions: BTreeMap::new(),
            semantic_context: None,
            target: ResearchTarget {
                language: "English".into(),
                ..ResearchTarget::default()
            },
            questions: vec!["question".into()],
            desired_source_diversity: 1,
            source_policy: SourcePolicy::default(),
            budgets: ResearchBudgets {
                max_model_turns: 2,
                max_searches: 1,
                max_fetched_pages: 1,
                max_fetched_bytes: 100,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_wall_clock_seconds: 60,
                max_cost_microusd: 100,
                max_retries_per_call: 0,
            },
            provider: ResearchProviderConfiguration {
                runtime: "pi".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
            },
            required_profile_sections: vec!["language".into()],
        })
        .unwrap()
    }

    #[test]
    fn state_machine_requires_explicit_success_or_failure() {
        let brief = brief();
        let mut run = ResearchRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        assert!(
            run.await_review(ResearchStopReason::SufficientEvidence)
                .is_err()
        );
        run.start().unwrap();
        run.set_plan(vec!["search".into()]).unwrap();
        run.await_review(ResearchStopReason::SufficientEvidence)
            .unwrap();
        assert_eq!(run.state, ResearchRunState::AwaitingReview);
    }

    #[test]
    fn usage_above_persisted_limit_never_commits() {
        let brief = brief();
        let mut run = ResearchRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let result = run.record_usage(
            &brief,
            ResearchUsage {
                searches: 2,
                ..ResearchUsage::default()
            },
        );
        assert!(matches!(result, Err(ResearchError::BudgetExhausted(_))));
        assert_eq!(run.usage.searches, 0);
    }

    #[test]
    fn cancellation_prevents_new_tool_calls() {
        let brief = brief();
        let mut run = ResearchRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        run.request_cancel().unwrap();
        assert!(
            ResearchToolCall::start(&run, 1, ResearchToolKind::SearchWeb, serde_json::json!({}))
                .is_err()
        );
    }
}
