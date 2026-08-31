//! Immutable redacted diagnosis input and durable bounded-advisor facts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    SupervisorError,
    contract::GenerationQualityContract,
    decision::{DeterministicQualityDecision, SupervisorDecisionState},
    fingerprint,
    observation::BatchQualityObservation,
    required,
    revision::{AdvisorUsage, PromptGuidanceVersion},
};

pub const SUPERVISOR_ADVISOR_CAPABILITY_SET_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorConfiguration {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub runtime_protocol_version: String,
    pub configuration_fingerprint: String,
}

impl AdvisorConfiguration {
    pub fn create(
        provider: impl Into<String>,
        model: impl Into<String>,
        api_key_env: Option<String>,
        runtime_protocol_version: impl Into<String>,
        configuration_fingerprint: impl Into<String>,
    ) -> Result<Self, SupervisorError> {
        let api_key_env = api_key_env
            .map(|value| required(value, "advisor API key environment variable"))
            .transpose()?;
        if api_key_env
            .as_deref()
            .is_some_and(|value| !valid_environment_name(value))
        {
            return Err(SupervisorError::Validation(
                "advisor API key must be an environment-variable name, never a secret".into(),
            ));
        }
        Ok(Self {
            provider: required(provider, "advisor provider")?,
            model: required(model, "advisor model")?,
            api_key_env,
            runtime_protocol_version: required(
                runtime_protocol_version,
                "advisor runtime protocol version",
            )?,
            configuration_fingerprint: required(
                configuration_fingerprint,
                "advisor configuration fingerprint",
            )?,
        })
    }
}

/// Pi-safe input: it contains aggregate metrics and prompt guidance, but no
/// member manifest, generated row text, raw source row, or sealed evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationQualityDiagnosisBrief {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub contract: GenerationQualityContract,
    pub decision: DeterministicQualityDecision,
    pub window: BatchQualityObservation,
    pub current_prompt: PromptGuidanceVersion,
    pub advisor: AdvisorConfiguration,
    pub capability_set_version: u32,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationQualityDiagnosisBrief {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        supervisor_run_id: Uuid,
        contract: GenerationQualityContract,
        decision: DeterministicQualityDecision,
        window: BatchQualityObservation,
        current_prompt: PromptGuidanceVersion,
        advisor: AdvisorConfiguration,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        contract.validate()?;
        decision.validate(&contract)?;
        current_prompt.validate()?;
        if id.is_nil()
            || supervisor_run_id.is_nil()
            || decision.supervisor_run_id != supervisor_run_id
            || window.supervisor_run_id != supervisor_run_id
            || current_prompt.supervisor_run_id != supervisor_run_id
            || decision.state != SupervisorDecisionState::PauseForDiagnosis
            || decision.window_id != window.id
            || decision.window_fingerprint != window.fingerprint
            || decision.prompt_version_id != current_prompt.id
            || decision.prompt_version_fingerprint != current_prompt.fingerprint
            || window.reproduce_fingerprint()? != window.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "diagnosis brief is not the exact redacted paused scope".into(),
            ));
        }
        let mut value = Self {
            id,
            supervisor_run_id,
            contract,
            decision,
            window,
            current_prompt,
            advisor,
            capability_set_version: SUPERVISOR_ADVISOR_CAPABILITY_SET_VERSION,
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

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.capability_set_version != SUPERVISOR_ADVISOR_CAPABILITY_SET_VERSION
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "diagnosis brief fingerprint does not reproduce".into(),
            ));
        }
        self.contract.validate()?;
        self.decision.validate(&self.contract)?;
        self.current_prompt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorSessionState {
    Queued,
    Running,
    AwaitingReview,
    Escalated,
    Failed,
    Cancelled,
}

