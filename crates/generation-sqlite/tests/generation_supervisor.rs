use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use dataset_quality_core::{
    assessment::{
        EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence,
        GeneratorEvaluatorRelationship,
    },
    policy::BasisPoints,
};
use generation_core::{
    domain::{
        DatasetDefinition, GeneratedRow, GenerationParameters, GenerationRequest, ValidationStatus,
    },
    jobs::{
        GenerationAttempt, GenerationBackendIdentity, GenerationExecutionPolicy,
        GenerationExecutionSpec, GenerationJob,
    },
    planning::{calculate_generation_needs, equal_target_plan},
    ports::{DatasetStore, GenerationExecutionStore, PlanStore, RowStore},
    prompting::{PromptBuilder, SupervisedGenerationSchedule, SupervisedRowGuidance},
};
use generation_supervisor_core::{
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, BaselinePolicy, BatchQualityThresholds,
        GenerationQualityContract, GeneratorIdentity, MonitoringPolicy, MonitoringScope,
        PromptRevisionKind, PromptRevisionPolicy, ProtectedPromptField, RevisionApprovalPolicy,
        RowQualityThresholds, SupervisorBudgets,
    },
    decision::{DeterministicQualityDecision, SupervisorDecisionState},
    lifecycle::{ChildKind, ChildOutcome, ChildOutcomeState, ChildReservation, SupervisorRun},
    observation::{
        BatchQualityObservation, QualityScope, QualityWindowKind, RowQualityObservation,
    },
    ports::GenerationSupervisorStore,
    revision::PromptGuidanceVersion,
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

