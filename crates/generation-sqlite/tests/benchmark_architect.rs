use std::collections::BTreeMap;

use artifact_core::{ArtifactKind, ProvenanceStore};
use benchmark_architect_core::{
    blueprint::{
        BenchmarkAcquisitionHandoff, BenchmarkArchitectureProposal, BenchmarkArchitectureReview,
        BenchmarkArchitectureReviewDecision, BenchmarkBlueprintDraft, BenchmarkCohortBlueprint,
        BenchmarkSuiteBlueprint, CohortFreshnessPolicy, RiskCoverageRequirement,
    },
    brief::{
        BenchmarkArchitectBriefDraft, BenchmarkArchitectBudgets, BenchmarkArchitectProvider,
        BenchmarkObjective, DeploymentContext, DeploymentRisk, LabelSemantic,
        ResolvedBenchmarkArchitectBrief, RiskLevel,
    },
    lifecycle::{
        BenchmarkArchitectRun, BenchmarkArchitectStopReason, BenchmarkArchitectToolCall,
        BenchmarkArchitectToolKind, BenchmarkArchitectUsage,
    },
    ports::BenchmarkArchitectStore,
};
use evaluation_core::domain::EvaluationProtocol;
use research_core::{
    brief::SourcePolicy,
    evidence::{EvidenceConfidence, ResearchEvidence, ResearchEvidenceDraft},
};
use serde_json::json;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    benchmark::{
        AcceptanceContract, BenchmarkMetric, BenchmarkSuiteKind, MetricRequirement, MetricTarget,
        RegressionRequirement,
    },
    benchmark_qualification::{BenchmarkQualificationPolicy, QualificationConfidence},
    governance::{CohortOrigin, CohortRole, DisclosureLevel},
};

fn brief() -> ResolvedBenchmarkArchitectBrief {
    ResolvedBenchmarkArchitectBrief::create(BenchmarkArchitectBriefDraft {
        task: "Classify support messages".into(),
        labels: vec!["billing".into(), "fraud".into()],
        label_semantics: vec![
            LabelSemantic {
                label: "billing".into(),
                meaning: "Charges and invoices".into(),
                inclusions: vec![],
                exclusions: vec![],
            },
            LabelSemantic {
                label: "fraud".into(),
                meaning: "Unauthorized activity".into(),
                inclusions: vec![],
                exclusions: vec![],
            },
        ],
        deployment: DeploymentContext {
            summary: "Consumer support intake".into(),
            users: vec!["customers".into()],
            channels: vec!["chat".into()],
            languages: vec!["English".into()],
            regions: vec![],
            time_horizon: Some("next 12 months".into()),
            constraints: vec![],
        },
        risks: vec![DeploymentRisk {
            key: "fraud_as_billing".into(),
            description: "Fraud is mistaken for billing".into(),
            consequence: "Delayed account protection".into(),
            likelihood: RiskLevel::Medium,
            severity: RiskLevel::Critical,
            weight: 100,
        }],
        objectives: vec![BenchmarkObjective {
            key: "boundary_recall".into(),
            description: "Measure the fraud/billing boundary".into(),
            weight: 100,
        }],
        candidate_sources: vec![],
        existing_benchmark: None,
        source_policy: SourcePolicy::default(),
        budgets: BenchmarkArchitectBudgets {
            max_model_turns: 10,
            max_tool_calls: 50,
            max_searches: 5,
            max_fetched_pages: 10,
            max_fetched_bytes: 100_000,
            max_blueprint_previews: 5,
            max_input_tokens: 100_000,
            max_output_tokens: 20_000,
            max_cost_microusd: 1_000_000,
            max_wall_clock_seconds: 600,
        },
        provider: BenchmarkArchitectProvider {
            runtime: "pi".into(),
            provider: "fake".into(),
            model: "scripted".into(),
            api_key_env: None,
        },
    })
    .expect("brief")
}

fn cohort(kind: BenchmarkSuiteKind, key: &str, role: CohortRole) -> BenchmarkCohortBlueprint {
    BenchmarkCohortBlueprint {
        key: key.into(),
        name: key.into(),
        purpose: "Measure production-like boundaries".into(),
        origin: CohortOrigin::InternalSnapshot,
        role,
        disclosure: DisclosureLevel::Aggregate,
        adaptation_eligible: kind == BenchmarkSuiteKind::Development,
        protocol: EvaluationProtocol {
            minimum_slice_support: 20,
            ..EvaluationProtocol::default()
        },
        minimum_total_support: 400,
        minimum_label_support: BTreeMap::from([("billing".into(), 200), ("fraud".into(), 200)]),
        required_dimension_values: BTreeMap::from([("channel".into(), vec!["chat".into()])]),
        required_slice_support: BTreeMap::new(),
        required_source_classes: vec!["reviewed_real_sample".into()],
        minimum_distinct_producers: 2,
        freshness: CohortFreshnessPolicy {
            maximum_age_days: 180,
            maximum_workflow_iterations: 3,
            maximum_adaptive_exposures: if kind == BenchmarkSuiteKind::Development {
                10
            } else {
                0
            },
            maximum_acceptance_exposures: if kind == BenchmarkSuiteKind::SealedAcceptance {
                1
            } else {
                0
            },
        },
        rationale: "Measure the highest-consequence class boundary".into(),
    }
}

fn contract(kind: BenchmarkSuiteKind) -> AcceptanceContract {
    AcceptanceContract {
        metric_requirements: vec![MetricRequirement {
            target: MetricTarget::Overall,
            metric: BenchmarkMetric::MacroF1,
            minimum: Some(0.85),
            maximum: None,
            minimum_support: 100,
        }],
        regression: (kind == BenchmarkSuiteKind::Development).then_some(RegressionRequirement {
            max_accuracy_drop: Some(0.02),
            max_macro_f1_drop: None,
            minimum_accuracy_delta_lower_bound: None,
            minimum_macro_f1_delta_lower_bound: None,
            require_mcnemar_significance: false,
        }),
    }
}

