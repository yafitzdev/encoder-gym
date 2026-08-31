use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use agent_runtime_core::{
    AgentAdapterError, AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentSession,
    AgentToolRequest, AgentToolResult, BoxFuture,
};
use chrono::{DateTime, TimeZone, Utc};
use dataset_quality_core::{
    assessment::{
        EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence,
        GeneratorEvaluatorRelationship,
    },
    policy::BasisPoints,
};
use generation_core::jobs::GenerationBackendIdentity;
use generation_supervisor_core::{
    SupervisorError,
    advisor::{
        AdvisorConfiguration, AdvisorModelCall, AdvisorSession, AdvisorSessionState,
        AdvisorToolCall, GenerationQualityDiagnosisBrief,
    },
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, BaselinePolicy, BatchQualityThresholds,
        GenerationQualityContract, GeneratorIdentity, MonitoringPolicy, MonitoringScope,
        PromptRevisionKind, PromptRevisionPolicy, ProtectedPromptField, RevisionApprovalPolicy,
        RowQualityThresholds, SupervisorBudgets,
    },
    decision::DeterministicQualityDecision,
    observation::{
        BatchCounts, BatchQualityObservation, BatchRates, QualityScope, QualityWindowKind,
        ScoreMeans, TextLengthSummary,
    },
    ports::SupervisorAdvisorStore,
    revision::{PromptGuidanceVersion, PromptRevisionProposal, SupervisorDiagnosis},
};
use generation_supervisor_runner::GenerationSupervisorAdvisorRunner;
use serde_json::json;
use uuid::Uuid;

fn bp(value: u16) -> BasisPoints {
    BasisPoints::new(value).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap()
}

