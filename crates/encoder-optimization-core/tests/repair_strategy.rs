use std::collections::{BTreeMap, BTreeSet};

use encoder_optimization_core::{
    agent::RepairStopReason,
    fingerprint,
    repair_strategy::{
        AdditionCount, AnchorAllocation, AnchorReference, ContrastPairAllocation, MetricDirection,
        ProvenRemoval, REPAIR_PLAN_SCHEMA_VERSION, RepairOperation, RepairPlan,
        RepairPlanConstraintCode, RepairPlanningAnchor, RepairPlanningCluster,
        RepairPlanningContext, RepairTarget, TargetMetric, compile_repair_plan, contrast_pair_id,
        verify_repair_plan_submission,
    },
};

fn fp(value: &str) -> String {
    fingerprint(&value).unwrap()
}

fn context() -> RepairPlanningContext {
    let anchor = |name: &str, input: &str, label: &str, duplicate: Option<&str>| {
        (
            name.into(),
            RepairPlanningAnchor {
                fingerprint: fp(&format!("row-{name}")),
                cluster_keys: BTreeSet::from(["cluster-search".into()]),
                native_context_fingerprint: fp("shared-context"),
                native_model_input_fingerprint: fp(input),
                label_fingerprint: fp(label),
                exact_duplicate_group_id: duplicate.map(str::to_owned),
            },
        )
    };
    RepairPlanningContext {
        dataset_rows: 1_000,
        remaining_row_changes: 32,
        clusters: BTreeMap::from([(
            "cluster-search".into(),
            RepairPlanningCluster {
                training_rows: 400,
                metrics: BTreeSet::from(["recall_at_1".into(), "mrr".into()]),
            },
        )]),
        anchors: BTreeMap::from([
            anchor("a", "input-a", "label-a", Some("duplicate-a")),
            anchor("b", "input-b", "label-b", None),
            anchor("c", "input-c", "label-a", None),
            anchor("d", "input-a", "label-a", Some("duplicate-a")),
        ]),
        evidence_ids: BTreeSet::from(["cluster-search".into()]),
        prior_interventions: BTreeSet::new(),
    }
}

fn common(operation: RepairOperation) -> RepairPlan {
    RepairPlan {
        schema_version: REPAIR_PLAN_SCHEMA_VERSION,
        summary: "Test a bounded search-coverage repair.".into(),
        stop: false,
        stop_reason: None,
        targets: vec![RepairTarget {
            target_id: "target-search".into(),
            cluster_keys: vec!["cluster-search".into()],
            evidence_ids: vec!["cluster-search".into()],
            hypothesis: "More varied search contexts may improve retrieval.".into(),
            evidence_limitations: "The disagreement list is sampled.".into(),
            intended_failure_pattern: "Search tools rank below write tools.".into(),
            alternative_explanation: "The evaluation slice may be noisy.".into(),
            operation,
            target_metric: TargetMetric {
                name: "recall_at_1".into(),
                direction: MetricDirection::Increase,
            },
        }],
    }
}

#[test]
fn relative_growth_compiles_to_exact_stable_anchor_allocation_and_preview() {
    let plan = common(RepairOperation::LabelPreservingVariants {
        count: AdditionCount::RelativeClusterGrowth {
            source_cluster_key: "cluster-search".into(),
            pinned_source_rows: 400,
            basis_points: 500,
            desired_rows: 20,
        },
        allocation_rationale: "Spread five-percent growth across distinct contexts.".into(),
        anchors: vec![
            AnchorAllocation {
                row_id: "c".into(),
                row_fingerprint: fp("row-c"),
                additions: 6,
            },
            AnchorAllocation {
                row_id: "a".into(),
                row_fingerprint: fp("row-a"),
                additions: 7,
            },
            AnchorAllocation {
                row_id: "b".into(),
                row_fingerprint: fp("row-b"),
                additions: 7,
            },
        ],
    });
    let compilation = compile_repair_plan(&plan, &context()).unwrap();
    assert!(compilation.constraints.is_empty());
    let preview = compilation.preview.unwrap();
    assert_eq!(preview.desired_additions, 20);
    assert_eq!(preview.executable_additions, 20);
    assert_eq!(preview.projected_rows, 1_020);
    assert_eq!(preview.unused_row_change_allowance, 12);
    assert_eq!(
        preview.projected_clusters["cluster-search"].projected_rows,
        420
    );
    assert_eq!(
        preview.projected_clusters["cluster-search"].projected_share_ppm,
        411_765
    );
    assert_eq!(
        verify_repair_plan_submission(&plan, &context(), &preview.fingerprint).unwrap(),
        preview
    );
    assert!(verify_repair_plan_submission(&plan, &context(), &fp("other")).is_err());
}

