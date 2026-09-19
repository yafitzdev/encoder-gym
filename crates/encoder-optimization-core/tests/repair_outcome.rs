use encoder_optimization_core::{
    fingerprint,
    repair_outcome::{
        OutcomeIdentity, REPAIR_OUTCOME_SCHEMA_VERSION, RepairEditCounts, RepairGlobalVerdict,
        RepairMetricObservation, RepairOutcome, TargetMetricChange, aggregate_change,
        classify_change, intervention_fingerprint, intervention_summary,
    },
    repair_strategy::{
        AdditionCount, AnchorAllocation, MetricDirection, RepairOperation, RepairTarget,
        TargetMetric,
    },
};
use uuid::Uuid;

fn target() -> RepairTarget {
    RepairTarget {
        target_id: "search-gap".into(),
        cluster_keys: vec!["cluster-b".into(), "cluster-a".into()],
        evidence_ids: vec!["cluster-a".into()],
        hypothesis: "More precise search variants may improve recall".into(),
        evidence_limitations: "Saved disagreements are sampled".into(),
        intended_failure_pattern: "Search ranks below a distractor".into(),
        alternative_explanation: "The model may lack capacity".into(),
        operation: RepairOperation::LabelPreservingVariants {
            count: AdditionCount::AbsoluteRows { desired_rows: 2 },
            allocation_rationale: "One per inspected context".into(),
            anchors: vec![
                AnchorAllocation {
                    row_id: "b".into(),
                    row_fingerprint: fingerprint(&"b").unwrap(),
                    additions: 1,
                },
                AnchorAllocation {
                    row_id: "a".into(),
                    row_fingerprint: fingerprint(&"a").unwrap(),
                    additions: 1,
                },
            ],
        },
        target_metric: TargetMetric {
            name: "recall_at_1".into(),
            direction: MetricDirection::Increase,
        },
    }
}

#[test]
fn intervention_identity_is_stable_but_excludes_persuasive_text_and_count() {
    let original = target();
    let mut reordered = original.clone();
    reordered.cluster_keys.reverse();
    if let RepairOperation::LabelPreservingVariants { count, anchors, .. } =
        &mut reordered.operation
    {
        anchors.reverse();
        *count = AdditionCount::AbsoluteRows { desired_rows: 7 };
    }
    reordered.hypothesis = "Different rhetoric".into();
    assert_eq!(
        intervention_fingerprint(&original).unwrap(),
        intervention_fingerprint(&reordered).unwrap()
    );
}

#[test]
fn metric_direction_is_descriptive_and_missing_or_mixed_evidence_is_conservative() {
    assert_eq!(
        classify_change(MetricDirection::Increase, Some(0.4), Some(0.5)),
        TargetMetricChange::Improved
    );
    assert_eq!(
        classify_change(MetricDirection::Decrease, Some(0.4), Some(0.5)),
        TargetMetricChange::Regressed
    );
    assert_eq!(
        classify_change(MetricDirection::Increase, Some(0.4), None),
        TargetMetricChange::Unavailable
    );
    assert_eq!(
        aggregate_change(&[TargetMetricChange::Improved, TargetMetricChange::Regressed]),
        TargetMetricChange::Regressed
    );
    assert_eq!(
        aggregate_change(&[
            TargetMetricChange::Improved,
            TargetMetricChange::Unavailable
        ]),
        TargetMetricChange::Unavailable
    );
}

fn measured_outcome() -> RepairOutcome {
    let input = OutcomeIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: fingerprint(&"input").unwrap(),
    };
    RepairOutcome {
        schema_version: REPAIR_OUTCOME_SCHEMA_VERSION,
        run_id: Uuid::new_v4(),
        iteration: 1,
        target_id: "search-gap".into(),
        repair_plan_fingerprint: fingerprint(&"plan").unwrap(),
        proposal_fingerprint: fingerprint(&"proposal").unwrap(),
        intervention_fingerprint: intervention_fingerprint(&target()).unwrap(),
        cluster_keys: vec!["cluster-a".into(), "cluster-b".into()],
        intervention: intervention_summary(&target()),
        hypothesis: "A search variant may improve recall".into(),
        input_dataset: input,
        output_dataset: OutcomeIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: fingerprint(&"output").unwrap(),
        },
        candidate: OutcomeIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: fingerprint(&"candidate").unwrap(),
        },
        input_development_evidence_fingerprint: fingerprint(&"before").unwrap(),
        output_development_evidence_fingerprint: fingerprint(&"after").unwrap(),
        reports: vec![OutcomeIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: fingerprint(&"report").unwrap(),
        }],
        edits: RepairEditCounts {
            requested_additions: 2,
            generated: 2,
            structurally_admitted: 2,
            semantically_admitted: 2,
            published_additions: 2,
            requested_removals: 0,
            published_removals: 0,
        },
        metric_change: TargetMetricChange::Improved,
        observations: vec![RepairMetricObservation {
            cluster_key: "cluster-a".into(),
            suite: "development".into(),
            metric: "recall_at_1".into(),
            expected_direction: MetricDirection::Increase,
            support: Some(20),
            original_baseline: Some(0.4),
            preceding_candidate: Some(0.5),
            candidate: Some(0.6),
            delta_from_original_baseline: Some(0.2),
            delta_from_preceding_candidate: Some(0.1),
            change: TargetMetricChange::Improved,
        }],
        global_verdict: RepairGlobalVerdict::Reject,
        limitations: vec!["The observed delta is descriptive, not causal.".into()],
    }
}

#[test]
fn measured_outcome_reproduces_deltas_and_aggregate_direction() {
    measured_outcome().validate().unwrap();

    let mut changed = measured_outcome();
    changed.observations[0].delta_from_preceding_candidate = Some(0.2);
    assert!(changed.validate().is_err());

    let mut changed = measured_outcome();
    changed.metric_change = TargetMetricChange::Unchanged;
    assert!(changed.validate().is_err());
}

#[test]
fn measured_outcome_requires_a_distinct_output_dataset() {
    let mut unchanged = measured_outcome();
    unchanged.output_dataset = unchanged.input_dataset.clone();
    assert!(unchanged.validate().is_err());
}
