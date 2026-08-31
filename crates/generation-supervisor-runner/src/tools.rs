use agent_runtime_core::{AgentToolRequest, AgentToolResult};
use chrono::Utc;
use generation_supervisor_core::{
    SupervisorError,
    advisor::{AdvisorToolCall, GenerationQualityDiagnosisBrief},
    revision::{
        AdvisorRuntimeIdentity, AdvisorUsage, DiagnosisCause, PromptRevisionProposal,
        SupervisorDiagnosis,
    },
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    ExecutionState, GenerationSupervisorAdvisorRunner, RunnerError,
    inputs::{FinishInput, FinishOutcome, PreviewInput, RevisionInput, only_scope},
};

impl GenerationSupervisorAdvisorRunner {
    pub(super) async fn handle_tool(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &mut generation_supervisor_core::advisor::AdvisorSession,
        state: &mut ExecutionState,
        request: AgentToolRequest,
    ) -> Result<AgentToolResult, RunnerError> {
        let mut call = AdvisorToolCall::start(
            Uuid::new_v4(),
            session.id,
            request.external_call_id,
            request.name.clone(),
            request.arguments.clone(),
            Utc::now(),
        )?;
        self.store.save_tool_call(&call).await?;
        let mut preflight = session.clone();
        if let Err(error) = preflight.record_usage(
            &brief.contract,
            AdvisorUsage {
                tool_calls: 1,
                ..AdvisorUsage::default()
            },
            Utc::now(),
        ) {
            call.fail(error.to_string(), Utc::now())?;
            self.store.save_tool_call(&call).await?;
            return Err(error.into());
        }
        session.record_usage(
            &brief.contract,
            AdvisorUsage {
                tool_calls: 1,
                ..AdvisorUsage::default()
            },
            Utc::now(),
        )?;
        self.store
            .save_tool_call_and_session(&call, session)
            .await?;

        let result = self
            .execute_tool(brief, session, state, &request.name, request.arguments)
            .await;
        match result {
            Ok((content, terminate)) => {
                call.succeed(&content, Utc::now())?;
                self.store
                    .save_tool_call_and_session(&call, session)
                    .await?;
                Ok(AgentToolResult {
                    content,
                    details: json!({"toolCallId": call.id}),
                    terminate,
                })
            }
            Err(error) => {
                call.fail(error.to_string(), Utc::now())?;
                self.store
                    .save_tool_call_and_session(&call, session)
                    .await?;
                Err(error)
            }
        }
    }

