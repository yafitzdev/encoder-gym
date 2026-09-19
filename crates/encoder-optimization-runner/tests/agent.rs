use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_runtime_core::{
    AgentAdapterError, AgentEvent, AgentMessage, AgentRequest, AgentRuntime, AgentSession,
    AgentToolRequest, AgentToolResult,
};
use encoder_optimization_core::{
    OptimizationError,
    agent::{
        AgentAnalysisScope, AgentCallReservation, AgentTurnRecord, InspectionItem, InspectionPage,
        InspectionSelection,
    },
    fingerprint,
    ports::{BoxFuture, OptimizationAgentStore, OptimizationInspection},
    repair_strategy::{
        AdditionCount, AnchorAllocation, MetricDirection, RepairOperation, RepairPlan,
        RepairPlanningAnchor, RepairPlanningCluster, RepairPlanningContext, RepairTarget,
        TargetMetric, compile_repair_plan,
    },
};
use encoder_optimization_runner::{AgentSelection, OptimizationAgent};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Default)]
struct Store {
    pending: Mutex<Option<AgentCallReservation>>,
    history: Mutex<Vec<AgentTurnRecord>>,
    captions: Mutex<Vec<String>>,
    stopped: AtomicBool,
}

impl OptimizationAgentStore for Store {
    fn history(&self, _scope: AgentAnalysisScope) -> BoxFuture<'_, Vec<AgentTurnRecord>> {
        Box::pin(async { Ok(self.history.lock().unwrap().clone()) })
    }
    fn reserve(&self, _scope: AgentAnalysisScope, call: AgentCallReservation) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut pending = self.pending.lock().unwrap();
            assert!(
                pending.is_none(),
                "cannot dispatch another call while one is pending"
            );
            assert_eq!(
                call.sequence as usize,
                self.history.lock().unwrap().len() + 1
            );
            *pending = Some(call);
            Ok(())
        })
    }
    fn finish(&self, _scope: AgentAnalysisScope, record: AgentTurnRecord) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            assert_eq!(
                self.pending.lock().unwrap().take(),
                Some(record.call.clone())
            );
            self.history.lock().unwrap().push(record);
            Ok(())
        })
    }
    fn stopped(&self, _run: Uuid) -> BoxFuture<'_, bool> {
        Box::pin(async { Ok(self.stopped.load(Ordering::Relaxed)) })
    }
    fn public_explanation(
        &self,
        _scope: AgentAnalysisScope,
        _call: Uuid,
        text: String,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.captions.lock().unwrap().push(text);
            Ok(())
        })
    }
}

struct Inspection;

fn page(id: &str, content: Value) -> InspectionPage {
    InspectionPage {
        items: vec![InspectionItem {
            id: id.into(),
            fingerprint: fingerprint(&content).unwrap(),
            content,
        }],
        next_offset: None,
        selection: None,
    }
}

fn v3_context() -> RepairPlanningContext {
    RepairPlanningContext {
        dataset_rows: 40,
        remaining_row_changes: 2,
        clusters: BTreeMap::from([(
            "dataset-cluster-1".into(),
            RepairPlanningCluster {
                training_rows: 8,
                metrics: BTreeSet::from(["recall_at_1".into()]),
            },
        )]),
        anchors: BTreeMap::from([(
            "row-1".into(),
            RepairPlanningAnchor {
                fingerprint: fingerprint(&"v3-anchor-row-1").unwrap(),
                cluster_keys: BTreeSet::from(["dataset-cluster-1".into()]),
                native_context_fingerprint: fingerprint(&"native-context-1").unwrap(),
                native_model_input_fingerprint: fingerprint(&"native-input-1").unwrap(),
                label_fingerprint: fingerprint(&"native-label-1").unwrap(),
                exact_duplicate_group_id: None,
            },
        )]),
        evidence_ids: BTreeSet::from(["dataset-cluster-1".into()]),
    }
}