impl AdvisorSessionState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::AwaitingReview | Self::Escalated | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorSession {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub protocol_version: u32,
    pub protocol_fingerprint: String,
    pub state: AdvisorSessionState,
    pub usage: AdvisorUsage,
    #[serde(default)]
    pub plan: Vec<String>,
    pub cancel_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AdvisorSession {
    pub fn queue(
        id: Uuid,
        brief: &GenerationQualityDiagnosisBrief,
        protocol_version: u32,
        protocol_fingerprint: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        brief.validate()?;
        if id.is_nil() || protocol_version == 0 {
            return Err(SupervisorError::Validation(
                "advisor session identity and protocol version are required".into(),
            ));
        }
        let mut value = Self {
            id,
            supervisor_run_id: brief.supervisor_run_id,
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            protocol_version,
            protocol_fingerprint: required(protocol_fingerprint, "advisor protocol fingerprint")?,
            state: AdvisorSessionState::Queued,
            usage: AdvisorUsage::default(),
            plan: vec![],
            cancel_requested: false,
            diagnosis_id: None,
            proposal_id: None,
            stop_reason: None,
            created_at,
            updated_at: created_at,
            fingerprint: String::new(),
        };
        value.refresh_fingerprint()?;
        Ok(value)
    }

    pub fn start(&mut self, now: DateTime<Utc>) -> Result<(), SupervisorError> {
        self.require_state(AdvisorSessionState::Queued)?;
        self.state = AdvisorSessionState::Running;
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn request_cancel(&mut self, now: DateTime<Utc>) -> Result<(), SupervisorError> {
        if self.state.is_terminal() {
            return Err(SupervisorError::InvalidTransition(
                "terminal advisor session cannot request cancellation".into(),
            ));
        }
        self.cancel_requested = true;
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn cancel(&mut self, now: DateTime<Utc>) -> Result<(), SupervisorError> {
        if self.state.is_terminal() {
            return Err(SupervisorError::InvalidTransition(
                "terminal advisor session cannot be cancelled again".into(),
            ));
        }
        self.cancel_requested = true;
        self.state = AdvisorSessionState::Cancelled;
        self.stop_reason = Some("cancelled by persisted request".into());
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn set_plan(
        &mut self,
        plan: Vec<String>,
        now: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        self.require_state(AdvisorSessionState::Running)?;
        let plan = plan
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .take(20)
            .collect::<Vec<_>>();
        if plan.is_empty() {
            return Err(SupervisorError::Validation(
                "advisor plan must contain at least one bounded step".into(),
            ));
        }
        self.plan = plan;
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn record_usage(
        &mut self,
        contract: &GenerationQualityContract,
        delta: AdvisorUsage,
        now: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        self.require_state(AdvisorSessionState::Running)?;
        let next = AdvisorUsage {
            model_turns: self
                .usage
                .model_turns
                .checked_add(delta.model_turns)
                .ok_or_else(|| SupervisorError::BudgetExhausted("model turns overflow".into()))?,
            tool_calls: self
                .usage
                .tool_calls
                .checked_add(delta.tool_calls)
                .ok_or_else(|| SupervisorError::BudgetExhausted("tool calls overflow".into()))?,
            input_tokens: self
                .usage
                .input_tokens
                .checked_add(delta.input_tokens)
                .ok_or_else(|| SupervisorError::BudgetExhausted("input tokens overflow".into()))?,
            output_tokens: self
                .usage
                .output_tokens
                .checked_add(delta.output_tokens)
                .ok_or_else(|| SupervisorError::BudgetExhausted("output tokens overflow".into()))?,
            cost_microunits: match (self.usage.cost_microunits, delta.cost_microunits) {
                (None, None) => None,
                (left, right) => Some(
                    left.unwrap_or(0)
                        .checked_add(right.unwrap_or(0))
                        .ok_or_else(|| {
                            SupervisorError::BudgetExhausted("advisor cost overflow".into())
                        })?,
                ),
            },
        };
        let budgets = &contract.budgets;
        if next.model_turns > budgets.maximum_pi_model_turns
            || next.tool_calls > budgets.maximum_pi_tool_calls
            || next.input_tokens > budgets.maximum_pi_input_tokens
            || next.output_tokens > budgets.maximum_pi_output_tokens
            || budgets
                .maximum_cost_microunits
                .is_some_and(|maximum| next.cost_microunits.unwrap_or(0) > maximum)
        {
            return Err(SupervisorError::BudgetExhausted(
                "advisor usage exceeds the quality contract".into(),
            ));
        }
        self.usage = next;
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn await_review(
        &mut self,
        diagnosis_id: Uuid,
        proposal_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        self.require_state(AdvisorSessionState::Running)?;
        if diagnosis_id.is_nil() || proposal_id.is_nil() {
            return Err(SupervisorError::Validation(
                "advisor finish requires diagnosis and proposal identities".into(),
            ));
        }
        self.diagnosis_id = Some(diagnosis_id);
        self.proposal_id = Some(proposal_id);
        self.state = AdvisorSessionState::AwaitingReview;
        self.stop_reason = Some("valid guidance revision submitted for review".into());
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn escalate(
        &mut self,
        diagnosis_id: Uuid,
        reason: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        self.require_state(AdvisorSessionState::Running)?;
        if diagnosis_id.is_nil() {
            return Err(SupervisorError::Validation(
                "escalation requires diagnosis identity".into(),
            ));
        }
        self.diagnosis_id = Some(diagnosis_id);
        self.state = AdvisorSessionState::Escalated;
        self.stop_reason = Some(required(reason, "advisor escalation reason")?);
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn fail(
        &mut self,
        reason: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if self.state.is_terminal() {
            return Err(SupervisorError::InvalidTransition(
                "terminal advisor session cannot fail again".into(),
            ));
        }
        self.state = AdvisorSessionState::Failed;
        self.stop_reason = Some(required(reason, "advisor failure reason")?);
        self.updated_at = now;
        self.refresh_fingerprint()
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.id.is_nil()
            || self.supervisor_run_id.is_nil()
            || self.brief_id.is_nil()
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "advisor session does not reproduce".into(),
            ));
        }
        Ok(())
    }

    fn require_state(&self, expected: AdvisorSessionState) -> Result<(), SupervisorError> {
        if self.state != expected {
            return Err(SupervisorError::InvalidTransition(format!(
                "advisor session must be {expected:?}, not {:?}",
                self.state
            )));
        }
        Ok(())
    }

    fn refresh_fingerprint(&mut self) -> Result<(), SupervisorError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorCallState {
    Reserved,
    Started,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorModelCall {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: u32,
    pub state: AdvisorCallState,
    pub input_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AdvisorUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub reserved_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
}

impl AdvisorModelCall {
    pub fn reserve(
        id: Uuid,
        session_id: Uuid,
        sequence: u32,
        input_fingerprint: impl Into<String>,
        reserved_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || session_id.is_nil() || sequence == 0 {
            return Err(SupervisorError::Validation(
                "model call reservation identity and sequence are required".into(),
            ));
        }
        let mut value = Self {
            id,
            session_id,
            sequence,
            state: AdvisorCallState::Reserved,
            input_fingerprint: required(input_fingerprint, "model call input fingerprint")?,
            usage: None,
            error: None,
            reserved_at,
            finished_at: None,
            fingerprint: String::new(),
        };
        value.refresh_fingerprint()?;
        Ok(value)
    }

    pub fn start(&mut self) -> Result<(), SupervisorError> {
        if self.state != AdvisorCallState::Reserved {
            return Err(SupervisorError::InvalidTransition(
                "only reserved model call may start".into(),
            ));
        }
        self.state = AdvisorCallState::Started;
        self.refresh_fingerprint()
    }

    pub fn succeed(
        &mut self,
        usage: AdvisorUsage,
        finished_at: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if !matches!(
            self.state,
            AdvisorCallState::Reserved | AdvisorCallState::Started
        ) {
            return Err(SupervisorError::InvalidTransition(
                "only open model call may succeed".into(),
            ));
        }
        self.state = AdvisorCallState::Succeeded;
        self.usage = Some(usage);
        self.finished_at = Some(finished_at);
        self.refresh_fingerprint()
    }

    pub fn interrupt(
        &mut self,
        reason: impl Into<String>,
        finished_at: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if !matches!(
            self.state,
            AdvisorCallState::Reserved | AdvisorCallState::Started
        ) {
            return Err(SupervisorError::InvalidTransition(
                "only open model call may be interrupted".into(),
            ));
        }
        self.state = AdvisorCallState::Interrupted;
        self.error = Some(required(reason, "model interruption reason")?);
        self.finished_at = Some(finished_at);
        self.refresh_fingerprint()
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn refresh_fingerprint(&mut self) -> Result<(), SupervisorError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorToolCall {
    pub id: Uuid,
    pub session_id: Uuid,
    pub external_call_id: String,
    pub name: String,
    pub arguments: Value,
    pub arguments_fingerprint: String,
    pub state: AdvisorCallState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
}

impl AdvisorToolCall {
    pub fn start(
        id: Uuid,
        session_id: Uuid,
        external_call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
        started_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || session_id.is_nil() {
            return Err(SupervisorError::Validation(
                "tool call identities must not be nil".into(),
            ));
        }
        let mut value = Self {
            id,
            session_id,
            external_call_id: required(external_call_id, "external tool call id")?,
            name: required(name, "advisor tool name")?,
            arguments_fingerprint: fingerprint(&arguments)?,
            arguments,
            state: AdvisorCallState::Started,
            result_fingerprint: None,
            error: None,
            started_at,
            finished_at: None,
            fingerprint: String::new(),
        };
        value.refresh_fingerprint()?;
        Ok(value)
    }

    pub fn succeed(
        &mut self,
        result: &Value,
        finished_at: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if self.state != AdvisorCallState::Started {
            return Err(SupervisorError::InvalidTransition(
                "only started tool call may succeed".into(),
            ));
        }
        self.state = AdvisorCallState::Succeeded;
        self.result_fingerprint = Some(fingerprint(result)?);
        self.finished_at = Some(finished_at);
        self.refresh_fingerprint()
    }

    pub fn fail(
        &mut self,
        reason: impl Into<String>,
        finished_at: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if self.state != AdvisorCallState::Started {
            return Err(SupervisorError::InvalidTransition(
                "only started tool call may fail".into(),
            ));
        }
        self.state = AdvisorCallState::Failed;
        self.error = Some(required(reason, "tool failure reason")?);
        self.finished_at = Some(finished_at);
        self.refresh_fingerprint()
    }

    pub fn interrupt(
        &mut self,
        reason: impl Into<String>,
        finished_at: DateTime<Utc>,
    ) -> Result<(), SupervisorError> {
        if self.state != AdvisorCallState::Started {
            return Err(SupervisorError::InvalidTransition(
                "only started tool call may be interrupted".into(),
            ));
        }
        self.state = AdvisorCallState::Interrupted;
        self.error = Some(required(reason, "tool interruption reason")?);
        self.finished_at = Some(finished_at);
        self.refresh_fingerprint()
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn refresh_fingerprint(&mut self) -> Result<(), SupervisorError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }
}

fn valid_environment_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|value| value == '_' || value.is_ascii_alphabetic())
        && characters.all(|value| value == '_' || value.is_ascii_alphanumeric())
}
