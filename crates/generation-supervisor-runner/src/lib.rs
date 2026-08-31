//! Host-controlled bounded Pi diagnosis for deterministic generation pauses.

mod inputs;
pub mod orchestration;
mod tools;

use std::{sync::Arc, time::Duration};

use agent_runtime_core::{
    AgentAdapterError, AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentSession,
};
use chrono::Utc;
use generation_supervisor_core::{
    SupervisorError,
    advisor::{
        AdvisorCallState, AdvisorModelCall, AdvisorSession, AdvisorSessionState,
        GenerationQualityDiagnosisBrief,
    },
    ports::SupervisorAdvisorStore,
    revision::{AdvisorUsage, PromptRevisionProposal, SupervisorDiagnosis},
};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const SUPERVISOR_ADVISOR_PROTOCOL_VERSION: u32 = 1;
pub const SUPERVISOR_ADVISOR_CAPABILITY_SET: &str = "generation_quality_supervisor_v1";
pub const SUPERVISOR_ADVISOR_SYSTEM_PROMPT: &str = r#"You are a bounded generation-quality diagnosis advisor. A deterministic host has already paused one exact generation scope. Inspect only the seven provided host tools and aggregate evidence. Never request raw rows, source text, files, shell access, databases, secrets, arbitrary network access, generation/evaluator calls, training data, predictions, or sealed evaluation evidence. You may diagnose, preview one guidance-only patch, submit one bounded prompt-guidance revision, or escalate. You cannot change labels, dimensions, schemas, semantic authority, construction, thresholds, budgets, safety instructions, or approve your own patch. State a concise plan, inspect the contract/window/failure breakdown/current guidance, then call finish_supervision."#;

const INITIAL_PROMPT_TEMPLATE: &str = r#"Diagnose this immutable, redacted generation-quality pause:
<generation_quality_diagnosis_brief_json>
{brief}
</generation_quality_diagnosis_brief_json>