fn v3_plan() -> RepairPlan {
    let context = v3_context();
    RepairPlan {
        schema_version: 3,
        summary: "Test one additional read-routing variant across inspected native context.".into(),
        stop: false,
        targets: vec![RepairTarget {
            target_id: "read-routing-repair".into(),
            cluster_keys: vec!["dataset-cluster-1".into()],
            evidence_ids: vec!["dataset-cluster-1".into()],
            hypothesis: "One more precise read-only question may reduce the observed miss rate."
                .into(),
            evidence_limitations: "Saved development evidence is diagnostic, not causal proof."
                .into(),
            intended_failure_pattern: "Read-only requests routed to another capability.".into(),
            alternative_explanation:
                "The weakness may come from model capacity rather than coverage.".into(),
            operation: RepairOperation::LabelPreservingVariants {
                count: AdditionCount::AbsoluteRows { desired_rows: 1 },
                allocation_rationale: "Use the single returned native context for a minimal test."
                    .into(),
                anchors: vec![AnchorAllocation {
                    row_id: "row-1".into(),
                    row_fingerprint: context.anchors["row-1"].fingerprint.clone(),
                    additions: 1,
                }],
            },
            target_metric: TargetMetric {
                name: "recall_at_1".into(),
                direction: MetricDirection::Increase,
            },
        }],
    }
}

impl OptimizationInspection for Inspection {
    fn development_failures(
        &self,
        _scope: AgentAnalysisScope,
        _offset: u64,
        _limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async {
            Ok(page(
                "failure-1",
                json!({"error":"missing read-only routing context","report":"development-only-report"}),
            ))
        })
    }
    fn training_rows(
        &self,
        _scope: AgentAnalysisScope,
        _offset: u64,
        _limit: u32,
        _query: Option<String>,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async {
            Ok(page(
                "row-1",
                json!({"question":"ambiguous route","task":"read"}),
            ))
        })
    }
    fn dataset_landscape(
        &self,
        _scope: AgentAnalysisScope,
        _offset: u64,
        _limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async {
            Ok(page(
                "dataset-cluster-1",
                json!({"kind":"dataset_cluster","cluster":{"dimension":"expected_capability","value":"read"},"training":{"rows":8,"sharePpm":200000},"development":[{"estimatedTop1Errors":5,"supportSharePpm":350000,"coverageGapPpm":150000}]}),
            ))
        })
    }
    fn dataset_cluster_rows(
        &self,
        _scope: AgentAnalysisScope,
        cluster_ids: Vec<String>,
        _examples_per_cluster: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            assert_eq!(cluster_ids, vec!["dataset-cluster-1"]);
            Ok(page(
                "row-1",
                json!({"question":"ambiguous route","selectedForClusters":cluster_ids}),
            ))
        })
    }

    fn dataset_investigation_rows(
        &self,
        _scope: AgentAnalysisScope,
        cluster_ids: Vec<String>,
        cursor: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            assert_eq!(cluster_ids, vec!["dataset-cluster-1"]);
            assert_eq!(cursor, 0);
            assert_eq!(limit, 1);
            let context = v3_context();
            let content = json!({
                "question": "ambiguous route",
                "selectedForClusters": cluster_ids,
                "investigation": {
                    "anchorFingerprint": context.anchors["row-1"].fingerprint,
                    "selectionReason": "distinct_native_context"
                }
            });
            Ok(InspectionPage {
                items: vec![InspectionItem {
                    id: "row-1".into(),
                    fingerprint: fingerprint(&content).unwrap(),
                    content,
                }],
                next_offset: None,
                selection: Some(InspectionSelection {
                    method: "distinct_native_context_then_content_fingerprint".into(),
                    cursor: 0,
                    selected_count: 1,
                    total_eligible_count: 1,
                    total_inspectable_count: 1,
                }),
            })
        })
    }

    fn repair_planning_context(
        &self,
        _scope: AgentAnalysisScope,
        inspected_cluster_ids: Vec<String>,
        inspected_row_ids: Vec<String>,
    ) -> BoxFuture<'_, RepairPlanningContext> {
        Box::pin(async move {
            assert_eq!(inspected_cluster_ids, vec!["dataset-cluster-1"]);
            assert_eq!(inspected_row_ids, vec!["row-1"]);
            Ok(v3_context())
        })
    }
}