fn bp(value: u16) -> BasisPoints {
    BasisPoints::new(value).expect("basis points")
}

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("generation-supervisor.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

#[tokio::test]
async fn supervisor_evidence_round_trips_with_generation_provenance_and_integrity_checks() {
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

    let contract = quality_contract(&store, &dataset, &plan).await;
    store
        .create_contract(&contract)
        .await
        .expect("contract persists");
    let run_id = Uuid::new_v4();
    let prompt = PromptGuidanceVersion::initial(
        Uuid::new_v4(),
        run_id,
        "base-prompt-v1",
        "protected-fields-v1",
        vec!["Use authentic support phrasing.".into()],
        Utc::now(),
    )
    .expect("prompt");
    let run = SupervisorRun::create(
        run_id,
        &contract,
        prompt.id,
        prompt.fingerprint.clone(),
        Utc::now(),
    )
    .expect("run");
    store
        .create_run(&run, &prompt, None)
        .await
        .expect("run bundle persists");

    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "deterministic-v1", 1);
    let schedule = SupervisedGenerationSchedule::create(
        run.id,
        prompt.id,
        prompt.fingerprint.clone(),
        prompt.guidance.clone(),
        None,
        vec![SupervisedRowGuidance {
            cell_key: plan.cells[0].cell.key(),
            row_sequence: 0,
            strategy_assignment_fingerprint: None,
            strategy_directive_id: None,
            strategy_instructions: vec![],
        }],
    )
    .expect("supervision schedule");
    let base_template = PromptBuilder::template_identity().expect("template");
    let spec = GenerationExecutionSpec::new(
        job.id,
        dataset.id,
        plan.id,
        calculate_generation_needs(&plan, &BTreeMap::new()),
        GenerationBackendIdentity {
            name: job.backend_name.clone(),
            model: job.backend_model.clone(),
            endpoint: None,
        },
        GenerationParameters::default(),
        GenerationExecutionPolicy {
            batch_size: 1,
            max_request_retries: 0,
            max_attempt_multiplier: 1,
            retry_delay_milliseconds: 0,
        },
        PromptBuilder::supervision_template_identity(&base_template).expect("supervision template"),
        "no-semantic-context",
    )
    .expect("execution spec")
    .with_supervision_schedule(schedule.fingerprint.clone())
    .expect("pinned schedule");
    let segment_id = Uuid::new_v4();
    let reservation = ChildReservation::create(
        Uuid::new_v4(),
        run.id,
        ChildKind::GenerationSegment,
        "initial:billing:0",
        segment_id,
        1,
        None,
        artifact_core::fingerprint(&spec).expect("spec fingerprint"),
        Utc::now(),
    )
    .expect("reservation");
    store
        .reserve_child(&reservation)
        .await
        .expect("reservation persists before work");
    store
        .create_generation_execution(&job, &spec)
        .await
        .expect("ordinary generation execution persists");

    let request = GenerationRequest {
        system_prompt: "system".into(),
        user_prompt: "user".into(),
        target: plan.cells[0].cell.clone(),
        requested_count: 1,
        parameters: GenerationParameters::default(),
        construction: None,
    };
    let mut attempt = GenerationAttempt::start(job.id, 1, 0, &request).expect("attempt");
    store
        .start_generation_attempt(&attempt)
        .await
        .expect("attempt starts");
    let row = generated_row(&job, &plan, "Why was I charged twice?");
    attempt
        .succeed(1, 1, 1, 0, None, serde_json::json!({}), vec![])
        .expect("attempt succeeds");
    store
        .finish_generation_attempt(&attempt, std::slice::from_ref(&row))
        .await
        .expect("row and attempt commit atomically");
    let child_outcome = ChildOutcome::record(
        Uuid::new_v4(),
        &reservation,
        ChildOutcomeState::Succeeded,
        Some((
            job.id,
            artifact_core::fingerprint(&job).expect("job fingerprint"),
        )),
        None,
        Utc::now(),
    )
    .expect("child outcome");
    store
        .finish_child(&child_outcome)
        .await
        .expect("child outcome persists");

    let observation = RowQualityObservation::create(
        Uuid::new_v4(),
        &contract,
        run.id,
        segment_id,
        attempt.id,
        prompt.id,
        prompt.fingerprint.clone(),
        &row,
        None,
        None,
        false,
        false,
        BTreeSet::new(),
        None,
        Utc::now(),
    )
    .expect("observation");
    store
        .save_row_observations(std::slice::from_ref(&observation))
        .await
        .expect("row evidence persists");

    let aggregated = BatchQualityObservation::aggregate(
        Uuid::new_v4(),
        Uuid::new_v4(),
        &contract,
        run.id,
        prompt.id,
        prompt.fingerprint.clone(),
        QualityWindowKind::InitialCanary,
        0,
        QualityScope::Cell {
            cell_key: row.cell_key.clone(),
        },
        std::slice::from_ref(&observation),
        Utc::now(),
    )
    .expect("quality window");
    store
        .save_quality_window(&aggregated.manifest, &aggregated.observation)
        .await
        .expect("quality window persists");
    let decision = DeterministicQualityDecision::evaluate(
        Uuid::new_v4(),
        &contract,
        &aggregated.observation,
        None,
        Utc::now(),
    )
    .expect("decision");
    assert_eq!(
        decision.state,
        SupervisorDecisionState::InsufficientEvidence
    );
    store
        .save_decision(&decision)
        .await
        .expect("decision persists");

    let trace = store
        .trace_supervised_row(row.id)
        .await
        .expect("trace query")
        .expect("trace exists");
    assert_eq!(trace.row_observation.generated_row_id, row.id);
    assert_eq!(trace.row_observation.generation_attempt_id, attempt.id);
    assert_eq!(trace.windows, vec![aggregated.observation.clone()]);
    assert_eq!(trace.decisions, vec![decision]);

    let report = store
        .verify_supervisor_integrity()
        .await
        .expect("integrity report");
    assert!(report.healthy(), "{:#?}", report.errors);
    assert_eq!(report.contracts, 1);
    assert_eq!(report.row_observations, 1);
    assert_eq!(report.quality_windows, 1);
    assert_eq!(report.decisions, 1);

    assert!(
        sqlx::query(
            "UPDATE generation_supervisor_row_observations SET cell_key = 'tampered' WHERE id = ?"
        )
        .bind(observation.id)
        .execute(store.pool())
        .await
        .is_err(),
        "append-only evidence must reject updates"
    );
    assert!(
        sqlx::query("DELETE FROM generation_supervisor_row_observations WHERE id = ?")
            .bind(observation.id)
            .execute(store.pool())
            .await
            .is_err(),
        "append-only evidence must reject deletion"
    );
}