    async fn execute_tool(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &mut generation_supervisor_core::advisor::AdvisorSession,
        state: &mut ExecutionState,
        name: &str,
        arguments: Value,
    ) -> Result<(Value, bool), RunnerError> {
        match name {
            "inspect_quality_contract" => {
                require_empty_object(&arguments)?;
                Ok((
                    json!({
                        "contractId": brief.contract.id,
                        "contractFingerprint": brief.contract.fingerprint,
                        "rowThresholds": brief.contract.row_thresholds,
                        "batchThresholds": brief.contract.batch_thresholds,
                        "monitoring": brief.contract.monitoring,
                        "remainingAdvisorBudget": {
                            "modelTurns": brief.contract.budgets.maximum_pi_model_turns.saturating_sub(session.usage.model_turns),
                            "toolCalls": brief.contract.budgets.maximum_pi_tool_calls.saturating_sub(session.usage.tool_calls),
                            "inputTokens": brief.contract.budgets.maximum_pi_input_tokens.saturating_sub(session.usage.input_tokens),
                            "outputTokens": brief.contract.budgets.maximum_pi_output_tokens.saturating_sub(session.usage.output_tokens),
                        },
                        "revisionPolicy": brief.contract.revision_policy,
                        "approvalPolicy": brief.contract.approval_policy,
                        "generatorEvaluatorRelationship": brief.contract.generator_evaluator_relationship,
                    }),
                    false,
                ))
            }
            "inspect_quality_window" => {
                require_empty_object(&arguments)?;
                Ok((serde_json::to_value(&brief.window)?, false))
            }
            "inspect_failure_breakdown" => {
                require_empty_object(&arguments)?;
                Ok((
                    json!({
                        "scope": brief.decision.scope,
                        "failureKind": brief.decision.failure_kind,
                        "issues": brief.decision.issues,
                        "thresholdObservations": brief.decision.threshold_observations,
                        "criterionFailures": brief.window.criterion_failures,
                        "issueCodeCounts": brief.window.issue_code_counts,
                        "missingRequiredPatterns": brief.window.missing_required_patterns,
                    }),
                    false,
                ))
            }
            "inspect_current_prompt_guidance" => {
                require_empty_object(&arguments)?;
                Ok((
                    json!({
                        "promptVersionId": brief.current_prompt.id,
                        "promptVersionFingerprint": brief.current_prompt.fingerprint,
                        "basePromptFingerprint": brief.current_prompt.base_prompt_fingerprint,
                        "protectedFieldsFingerprint": brief.current_prompt.protected_fields_fingerprint,
                        "guidance": brief.current_prompt.guidance,
                    }),
                    false,
                ))
            }
            "preview_prompt_revision" => {
                let input: PreviewInput = parse(arguments)?;
                validate_preview(brief, &input)?;
                Ok((
                    json!({
                        "valid": true,
                        "affectedScope": brief.decision.scope,
                        "replacementGuidance": input.replacement_guidance,
                        "expectedImprovements": input.expected_improvements,
                        "protectedFieldsUnchanged": true,
                        "protectedFieldsFingerprint": brief.current_prompt.protected_fields_fingerprint,
                        "requiresCanary": true,
                        "requiresReview": matches!(brief.contract.approval_policy, generation_supervisor_core::contract::RevisionApprovalPolicy::ExplicitReview),
                    }),
                    false,
                ))
            }
            "submit_prompt_revision" => {
                if state.proposal.is_some() {
                    return Err(RunnerError::Validation(
                        "this diagnosis session already submitted a revision".into(),
                    ));
                }
                let input: RevisionInput = parse(arguments)?;
                let preview = PreviewInput {
                    replacement_guidance: input.replacement_guidance.clone(),
                    expected_improvements: input.expected_improvements.clone(),
                };
                validate_preview(brief, &preview)?;
                if input.cause == DiagnosisCause::NotSafelyRepairable {
                    return Err(RunnerError::Validation(
                        "not-safely-repairable must escalate instead of submitting a patch".into(),
                    ));
                }
                let runtime = runtime_identity(brief);
                let usage = session.usage.clone();
                let diagnosis = SupervisorDiagnosis::create(
                    Uuid::new_v4(),
                    brief.supervisor_run_id,
                    &brief.decision,
                    input.cause,
                    input.summary,
                    true,
                    runtime.clone(),
                    usage.clone(),
                    Utc::now(),
                )?;
                let proposal = PromptRevisionProposal::create(
                    Uuid::new_v4(),
                    &brief.contract,
                    &brief.decision,
                    &diagnosis,
                    &brief.current_prompt,
                    1,
                    only_scope(&brief.decision.scope),
                    input.replacement_guidance,
                    input.expected_improvements,
                    runtime,
                    usage,
                    Utc::now(),
                )?;
                self.store.save_diagnosis(session.id, &diagnosis).await?;
                self.store.save_proposal(session.id, &proposal).await?;
                let response = json!({
                    "diagnosisId": diagnosis.id,
                    "diagnosisFingerprint": diagnosis.fingerprint,
                    "proposalId": proposal.id,
                    "proposalFingerprint": proposal.fingerprint,
                    "validated": true,
                    "protectedFieldsUnchanged": true,
                });
                state.diagnosis = Some(diagnosis);
                state.proposal = Some(proposal);
                Ok((response, false))
            }
            "finish_supervision" => {
                let input: FinishInput = parse(arguments)?;
                match input.outcome {
                    FinishOutcome::RevisionSubmitted => {
                        let diagnosis = state.diagnosis.as_ref().ok_or_else(|| {
                            RunnerError::Validation(
                                "finish requires a persisted diagnosis and revision".into(),
                            )
                        })?;
                        let proposal = state.proposal.as_ref().ok_or_else(|| {
                            RunnerError::Validation(
                                "finish requires a persisted diagnosis and revision".into(),
                            )
                        })?;
                        session.await_review(diagnosis.id, proposal.id, Utc::now())?;
                    }
                    FinishOutcome::Escalate => {
                        if state.proposal.is_some() {
                            return Err(RunnerError::Validation(
                                "session with a submitted revision cannot silently escalate it"
                                    .into(),
                            ));
                        }
                        let cause = input.cause.ok_or_else(|| {
                            RunnerError::Validation(
                                "escalation requires an explicit bounded diagnosis cause".into(),
                            )
                        })?;
                        let diagnosis = SupervisorDiagnosis::create(
                            Uuid::new_v4(),
                            brief.supervisor_run_id,
                            &brief.decision,
                            cause,
                            input.summary.clone(),
                            false,
                            runtime_identity(brief),
                            session.usage.clone(),
                            Utc::now(),
                        )?;
                        self.store.save_diagnosis(session.id, &diagnosis).await?;
                        session.escalate(diagnosis.id, input.summary, Utc::now())?;
                        state.diagnosis = Some(diagnosis);
                    }
                }
                self.store.save_session(session).await?;
                Ok((
                    json!({
                        "sessionId": session.id,
                        "state": session.state,
                        "finishAccepted": true,
                    }),
                    true,
                ))
            }
            _ => Err(RunnerError::Validation(format!(
                "unknown generation supervisor advisor tool {name:?}"
            ))),
        }
    }
}