struct Runtime {
    store: Arc<Store>,
    turns: Mutex<VecDeque<Vec<AgentMessage>>>,
    requests: Mutex<Vec<AgentRequest>>,
}
impl AgentRuntime for Runtime {
    fn start(
        &self,
        request: AgentRequest,
    ) -> agent_runtime_core::BoxFuture<'_, Result<Box<dyn AgentSession>, AgentAdapterError>> {
        Box::pin(async move {
            let call = self
                .store
                .pending
                .lock()
                .unwrap()
                .clone()
                .expect("durable reservation precedes runtime.start");
            assert_eq!(call.request_fingerprint, fingerprint(&request).unwrap());
            assert_eq!(request.max_model_turns, 1);
            assert_eq!(request.model, "selected-flash");
            self.requests.lock().unwrap().push(request);
            let messages = self
                .turns
                .lock()
                .unwrap()
                .pop_front()
                .expect("no extra model turns")
                .into();
            Ok(Box::new(Session { messages }) as Box<dyn AgentSession>)
        })
    }
}

struct Session {
    messages: VecDeque<AgentMessage>,
}
impl AgentSession for Session {
    fn next_message(
        &mut self,
    ) -> agent_runtime_core::BoxFuture<'_, Result<AgentMessage, AgentAdapterError>> {
        Box::pin(async { Ok(self.messages.pop_front().unwrap()) })
    }
    fn send_tool_result(
        &mut self,
        _id: &str,
        _result: AgentToolResult,
    ) -> agent_runtime_core::BoxFuture<'_, Result<(), AgentAdapterError>> {
        Box::pin(async { Ok(()) })
    }
    fn send_tool_error(
        &mut self,
        _id: &str,
        _message: &str,
    ) -> agent_runtime_core::BoxFuture<'_, Result<(), AgentAdapterError>> {
        Box::pin(async { Ok(()) })
    }
    fn cancel(
        &mut self,
        _run: Uuid,
    ) -> agent_runtime_core::BoxFuture<'_, Result<(), AgentAdapterError>> {
        Box::pin(async { Ok(()) })
    }
}

fn turn(name: &str, arguments: Value, text: &str) -> Vec<AgentMessage> {
    vec![
        AgentMessage::Event {
            event: AgentEvent::ModelTurnStarted { sequence: 1 },
        },
        AgentMessage::Event {
            event: AgentEvent::AgentText { text: text.into() },
        },
        AgentMessage::ToolRequest {
            request: AgentToolRequest {
                external_call_id: Uuid::new_v4().to_string(),
                name: name.into(),
                arguments,
            },
        },
        AgentMessage::Event {
            event: AgentEvent::ModelTurnCompleted {
                sequence: 1,
                input_tokens: 70,
                output_tokens: 20,
                cost_microusd: 0,
            },
        },
        AgentMessage::Completed,
    ]
}

fn scope() -> AgentAnalysisScope {
    AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: Uuid::new_v4(),
        dataset_fingerprint: fingerprint(&"training-members").unwrap(),
        development_evidence_fingerprint: fingerprint(&"development-result").unwrap(),
        objective: "Improve read-only routing".into(),
        analysis_protocol: 1,
        maximum_turns: 4,
        maximum_row_changes: 2,
    }
}

fn proposal() -> Value {
    json!({"summary":"The failure points to ambiguous read-only examples.","stop":false,
        "removals":[{"rowId":"row-1","reason":"The existing label conflicts with the intended task.","evidenceIds":["failure-1"]}],
        "additions":[{"templateRowId":"row-1","instruction":"Generate an unambiguous read-only routing example.","count":1,"evidenceIds":["failure-1"]}]})
}

fn setup(turns: Vec<Vec<AgentMessage>>) -> (OptimizationAgent, Arc<Store>, Arc<Runtime>) {
    setup_with_inspection(turns, Arc::new(Inspection))
}