fn contract() -> GenerationQualityContract {
    GenerationQualityContract::create(
        Uuid::from_u128(1),
        ArtifactBinding::new(Uuid::from_u128(2), "dataset-fp").unwrap(),
        ArtifactBinding::new(Uuid::from_u128(3), "plan-fp").unwrap(),
        AcceptedCoverageBinding {
            accepted_rows: 0,
            fingerprint: "coverage-fp".into(),
        },
        Some(ArtifactBinding::new(Uuid::from_u128(4), "semantic-fp").unwrap()),
        None,
        None,
        None,
        GeneratorIdentity::create(
            GenerationBackendIdentity {
                name: "writer".into(),
                model: "writer-v1".into(),
                endpoint: None,
            },
            "generation-v1",
            "writer-config",
        )
        .unwrap(),
        EvaluatorIdentity::new(
            "judge",
            "judge-v1",
            "quality-v1",
            "judge-config",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .unwrap(),
        GeneratorEvaluatorRelationship::IndependentBackend,
        RowQualityThresholds {
            minimum_assigned_label_score: bp(7_000),
            minimum_label_margin: bp(1_000),
            minimum_dimension_score: bp(7_000),
            minimum_difficulty_score: None,
            minimum_authenticity_score: None,
            minimum_strategy_score: None,
            maximum_label_leakage_risk: bp(2_000),
            maximum_shortcut_risk: bp(2_000),
            minimum_evaluator_confidence: bp(7_000),
        },
        BatchQualityThresholds {
            minimum_qualified_rate: bp(8_000),
            maximum_borderline_rate: bp(1_000),
            maximum_quarantined_rate: bp(2_000),
            maximum_invalid_rate: bp(1_000),
            maximum_normalized_duplicate_rate: bp(1_000),
            maximum_template_repetition_rate: bp(2_000),
            maximum_qualified_rate_drop: bp(1_000),
            maximum_shortcut_concentration: bp(2_000),
            required_patterns: BTreeSet::new(),
        },
        MonitoringPolicy {
            initial_canary_rows_per_scope: 5,
            revision_canary_rows_per_scope: 5,
            rolling_window_rows_per_scope: 5,
            minimum_evidence_rows_per_scope: 3,
            baseline_minimum_rows_per_scope: 3,
            scope: MonitoringScope::CellAndStrategy,
            baseline_policy: BaselinePolicy::InitialCanary,
            systemic_pause_minimum_scopes: 2,
        },
        SupervisorBudgets {
            maximum_generation_segments: 5,
            maximum_generated_rows: 100,
            maximum_quality_audits: 5,
            maximum_evaluator_requests: 20,
            maximum_evaluator_attempts: 25,
            maximum_prompt_revisions: 2,
            maximum_revision_canaries: 2,
            maximum_pi_model_turns: 4,
            maximum_pi_tool_calls: 10,
            maximum_pi_input_tokens: 10_000,
            maximum_pi_output_tokens: 2_000,
            maximum_retries_per_external_call: 2,
            maximum_duration_seconds: 30,
            maximum_cost_microunits: Some(10_000),
        },
        RevisionApprovalPolicy::ExplicitReview,
        PromptRevisionPolicy {
            maximum_instructions: 4,
            maximum_characters_per_instruction: 300,
            maximum_total_characters: 800,
            allowed_kind: PromptRevisionKind::ReplaceGenerationGuidance,
            protected_fields: BTreeSet::from([
                ProtectedPromptField::SystemPrompt,
                ProtectedPromptField::OutputSchema,
                ProtectedPromptField::TargetLabel,
                ProtectedPromptField::TargetDimensions,
                ProtectedPromptField::ConstructionGraph,
                ProtectedPromptField::SemanticAuthority,
                ProtectedPromptField::QualityThresholds,
                ProtectedPromptField::Budgets,
                ProtectedPromptField::SafetyInstructions,
            ]),
        },
        now(),
    )
    .unwrap()
}

fn brief() -> GenerationQualityDiagnosisBrief {
    let contract = contract();
    let run_id = Uuid::from_u128(10);
    let prompt = PromptGuidanceVersion::initial(
        Uuid::from_u128(11),
        run_id,
        "base-prompt-fp",
        "protected-fields-fp",
        vec!["Write a short support request.".into()],
        now(),
    )
    .unwrap();
    let mut window = BatchQualityObservation {
        id: Uuid::from_u128(12),
        contract_id: contract.id,
        contract_fingerprint: contract.fingerprint.clone(),
        supervisor_run_id: run_id,
        prompt_version_id: prompt.id,
        prompt_version_fingerprint: prompt.fingerprint.clone(),
        kind: QualityWindowKind::InitialCanary,
        sequence: 0,
        scope: QualityScope::CellStrategy {
            cell_key: "billing/easy".into(),
            directive_id: None,
        },
        manifest_id: Uuid::from_u128(13),
        manifest_fingerprint: "protected-manifest-fp".into(),
        counts: BatchCounts {
            attempted: 5,
            structurally_accepted: 5,
            assessed: 5,
            qualified: 1,
            borderline: 0,
            quarantined: 4,
            unassessed: 0,
            invalid: 0,
            exact_duplicates: 0,
            normalized_duplicates: 0,
            template_repetitions: 3,
            shortcut_rows: 0,
        },
        rates: BatchRates {
            qualified: bp(2_000),
            borderline: bp(0),
            quarantined: bp(8_000),
            unassessed: bp(0),
            invalid: bp(0),
            normalized_duplicates: bp(0),
            template_repetitions: bp(6_000),
            shortcut_concentration: bp(0),
        },
        score_means: ScoreMeans {
            assigned_label: Some(bp(6_000)),
            difficulty: None,
            authenticity: None,
            strategy: None,
            label_leakage_risk: Some(bp(500)),
            shortcut_risk: Some(bp(500)),
            confidence: Some(bp(9_000)),
        },
        criterion_failures: BTreeMap::from([(
            generation_supervisor_core::observation::RowCriterionFailure::AssignedLabel,
            4,
        )]),
        issue_code_counts: BTreeMap::new(),
        text_lengths: Some(TextLengthSummary {
            minimum_characters: 20,
            maximum_characters: 35,
            mean_characters: 25,
        }),
        observed_patterns: BTreeSet::new(),
        missing_required_patterns: BTreeSet::new(),
        created_at: now(),
        fingerprint: String::new(),
    };
    window.fingerprint = window.reproduce_fingerprint().unwrap();
    let decision = DeterministicQualityDecision::evaluate(
        Uuid::from_u128(14),
        &contract,
        &window,
        None,
        now(),
    )
    .unwrap();
    GenerationQualityDiagnosisBrief::create(
        Uuid::from_u128(15),
        run_id,
        contract,
        decision,
        window,
        prompt,
        AdvisorConfiguration::create("fake", "scripted", None, "pi-jsonl-v1", "advisor-config-fp")
            .unwrap(),
        now(),
    )
    .unwrap()
}

fn event(event: AgentEvent) -> AgentMessage {
    AgentMessage::Event { event }
}

fn tool(call: &str, name: &str, arguments: serde_json::Value) -> AgentMessage {
    AgentMessage::ToolRequest {
        request: AgentToolRequest {
            external_call_id: call.into(),
            name: name.into(),
            arguments,
        },
    }
}

struct ScriptedRuntime(Mutex<Option<VecDeque<AgentMessage>>>);

impl AgentRuntime for ScriptedRuntime {
    fn start(
        &self,
        request: AgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn AgentSession>, AgentAdapterError>> {
        Box::pin(async move {
            assert_eq!(request.capability_set, "generation_quality_supervisor_v1");
            assert!(!request.initial_prompt.contains("generated_row_id"));
            let messages = self
                .0
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| AgentAdapterError("script already consumed".into()))?;
            Ok(Box::new(ScriptedSession {
                messages,
                tool_results: vec![],
            }) as Box<dyn AgentSession>)
        })
    }
}

struct ScriptedSession {
    messages: VecDeque<AgentMessage>,
    tool_results: Vec<AgentToolResult>,
}

impl AgentSession for ScriptedSession {
    fn next_message(&mut self) -> BoxFuture<'_, Result<AgentMessage, AgentAdapterError>> {
        Box::pin(async move {
            self.messages
                .pop_front()
                .ok_or_else(|| AgentAdapterError("script ended".into()))
        })
    }

    fn send_tool_result(
        &mut self,
        _external_call_id: &str,
        result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), AgentAdapterError>> {
        self.tool_results.push(result);
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

#[derive(Default)]
struct MemoryData {
    briefs: HashMap<Uuid, GenerationQualityDiagnosisBrief>,
    sessions: HashMap<Uuid, AdvisorSession>,
    model_calls: HashMap<Uuid, Vec<AdvisorModelCall>>,
    tool_calls: HashMap<Uuid, Vec<AdvisorToolCall>>,
    diagnoses: HashMap<Uuid, SupervisorDiagnosis>,
    proposals: HashMap<Uuid, PromptRevisionProposal>,
}

#[derive(Default)]
struct MemoryStore(Mutex<MemoryData>);

impl SupervisorAdvisorStore for MemoryStore {
    fn create_session(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &AdvisorSession,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let brief = brief.clone();
        let session = session.clone();
        Box::pin(async move {
            let mut data = self.0.lock().unwrap();
            data.briefs.insert(brief.id, brief);
            data.sessions.insert(session.id, session);
            Ok(())
        })
    }

    fn get_brief(
        &self,
        brief_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Option<GenerationQualityDiagnosisBrief>, SupervisorError>,
    > {
        Box::pin(async move { Ok(self.0.lock().unwrap().briefs.get(&brief_id).cloned()) })
    }

    fn get_session(
        &self,
        session_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Option<AdvisorSession>, SupervisorError>,
    > {
        Box::pin(async move { Ok(self.0.lock().unwrap().sessions.get(&session_id).cloned()) })
    }

    fn save_session(
        &self,
        session: &AdvisorSession,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let session = session.clone();
        Box::pin(async move {
            self.0.lock().unwrap().sessions.insert(session.id, session);
            Ok(())
        })
    }

    fn reserve_model_calls(
        &self,
        calls: &[AdvisorModelCall],
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let calls = calls.to_vec();
        Box::pin(async move {
            let session_id = calls
                .first()
                .ok_or_else(|| SupervisorError::Validation("model reservations empty".into()))?
                .session_id;
            self.0.lock().unwrap().model_calls.insert(session_id, calls);
            Ok(())
        })
    }

    fn list_model_calls(
        &self,
        session_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Vec<AdvisorModelCall>, SupervisorError>,
    > {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .model_calls
                .get(&session_id)
                .cloned()
                .unwrap_or_default())
        })
    }

    fn save_model_call_and_session(
        &self,
        call: &AdvisorModelCall,
        session: &AdvisorSession,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        let session = session.clone();
        Box::pin(async move {
            let mut data = self.0.lock().unwrap();
            let calls = data.model_calls.entry(call.session_id).or_default();
            let target = calls
                .iter_mut()
                .find(|value| value.id == call.id)
                .ok_or_else(|| SupervisorError::Integrity("unknown model call".into()))?;
            *target = call;
            data.sessions.insert(session.id, session);
            Ok(())
        })
    }

    fn save_tool_call(
        &self,
        call: &AdvisorToolCall,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut data = self.0.lock().unwrap();
            let calls = data.tool_calls.entry(call.session_id).or_default();
            if let Some(target) = calls.iter_mut().find(|value| value.id == call.id) {
                *target = call;
            } else {
                calls.push(call);
            }
            Ok(())
        })
    }

    fn save_tool_call_and_session(
        &self,
        call: &AdvisorToolCall,
        session: &AdvisorSession,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        let session = session.clone();
        Box::pin(async move {
            let mut data = self.0.lock().unwrap();
            let calls = data.tool_calls.entry(call.session_id).or_default();
            let target = calls
                .iter_mut()
                .find(|value| value.id == call.id)
                .ok_or_else(|| SupervisorError::Integrity("unknown tool call".into()))?;
            *target = call;
            data.sessions.insert(session.id, session);
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        session_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Vec<AdvisorToolCall>, SupervisorError>,
    > {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .tool_calls
                .get(&session_id)
                .cloned()
                .unwrap_or_default())
        })
    }

    fn save_diagnosis(
        &self,
        session_id: Uuid,
        diagnosis: &SupervisorDiagnosis,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let diagnosis = diagnosis.clone();
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .diagnoses
                .insert(session_id, diagnosis);
            Ok(())
        })
    }

    fn save_proposal(
        &self,
        session_id: Uuid,
        proposal: &PromptRevisionProposal,
    ) -> generation_supervisor_core::ports::BoxFuture<'_, Result<(), SupervisorError>> {
        let proposal = proposal.clone();
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .proposals
                .insert(session_id, proposal);
            Ok(())
        })
    }

    fn latest_diagnosis(
        &self,
        session_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Option<SupervisorDiagnosis>, SupervisorError>,
    > {
        Box::pin(async move { Ok(self.0.lock().unwrap().diagnoses.get(&session_id).cloned()) })
    }

    fn latest_proposal(
        &self,
        session_id: Uuid,
    ) -> generation_supervisor_core::ports::BoxFuture<
        '_,
        Result<Option<PromptRevisionProposal>, SupervisorError>,
    > {
        Box::pin(async move { Ok(self.0.lock().unwrap().proposals.get(&session_id).cloned()) })
    }
}

