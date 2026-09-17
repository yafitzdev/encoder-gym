use std::{
    collections::VecDeque,
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
    },
    fingerprint,
    ports::{BoxFuture, OptimizationAgentStore, OptimizationInspection},
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
    let store = Arc::new(Store::default());
    let runtime = Arc::new(Runtime {
        store: store.clone(),
        turns: Mutex::new(turns.into()),
        requests: Mutex::default(),
    });
    let agent = OptimizationAgent::new(
        runtime.clone(),
        store.clone(),
        Arc::new(Inspection),
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