fn setup_with_inspection(
    turns: Vec<Vec<AgentMessage>>,
    inspection: Arc<dyn OptimizationInspection>,
) -> (OptimizationAgent, Arc<Store>, Arc<Runtime>) {
    let store = Arc::new(Store::default());
    let runtime = Arc::new(Runtime {
        store: store.clone(),
        turns: Mutex::new(turns.into()),
        requests: Mutex::default(),
    });
    let agent = OptimizationAgent::new(
        runtime.clone(),
        store.clone(),
        inspection,
        AgentSelection {
            provider: "project-provider".into(),
            model: "selected-flash".into(),
            api_key_env: None,
            maximum_output_tokens_per_turn: 512,
            maximum_cost_microusd_per_turn: 1000,
            runtime_cost_is_known: false,
        },
    );
    (agent, store, runtime)
}

struct LargeInspection;

impl OptimizationInspection for LargeInspection {
    fn development_failures(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Inspection.development_failures(scope, offset, limit)
    }

    fn training_rows(
        &self,
        _scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
        _query: Option<String>,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            Ok(InspectionPage {
                items: (offset..20)
                    .take(limit as usize)
                    .map(|index| {
                        let content =
                            json!({"question":format!("row-{index}"),"context":"x".repeat(5000)});
                        InspectionItem {
                            id: format!("row-{index}"),
                            fingerprint: fingerprint(&content).unwrap(),
                            content,
                        }
                    })
                    .collect(),
                next_offset: (offset + u64::from(limit) < 20).then_some(offset + u64::from(limit)),
                selection: None,
            })
        })
    }
}

#[tokio::test]
async fn large_inspection_is_paged_before_recording_and_correction_preserves_the_bound() {
    let mut turns = inspected_turns();
    let mut invalid = proposal();
    invalid["removals"][0]["rowId"] = "row-9".into();
    turns.push(turn(
        "propose_dataset_edits",
        invalid,
        "Try a row not in the returned page.",
    ));
    turns.push(turn(
        "propose_dataset_edits",
        proposal(),
        "Use the returned row.",
    ));
    let (agent, store, runtime) = setup_with_inspection(turns, Arc::new(LargeInspection));
    let scope = scope();
    let accepted = agent.analyze(scope.clone()).await.unwrap();
    {
        let history = store.history.lock().unwrap();
        let page: InspectionPage =
            serde_json::from_value(history[1].tools[0].result.clone()).unwrap();
        assert_eq!(page.items.len(), 6);
        assert_eq!(page.next_offset, Some(6));
        assert_eq!(page.items[1].content["context"], "x".repeat(5000));
        assert!(
            history[2].tools[0].failed,
            "unreturned rows must not become inspected"
        );
        assert!(
            history
                .iter()
                .all(|record| record.call.input_token_ceiling < 50_000)
        );
        assert!(history.iter().all(|record| record.validate().is_ok()));
    }
    let saved = store.history.lock().unwrap().clone();
    assert_eq!(agent.analyze(scope).await.unwrap(), accepted);
    assert_eq!(
        *store.history.lock().unwrap(),
        saved,
        "replay must preserve immutable history"
    );
    assert_eq!(runtime.requests.lock().unwrap().len(), 4);
}

fn inspected_turns() -> Vec<Vec<AgentMessage>> {
    vec![
        turn(
            "inspect_development_failures",
            json!({"offset":0,"limit":10}),
            "I will inspect development failures.",
        ),
        turn(
            "inspect_training_rows",
            json!({"offset":0,"limit":10}),
            "The failures suggest inspecting read-only examples.",
        ),
    ]
}

