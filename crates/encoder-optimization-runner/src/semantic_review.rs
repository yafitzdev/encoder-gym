//! Pi-backed transport for the two provider-neutral native review passes.
//! Admission remains host-derived in `dataset-quality-core`; this adapter only
//! obtains and normalizes one bounded tool submission per reserved call.

use std::{collections::BTreeSet, sync::Arc};

use agent_runtime_core::{AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentToolResult};
use dataset_quality_core::{
    assessment::{EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence},
    native_assessment::{
        NativeBlindAssessmentDraft, NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest,
        NativeReviewUsage, NativeTargetFitDraft, NativeTargetFitEvidence, NativeTargetFitRequest,
    },
    ports::{
        BoxFuture, NativeBlindBatchOutput, NativeSemanticReviewer, NativeTargetFitBatchOutput,
        QualityEvaluationError, QualityEvaluationErrorKind,
    },
};
use encoder_optimization_core::fingerprint;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AgentSelection;

const BLIND_CAPABILITY: &str = "encoder_optimization_native_blind_v1";
const BLIND_TOOL: &str = "submit_native_blind_assessments";
const TARGET_FIT_CAPABILITY: &str = "encoder_optimization_native_target_fit_v1";
const TARGET_FIT_TOOL: &str = "submit_native_target_fit_assessments";
const REVIEW_PROTOCOL: &str = "native_semantic_review_v1";
const REVIEW_SYSTEM_PROMPT: &str = "Return exactly one bounded native semantic assessment for every supplied row through the sole required tool. Copy all host identities and fingerprints exactly. The host validates the output and owns admission.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewerConfiguration<'a> {
    provider: &'a str,
    model: &'a str,
    maximum_output_tokens: u32,
    maximum_cost_microusd: u64,
    runtime_cost_is_known: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission<T> {
    assessments: Vec<T>,
}

#[derive(Debug)]
struct ReviewTurn {
    arguments: Value,
    usage: NativeReviewUsage,
    metadata: Value,
}

pub struct PiNativeSemanticReviewer {
    runtime: Arc<dyn AgentRuntime>,
    selection: AgentSelection,
    identity: EvaluatorIdentity,
}

impl PiNativeSemanticReviewer {
    pub fn new(
        runtime: Arc<dyn AgentRuntime>,
        selection: AgentSelection,
    ) -> Result<Self, QualityEvaluationError> {
        if selection.provider.trim().is_empty()
            || selection.model.trim().is_empty()
            || !(1..=65_536).contains(&selection.maximum_output_tokens_per_turn)
        {
            return Err(configuration_error("invalid native reviewer selection"));
        }
        let configuration_fingerprint = fingerprint(&ReviewerConfiguration {
            provider: &selection.provider,
            model: &selection.model,
            maximum_output_tokens: selection.maximum_output_tokens_per_turn,
            maximum_cost_microusd: selection.maximum_cost_microusd_per_turn,
            runtime_cost_is_known: selection.runtime_cost_is_known,
        })
        .map_err(|_| configuration_error("native reviewer configuration is invalid"))?;
        let identity = EvaluatorIdentity::new(
            selection.provider.clone(),
            selection.model.clone(),
            REVIEW_PROTOCOL,
            configuration_fingerprint,
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::ExternalService,
        )
        .map_err(|_| configuration_error("native reviewer identity is invalid"))?;
        Ok(Self {
            runtime,
            selection,
            identity,
        })
    }

