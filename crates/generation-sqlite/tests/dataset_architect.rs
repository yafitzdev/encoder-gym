use artifact_core::{ArtifactKind, ProvenanceStore};
use dataset_architect_core::{
    brief::{
        ArchitectBudgets, ArchitectProviderConfiguration, GenerationCostModel, PlanningPriority,
        ResolvedArchitectBrief,
    },
    lifecycle::{ArchitectRun, ArchitectStopReason},
    ports::ArchitectStore,
    proposal::{
        ArchitectProposalDraft, ArchitectProposalReview, ArchitectReviewDecision,
        CellAllocationRecommendation, DatasetArchitectureProposal, ExpectedBenefit,
        RecommendationConfidence, apply_approved_proposal,
    },
};
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, DimensionDefinition},
    ports::{DatasetStore, PlanStore},
};
use synthetic_data_sqlite::SqliteStore;

fn brief(dataset: DatasetDefinition) -> ResolvedArchitectBrief {
    ResolvedArchitectBrief::create(
        dataset,
        20,
        0,
        vec![],
        vec![],
        vec![PlanningPriority {
            name: "coverage".into(),
            description: "Cover all cells".into(),
            weight: 100,
        }],
        None,
        None,
        None,
        GenerationCostModel {
            rows_per_request: 10,
            estimated_input_tokens_per_request: 100,
            estimated_output_tokens_per_row: 20,
            input_cost_microusd_per_million_tokens: None,
            output_cost_microusd_per_million_tokens: None,
        },
        ArchitectBudgets {
            max_model_turns: 5,
            max_tool_calls: 20,
            max_allocation_previews: 3,
            max_input_tokens: 10_000,
            max_output_tokens: 5_000,
            max_cost_microusd: 100_000,
            max_wall_clock_seconds: 60,
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

#[tokio::test]
async fn persists_reviewed_architecture_and_ordinary_generation_inputs_atomically() {
    let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
    let dataset = DatasetDefinition::new(
        "support",
        "Classify support requests",
        vec!["billing".into(), "fraud".into()],
        vec![DimensionDefinition::new("style", vec!["clean".into(), "messy".into()]).unwrap()],
    )
    .unwrap();
    store.create_dataset(&dataset).await.unwrap();
    let brief = brief(dataset.clone());
    let mut run = ArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
    store.create_run(&brief, &run).await.unwrap();
    run.start().unwrap();
    store.save_run(&run).await.unwrap();
    let proposal = DatasetArchitectureProposal::create(
        &brief,
        &run,
        ArchitectProposalDraft {
            summary: "Balanced explicit coverage".into(),
            allocations: expand_generation_cells(&dataset)
                .into_iter()
                .map(|cell| CellAllocationRecommendation {
                    cell,
                    target: 5,
                    rationale: "Equal baseline".into(),
                    confidence: RecommendationConfidence::High,
                    expected_benefits: vec![ExpectedBenefit::Coverage],
                })
                .collect(),
            strategies: vec![],
            tradeoffs: vec![],
            uncertainties: vec![],
        },
    )
    .unwrap();
    run.await_review(ArchitectStopReason::ProposalSubmitted)
        .unwrap();
    store.save_proposal_and_run(&proposal, &run).await.unwrap();
    let review = ArchitectProposalReview::create(
        &proposal,
        None,
        ArchitectReviewDecision::Approve,
        "operator".into(),
        "Approved for generation".into(),
    )
    .unwrap();
    store.append_review(&review).await.unwrap();
    let applied =
        apply_approved_proposal(&proposal, &brief, &review, &brief.current_coverage).unwrap();
    store
        .save_application(
            &applied.application,
            &applied.plan,
            &applied.strategy_context,
        )
        .await
        .unwrap();

    assert_eq!(
        store.get_plan(applied.plan.id).await.unwrap(),
        Some(applied.plan.clone())
    );
    assert_eq!(
        store
            .generation_strategy_context_for_plan(applied.plan.id)
            .await
            .unwrap(),
        Some(applied.strategy_context)
    );
    assert_eq!(
        store.get_application(proposal.id).await.unwrap(),
        Some(applied.application)
    );
    let provenance = store
        .trace_provenance(ArtifactKind::GenerationPlan, applied.plan.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        provenance
            .parents
            .iter()
            .any(|parent| { parent.kind == ArtifactKind::DatasetArchitectureApplication })
    );
}
