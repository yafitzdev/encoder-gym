//! Pi-backed transport for the two provider-neutral native review passes.
//! Admission remains host-derived in `dataset-quality-core`; this adapter only
//! obtains and normalizes one bounded tool submission per reserved call.

use std::{collections::BTreeSet, sync::Arc};

use agent_runtime_core::{AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentToolResult};
use dataset_quality_core::{
    assessment::{EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence},
    native_assessment::{
        NativeBlindAssessmentDraft, NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest,
        NativeReviewCallOutcome, NativeReviewCallReservation, NativeReviewFailure,
        NativeReviewFailureKind, NativeReviewOperationCategory, NativeReviewRequest,
        NativeReviewResponse, NativeReviewUsage, NativeTargetFitDraft, NativeTargetFitEvidence,
        NativeTargetFitRequest,
    },
    ports::{
        BoxFuture, NativeBlindBatchOutput, NativeReviewStore, NativeSemanticReviewer,
        NativeTargetFitBatchOutput, QualityEvaluationError, QualityEvaluationErrorKind,
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
const REVIEW_INPUT_FRAMING_ALLOWANCE: u64 = 16_384;

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

/// Reservation-first coordinator. It never retries inside one invocation;
/// callers must explicitly opt into the sole replacement attempt after a
/// durably interrupted/unknown first call.
pub struct DurableNativeReviewRunner {
    reviewer: Arc<dyn NativeSemanticReviewer>,
    store: Arc<dyn NativeReviewStore>,
    selection: AgentSelection,
}

impl DurableNativeReviewRunner {
    pub fn new(
        reviewer: Arc<dyn NativeSemanticReviewer>,
        store: Arc<dyn NativeReviewStore>,
        selection: AgentSelection,
    ) -> Result<Self, QualityEvaluationError> {
        if selection.provider.trim().is_empty()
            || selection.model.trim().is_empty()
            || !(1..=65_536).contains(&selection.maximum_output_tokens_per_turn)
        {
            return Err(configuration_error("invalid durable reviewer selection"));
        }
        Ok(Self {
            reviewer,
            store,
            selection,
        })
    }

    pub async fn assess_blind(
        &self,
        request: NativeBlindAssessmentRequest,
        resume_interrupted: bool,
    ) -> Result<NativeBlindBatchOutput, QualityEvaluationError> {
        request
            .validate()
            .map_err(|_| configuration_error("native blind request is invalid"))?;
        let durable_request = NativeReviewRequest::Blind(request.clone());
        let attempt = self
            .next_attempt(&durable_request, resume_interrupted)
            .await?;
        if let Some(output) = attempt.replayed_blind {
            return Ok(output);
        }
        let reservation = self.reservation(
            durable_request,
            NativeReviewOperationCategory::BlindSemanticAssessment,
            attempt.number,
        )?;
        self.store
            .reserve(reservation.clone())
            .await
            .map_err(store_error)?;
        match self.reviewer.assess_blind(request.clone()).await {
            Ok(output) => {
                let response = output
                    .assessments
                    .iter()
                    .cloned()
                    .map(|draft| NativeBlindAssessmentEvidence::record(&request, draft))
                    .collect::<Result<Vec<_>, _>>()
                    .map(NativeReviewResponse::Blind)
                    .map_err(|_| invalid_response("native blind response is invalid"));
                let response = match response {
                    Ok(response) => response,
                    Err(error) => {
                        self.finish_error(reservation, &error).await?;
                        return Err(error);
                    }
                };
                self.finish_success(reservation, output.usage, response)
                    .await?;
                Ok(output)
            }
            Err(error) => {
                self.finish_error(reservation, &error).await?;
                Err(error)
            }
        }
    }

    pub async fn assess_target_fit(
        &self,
        request: NativeTargetFitRequest,
        resume_interrupted: bool,
    ) -> Result<NativeTargetFitBatchOutput, QualityEvaluationError> {
        request
            .validate()
            .map_err(|_| configuration_error("native target-fit request is invalid"))?;
        let durable_request = NativeReviewRequest::TargetFit(request.clone());
        let attempt = self
            .next_attempt(&durable_request, resume_interrupted)
            .await?;
        if let Some(output) = attempt.replayed_target_fit {
            return Ok(output);
        }
        let reservation = self.reservation(
            durable_request,
            NativeReviewOperationCategory::RepairTargetFitAssessment,
            attempt.number,
        )?;
        self.store
            .reserve(reservation.clone())
            .await
            .map_err(store_error)?;
        match self.reviewer.assess_target_fit(request.clone()).await {
            Ok(output) => {
                let response = output
                    .assessments
                    .iter()
                    .cloned()
                    .map(|draft| NativeTargetFitEvidence::record(&request, draft))
                    .collect::<Result<Vec<_>, _>>()
                    .map(NativeReviewResponse::TargetFit)
                    .map_err(|_| invalid_response("native target-fit response is invalid"));
                let response = match response {
                    Ok(response) => response,
                    Err(error) => {
                        self.finish_error(reservation, &error).await?;
                        return Err(error);
                    }
                };
                self.finish_success(reservation, output.usage, response)
                    .await?;
                Ok(output)
            }
            Err(error) => {
                self.finish_error(reservation, &error).await?;
                Err(error)
            }
        }
    }

    async fn next_attempt(
        &self,
        request: &NativeReviewRequest,
        resume_interrupted: bool,
    ) -> Result<Attempt, QualityEvaluationError> {
        let history = self
            .store
            .history(request.clone())
            .await
            .map_err(store_error)?;
        match history.as_slice() {
            [] => Ok(Attempt::new(1)),
            [outcome] if outcome.interrupted && resume_interrupted => Ok(Attempt::new(2)),
            [outcome] if outcome.interrupted => Err(transport_error(
                "native review was interrupted; explicit resume is required",
            )),
            [outcome] => replay(outcome),
            [first, second] if first.interrupted => replay(second),
            _ => Err(invalid_response("native review history is not finite")),
        }
    }

    fn reservation(
        &self,
        request: NativeReviewRequest,
        category: NativeReviewOperationCategory,
        attempt: u32,
    ) -> Result<NativeReviewCallReservation, QualityEvaluationError> {
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|_| configuration_error("native review request could not be encoded"))?
            .len() as u64;
        let reservation = NativeReviewCallReservation {
            id: Uuid::new_v4(),
            category,
            request,
            attempt,
            input_token_ceiling: request_bytes
                .checked_add(REVIEW_INPUT_FRAMING_ALLOWANCE)
                .ok_or_else(|| configuration_error("native review input ceiling overflow"))?,
            output_token_ceiling: u64::from(self.selection.maximum_output_tokens_per_turn),
            cost_ceiling_microusd: self.selection.maximum_cost_microusd_per_turn,
        };
        reservation
            .validate()
            .map_err(|_| configuration_error("native review reservation is invalid"))?;
        Ok(reservation)
    }

    async fn finish_success(
        &self,
        reservation: NativeReviewCallReservation,
        usage: NativeReviewUsage,
        response: NativeReviewResponse,
    ) -> Result<(), QualityEvaluationError> {
        let (response, failure) = if usage.exceeds(
            reservation.input_token_ceiling,
            reservation.output_token_ceiling,
            reservation.cost_ceiling_microusd,
        ) {
            (
                None,
                Some(NativeReviewFailure {
                    kind: NativeReviewFailureKind::Budget,
                    summary: "Native reviewer reported usage above its reservation.".into(),
                }),
            )
        } else {
            (Some(response), None)
        };
        let overrun = failure.is_some();
        self.store
            .finish(NativeReviewCallOutcome {
                reservation,
                usage,
                response,
                failure,
                interrupted: false,
            })
            .await
            .map_err(store_error)?;
        if overrun {
            return Err(invalid_response(
                "native reviewer reported usage above its reservation",
            ));
        }
        Ok(())
    }

    async fn finish_error(
        &self,
        reservation: NativeReviewCallReservation,
        error: &QualityEvaluationError,
    ) -> Result<(), QualityEvaluationError> {
        let interrupted = error.kind == QualityEvaluationErrorKind::Transport;
        let summary = error.message.trim();
        let failure = (!interrupted).then(|| NativeReviewFailure {
            kind: failure_kind(error.kind),
            summary: if summary.is_empty() {
                "Native reviewer failed.".into()
            } else {
                summary.chars().take(400).collect()
            },
        });
        self.store
            .finish(NativeReviewCallOutcome {
                reservation,
                usage: NativeReviewUsage::default(),
                response: None,
                failure,
                interrupted,
            })
            .await
            .map_err(store_error)
    }
}

struct Attempt {
    number: u32,
    replayed_blind: Option<NativeBlindBatchOutput>,
    replayed_target_fit: Option<NativeTargetFitBatchOutput>,
}

impl Attempt {
    fn new(number: u32) -> Self {
        Self {
            number,
            replayed_blind: None,
            replayed_target_fit: None,
        }
    }
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

fn replay(outcome: &NativeReviewCallOutcome) -> Result<Attempt, QualityEvaluationError> {
    outcome
        .validate()
        .map_err(|_| invalid_response("native review history failed integrity validation"))?;
    if outcome.interrupted {
        return Err(transport_error(
            "native review exhausted its sole explicit interrupted-call replacement",
        ));
    }
    if let Some(failure) = &outcome.failure {
        return Err(QualityEvaluationError::new(
            quality_failure_kind(failure.kind),
            failure.summary.clone(),
        ));
    }
    match outcome.response.as_ref() {
        Some(NativeReviewResponse::Blind(values)) => Ok(Attempt {
            number: outcome.reservation.attempt,
            replayed_blind: Some(NativeBlindBatchOutput {
                assessments: values.iter().map(|value| value.draft.clone()).collect(),
                usage: outcome.usage,
                metadata: json!({"replayed": true}),
            }),
            replayed_target_fit: None,
        }),
        Some(NativeReviewResponse::TargetFit(values)) => Ok(Attempt {
            number: outcome.reservation.attempt,
            replayed_blind: None,
            replayed_target_fit: Some(NativeTargetFitBatchOutput {
                assessments: values.iter().map(|value| value.draft.clone()).collect(),
                usage: outcome.usage,
                metadata: json!({"replayed": true}),
            }),
        }),
        None => Err(invalid_response("native review history has no outcome")),
    }
}

fn failure_kind(kind: QualityEvaluationErrorKind) -> NativeReviewFailureKind {
    match kind {
        QualityEvaluationErrorKind::Configuration => NativeReviewFailureKind::Configuration,
        QualityEvaluationErrorKind::Authentication => NativeReviewFailureKind::Authentication,
        QualityEvaluationErrorKind::InvalidResponse => NativeReviewFailureKind::InvalidResponse,
        QualityEvaluationErrorKind::RateLimit => NativeReviewFailureKind::RateLimit,
        QualityEvaluationErrorKind::Transport => NativeReviewFailureKind::Transport,
        QualityEvaluationErrorKind::Provider => NativeReviewFailureKind::Provider,
    }
}

fn quality_failure_kind(kind: NativeReviewFailureKind) -> QualityEvaluationErrorKind {
    match kind {
        NativeReviewFailureKind::Configuration
        | NativeReviewFailureKind::Budget
        | NativeReviewFailureKind::Stopped => QualityEvaluationErrorKind::Configuration,
        NativeReviewFailureKind::Authentication => QualityEvaluationErrorKind::Authentication,
        NativeReviewFailureKind::InvalidResponse => QualityEvaluationErrorKind::InvalidResponse,
        NativeReviewFailureKind::RateLimit => QualityEvaluationErrorKind::RateLimit,
        NativeReviewFailureKind::Transport => QualityEvaluationErrorKind::Transport,
        NativeReviewFailureKind::Provider => QualityEvaluationErrorKind::Provider,
    }
}

fn store_error(error: dataset_quality_core::ports::QualityAdapterError) -> QualityEvaluationError {
    let kind = if error.0.to_ascii_lowercase().contains("budget") {
        QualityEvaluationErrorKind::Configuration
    } else {
        QualityEvaluationErrorKind::Provider
    };
    QualityEvaluationError::new(kind, error.0)
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