The host's deterministic decision is authoritative. If a bounded guidance repair is safe, preview and submit it. Otherwise explicitly escalate."#;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("generation supervisor advisor artifact was not found: {0}")]
    NotFound(String),
    #[error("generation supervisor advisor rejected the operation: {0}")]
    Validation(String),
    #[error(transparent)]
    Domain(#[from] SupervisorError),
    #[error(transparent)]
    Adapter(#[from] AgentAdapterError),
    #[error("generation supervisor advisor JSON was invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvisorOutcome {
    pub session: AdvisorSession,
    pub diagnosis: Option<SupervisorDiagnosis>,
    pub proposal: Option<PromptRevisionProposal>,
}

#[derive(Default)]
struct ExecutionState {
    diagnosis: Option<SupervisorDiagnosis>,
    proposal: Option<PromptRevisionProposal>,
}

pub struct GenerationSupervisorAdvisorRunner {
    runtime: Arc<dyn AgentRuntime>,
    store: Arc<dyn SupervisorAdvisorStore>,
}

impl GenerationSupervisorAdvisorRunner {
    pub fn new(runtime: Arc<dyn AgentRuntime>, store: Arc<dyn SupervisorAdvisorStore>) -> Self {
        Self { runtime, store }
    }

    pub async fn queue(
        &self,
        brief: GenerationQualityDiagnosisBrief,
    ) -> Result<AdvisorSession, RunnerError> {
        let session = AdvisorSession::queue(
            Uuid::new_v4(),
            &brief,
            SUPERVISOR_ADVISOR_PROTOCOL_VERSION,
            protocol_fingerprint()?,
            Utc::now(),
        )?;
        self.store.create_session(&brief, &session).await?;
        Ok(session)
    }

    pub async fn run(&self, session_id: Uuid) -> Result<AdvisorOutcome, RunnerError> {
        let mut session = self
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("advisor session {session_id}")))?;
        let brief = self
            .store
            .get_brief(session.brief_id)
            .await?
            .ok_or_else(|| {
                RunnerError::NotFound(format!("diagnosis brief {}", session.brief_id))
            })?;
        self.validate_identity(&session, &brief)?;

        let sessions = self.store.list_sessions(brief.supervisor_run_id).await?;
        let prior_sessions = sessions
            .iter()
            .filter(|value| value.id != session.id)
            .collect::<Vec<_>>();
        let mut reserved_turns = 0_u32;
        for prior in &prior_sessions {
            reserved_turns = reserved_turns
                .checked_add(
                    u32::try_from(self.store.list_model_calls(prior.id).await?.len()).map_err(
                        |_| RunnerError::Validation("model-call count exceeds u32".into()),
                    )?,
                )
                .ok_or_else(|| {
                    RunnerError::Validation("model-call reservations overflow".into())
                })?;
        }
        let remaining_turns = brief
            .contract
            .budgets
            .maximum_pi_model_turns
            .checked_sub(reserved_turns)
            .ok_or_else(|| {
                RunnerError::Domain(SupervisorError::BudgetExhausted(
                    "advisor model-turn reservations exceed the quality contract".into(),
                ))
            })?;
        if remaining_turns == 0 {
            return Err(RunnerError::Domain(SupervisorError::BudgetExhausted(
                "advisor model-turn budget is exhausted".into(),
            )));
        }
        let remaining_sessions = brief
            .contract
            .budgets
            .maximum_prompt_revisions
            .max(1)
            .saturating_sub(u32::try_from(prior_sessions.len()).unwrap_or(u32::MAX))
            .max(1);
        let maximum_model_turns = remaining_turns.div_ceil(remaining_sessions).max(1);
        let model_calls = (1..=maximum_model_turns)
            .map(|sequence| {
                AdvisorModelCall::reserve(
                    Uuid::new_v4(),
                    session.id,
                    sequence,
                    artifact_core::fingerprint(&(
                        brief.fingerprint.as_str(),
                        sequence,
                        SUPERVISOR_ADVISOR_CAPABILITY_SET,
                    ))
                    .map_err(|error| RunnerError::Validation(error.to_string()))?,
                    Utc::now(),
                )
                .map_err(RunnerError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.store.reserve_model_calls(&model_calls).await?;
        session.start(Utc::now())?;
        self.store.save_session(&session).await?;

        let request = AgentRequest {
            protocol_version: SUPERVISOR_ADVISOR_PROTOCOL_VERSION,
            capability_set: SUPERVISOR_ADVISOR_CAPABILITY_SET.into(),
            run_id: session.id,
            run_specification_fingerprint: brief.fingerprint.clone(),
            provider: brief.advisor.provider.clone(),
            model: brief.advisor.model.clone(),
            api_key_env: brief.advisor.api_key_env.clone(),
            system_prompt: SUPERVISOR_ADVISOR_SYSTEM_PROMPT.into(),
            initial_prompt: INITIAL_PROMPT_TEMPLATE
                .replace("{brief}", &serde_json::to_string_pretty(&brief)?),
            max_model_turns: maximum_model_turns,
        };
        let mut agent = match self.runtime.start(request).await {
            Ok(agent) => agent,
            Err(error) => {
                self.interrupt_uncertain_calls(session.id).await?;
                session.fail(error.to_string(), Utc::now())?;
                self.store.save_session(&session).await?;
                return Err(error.into());
            }
        };
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(brief.contract.budgets.maximum_duration_seconds);
        let mut state = ExecutionState::default();

        loop {
            if self
                .refresh_cancellation(&mut session, agent.as_mut())
                .await?
            {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = agent.cancel(session.id).await;
                self.interrupt_uncertain_calls(session.id).await?;
                session.fail("advisor wall-clock budget exhausted", Utc::now())?;
                self.store.save_session(&session).await?;
                break;
            }
            let message = match tokio::time::timeout(remaining, agent.next_message()).await {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    self.interrupt_uncertain_calls(session.id).await?;
                    session.fail(error.to_string(), Utc::now())?;
                    self.store.save_session(&session).await?;
                    return Err(error.into());
                }
                Err(_) => {
                    let _ = agent.cancel(session.id).await;
                    self.interrupt_uncertain_calls(session.id).await?;
                    session.fail("advisor wall-clock budget exhausted", Utc::now())?;
                    self.store.save_session(&session).await?;
                    break;
                }
            };
            match message {
                AgentMessage::Event { event } => {
                    if let Err(error) = self.handle_event(&brief, &mut session, event).await {
                        let _ = agent.cancel(session.id).await;
                        self.interrupt_uncertain_calls(session.id).await?;
                        session.fail(error.to_string(), Utc::now())?;
                        self.store.save_session(&session).await?;
                        return Err(error);
                    }
                }
                AgentMessage::ToolRequest { request } => {
                    let external_call_id = request.external_call_id.clone();
                    match self
                        .handle_tool(&brief, &mut session, &mut state, request)
                        .await
                    {
                        Ok(result) => agent.send_tool_result(&external_call_id, result).await?,
                        Err(error) => {
                            agent
                                .send_tool_error(&external_call_id, &error.to_string())
                                .await?;
                        }
                    }
                }
                AgentMessage::Completed => {
                    if session.state == AdvisorSessionState::Running {
                        session.fail(
                            "Pi completed without a valid finish_supervision call",
                            Utc::now(),
                        )?;
                        self.store.save_session(&session).await?;
                    }
                    break;
                }
                AgentMessage::Failed { message } => {
                    self.interrupt_uncertain_calls(session.id).await?;
                    session.fail(message, Utc::now())?;
                    self.store.save_session(&session).await?;
                    break;
                }
            }
        }

        Ok(AdvisorOutcome {
            diagnosis: self.store.latest_diagnosis(session.id).await?,
            proposal: self.store.latest_proposal(session.id).await?,
            session,
        })
    }

    fn validate_identity(
        &self,
        session: &AdvisorSession,
        brief: &GenerationQualityDiagnosisBrief,
    ) -> Result<(), RunnerError> {
        brief.validate()?;
        session.validate()?;
        if session.state != AdvisorSessionState::Queued
            || session.brief_id != brief.id
            || session.brief_fingerprint != brief.fingerprint
            || session.supervisor_run_id != brief.supervisor_run_id
            || session.protocol_version != SUPERVISOR_ADVISOR_PROTOCOL_VERSION
            || session.protocol_fingerprint != protocol_fingerprint()?
        {
            return Err(RunnerError::Validation(
                "advisor session, brief, or capability protocol does not match".into(),
            ));
        }
        Ok(())
    }

    async fn handle_event(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &mut AdvisorSession,
        event: AgentEvent,
    ) -> Result<(), RunnerError> {
        match event {
            AgentEvent::AgentText { text } if session.plan.is_empty() => {
                session.set_plan(text.lines().map(ToOwned::to_owned).collect(), Utc::now())?;
                self.store.save_session(session).await?;
            }
            AgentEvent::ModelTurnStarted { sequence } => {
                let mut call = self.model_call(session.id, sequence).await?;
                call.start()?;
                self.store
                    .save_model_call_and_session(&call, session)
                    .await?;
            }
            AgentEvent::ModelTurnCompleted {
                sequence,
                input_tokens,
                output_tokens,
                cost_microusd,
            } => {
                let usage = AdvisorUsage {
                    model_turns: 1,
                    input_tokens,
                    output_tokens,
                    cost_microunits: Some(cost_microusd),
                    ..AdvisorUsage::default()
                };
                let mut call = self.model_call(session.id, sequence).await?;
                call.succeed(usage.clone(), Utc::now())?;
                session.record_usage(&brief.contract, usage, Utc::now())?;
                self.store
                    .save_model_call_and_session(&call, session)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn model_call(
        &self,
        session_id: Uuid,
        sequence: u32,
    ) -> Result<AdvisorModelCall, RunnerError> {
        self.store
            .list_model_calls(session_id)
            .await?
            .into_iter()
            .find(|call| call.sequence == sequence)
            .ok_or_else(|| {
                RunnerError::Validation(format!(
                    "model turn {sequence} has no durable pre-I/O reservation"
                ))
            })
    }

    async fn refresh_cancellation(
        &self,
        session: &mut AdvisorSession,
        agent: &mut dyn AgentSession,
    ) -> Result<bool, RunnerError> {
        let persisted = self
            .store
            .get_session(session.id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("advisor session {}", session.id)))?;
        if persisted.cancel_requested {
            let _ = agent.cancel(session.id).await;
            self.interrupt_uncertain_calls(session.id).await?;
            session.cancel(Utc::now())?;
            self.store.save_session(session).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn interrupt_uncertain_calls(&self, session_id: Uuid) -> Result<(), RunnerError> {
        for mut call in self.store.list_tool_calls(session_id).await? {
            if call.state == AdvisorCallState::Started {
                call.interrupt("host stopped before tool outcome was durable", Utc::now())?;
                self.store.save_tool_call(&call).await?;
            }
        }
        if let Some(mut call) = self
            .store
            .list_model_calls(session_id)
            .await?
            .into_iter()
            .filter(|call| {
                matches!(
                    call.state,
                    AdvisorCallState::Reserved | AdvisorCallState::Started
                )
            })
            .min_by_key(|call| call.sequence)
        {
            call.interrupt("host stopped with uncertain provider outcome", Utc::now())?;
            let session =
                self.store.get_session(session_id).await?.ok_or_else(|| {
                    RunnerError::NotFound(format!("advisor session {session_id}"))
                })?;
            self.store
                .save_model_call_and_session(&call, &session)
                .await?;
        }
        Ok(())
    }

    pub async fn recover(&self, session_id: Uuid) -> Result<AdvisorSession, RunnerError> {
        let mut session = self
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("advisor session {session_id}")))?;
        if session.state != AdvisorSessionState::Running {
            return Err(RunnerError::Validation(
                "only an interrupted running advisor session can be recovered".into(),
            ));
        }
        self.interrupt_uncertain_calls(session.id).await?;
        session.fail(
            "advisor host stopped; uncertain paid calls were not replayed",
            Utc::now(),
        )?;
        self.store.save_session(&session).await?;
        Ok(session)
    }
}

pub fn protocol_fingerprint() -> Result<String, RunnerError> {
    artifact_core::fingerprint(&(
        SUPERVISOR_ADVISOR_PROTOCOL_VERSION,
        SUPERVISOR_ADVISOR_CAPABILITY_SET,
        SUPERVISOR_ADVISOR_SYSTEM_PROMPT,
        INITIAL_PROMPT_TEMPLATE,
        [
            "inspect_quality_contract",
            "inspect_quality_window",
            "inspect_failure_breakdown",
            "inspect_current_prompt_guidance",
            "preview_prompt_revision",
            "submit_prompt_revision",
            "finish_supervision",
        ],
    ))
    .map_err(|error| RunnerError::Validation(error.to_string()))
}
