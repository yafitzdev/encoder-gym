//! Host-controlled execution of bounded Dataset Architect agents.

mod inputs;
mod tools;

use std::{sync::Arc, time::Duration};

use dataset_architect_core::{
    ArchitectError,
    brief::ResolvedArchitectBrief,
    lifecycle::{
        ArchitectRun, ArchitectRunState, ArchitectStopReason, ArchitectToolCallState,
        ArchitectUsage,
    },
    ports::{
        ArchitectAdapterError, ArchitectAgentEvent, ArchitectAgentMessage, ArchitectAgentRequest,
        ArchitectAgentRuntime, ArchitectAgentSession, ArchitectStore,
    },
    proposal::DatasetArchitectureProposal,
};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const ARCHITECT_PROTOCOL_VERSION: u32 = 1;
pub const ARCHITECT_SYSTEM_PROMPT: &str = "You are an advisory Dataset Architect. State a concise plan, inspect the pinned facts, compare at least one explicit allocation through preview_allocation, estimate its cost, submit one complete proposal, and finish. Never request or infer sealed evidence. Never create plans, generate rows, train models, or approve your own work.";

const INITIAL_PROMPT_TEMPLATE: &str = "Architect a synthetic dataset from this immutable brief:\n<dataset_architect_brief_json>\n{brief}\n</dataset_architect_brief_json>\nThe existing deterministic allocator is authoritative. Explain why every target and strategy earns its budget.";

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("dataset architect artifact was not found: {0}")]
    NotFound(String),
    #[error("dataset architect runner rejected the operation: {0}")]
    Validation(String),
    #[error(transparent)]
    Domain(#[from] ArchitectError),
    #[error(transparent)]
    Adapter(#[from] ArchitectAdapterError),
    #[error("dataset architect JSON was invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("deterministic allocation failed: {0}")]
    Allocation(String),
}

impl From<workflow_core::allocation::InitialAllocationError> for RunnerError {
    fn from(value: workflow_core::allocation::InitialAllocationError) -> Self {
        Self::Allocation(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchitectOutcome {
    pub run: ArchitectRun,
    pub proposal: Option<DatasetArchitectureProposal>,
}

pub struct ArchitectRunner {
    runtime: Arc<dyn ArchitectAgentRuntime>,
    store: Arc<dyn ArchitectStore>,
}

impl ArchitectRunner {
    pub fn new(runtime: Arc<dyn ArchitectAgentRuntime>, store: Arc<dyn ArchitectStore>) -> Self {
        Self { runtime, store }
    }

    pub async fn queue(&self, brief: ResolvedArchitectBrief) -> Result<ArchitectRun, RunnerError> {
        let run = ArchitectRun::queue(&brief, ARCHITECT_PROTOCOL_VERSION, protocol_fingerprint()?)?;
        self.store.create_run(&brief, &run).await?;
        Ok(run)
    }

    pub async fn run(&self, run_id: Uuid) -> Result<ArchitectOutcome, RunnerError> {
        let mut run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("architect run {run_id}")))?;
        let brief =
            self.store.get_brief(run.brief_id).await?.ok_or_else(|| {
                RunnerError::NotFound(format!("architect brief {}", run.brief_id))
            })?;
        self.validate_identity(&run, &brief)?;
        run.start()?;
        self.store.save_run(&run).await?;

        let request = ArchitectAgentRequest {
            protocol_version: run.protocol_version,
            capability_set: "dataset_architect_v1".into(),
            run_id: run.id,
            run_specification_fingerprint: run.specification_fingerprint.clone(),
            provider: brief.provider.provider.clone(),
            model: brief.provider.model.clone(),
            api_key_env: brief.provider.api_key_env.clone(),
            system_prompt: ARCHITECT_SYSTEM_PROMPT.into(),
            initial_prompt: INITIAL_PROMPT_TEMPLATE
                .replace("{brief}", &serde_json::to_string_pretty(&brief)?),
            max_model_turns: brief.budgets.max_model_turns,
        };
        let mut session = match self.runtime.start(request).await {
            Ok(session) => session,
            Err(error) => {
                self.fail_run(
                    &mut run,
                    ArchitectStopReason::ProviderFailure,
                    error.to_string(),
                )
                .await?;
                return Err(error.into());
            }
        };
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(brief.budgets.max_wall_clock_seconds);
        let mut state = ExecutionState::default();
        loop {
            if self
                .refresh_cancellation(&mut run, session.as_mut())
                .await?
            {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = session.cancel(run.id).await;
                self.finish_for_budget(&mut run, &mut state).await?;
                break;
            }
            let message = match tokio::time::timeout(remaining, session.next_message()).await {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    self.fail_run(
                        &mut run,
                        ArchitectStopReason::ProviderFailure,
                        error.to_string(),
                    )
                    .await?;
                    return Err(error.into());
                }
                Err(_) => {
                    let _ = session.cancel(run.id).await;
                    self.interrupt_open_calls(run.id).await?;
                    self.finish_for_budget(&mut run, &mut state).await?;
                    break;
                }
            };
            match message {
                ArchitectAgentMessage::Event { event } => {
                    if let Err(error) = self.handle_event(&brief, &mut run, event).await {
                        let _ = session.cancel(run.id).await;
                        self.fail_run(
                            &mut run,
                            ArchitectStopReason::BudgetExhausted,
                            error.to_string(),
                        )
                        .await?;
                        return Err(error);
                    }
                }
                ArchitectAgentMessage::ToolRequest { request } => {
                    let call_id = request.external_call_id.clone();
                    match self
                        .handle_tool(&brief, &mut run, &mut state, request)
                        .await
                    {
                        Ok(result) => session.send_tool_result(&call_id, result).await?,
                        Err(error) => {
                            session
                                .send_tool_error(&call_id, &error.to_string())
                                .await?
                        }
                    }
                }
                ArchitectAgentMessage::Completed => {
                    if run.state == ArchitectRunState::Running {
                        if state.pending_finish.is_some() {
                            self.finalize_proposal(&mut run, &mut state).await?;
                        } else {
                            self.fail_run(
                                &mut run,
                                ArchitectStopReason::ValidationFailure,
                                "Pi completed without finish_architecture".into(),
                            )
                            .await?;
                        }
                    }
                    break;
                }
                ArchitectAgentMessage::Failed { message } => {
                    self.fail_run(&mut run, ArchitectStopReason::ProviderFailure, message)
                        .await?;
                    break;
                }
            }
        }
        Ok(ArchitectOutcome {
            proposal: self.store.latest_proposal_for_run(run.id).await?,
            run,
        })
    }

    fn validate_identity(
        &self,
        run: &ArchitectRun,
        brief: &ResolvedArchitectBrief,
    ) -> Result<(), RunnerError> {
        if run.state != ArchitectRunState::Queued
            || run.protocol_version != ARCHITECT_PROTOCOL_VERSION
            || run.protocol_fingerprint != protocol_fingerprint()?
            || run.brief_id != brief.id
            || run.brief_fingerprint != brief.fingerprint
            || run.reproduce_specification_fingerprint()? != run.specification_fingerprint
            || brief.reproduce_fingerprint()? != brief.fingerprint
        {
            return Err(RunnerError::Validation(
                "architect run, brief, or protocol identity does not match".into(),
            ));
        }
        Ok(())
    }

    async fn handle_event(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &mut ArchitectRun,
        event: ArchitectAgentEvent,
    ) -> Result<(), RunnerError> {
        match event {
            ArchitectAgentEvent::AgentText { text } if run.plan.is_empty() => {
                run.set_plan(vec![text])?;
                self.store.save_run(run).await?;
            }
            ArchitectAgentEvent::ModelTurnCompleted {
                input_tokens,
                output_tokens,
                cost_microusd,
                ..
            } => {
                run.record_usage(
                    brief,
                    ArchitectUsage {
                        model_turns: 1,
                        input_tokens,
                        output_tokens,
                        cost_microusd,
                        ..ArchitectUsage::default()
                    },
                )?;
                self.store.save_run(run).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn refresh_cancellation(
        &self,
        run: &mut ArchitectRun,
        session: &mut dyn ArchitectAgentSession,
    ) -> Result<bool, RunnerError> {
        let persisted = self
            .store
            .get_run(run.id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("architect run {}", run.id)))?;
        if persisted.cancel_requested {
            let _ = session.cancel(run.id).await;
            run.cancel()?;
            self.store.save_run(run).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn finalize_proposal(
        &self,
        run: &mut ArchitectRun,
        state: &mut ExecutionState,
    ) -> Result<(), RunnerError> {
        let pending = state.pending_finish.take().ok_or_else(|| {
            RunnerError::Validation("architect run has no pending proposal".into())
        })?;
        run.await_review(pending.reason)?;
        self.store
            .save_proposal_and_run(&pending.proposal, run)
            .await?;
        Ok(())
    }

    async fn finish_for_budget(
        &self,
        run: &mut ArchitectRun,
        state: &mut ExecutionState,
    ) -> Result<(), RunnerError> {
        if let Some(proposal) = state
            .pending_finish
            .take()
            .map(|value| value.proposal)
            .or_else(|| state.proposal.take())
        {
            run.await_review(ArchitectStopReason::BudgetExhausted)?;
            self.store.save_proposal_and_run(&proposal, run).await?;
        } else {
            self.fail_run(
                run,
                ArchitectStopReason::BudgetExhausted,
                "architect wall-clock budget ended before a valid proposal was submitted".into(),
            )
            .await?;
        }
        Ok(())
    }

    async fn fail_run(
        &self,
        run: &mut ArchitectRun,
        reason: ArchitectStopReason,
        message: String,
    ) -> Result<(), RunnerError> {
        self.interrupt_open_calls(run.id).await?;
        run.fail(reason, message)?;
        self.store.save_run(run).await?;
        Ok(())
    }

    async fn interrupt_open_calls(&self, run_id: Uuid) -> Result<(), RunnerError> {
        for mut call in self.store.list_tool_calls(run_id).await? {
            if call.state == ArchitectToolCallState::Started {
                call.interrupt()?;
                self.store.record_tool_call(&call).await?;
            }
        }
        Ok(())
    }

    pub async fn recover(&self, run_id: Uuid) -> Result<ArchitectRun, RunnerError> {
        let mut run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or_else(|| RunnerError::NotFound(format!("architect run {run_id}")))?;
        if run.state != ArchitectRunState::Running {
            return Err(RunnerError::Validation(
                "only an interrupted running architect run can be recovered".into(),
            ));
        }
        self.interrupt_open_calls(run.id).await?;
        run.fail(
            ArchitectStopReason::Interrupted,
            "architect host stopped; paid model turns and open tool calls were not replayed".into(),
        )?;
        self.store.save_run(&run).await?;
        Ok(run)
    }
}

#[derive(Default)]
struct ExecutionState {
    proposal: Option<DatasetArchitectureProposal>,
    pending_finish: Option<PendingFinish>,
}

struct PendingFinish {
    proposal: DatasetArchitectureProposal,
    reason: ArchitectStopReason,
}

pub fn protocol_fingerprint() -> Result<String, RunnerError> {
    artifact_core::fingerprint(&(
        ARCHITECT_PROTOCOL_VERSION,
        ARCHITECT_SYSTEM_PROMPT,
        [
            "inspect_dataset",
            "inspect_semantics",
            "inspect_authenticity",
            "inspect_coverage",
            "inspect_development_evidence",
            "preview_allocation",
            "estimate_cost",
            "submit_proposal",
            "finish_architecture",
        ],
    ))
    .map_err(|error| RunnerError::Validation(error.to_string()))
}