#[tokio::test]
async fn scripted_pi_can_only_submit_a_valid_guidance_patch_for_review() {
    let script = VecDeque::from([
        event(AgentEvent::ModelTurnStarted { sequence: 1 }),
        event(AgentEvent::ModelTurnCompleted {
            sequence: 1,
            input_tokens: 200,
            output_tokens: 80,
            cost_microusd: 100,
        }),
        event(AgentEvent::AgentText {
            text: "Inspect evidence, preview a scoped patch, submit, and finish.".into(),
        }),
        tool("c1", "inspect_quality_contract", json!({})),
        tool("c2", "inspect_quality_window", json!({})),
        tool("c3", "inspect_failure_breakdown", json!({})),
        tool("c4", "inspect_current_prompt_guidance", json!({})),
        tool(
            "c5",
            "preview_prompt_revision",
            json!({
                "replacement_guidance": ["Vary phrasing and add concrete situational detail without naming the label."],
                "expected_improvements": [{"metric": "qualified_rate", "minimum_delta_basis_points": 1500}]
            }),
        ),
        tool(
            "c6",
            "submit_prompt_revision",
            json!({
                "cause": "repetition_mode_collapse",
                "summary": "The current guidance encourages short templates with too little variation.",
                "replacement_guidance": ["Vary phrasing and add concrete situational detail without naming the label."],
                "expected_improvements": [{"metric": "qualified_rate", "minimum_delta_basis_points": 1500}]
            }),
        ),
        tool(
            "c7",
            "finish_supervision",
            json!({
                "outcome": "revision_submitted",
                "summary": "A bounded guidance-only repair is ready for operator review."
            }),
        ),
        AgentMessage::Completed,
    ]);
    let runtime = Arc::new(ScriptedRuntime(Mutex::new(Some(script))));
    let store = Arc::new(MemoryStore::default());
    let runner = GenerationSupervisorAdvisorRunner::new(runtime, store.clone());
    let session = runner.queue(brief()).await.unwrap();
    let outcome = runner.run(session.id).await.unwrap();

    assert_eq!(outcome.session.state, AdvisorSessionState::AwaitingReview);
    assert_eq!(outcome.session.usage.model_turns, 1);
    assert_eq!(outcome.session.usage.tool_calls, 7);
    assert!(outcome.diagnosis.is_some());
    let proposal = outcome.proposal.unwrap();
    assert_eq!(proposal.affected_scopes.len(), 1);
    assert_eq!(
        proposal.protected_field_proof.before_fingerprint,
        proposal.protected_field_proof.after_fingerprint
    );
    let data = store.0.lock().unwrap();
    assert_eq!(data.model_calls[&session.id].len(), 4);
    assert_eq!(data.tool_calls[&session.id].len(), 7);
}

#[tokio::test]
async fn arbitrary_capability_is_rejected_and_persisted_without_execution() {
    let script = VecDeque::from([
        tool("hostile", "shell", json!({"command": "whoami"})),
        AgentMessage::Completed,
    ]);
    let runtime = Arc::new(ScriptedRuntime(Mutex::new(Some(script))));
    let store = Arc::new(MemoryStore::default());
    let runner = GenerationSupervisorAdvisorRunner::new(runtime, store.clone());
    let session = runner.queue(brief()).await.unwrap();
    let outcome = runner.run(session.id).await.unwrap();
    assert_eq!(outcome.session.state, AdvisorSessionState::Failed);
    assert!(outcome.proposal.is_none());
    let calls = &store.0.lock().unwrap().tool_calls[&session.id];
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].state,
        generation_supervisor_core::advisor::AdvisorCallState::Failed
    );
}
