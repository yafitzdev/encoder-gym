use std::{collections::BTreeSet, sync::Arc, time::Duration};

use chrono::Utc;
use dataset_quality_core::{
    assessment::{EvaluatorGuidance, GeneratorEvaluatorRelationship},
    policy::{
        AuditBudgets, AuditMode, BorderlineReviewPolicy, EvaluatorEgressPolicy,
        InvalidEvaluatorOutputPolicy, QualityPolicy, QualityThresholds,
    },
    ports::{QualityCandidateSource, QualityEvaluator},
};
use dataset_quality_fake::FakeQualityEvaluator;
use generation_core::{
    construction::RowConstructionPlan,
    domain::{DatasetDefinition, GenerationParameters},
    jobs::{GenerationBackendIdentity, JobRunnerPolicy},
    planning::equal_target_plan,
    ports::{DatasetStore, GenerationStore, PlanStore, RowQuery, RowStore},
};
use generation_fake::FakeGenerationBackend;
use generation_supervisor_core::{
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, BaselinePolicy, BatchQualityThresholds,
        GenerationQualityContract, GeneratorIdentity, MonitoringPolicy, MonitoringScope,
        PromptRevisionKind, PromptRevisionPolicy, ProtectedPromptField, RevisionApprovalPolicy,
        RowQualityThresholds, SupervisorBudgets,
    },
    decision::{SupervisorDecisionState, SupervisorIssueCode},
    lifecycle::SupervisorRunState,
    ports::{GenerationSupervisorStore, SupervisedRowTrace},
};
use generation_supervisor_runner::orchestration::{
    GenerationQualitySupervisorRunner, SupervisorExecutionConfiguration,
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

fn bp(value: u16) -> dataset_quality_core::policy::BasisPoints {
    dataset_quality_core::policy::BasisPoints::new(value).expect("basis points")
}

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("supervisor-loop.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

#[tokio::test]
async fn weak_generated_segment_is_independently_audited_and_paused() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into()],
        vec![],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persists");
    let plan = equal_target_plan(&dataset, 1).expect("plan");
    store.create_plan(&plan).await.expect("plan persists");

    let evaluator: Arc<dyn QualityEvaluator> = Arc::new(FakeQualityEvaluator::default());
    let row_thresholds = RowQualityThresholds {
        minimum_assigned_label_score: bp(7_000),
        minimum_label_margin: bp(500),
        minimum_dimension_score: bp(7_000),
        minimum_difficulty_score: None,
        minimum_authenticity_score: None,
        minimum_strategy_score: None,
        maximum_label_leakage_risk: bp(2_000),
        maximum_shortcut_risk: bp(2_000),
        minimum_evaluator_confidence: bp(7_000),
    };
    let contract = GenerationQualityContract::create(
        Uuid::new_v4(),
        ArtifactBinding::new(
            dataset.id,
            artifact_core::fingerprint(&dataset).expect("dataset fingerprint"),
        )
        .expect("dataset binding"),
        ArtifactBinding::new(
            plan.id,
            artifact_core::fingerprint(&plan).expect("plan fingerprint"),
        )
        .expect("plan binding"),
        AcceptedCoverageBinding::from_counts(
            &store
                .dataset_cell_counts(dataset.id)
                .await
                .expect("coverage"),
        )
        .expect("coverage binding"),
        None,
        None,
        None,
        None,
        GeneratorIdentity::create(
            GenerationBackendIdentity {
                name: "fake".into(),
                model: "deterministic-v1".into(),
                endpoint: None,
            },
            "generation-v1",
            "fake-generator-config-v1",
        )
        .expect("generator"),
        evaluator.identity(),
        GeneratorEvaluatorRelationship::IndependentBackend,
        row_thresholds.clone(),
        BatchQualityThresholds {
            minimum_qualified_rate: bp(8_000),
            maximum_borderline_rate: bp(2_000),
            maximum_quarantined_rate: bp(2_000),
            maximum_invalid_rate: bp(1_000),
            maximum_exact_duplicate_rate: bp(0),
            maximum_normalized_duplicate_rate: bp(0),
            maximum_template_repetition_rate: bp(2_000),
            maximum_qualified_rate_drop: bp(1_000),
            maximum_shortcut_concentration: bp(2_000),
            required_patterns: BTreeSet::new(),
        },
        MonitoringPolicy {
            initial_canary_rows_per_scope: 1,
            revision_canary_rows_per_scope: 1,
            rolling_window_rows_per_scope: 1,
            minimum_evidence_rows_per_scope: 1,
            baseline_minimum_rows_per_scope: 1,
            scope: MonitoringScope::Cell,
            baseline_policy: BaselinePolicy::InitialCanary,
            systemic_pause_minimum_scopes: 2,
        },
        SupervisorBudgets {
            maximum_generation_segments: 2,
            maximum_generated_rows: 4,
            maximum_quality_audits: 2,
            maximum_evaluator_requests: 2,
            maximum_evaluator_attempts: 2,
            maximum_prompt_revisions: 0,
            maximum_revision_canaries: 1,
            maximum_pi_model_turns: 1,
            maximum_pi_tool_calls: 1,
            maximum_pi_input_tokens: 1_000,
            maximum_pi_output_tokens: 500,
            maximum_retries_per_external_call: 1,
            maximum_duration_seconds: 60,
            maximum_cost_microunits: None,
        },
        RevisionApprovalPolicy::ExplicitReview,
        PromptRevisionPolicy {
            maximum_instructions: 2,
            maximum_characters_per_instruction: 200,
            maximum_total_characters: 300,
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
        Utc::now(),
    )
    .expect("contract");
    store
        .create_contract(&contract)
        .await
        .expect("contract persists");

    let quality_policy = QualityPolicy::new(
        None,
        QualityThresholds {
            minimum_assigned_label_score: row_thresholds.minimum_assigned_label_score,
            minimum_label_margin: row_thresholds.minimum_label_margin,
            minimum_dimension_adherence_score: row_thresholds.minimum_dimension_score,
            minimum_authenticity_score: None,
            maximum_label_leakage_risk: row_thresholds.maximum_label_leakage_risk,
            maximum_shortcut_risk: row_thresholds.maximum_shortcut_risk,
            minimum_evaluator_confidence: row_thresholds.minimum_evaluator_confidence,
            borderline_margin: bp(500),
        },
        InvalidEvaluatorOutputPolicy::Quarantine,
        BorderlineReviewPolicy::None,
        AuditBudgets {
            maximum_rows_per_batch: 1,
            maximum_evaluator_requests: 1,
            maximum_attempts_per_request: 1,
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 10_000,
            maximum_total_tokens: 20_000,
            maximum_cost_microusd: None,
        },
        EvaluatorEgressPolicy::LocalOnly,
        AuditMode::FullPopulation,
    )
    .expect("quality policy");
    let shared = Arc::new(store.clone());
    let generation_store: Arc<dyn GenerationStore> = shared.clone();
    let supervisor_store: Arc<dyn GenerationSupervisorStore> = shared.clone();
    let quality_store: Arc<dyn dataset_quality_core::ports::DatasetQualityStore> = shared.clone();
    let candidates: Arc<dyn QualityCandidateSource> = shared;
    let runner = GenerationQualitySupervisorRunner::new(
        generation_store,
        supervisor_store,
        quality_store,
        candidates,
        Arc::new(FakeGenerationBackend::default()),
        vec![evaluator],
        SupervisorExecutionConfiguration {
            generation_parameters: GenerationParameters::default(),
            generation_policy: JobRunnerPolicy {
                batch_size: 1,
                max_request_retries: 0,
                max_attempt_multiplier: 2,
                retry_delay: Duration::ZERO,
            },
            quality_policy,
            evaluator_guidance: EvaluatorGuidance::default(),
            text_length: None,
            construction_plan: RowConstructionPlan::llm_text_default().expect("construction plan"),
            semantic_context: None,
            authenticity_context: None,
            authenticity_source_excerpts: vec![],
            strategy_context: None,
        },
    )
    .expect("runner");

    let queued = runner
        .start(
            contract.id,
            vec!["Use natural customer phrasing.".into()],
            7,
        )
        .await
        .expect("run starts");
    assert_eq!(queued.state, SupervisorRunState::Queued);
    let outcome = runner
        .run_until_boundary(queued.run.id)
        .await
        .expect("finite loop executes");

    assert_eq!(outcome.status.state, SupervisorRunState::Paused);
    assert_eq!(outcome.status.generation_segments, 1);
    assert_eq!(outcome.status.quality_audits, 1);
    assert_eq!(outcome.status.observed_rows, 1);
    assert_eq!(outcome.decisions.len(), 1);
    assert_eq!(
        outcome.decisions[0].state,
        SupervisorDecisionState::PauseForDiagnosis
    );
    assert!(
        outcome.decisions[0]
            .issues
            .contains(&SupervisorIssueCode::ShortcutRisk)
            || outcome.decisions[0]
                .issues
                .contains(&SupervisorIssueCode::ShortcutConcentration)
    );

    let generated_rows = store
        .list_rows(RowQuery {
            job_id: outcome.generation_job_id,
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rows load");
    let trace: SupervisedRowTrace = store
        .trace_supervised_row(generated_rows[0].id)
        .await
        .expect("trace loads")
        .expect("trace exists");
    assert_eq!(trace.decisions, outcome.decisions);
    let integrity = runner.verify_integrity().await.expect("integrity verifies");
    assert!(integrity.healthy(), "{:#?}", integrity.errors);
}
