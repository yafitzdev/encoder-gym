use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use agent_runtime_core::{
    AgentAdapterError, AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentSession,
    AgentToolRequest, AgentToolResult, BoxFuture,
};
use dataset_quality_core::{
    native_assessment::{
        NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest, NativeBlindContext,
        NativeBlindRow, NativeCandidateSemantics, NativeMetricDirection, NativeRepairStrategy,
        NativeTargetBrief, NativeTargetFitEvidence, NativeTargetFitRequest,
    },
    ports::NativeSemanticReviewer,
};
use encoder_optimization_runner::{AgentSelection, semantic_review::PiNativeSemanticReviewer};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Default)]
struct Runtime {
    requests: Mutex<Vec<AgentRequest>>,
    omit_tool: bool,
}

impl AgentRuntime for Runtime {
    fn start(
        &self,
        request: AgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn AgentSession>, AgentAdapterError>> {
        self.requests.lock().unwrap().push(request.clone());
        let omit_tool = self.omit_tool;
        Box::pin(async move {
            let arguments = match request.capability_set.as_str() {
                "encoder_optimization_native_blind_v1" => {
                    let parsed: NativeBlindAssessmentRequest =
                        serde_json::from_str(&request.initial_prompt).unwrap();
                    json!({
                        "assessments": parsed.rows.iter().map(|row| json!({
                            "rowId": row.row_id,
                            "rowFingerprint": row.row_fingerprint,
                            "requestFingerprint": parsed.fingerprint,
                            "supportedCandidateIds": ["read"],
                            "ambiguous": false,
                            "contextConsistent": true,
                            "issueCodes": [],
                            "rationale": "The question asks to retrieve the saved object."
                        })).collect::<Vec<_>>()
                    })
                }
                "encoder_optimization_native_target_fit_v1" => {
                    let parsed: NativeTargetFitRequest =
                        serde_json::from_str(&request.initial_prompt).unwrap();
                    json!({
                        "assessments": parsed.rows.iter().map(|row| json!({
                            "rowId": row.row.row_id,
                            "rowFingerprint": row.row.row_fingerprint,
                            "requestFingerprint": parsed.fingerprint,
                            "blindAssessmentFingerprint": row.blind_assessment_fingerprint,
                            "targetFits": true,
                            "issueCodes": [],
                            "rationale": "The question is a label-preserving variant in the target cluster."
                        })).collect::<Vec<_>>()
                    })
                }
                other => panic!("unexpected capability {other}"),
            };
            let tool = match request.capability_set.as_str() {
                "encoder_optimization_native_blind_v1" => "submit_native_blind_assessments",
                _ => "submit_native_target_fit_assessments",
            };
            let mut messages = VecDeque::from([
                AgentMessage::Event {
                    event: AgentEvent::ModelTurnCompleted {
                        sequence: 1,
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_microusd: 0,
                    },
                },
                AgentMessage::ToolRequest {
                    request: AgentToolRequest {
                        external_call_id: "review-call".into(),
                        name: tool.into(),
                        arguments,
                    },
                },
                AgentMessage::Completed,
            ]);
            if omit_tool {
                messages.remove(1);
            }
            Ok(Box::new(Session {
                messages,
                acknowledgements: Vec::new(),
            }) as Box<dyn AgentSession>)
        })
    }
}

struct Session {
    messages: VecDeque<AgentMessage>,
    acknowledgements: Vec<(String, AgentToolResult)>,
}

impl AgentSession for Session {
    fn next_message(&mut self) -> BoxFuture<'_, Result<AgentMessage, AgentAdapterError>> {
        Box::pin(async move {
            self.messages
                .pop_front()
                .ok_or_else(|| AgentAdapterError("fixture exhausted".into()))
        })
    }

    fn send_tool_result(
        &mut self,
        external_call_id: &str,
        result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), AgentAdapterError>> {
        self.acknowledgements
            .push((external_call_id.into(), result));
        Box::pin(async { Ok(()) })
    }