    async fn run<T: Serialize>(
        &self,
        run_id: Uuid,
        request_fingerprint: &str,
        request: &T,
        capability: &str,
        required_tool: &str,
    ) -> Result<ReviewTurn, QualityEvaluationError> {
        let initial_prompt = serde_json::to_string(request)
            .map_err(|_| configuration_error("native reviewer request could not be encoded"))?;
        let agent_request = AgentRequest {
            protocol_version: 1,
            capability_set: capability.into(),
            run_id,
            run_specification_fingerprint: request_fingerprint.into(),
            provider: self.selection.provider.clone(),
            model: self.selection.model.clone(),
            api_key_env: self.selection.api_key_env.clone(),
            system_prompt: REVIEW_SYSTEM_PROMPT.into(),
            initial_prompt,
            max_model_turns: 1,
        };
        let mut session = self
            .runtime
            .start(agent_request)
            .await
            .map_err(|_| transport_error("native reviewer could not start"))?;
        let mut arguments = None;
        let mut completed_turns = 0_u32;
        let mut usage = NativeReviewUsage::default();
        loop {
            let message = session
                .next_message()
                .await
                .map_err(|_| transport_error("native reviewer connection was interrupted"))?;
            match message {
                AgentMessage::Event {
                    event:
                        AgentEvent::ModelTurnCompleted {
                            input_tokens,
                            output_tokens,
                            cost_microusd,
                            ..
                        },
                } => {
                    completed_turns = completed_turns.saturating_add(1);
                    if completed_turns > 1 {
                        let _ = session.cancel(run_id).await;
                        return Err(invalid_response(
                            "native reviewer exceeded its one-turn contract",
                        ));
                    }
                    // A nonempty prompt cannot use zero tokens. Pi maps missing
                    // provider usage to zero, so retain that distinction.
                    usage.input_tokens = (input_tokens > 0).then_some(input_tokens);
                    usage.output_tokens = (output_tokens > 0).then_some(output_tokens);
                    usage.cost_microusd = self
                        .selection
                        .runtime_cost_is_known
                        .then_some(cost_microusd);
                }
                AgentMessage::ToolRequest { request } => {
                    if request.name != required_tool || arguments.is_some() {
                        let _ = session.cancel(run_id).await;
                        return Err(invalid_response(
                            "native reviewer ignored the required sole tool",
                        ));
                    }
                    arguments = Some(request.arguments);
                    session
                        .send_tool_result(
                            &request.external_call_id,
                            AgentToolResult {
                                content: json!({"recorded": true}),
                                details: Value::Null,
                                terminate: true,
                            },
                        )
                        .await
                        .map_err(|_| {
                            transport_error("native reviewer result acknowledgement failed")
                        })?;
                }
                AgentMessage::Completed => {
                    if completed_turns != 1 {
                        return Err(invalid_response(
                            "native reviewer ended without one completed model turn",
                        ));
                    }
                    let arguments = arguments.ok_or_else(|| {
                        invalid_response("native reviewer omitted the required tool call")
                    })?;
                    return Ok(ReviewTurn {
                        arguments,
                        usage,
                        metadata: json!({
                            "capabilitySet": capability,
                            "tool": required_tool,
                        }),
                    });
                }
                AgentMessage::Failed { message } => return Err(provider_error(&message)),
                AgentMessage::Event { .. } => {}
            }
        }
    }
}

impl NativeSemanticReviewer for PiNativeSemanticReviewer {
    fn native_identity(&self) -> EvaluatorIdentity {
        self.identity.clone()
    }