async fn quality_contract(
    store: &SqliteStore,
    dataset: &DatasetDefinition,
    plan: &generation_core::domain::GenerationPlan,
) -> GenerationQualityContract {
    let coverage = store
        .dataset_cell_counts(dataset.id)
        .await
        .expect("coverage");
    let generator = GeneratorIdentity::create(
        GenerationBackendIdentity {
            name: "fake".into(),
            model: "deterministic-v1".into(),
            endpoint: None,
        },
        "generation-v1",
        "generator-config-v1",
    )
    .expect("generator identity");
    let evaluator = EvaluatorIdentity::new(
        "quality-fake",
        "judge-v1",
        "quality-v1",
        "evaluator-config-v1",
        EvaluatorIndependence::Primary,
        EvaluatorExecutionLocation::LocalProcess,
    )
    .expect("evaluator identity");
    GenerationQualityContract::create(
        Uuid::new_v4(),
        ArtifactBinding::new(
            dataset.id,
            artifact_core::fingerprint(dataset).expect("dataset fingerprint"),
        )
        .expect("dataset binding"),
        ArtifactBinding::new(
            plan.id,
            artifact_core::fingerprint(plan).expect("plan fingerprint"),
        )
        .expect("plan binding"),
        AcceptedCoverageBinding::from_counts(&coverage).expect("coverage binding"),
        None,
        None,
        None,
        None,
        generator,
        evaluator,
        GeneratorEvaluatorRelationship::IndependentBackend,
        RowQualityThresholds {
            minimum_assigned_label_score: bp(7_000),
            minimum_label_margin: bp(500),
            minimum_dimension_score: bp(7_000),
            minimum_difficulty_score: None,
            minimum_authenticity_score: None,
            minimum_strategy_score: None,
            maximum_label_leakage_risk: bp(2_000),
            maximum_shortcut_risk: bp(2_000),
            minimum_evaluator_confidence: bp(7_000),
        },
        BatchQualityThresholds {
            minimum_qualified_rate: bp(8_000),
            maximum_borderline_rate: bp(2_000),
            maximum_quarantined_rate: bp(2_000),
            maximum_invalid_rate: bp(1_000),
            maximum_exact_duplicate_rate: bp(0),
            maximum_normalized_duplicate_rate: bp(1_000),
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
            maximum_generated_rows: 10,
            maximum_quality_audits: 2,
            maximum_evaluator_requests: 2,
            maximum_evaluator_attempts: 4,
            maximum_prompt_revisions: 1,
            maximum_revision_canaries: 1,
            maximum_pi_model_turns: 2,
            maximum_pi_tool_calls: 4,
            maximum_pi_input_tokens: 2_000,
            maximum_pi_output_tokens: 1_000,
            maximum_retries_per_external_call: 1,
            maximum_duration_seconds: 60,
            maximum_cost_microunits: None,
        },
        RevisionApprovalPolicy::ExplicitReview,
        PromptRevisionPolicy {
            maximum_instructions: 4,
            maximum_characters_per_instruction: 200,
            maximum_total_characters: 500,
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
    .expect("quality contract")
}

fn generated_row(
    job: &GenerationJob,
    plan: &generation_core::domain::GenerationPlan,
    text: &str,
) -> GeneratedRow {
    GeneratedRow {
        id: Uuid::new_v4(),
        dataset_id: job.dataset_id,
        plan_id: job.plan_id,
        generation_job_id: job.id,
        cell_key: plan.cells[0].cell.key(),
        text: text.into(),
        normalized_text: text.to_ascii_lowercase(),
        label: plan.cells[0].cell.label.clone(),
        dimensions: BTreeMap::new(),
        fields: BTreeMap::new(),
        construction: None,
        generator_backend: job.backend_name.clone(),
        generator_model: job.backend_model.clone(),
        created_at: Utc::now(),
        validation_status: ValidationStatus::Accepted,
        validation_errors: vec![],
        generation_metadata: serde_json::json!({}),
    }
}