#[tokio::test]
async fn evidence_and_row_inspection_drive_real_tool_proposal_and_completed_work_is_reused() {
    let mut turns = inspected_turns();
    turns.push(turn(
        "propose_dataset_edits",
        proposal(),
        "Remove the ambiguous row and generate a precise replacement.",
    ));
    let (agent, store, runtime) = setup(turns);
    let scope = scope();
    let result = agent.analyze(scope.clone()).await.unwrap();
    assert_eq!(result.removals[0].row_id, "row-1");
    assert_eq!(result.additions[0].count, 1);
    {
        let requests = runtime.requests.lock().unwrap();
        assert!(
            requests[1]
                .initial_prompt
                .contains("missing read-only routing context")
        );
        assert!(requests[2].initial_prompt.contains("ambiguous route"));
        assert_eq!(
            requests[2].capability_set,
            "encoder_optimization_proposal_v1"
        );
        assert!(requests[2].initial_prompt.contains("\"proposalOnly\":true"));
    }
    assert_eq!(store.captions.lock().unwrap().len(), 4);
    assert!(store.captions.lock().unwrap().contains(&result.summary));
    assert!(
        store
            .history
            .lock()
            .unwrap()
            .iter()
            .all(|record| record.usage.cost_microusd.is_none())
    );
    assert_eq!(agent.analyze(scope).await.unwrap(), result);
    assert_eq!(
        runtime.requests.lock().unwrap().len(),
        3,
        "resume must not repeat completed model calls"
    );
}

#[tokio::test]
async fn v2_requires_landscape_then_cluster_rows_and_uses_cluster_evidence() {
    let turns = vec![
        turn(
            "inspect_dataset_landscape",
            json!({"offset":0,"limit":20}),
            "Compare weak evaluation dimensions with training coverage.",
        ),
        turn(
            "inspect_dataset_clusters",
            json!({"clusterIds":["dataset-cluster-1"],"examplesPerCluster":2}),
            "Inspect representative rows for the underrepresented cluster.",
        ),
        turn(
            "propose_dataset_edits",
            json!({"summary":"Increase read coverage by one row as a bounded test.","stop":false,
                "removals":[],
                "additions":[{"templateRowId":"row-1","instruction":"Generate one precise read-only route for the weak cluster.","count":1,"evidenceIds":["dataset-cluster-1"]}]}),
            "Submit the bounded coverage shift.",
        ),
    ];
    let (agent, store, runtime) = setup(turns);
    let mut scope = scope();
    scope.analysis_protocol = 2;
    let proposal = agent.analyze(scope).await.unwrap();
    assert_eq!(proposal.additions[0].template_row_id, "row-1");
    assert_eq!(
        proposal.additions[0].evidence_ids,
        vec!["dataset-cluster-1"]
    );
    let requests = runtime.requests.lock().unwrap();
    assert_eq!(requests[0].capability_set, "encoder_optimization_v2");
    assert_eq!(requests[1].capability_set, "encoder_optimization_v2");
    assert_eq!(
        requests[2].capability_set,
        "encoder_optimization_proposal_v2"
    );
    assert!(requests[1].initial_prompt.contains("coverageGapPpm"));
    assert!(
        store
            .history
            .lock()
            .unwrap()
            .iter()
            .all(|record| record.validate().is_ok())
    );
}