    fn send_tool_error(
        &mut self,
        _external_call_id: &str,
        _message: &str,
    ) -> BoxFuture<'_, Result<(), AgentAdapterError>> {
        Box::pin(async { Ok(()) })
    }

    fn cancel(&mut self, _run_id: Uuid) -> BoxFuture<'_, Result<(), AgentAdapterError>> {
        Box::pin(async { Ok(()) })
    }
}

fn selection() -> AgentSelection {
    AgentSelection {
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        api_key_env: None,
        maximum_output_tokens_per_turn: 2_048,
        maximum_cost_microusd_per_turn: 50_000,
        runtime_cost_is_known: false,
    }
}

fn row() -> NativeBlindRow {
    NativeBlindRow {
        row_id: "generated-row-1".into(),
        row_fingerprint: format!("sha256:{}", "1".repeat(64)),
        question: "Show me the saved invoice.".into(),
        context: NativeBlindContext::new(BTreeMap::from([(
            "task_kind".into(),
            Value::String("retrieval".into()),
        )]))
        .unwrap(),
        candidates: vec![
            NativeCandidateSemantics::new(
                "read",
                "Retrieve a saved invoice",
                vec!["read saved invoices".into()],
            )
            .unwrap(),
            NativeCandidateSemantics::new(
                "write",
                "Modify a saved invoice",
                vec!["change saved invoices".into()],
            )
            .unwrap(),
        ],
    }
}

fn blind_request(reviewer: &PiNativeSemanticReviewer) -> NativeBlindAssessmentRequest {
    NativeBlindAssessmentRequest::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        1,
        reviewer.native_identity().fingerprint,
        vec![row()],
    )
    .unwrap()
}

#[tokio::test]
async fn pi_reviewer_runs_separate_blind_and_target_fit_tools() {
    let runtime = Arc::new(Runtime::default());
    let reviewer = PiNativeSemanticReviewer::new(runtime.clone(), selection()).unwrap();
    let blind = blind_request(&reviewer);

    let blind_output = reviewer.assess_blind(blind.clone()).await.unwrap();

    assert_eq!(blind_output.assessments.len(), 1);
    assert_eq!(
        blind_output.assessments[0].supported_candidate_ids,
        ["read"]
    );
    assert_eq!(blind_output.usage.input_tokens, None);
    assert_eq!(blind_output.usage.output_tokens, None);
    assert_eq!(blind_output.usage.cost_microusd, None);
    let evidence =
        NativeBlindAssessmentEvidence::record(&blind, blind_output.assessments[0].clone()).unwrap();
    let target = NativeTargetBrief::new(
        NativeRepairStrategy::LabelPreservingVariants,
        vec!["intent:read".into()],
        "development_recall",
        NativeMetricDirection::Increase,
    )
    .unwrap();
    let target_request = NativeTargetFitRequest::new(
        Uuid::new_v4(),
        &blind,
        reviewer.native_identity().fingerprint,
        vec![(evidence, target)],
    )
    .unwrap();

    let fit_output = reviewer
        .assess_target_fit(target_request.clone())
        .await
        .unwrap();

    assert_eq!(fit_output.assessments.len(), 1);
    assert!(fit_output.assessments[0].target_fits);
    NativeTargetFitEvidence::record(&target_request, fit_output.assessments[0].clone()).unwrap();
    let requests = runtime.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].capability_set,
        "encoder_optimization_native_blind_v1"
    );
    assert_eq!(
        requests[1].capability_set,
        "encoder_optimization_native_target_fit_v1"
    );
    assert_eq!(requests[0].max_model_turns, 1);
    assert_eq!(requests[1].max_model_turns, 1);
}

#[tokio::test]
async fn reviewer_fails_closed_when_provider_omits_required_tool() {
    let runtime = Arc::new(Runtime {
        omit_tool: true,
        ..Runtime::default()
    });
    let reviewer = PiNativeSemanticReviewer::new(runtime, selection()).unwrap();
    let error = reviewer
        .assess_blind(blind_request(&reviewer))
        .await
        .unwrap_err();

    assert_eq!(
        error.kind,
        dataset_quality_core::ports::QualityEvaluationErrorKind::InvalidResponse
    );
    assert!(error.message.contains("omitted the required tool"));
}