    fn assess_blind(
        &self,
        request: NativeBlindAssessmentRequest,
    ) -> BoxFuture<'_, Result<NativeBlindBatchOutput, QualityEvaluationError>> {
        Box::pin(async move {
            request
                .validate()
                .map_err(|_| configuration_error("native blind request is invalid"))?;
            if request.evaluator_identity_fingerprint != self.identity.fingerprint {
                return Err(configuration_error(
                    "native blind request targets a different reviewer identity",
                ));
            }
            let turn = self
                .run(
                    request.run_id,
                    &request.fingerprint,
                    &request,
                    BLIND_CAPABILITY,
                    BLIND_TOOL,
                )
                .await?;
            let drafts: Vec<NativeBlindAssessmentDraft> = submission(turn.arguments)?;
            let expected = request
                .rows
                .iter()
                .map(|row| row.row_id.as_str())
                .collect::<BTreeSet<_>>();
            let actual = drafts
                .iter()
                .map(|draft| draft.row_id.as_str())
                .collect::<BTreeSet<_>>();
            if drafts.len() != request.rows.len() || actual != expected {
                return Err(invalid_response(
                    "native blind submission must cover every requested row exactly once",
                ));
            }
            let assessments = drafts
                .into_iter()
                .map(|draft| NativeBlindAssessmentEvidence::record(&request, draft))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid_response("native blind submission is invalid"))?
                .into_iter()
                .map(|evidence| evidence.draft)
                .collect();
            Ok(NativeBlindBatchOutput {
                assessments,
                usage: turn.usage,
                metadata: turn.metadata,
            })
        })
    }

    fn assess_target_fit(
        &self,
        request: NativeTargetFitRequest,
    ) -> BoxFuture<'_, Result<NativeTargetFitBatchOutput, QualityEvaluationError>> {
        Box::pin(async move {
            request
                .validate()
                .map_err(|_| configuration_error("native target-fit request is invalid"))?;
            if request.evaluator_identity_fingerprint != self.identity.fingerprint {
                return Err(configuration_error(
                    "native target-fit request targets a different reviewer identity",
                ));
            }
            let turn = self
                .run(
                    request.run_id,
                    &request.fingerprint,
                    &request,
                    TARGET_FIT_CAPABILITY,
                    TARGET_FIT_TOOL,
                )
                .await?;
            let drafts: Vec<NativeTargetFitDraft> = submission(turn.arguments)?;
            let expected = request
                .rows
                .iter()
                .map(|row| row.row.row_id.as_str())
                .collect::<BTreeSet<_>>();
            let actual = drafts
                .iter()
                .map(|draft| draft.row_id.as_str())
                .collect::<BTreeSet<_>>();
            if drafts.len() != request.rows.len() || actual != expected {
                return Err(invalid_response(
                    "native target-fit submission must cover every requested row exactly once",
                ));
            }
            let assessments = drafts
                .into_iter()
                .map(|draft| NativeTargetFitEvidence::record(&request, draft))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid_response("native target-fit submission is invalid"))?
                .into_iter()
                .map(|evidence| evidence.draft)
                .collect();
            Ok(NativeTargetFitBatchOutput {
                assessments,
                usage: turn.usage,
                metadata: turn.metadata,
            })
        })
    }
}

fn submission<T: DeserializeOwned>(value: Value) -> Result<Vec<T>, QualityEvaluationError> {
    serde_json::from_value::<Submission<T>>(value)
        .map(|submission| submission.assessments)
        .map_err(|_| invalid_response("native reviewer tool arguments are malformed"))
}

fn configuration_error(message: &str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Configuration, message)
}

fn invalid_response(message: &str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::InvalidResponse, message)
}

fn transport_error(message: &str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Transport, message)
}

fn provider_error(message: &str) -> QualityEvaluationError {
    let status = message
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| part.len() == 3)
        .find_map(|part| part.parse::<u16>().ok())
        .filter(|status| (400..=599).contains(status));
    let (kind, summary) = match status {
        Some(401 | 403) => (
            QualityEvaluationErrorKind::Authentication,
            "native reviewer authorization failed",
        ),
        Some(408 | 429) => (
            QualityEvaluationErrorKind::RateLimit,
            "native reviewer was temporarily rejected",
        ),
        Some(500..=599) => (
            QualityEvaluationErrorKind::Provider,
            "native reviewer provider is unavailable",
        ),
        Some(400 | 404) => (
            QualityEvaluationErrorKind::Configuration,
            "native reviewer endpoint or model rejected the request",
        ),
        _ => (
            QualityEvaluationErrorKind::Provider,
            "native reviewer provider failed",
        ),
    };
    QualityEvaluationError::new(kind, summary)
}