#[tokio::test]
async fn v3_requires_four_persisted_stages_and_replays_the_exact_preview() {
    let plan = v3_plan();
    let preview = compile_repair_plan(&plan, &v3_context())
        .unwrap()
        .preview
        .unwrap();
    let turns = vec![
        turn(
            "inspect_dataset_landscape",
            json!({"offset":0,"limit":20}),
            "Inspect the ranked native inventory before selecting a repair target.",
        ),
        turn(
            "inspect_dataset_clusters",
            json!({"clusterIds":["dataset-cluster-1"],"cursor":0,"limit":1}),
            "Inspect one context-diverse anchor from the weak cluster.",
        ),
        turn(
            "preview_repair_plan",
            serde_json::to_value(&plan).unwrap(),
            "Preview the exact bounded intervention before submission.",
        ),
        turn(
            "submit_repair_plan",
            json!({"plan":plan,"previewFingerprint":preview.fingerprint}),
            "Submit the exact feasible preview in the reserved turn.",
        ),
    ];
    let (agent, store, runtime) = setup(turns);
    let mut scope = scope();
    scope.analysis_protocol = 3;
    let proposal = agent.analyze(scope.clone()).await.unwrap();
    assert_eq!(proposal.additions.len(), 1);
    assert_eq!(proposal.additions[0].template_row_id, "row-1");
    assert_eq!(proposal.additions[0].count, 1);
    assert!(
        proposal.additions[0]
            .instruction
            .contains("label_preserving_variants")
    );

    let saved = store.history.lock().unwrap().clone();
    assert_eq!(saved.len(), 4);
    assert!(saved.iter().all(|record| record.validate().is_ok()));
    {
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.capability_set.as_str())
                .collect::<Vec<_>>(),
            vec![
                "encoder_optimization_v3",
                "encoder_optimization_v3",
                "encoder_optimization_v3",
                "encoder_optimization_proposal_v3",
            ]
        );
        assert!(
            requests[..3]
                .iter()
                .all(|request| request.initial_prompt.contains("\"proposalOnly\":false"))
        );
        assert!(requests[3].initial_prompt.contains("\"proposalOnly\":true"));
    }

    assert_eq!(agent.analyze(scope).await.unwrap(), proposal);
    assert_eq!(*store.history.lock().unwrap(), saved);
    assert_eq!(runtime.requests.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn invented_row_or_evidence_is_rejected_and_the_agent_receives_the_validation_error() {
    let mut turns = inspected_turns();
    let mut invalid = proposal();
    invalid["removals"][0]["rowId"] = "not-inspected".into();
    turns.push(turn(
        "propose_dataset_edits",
        invalid,
        "Try removing a guessed row.",
    ));
    turns.push(turn(
        "propose_dataset_edits",
        proposal(),
        "Use the inspected identity instead.",
    ));
    let (agent, store, runtime) = setup(turns);
    agent.analyze(scope()).await.unwrap();
    assert!(store.history.lock().unwrap()[2].tools[0].failed);
    assert!(
        runtime.requests.lock().unwrap()[3]
            .initial_prompt
            .contains("uninspected training row")
    );
}

#[tokio::test]
async fn proposals_receive_exact_reference_contract_and_actionable_mixed_id_rejection() {
    let mut turns = inspected_turns();
    let mut invalid = proposal();
    invalid["additions"][0]["evidenceIds"] = json!(["report-id", "failure-1", "row-1"]);
    turns.push(turn(
        "propose_dataset_edits",
        invalid.clone(),
        "Attempt mixed reference namespaces.",
    ));
    turns.push(turn(
        "propose_dataset_edits",
        proposal(),
        "Use only the failure-item ID.",
    ));
    let (agent, store, runtime) = setup(turns);
    agent.analyze(scope()).await.unwrap();
    let requests = runtime.requests.lock().unwrap();
    let first: Value = serde_json::from_str(&requests[2].initial_prompt).unwrap();
    assert_eq!(first["proposalRequirements"]["maximumRowChanges"], 2);
    assert_eq!(
        first["proposalRequirements"]["trainingRowIds"],
        json!(["row-1"])
    );
    assert_eq!(
        first["proposalRequirements"]["developmentEvidenceIds"],
        json!(["failure-1"])
    );
    let correction: Value = serde_json::from_str(&requests[3].initial_prompt).unwrap();
    let error = correction["previousTurns"][2]["tools"][0]["result"]["error"]
        .as_str()
        .unwrap();
    assert!(error.contains("additions[0].evidenceIds"));
    assert!(error.contains("[0, 2]"));
    assert!(error.contains("failure-1"));
    assert_eq!(store.history.lock().unwrap()[2].tools[0].arguments, invalid);
}

#[tokio::test]
async fn malformed_proposals_are_recorded_and_corrected_within_reserved_turns() {
    for (field, value, expected_error) in [
        (
            "summary",
            json!("x".repeat(401)),
            "summary exceeds 400 characters",
        ),
        (
            "stop",
            json!("false"),
            "Tool arguments do not match the declared schema",
        ),
        (
            "unexpected",
            json!(true),
            "Tool arguments do not match the declared schema",
        ),
    ] {
        let mut invalid = proposal();
        invalid[field] = value;
        let mut turns = inspected_turns();
        turns.push(turn(
            "propose_dataset_edits",
            invalid.clone(),
            "Submit a proposal.",
        ));
        turns.push(turn(
            "propose_dataset_edits",
            proposal(),
            "Correct the rejected proposal.",
        ));
        let (agent, store, runtime) = setup(turns);
        let scope = scope();
        let accepted = agent.analyze(scope.clone()).await.unwrap();
        assert_eq!(serde_json::to_value(&accepted).unwrap(), proposal());
        {
            let history = store.history.lock().unwrap();
            assert_eq!(history.len(), 4);
            assert_eq!(history[2].tools[0].arguments, invalid);
            assert!(history[2].tools[0].failed);
            assert!(history[2].proposal.is_none());
            assert!(!history[2].interrupted);
            assert!(
                history[2].tools[0]
                    .result
                    .to_string()
                    .contains(expected_error)
            );
            assert_eq!(history[2].usage.output_tokens, Some(20));
            assert!(history.iter().all(|record| record.validate().is_ok()));
        }
        assert!(
            runtime.requests.lock().unwrap()[3]
                .initial_prompt
                .contains(expected_error)
        );
        assert_eq!(agent.analyze(scope).await.unwrap(), accepted);
        assert_eq!(
            runtime.requests.lock().unwrap().len(),
            4,
            "completed correction must not be repeated on resume"
        );
    }
}

#[tokio::test]
async fn unknown_tools_and_sealed_requests_never_reach_inspection() {
    let (agent, store, _) = setup(vec![turn(
        "read_sealed_rows",
        json!({}),
        "Try an unauthorized tool.",
    )]);
    let mut scope = scope();
    scope.maximum_turns = 1;
    let error = agent.analyze(scope).await.unwrap_err().to_string();
    assert!(error.contains("ignored the required proposal tool call"));
    let history = store.history.lock().unwrap();
    assert!(history[0].tools[0].failed);
    assert!(history[0].proposal.is_none());
}

#[tokio::test]
async fn proposal_only_turn_without_the_required_tool_fails_without_paid_retries() {
    let mut turns = inspected_turns();
    turns.push(vec![
        AgentMessage::Event {
            event: AgentEvent::ModelTurnStarted { sequence: 1 },
        },
        AgentMessage::Event {
            event: AgentEvent::ModelTurnCompleted {
                sequence: 1,
                input_tokens: 100,
                output_tokens: 500,
                cost_microusd: 0,
            },
        },
        AgentMessage::Completed,
    ]);
    let (agent, store, runtime) = setup(turns);

    let error = agent.analyze(scope()).await.unwrap_err().to_string();

    assert!(error.contains("ignored the required proposal tool call"));
    assert_eq!(runtime.requests.lock().unwrap().len(), 3);
    let history = store.history.lock().unwrap();
    assert_eq!(history.len(), 3);
    assert!(history[2].interrupted);
    assert_eq!(history[2].usage.output_tokens, Some(500));
}

#[tokio::test]
async fn stop_before_analysis_dispatches_no_provider_request() {
    let (agent, store, runtime) = setup(Vec::new());
    store.stopped.store(true, Ordering::Relaxed);
    assert!(matches!(
        agent.analyze(scope()).await,
        Err(OptimizationError::Stopped)
    ));
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_failure_is_persisted_as_unknown_not_as_free_or_valid_edits() {
    let (agent, store, _) = setup(vec![vec![AgentMessage::Failed {
        message: "must not disclose raw provider bodies".into(),
    }]]);
    let error = agent.analyze(scope()).await.unwrap_err().to_string();
    assert!(!error.contains("raw provider bodies"));
    let history = store.history.lock().unwrap();
    assert!(history[0].interrupted);
    assert!(history[0].usage.input_tokens.is_none());
    assert!(history[0].proposal.is_none());
}
