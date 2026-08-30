use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use dataset_architect_core::{
    brief::{
        ArchitectBudgets, ArchitectProviderConfiguration, GenerationCostModel, PlanningPriority,
        ResolvedArchitectBrief,
    },
    lifecycle::{ArchitectRun, ArchitectRunState, ArchitectToolCall},
    ports::{
        AgentToolRequest, AgentToolResult, ArchitectAdapterError, ArchitectAgentEvent,
        ArchitectAgentMessage, ArchitectAgentRequest, ArchitectAgentRuntime, ArchitectAgentSession,
        ArchitectStore, BoxFuture,
    },
    proposal::{ArchitectProposalReview, DatasetArchitectureProposal},
};
use dataset_architect_runner::ArchitectRunner;
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, DimensionDefinition},
};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn iterative_agent_previews_before_submitting_a_deterministically_validated_proposal() {
    let brief = brief();
    let allocations = expand_generation_cells(&brief.dataset)
        .into_iter()
        .map(|cell| json!({
            "cell": cell,
            "target": 10,
            "rationale": "Preserve complete coverage while prioritizing later strategy variants.",
            "confidence": "medium",
            "expectedBenefits": ["coverage"],
        }))
        .collect::<Vec<_>>();
    let preview = allocations
        .iter()
        .map(|value| json!({"cell": value["cell"], "target": value["target"]}))
        .collect::<Vec<_>>();
    let messages = VecDeque::from([
        event(ArchitectAgentEvent::AgentText {
            text: "Inspect facts, compare a candidate, estimate cost, then submit.".into(),
        }),
        tool("inspect_dataset", json!({})),
        turn(1),
        tool("preview_allocation", json!({"allocations": preview})),
        turn(2),
        tool(
            "submit_proposal",
            json!({
                "summary": "Balanced coverage with controlled messy-style noise.",
                "allocations": allocations,
                "strategies": [{
                    "selector": {"dimensions": {"style": "messy"}},
                    "kind": "noise",
                    "shareBasisPoints": 3000,
                    "instructions": ["Use plausible typos and incomplete punctuation."],
                    "relatedLabels": [],
                    "rationale": "Robustness requires realistic channel noise.",
                    "confidence": "high"
                }],
                "tradeoffs": ["Noise can make class boundaries harder."],
                "uncertainties": ["No development diagnostics are pinned yet."]
            }),
        ),
        turn(3),
        tool(
            "finish_architecture",
            json!({
                "reason": "proposal_submitted",
                "summary": "Validated proposal submitted.",
                "confidence": "medium"
            }),
        ),
        turn(4),
        ArchitectAgentMessage::Completed,
    ]);
    let store = Arc::new(MemoryStore::default());
    let runner = ArchitectRunner::new(
        Arc::new(ScriptedRuntime(Mutex::new(Some(messages)))),
        store.clone(),
    );
    let queued = runner.queue(brief).await.unwrap();
    let outcome = runner.run(queued.id).await.unwrap();

    assert_eq!(outcome.run.state, ArchitectRunState::AwaitingReview);
    assert_eq!(outcome.run.usage.model_turns, 4);
    assert_eq!(outcome.run.usage.tool_calls, 4);
    assert_eq!(outcome.run.usage.allocation_previews, 1);
    assert_eq!(outcome.run.plan.len(), 1);
    let proposal = outcome.proposal.unwrap();
    assert_eq!(proposal.validated_allocation.allocated_target_rows, 40);
    assert_eq!(proposal.strategies.len(), 1);
    assert_eq!(store.calls.lock().unwrap().len(), 4);
}

fn brief() -> ResolvedArchitectBrief {
    ResolvedArchitectBrief::create(
        DatasetDefinition::new(
            "support",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![DimensionDefinition::new("style", vec!["clean".into(), "messy".into()]).unwrap()],
        )
        .unwrap(),
        40,
        0,
        vec![],
        vec![],
        vec![PlanningPriority {
            name: "robustness".into(),
            description: "Prefer realistic class boundaries".into(),
            weight: 100,
        }],
        None,
        None,
        None,
        GenerationCostModel {
            rows_per_request: 10,
            estimated_input_tokens_per_request: 500,
            estimated_output_tokens_per_row: 50,
            input_cost_microusd_per_million_tokens: None,
            output_cost_microusd_per_million_tokens: None,
        },
        ArchitectBudgets {
            max_model_turns: 8,
            max_tool_calls: 12,
            max_allocation_previews: 3,
            max_input_tokens: 10_000,
            max_output_tokens: 10_000,
            max_cost_microusd: 1_000_000,
            max_wall_clock_seconds: 30,
        },
        ArchitectProviderConfiguration {
            runtime: "pi".into(),
            provider: "fake".into(),
            model: "scripted".into(),
            api_key_env: None,
        },
    )
    .unwrap()
}