#[test]
fn contrast_allocates_pairs_first_and_requires_compatible_contexts_and_labels() {
    let pair_id = contrast_pair_id("a", "b");
    let plan = common(RepairOperation::ExistingAnchorContrast {
        count: AdditionCount::AbsoluteRows { desired_rows: 6 },
        allocation_rationale: "Generate equal variants on both supported sides.".into(),
        pairs: vec![ContrastPairAllocation {
            pair_id,
            left: AnchorReference {
                row_id: "a".into(),
                row_fingerprint: fp("row-a"),
            },
            right: AnchorReference {
                row_id: "b".into(),
                row_fingerprint: fp("row-b"),
            },
            additions_per_side: 3,
        }],
    });
    assert!(
        compile_repair_plan(&plan, &context())
            .unwrap()
            .preview
            .is_some()
    );

    let mut incompatible = context();
    incompatible
        .anchors
        .get_mut("b")
        .unwrap()
        .native_context_fingerprint = fp("other-context");
    let failed = compile_repair_plan(&plan, &incompatible).unwrap();
    assert!(failed.preview.is_none());
    assert!(failed.constraints.iter().any(|constraint| {
        constraint.code == RepairPlanConstraintCode::ContrastContextMismatch
    }));
}

#[test]
fn proven_redundant_removal_is_distinct_from_relabeling_and_updates_projection() {
    let plan = common(RepairOperation::ProvenRedundantRowRemoval {
        removals: vec![ProvenRemoval {
            row_id: "d".into(),
            row_fingerprint: fp("row-d"),
            retained_row_id: "a".into(),
            retained_row_fingerprint: fp("row-a"),
            reason: "Exact native input and label duplicate; retain row a.".into(),
        }],
    });
    let preview = compile_repair_plan(&plan, &context())
        .unwrap()
        .preview
        .unwrap();
    assert_eq!(preview.removals, 1);
    assert_eq!(preview.projected_rows, 999);
    assert_eq!(
        preview.projected_clusters["cluster-search"].projected_rows,
        399
    );

    let mut unproven = context();
    unproven
        .anchors
        .get_mut("d")
        .unwrap()
        .exact_duplicate_group_id = None;
    let failed = compile_repair_plan(&plan, &unproven).unwrap();
    assert!(failed.constraints.iter().any(|constraint| {
        constraint.code == RepairPlanConstraintCode::DuplicateRemovalUnproven
    }));
}

#[test]
fn infeasible_allocations_return_typed_constraints_without_silent_trimming() {
    let plan = common(RepairOperation::LabelPreservingVariants {
        count: AdditionCount::AbsoluteRows { desired_rows: 17 },
        allocation_rationale: "This deliberately exceeds one anchor's ceiling.".into(),
        anchors: vec![AnchorAllocation {
            row_id: "a".into(),
            row_fingerprint: fp("row-a"),
            additions: 16,
        }],
    });
    let failed = compile_repair_plan(&plan, &context()).unwrap();
    assert!(failed.preview.is_none());
    let codes = failed
        .constraints
        .iter()
        .map(|constraint| constraint.code)
        .collect::<BTreeSet<_>>();
    assert!(codes.contains(&RepairPlanConstraintCode::AllocationMismatch));
    assert!(codes.contains(&RepairPlanConstraintCode::AdditionPerAnchorExceeded));
}

#[test]
fn unchanged_prior_intervention_is_a_typed_constraint() {
    let plan = common(RepairOperation::LabelPreservingVariants {
        count: AdditionCount::AbsoluteRows { desired_rows: 2 },
        allocation_rationale: "Split over the inspected contexts".into(),
        anchors: vec![
            AnchorAllocation {
                row_id: "a".into(),
                row_fingerprint: fp("row-a"),
                additions: 1,
            },
            AnchorAllocation {
                row_id: "b".into(),
                row_fingerprint: fp("row-b"),
                additions: 1,
            },
        ],
    });
    let mut context = context();
    context.prior_interventions.insert(
        encoder_optimization_core::repair_outcome::intervention_fingerprint(&plan.targets[0])
            .unwrap(),
    );
    let compiled = compile_repair_plan(&plan, &context).unwrap();
    assert!(compiled.preview.is_none());
    assert!(compiled.constraints.iter().any(|constraint| {
        constraint.code == RepairPlanConstraintCode::RepeatedUnchangedIntervention
    }));
}

#[test]
fn stop_plan_requires_an_explicit_v3_disposition() {
    let mut plan = common(RepairOperation::LabelPreservingVariants {
        count: AdditionCount::AbsoluteRows { desired_rows: 1 },
        allocation_rationale: "unused for stop fixture".into(),
        anchors: vec![],
    });
    plan.stop = true;
    plan.targets.clear();
    assert!(
        compile_repair_plan(&plan, &context())
            .unwrap()
            .preview
            .is_none()
    );
    for reason in [
        RepairStopReason::NoChange,
        RepairStopReason::UnsupportedRepair,
    ] {
        plan.stop_reason = Some(reason);
        let preview = compile_repair_plan(&plan, &context())
            .unwrap()
            .preview
            .unwrap();
        assert_eq!(preview.desired_additions, 0);
        assert_eq!(preview.removals, 0);
    }
}
