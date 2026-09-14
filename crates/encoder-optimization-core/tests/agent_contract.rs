use std::collections::BTreeSet;

use encoder_optimization_core::{
    agent::{AgentAnalysisScope, DatasetEditProposal, GenerationTarget, RowRemoval},
    fingerprint,
};
use uuid::Uuid;

fn scope() -> AgentAnalysisScope {
    AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: Uuid::new_v4(),
        dataset_fingerprint: fingerprint(&"rows").unwrap(),
        development_evidence_fingerprint: fingerprint(&"development").unwrap(),
        objective: String::new(),
        maximum_turns: 4,
        maximum_row_changes: 2,
    }
}

fn proposal() -> DatasetEditProposal {
    DatasetEditProposal {
        summary: "Remove the conflicting example and fill the observed gap.".into(),
        stop: false,
        removals: vec![RowRemoval {
            row_id: "row".into(),
            reason: "Conflicting label".into(),
            evidence_ids: vec!["failure".into()],
        }],
        additions: vec![GenerationTarget {
            template_row_id: "row".into(),
            instruction: "Generate a clear example for the observed task".into(),
            count: 1,
            evidence_ids: vec!["failure".into()],
        }],
    }
}

#[test]
fn all_edits_require_inspected_membership_and_development_evidence() {
    let rows = BTreeSet::from(["row".into()]);
    let evidence = BTreeSet::from(["failure".into()]);
    let scope = scope();
    proposal().validate(&scope, &rows, &evidence).unwrap();
    assert!(
        proposal()
            .validate(&scope, &BTreeSet::new(), &evidence)
            .is_err()
    );
    assert!(
        proposal()
            .validate(&scope, &rows, &BTreeSet::new())
            .is_err()
    );
    let mut value = proposal();
    value.additions[0].evidence_ids.clear();
    assert!(value.validate(&scope, &rows, &evidence).is_err());
    value = proposal();
    value.removals.push(value.removals[0].clone());
    assert!(value.validate(&scope, &rows, &evidence).is_err());
    value = proposal();
    value.additions[0].count = u32::MAX;
    assert!(value.validate(&scope, &rows, &evidence).is_err());
}

#[test]
fn stop_is_an_explicit_no_change_decision_and_unknown_fields_cannot_grant_authority() {
    let scope = scope();
    let rows = BTreeSet::from(["row".into()]);
    let evidence = BTreeSet::from(["failure".into()]);
    let mut value = proposal();
    value.stop = true;
    assert!(value.validate(&scope, &rows, &evidence).is_err());
    value.removals.clear();
    value.additions.clear();
    value
        .validate(&scope, &BTreeSet::new(), &BTreeSet::new())
        .unwrap();
    value.stop = false;
    assert!(value.validate(&scope, &rows, &evidence).is_err());
    let mut raw = serde_json::to_value(proposal()).unwrap();
    raw["approveSealedEvaluation"] = true.into();
    assert!(serde_json::from_value::<DatasetEditProposal>(raw).is_err());
}