fn event(event: ArchitectAgentEvent) -> ArchitectAgentMessage {
    ArchitectAgentMessage::Event { event }
}

fn turn(sequence: u32) -> ArchitectAgentMessage {
    event(ArchitectAgentEvent::ModelTurnCompleted {
        sequence,
        input_tokens: 100,
        output_tokens: 50,
        cost_microusd: 100,
    })
}

fn tool(name: &str, arguments: serde_json::Value) -> ArchitectAgentMessage {
    ArchitectAgentMessage::ToolRequest {
        request: AgentToolRequest {
            external_call_id: format!("call-{name}"),
            name: name.into(),
            arguments,
        },
    }
}

struct ScriptedRuntime(Mutex<Option<VecDeque<ArchitectAgentMessage>>>);

impl ArchitectAgentRuntime for ScriptedRuntime {
    fn start(
        &self,
        request: ArchitectAgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn ArchitectAgentSession>, ArchitectAdapterError>> {
        Box::pin(async move {
            assert_eq!(request.capability_set, "dataset_architect_v1");
            let messages = self
                .0
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| ArchitectAdapterError("script already consumed".into()))?;
            Ok(Box::new(ScriptedSession(messages)) as Box<dyn ArchitectAgentSession>)
        })
    }
}

struct ScriptedSession(VecDeque<ArchitectAgentMessage>);

impl ArchitectAgentSession for ScriptedSession {
    fn next_message(
        &mut self,
    ) -> BoxFuture<'_, Result<ArchitectAgentMessage, ArchitectAdapterError>> {
        Box::pin(async move {
            self.0
                .pop_front()
                .ok_or_else(|| ArchitectAdapterError("script ended".into()))
        })
    }

    fn send_tool_result(
        &mut self,
        _external_call_id: &str,
        _result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        Box::pin(async { Ok(()) })
    }

    fn send_tool_error(
        &mut self,
        _external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let message = message.to_owned();
        Box::pin(async move { Err(ArchitectAdapterError(message)) })
    }

    fn cancel(&mut self, _run_id: Uuid) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct MemoryStore {
    brief: Mutex<Option<ResolvedArchitectBrief>>,
    run: Mutex<Option<ArchitectRun>>,
    calls: Mutex<Vec<ArchitectToolCall>>,
    proposal: Mutex<Option<DatasetArchitectureProposal>>,
    reviews: Mutex<Vec<ArchitectProposalReview>>,
}

impl ArchitectStore for MemoryStore {
    fn create_run(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let brief = brief.clone();
        let run = run.clone();
        Box::pin(async move {
            *self.brief.lock().unwrap() = Some(brief);
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedArchitectBrief>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .brief
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectRun>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .run
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn save_run(&self, run: &ArchitectRun) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn record_tool_call(
        &self,
        call: &ArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut calls = self.calls.lock().unwrap();
            if let Some(existing) = calls.iter_mut().find(|value| value.id == call.id) {
                *existing = call;
            } else {
                calls.push(call);
            }
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ArchitectToolCall>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|value| value.run_id == run_id)
                .cloned()
                .collect())
        })
    }

    fn save_proposal_and_run(
        &self,
        proposal: &DatasetArchitectureProposal,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let proposal = proposal.clone();
        let run = run.clone();
        Box::pin(async move {
            *self.proposal.lock().unwrap() = Some(proposal);
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .proposal
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .proposal
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.run_id == run_id))
        })
    }

    fn append_review(
        &self,
        review: &ArchitectProposalReview,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            self.reviews.lock().unwrap().push(review);
            Ok(())
        })
    }

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectProposalReview>, ArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .reviews
                .lock()
                .unwrap()
                .iter()
                .filter(|value| value.proposal_id == proposal_id)
                .max_by_key(|value| (value.created_at, value.id))
                .cloned())
        })
    }
}
