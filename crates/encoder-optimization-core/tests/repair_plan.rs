use encoder_optimization_core::{agent::*, fingerprint, generation::*, repair_plan::*};
use generation_core::structured::StructuredGenerationRequest;
use serde_json::json;
use uuid::Uuid;

fn fixture() -> (
    AgentAnalysisScope,
    Vec<(AgentCallReservation, Option<AgentTurnRecord>)>,
) {
    let scope = AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: Uuid::new_v4(),
        dataset_fingerprint: fingerprint(&"rows").unwrap(),
        development_evidence_fingerprint: fingerprint(&"dev").unwrap(),
        objective: String::new(),
        analysis_protocol: 1,
        maximum_turns: 4,
        maximum_row_changes: 12,
    };
    let call = AgentCallReservation {
        id: Uuid::new_v4(),
        scope_fingerprint: scope.fingerprint().unwrap(),
        sequence: 1,
        request_fingerprint: fingerprint(&"request").unwrap(),
        input_token_ceiling: 100,
        output_token_ceiling: 100,
        cost_ceiling_microusd: 100,
    };
    let proposal = DatasetEditProposal {
        summary: "Fill the observed gap".into(),
        stop: false,
        removals: vec![RowRemoval {
            row_id: "row".into(),
            reason: "Conflicting example".into(),
            evidence_ids: vec!["evidence".into()],
        }],
        additions: vec![GenerationTarget {
            template_row_id: "row".into(),
            instruction: "Target the observed ambiguity".into(),
            count: 10,
            evidence_ids: vec!["evidence".into()],
        }],
    };
    let inspection = |name: &str, id: &str| {
        let content = json!({"kind":id,"private":"never serialize raw inspection content"});
        RecordedAgentTool {
            call_id: name.into(),
            name: name.into(),
            arguments: json!({}),
            result: serde_json::to_value(InspectionPage {
                items: vec![InspectionItem {
                    id: id.into(),
                    fingerprint: fingerprint(&content).unwrap(),
                    content,
                }],
                next_offset: None,
            })
            .unwrap(),
            failed: false,
        }
    };
    let record = AgentTurnRecord {
        call: call.clone(),
        explanations: vec![],
        tools: vec![
            inspection("inspect_training_rows", "row"),
            inspection("inspect_development_failures", "evidence"),
            RecordedAgentTool {
                call_id: "decision".into(),
                name: "propose_dataset_edits".into(),
                arguments: serde_json::to_value(&proposal).unwrap(),
                result: json!({"accepted":true}),
                failed: false,
            },
        ],
        usage: AgentTokenUsage::default(),
        proposal: Some(proposal),
        interrupted: false,
    };
    (scope, vec![(call, Some(record))])
}

fn generated(scope: &AgentAnalysisScope, proposal: &DatasetEditProposal) -> GenerationRecord {
    let task = GenerationTask {
        id: Uuid::new_v4(),
        run_id: scope.run_id,
        iteration: scope.iteration,
        proposal_fingerprint: fingerprint(proposal).unwrap(),
        template_row_id: "row".into(),
        template_fingerprint: fingerprint(&"row").unwrap(),
        target_index: 0,
        first_row: 0,
        requested_rows: 8,
        request: StructuredGenerationRequest {
            system_prompt: "system".into(),
            user_prompt: "generate".into(),
            maximum_output_tokens: 100,
        },
    };
    let call = GenerationReservation {
        id: Uuid::new_v4(),
        task_id: task.id,
        task_fingerprint: task.fingerprint().unwrap(),
        attempt: 1,
        input_token_ceiling: 100,
        output_token_ceiling: 100,
        cost_ceiling_microusd: 100,
    };
    let content = json!({"question":"bounded sample"});
    let admission = GenerationAdmission {
        accepted: vec![AdmittedGenerationRow {
            index: 0,
            fingerprint: fingerprint(&content).unwrap(),
            deduplication_fingerprint: fingerprint(&"input").unwrap(),
            content,
        }],
        rejected: (1..8)
            .map(|index| RejectedGenerationRow {
                index,
                reason: "Duplicate native input".into(),
            })
            .collect(),
    };
    let outcome = GenerationOutcome {
        reservation: call.clone(),
        usage: AgentTokenUsage::default(),
        admission: Some(admission),
        interrupted: false,
    };
    (task, call, Some(outcome))
}

#[test]
fn requested_admitted_rejected_and_unresolved_are_separate_and_raw_evidence_is_not_serialized() {
    let (scope, calls) = fixture();
    let pending = recorded_plan(&scope, &calls, &[]).unwrap().unwrap();
    assert_eq!(pending.plan.generation[0].unresolved, 10);
    let record = generated(&scope, &pending.plan.proposal);
    let observed = recorded_plan(&scope, &calls, &[record]).unwrap().unwrap();
    let target = &observed.plan.generation[0];
    assert_eq!(
        (
            target.requested,
            target.admitted,
            target.rejected,
            target.unresolved
        ),
        (10, 1, 7, 2)
    );
    assert_eq!(target.rejection_reasons["Duplicate native input"], 7);
    assert_eq!(observed.evidence.len(), 1);
    assert!(
        !serde_json::to_string(&observed.plan)
            .unwrap()
            .contains("private")
    );
}

#[test]
fn partial_and_no_change_decisions_remain_distinct() {
    let (scope, mut calls) = fixture();
    let saved = calls[0].1.clone().unwrap();
    calls[0].1 = None;
    assert!(recorded_plan(&scope, &calls, &[]).unwrap().is_none());
    let record = generated(&scope, saved.proposal.as_ref().unwrap());
    assert!(recorded_plan(&scope, &calls, &[record]).is_err());
    let mut saved = saved;
    let proposal = DatasetEditProposal {
        summary: "No justified change".into(),
        stop: true,
        removals: vec![],
        additions: vec![],
    };
    saved.proposal = Some(proposal.clone());
    saved.tools.last_mut().unwrap().arguments = serde_json::to_value(proposal).unwrap();
    calls[0].1 = Some(saved);
    let result = recorded_plan(&scope, &calls, &[]).unwrap().unwrap();
    assert!(result.plan.proposal.stop && result.plan.generation.is_empty());
}

#[test]
fn unknown_attempts_stay_unresolved_and_retries_cannot_double_count_a_completed_slot() {
    let (scope, calls) = fixture();
    let proposal = calls[0].1.as_ref().unwrap().proposal.as_ref().unwrap();
    let mut first = generated(&scope, proposal);
    first.2.as_mut().unwrap().admission = None;
    first.2.as_mut().unwrap().interrupted = true;
    let mut second = first.clone();
    second.1.id = Uuid::new_v4();
    second.1.attempt = 2;
    second.2 = None;
    let result = recorded_plan(&scope, &calls, &[first.clone(), second])
        .unwrap()
        .unwrap();
    assert_eq!(result.plan.generation[0].unresolved, 10);
    assert_eq!(result.plan.generation[0].attempts, 2);
    let completed = generated(&scope, proposal);
    assert!(recorded_plan(&scope, &calls, &[completed.clone(), completed]).is_err());
    first.0.run_id = Uuid::new_v4();
    assert!(recorded_plan(&scope, &calls, &[first]).is_err());
    let mut foreign = scope.clone();
    foreign.dataset_version_id = Uuid::new_v4();
    assert!(recorded_plan(&foreign, &calls, &[]).is_err());
}