fn draft(evidence_id: uuid::Uuid) -> BenchmarkBlueprintDraft {
    BenchmarkBlueprintDraft {
        summary: "Use development and single-use sealed evidence".into(),
        suites: vec![
            BenchmarkSuiteBlueprint {
                kind: BenchmarkSuiteKind::Development,
                name: "development".into(),
                purpose: "Guide bounded iteration".into(),
                required_model_formats: vec![],
                cohorts: vec![cohort(
                    BenchmarkSuiteKind::Development,
                    "dev-boundary",
                    CohortRole::Development,
                )],
                contract: contract(BenchmarkSuiteKind::Development),
            },
            BenchmarkSuiteBlueprint {
                kind: BenchmarkSuiteKind::SealedAcceptance,
                name: "sealed".into(),
                purpose: "One final acceptance decision".into(),
                required_model_formats: vec![],
                cohorts: vec![cohort(
                    BenchmarkSuiteKind::SealedAcceptance,
                    "sealed-boundary",
                    CohortRole::SealedAcceptance,
                )],
                contract: contract(BenchmarkSuiteKind::SealedAcceptance),
            },
        ],
        qualification_policy: BenchmarkQualificationPolicy {
            minimum_overall_support: 100,
            minimum_label_support: 20,
            confidence: QualificationConfidence::NinetyFive,
            maximum_proportion_margin_of_error: 0.10,
            maximum_normalized_duplicate_rate: 0.01,
            minimum_distinct_producers: 2,
            maximum_single_producer_share: 0.8,
            maximum_label_imbalance_ratio: 2.0,
        },
        risk_coverage: vec![RiskCoverageRequirement {
            key: "fraud-boundary".into(),
            deployment_risk_key: "fraud_as_billing".into(),
            cohort_keys: vec!["dev-boundary".into(), "sealed-boundary".into()],
            rationale: "Cover the boundary in adaptive and final evidence".into(),
            supporting_evidence_ids: vec![evidence_id],
            conflicting_evidence_ids: vec![],
            inference: false,
        }],
        tradeoffs: vec![],
        evidence_gaps: vec!["True channel prevalence is not measured".into()],
    }
}

#[tokio::test]
async fn persists_tamper_evident_reviewed_blueprint_and_handoff() {
    let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
    let brief = brief();
    let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
    store.create_run(&brief, &run).await.unwrap();
    run.start().unwrap();
    store.save_run(&run).await.unwrap();

    let mut call = BenchmarkArchitectToolCall::start(
        &run,
        1,
        BenchmarkArchitectToolKind::RecordEvidence,
        json!({"key": "fraud-boundary"}),
    )
    .unwrap();
    store.record_tool_call(&call).await.unwrap();
    let evidence = ResearchEvidence::create(
        run.id,
        call.id,
        &brief.source_policy,
        ResearchEvidenceDraft {
            url: "https://example.com/fraud-support-study".into(),
            title: "Fraud support study".into(),
            query: "support fraud billing confusion".into(),
            source_class: "industry_report".into(),
            content_hash: "sha256:page".into(),
            excerpt: "Users often describe fraud as an unexpected charge".into(),
            location: None,
            observation: "Fraud and billing language overlap".into(),
            applicability: "Boundary cohort design".into(),
            confidence: EvidenceConfidence::High,
        },
    )
    .unwrap();
    store.record_evidence(&evidence).await.unwrap();
    call.succeed(
        json!({"evidence_id": evidence.id}),
        BenchmarkArchitectUsage::default(),
    )
    .unwrap();
    store.record_tool_call(&call).await.unwrap();

    let proposal = BenchmarkArchitectureProposal::create(
        &brief,
        &run,
        std::slice::from_ref(&evidence),
        draft(evidence.id),
    )
    .unwrap();
    run.await_review(BenchmarkArchitectStopReason::ProposalSubmitted)
        .unwrap();
    store.save_proposal_and_run(&proposal, &run).await.unwrap();
    let approval = BenchmarkArchitectureReview::create(
        &proposal,
        None,
        BenchmarkArchitectureReviewDecision::Approve,
        "operator".into(),
        "Evidence, support, and renewal bounds are acceptable".into(),
    )
    .unwrap();
    store.append_review(&approval).await.unwrap();
    let handoff = BenchmarkAcquisitionHandoff::create(
        &brief,
        &proposal,
        &approval,
        std::slice::from_ref(&evidence),
    )
    .unwrap();
    store.save_handoff(&handoff).await.unwrap();

    assert_eq!(
        store.get_proposal(proposal.id).await.unwrap(),
        Some(proposal)
    );
    assert_eq!(
        store.latest_review(handoff.proposal_id).await.unwrap(),
        Some(approval)
    );
    assert_eq!(
        store.get_handoff_by_id(handoff.id).await.unwrap(),
        Some(handoff.clone())
    );
    let provenance = store
        .trace_provenance(ArtifactKind::BenchmarkAcquisitionHandoff, handoff.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        provenance
            .parents
            .iter()
            .any(|parent| parent.kind == ArtifactKind::BenchmarkArchitectureProposal)
    );
    assert!(
        provenance
            .parents
            .iter()
            .any(|parent| parent.kind == ArtifactKind::BenchmarkArchitectureReview)
    );

    assert!(
        sqlx::query("UPDATE benchmark_acquisition_handoffs SET handoff_json = '{}' WHERE id = ?")
            .bind(handoff.id)
            .execute(store.pool())
            .await
            .is_err(),
        "database trigger must reject handoff mutation"
    );
}