fn validate_preview(
    brief: &GenerationQualityDiagnosisBrief,
    input: &PreviewInput,
) -> Result<(), RunnerError> {
    let policy = &brief.contract.revision_policy;
    if input.replacement_guidance.is_empty()
        || input.replacement_guidance.len()
            > usize::try_from(policy.maximum_instructions).unwrap_or(usize::MAX)
        || input.replacement_guidance.iter().any(|value| {
            value.trim().is_empty()
                || value.chars().count()
                    > usize::try_from(policy.maximum_characters_per_instruction)
                        .unwrap_or(usize::MAX)
        })
        || input
            .replacement_guidance
            .iter()
            .map(|value| value.chars().count())
            .sum::<usize>()
            > usize::try_from(policy.maximum_total_characters).unwrap_or(usize::MAX)
        || input.expected_improvements.is_empty()
        || input.expected_improvements.iter().any(|value| {
            value.metric.trim().is_empty() || value.minimum_delta_basis_points > 10_000
        })
    {
        return Err(RunnerError::Validation(
            "prompt revision preview exceeds policy or lacks measurable improvement".into(),
        ));
    }
    Ok(())
}

fn runtime_identity(brief: &GenerationQualityDiagnosisBrief) -> AdvisorRuntimeIdentity {
    AdvisorRuntimeIdentity {
        runtime: "pi".into(),
        model: brief.advisor.model.clone(),
        protocol_version: brief.advisor.runtime_protocol_version.clone(),
        capability_set_version: brief.capability_set_version,
        configuration_fingerprint: brief.advisor.configuration_fingerprint.clone(),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, RunnerError> {
    serde_json::from_value(value).map_err(Into::into)
}

fn require_empty_object(value: &Value) -> Result<(), RunnerError> {
    if value.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err(RunnerError::Validation(
            "inspection tool accepts only an empty object".into(),
        ))
    }
}

#[allow(dead_code)]
fn _map_domain(error: SupervisorError) -> RunnerError {
    error.into()
}
