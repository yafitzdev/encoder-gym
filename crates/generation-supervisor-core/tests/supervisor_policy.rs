use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, TimeZone, Utc};
use dataset_quality_core::{
    assessment::{
        EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence,
        GeneratorEvaluatorRelationship, QualityIssueCode, QualityVerdict,
    },
    policy::{
        AuditBudgets, AuditMode, BasisPoints, BorderlineReviewPolicy, EvaluatorEgressPolicy,
        InvalidEvaluatorOutputPolicy, QualityPolicy, QualityThresholds,
    },
};
use generation_core::{
    domain::{GeneratedRow, ValidationStatus},
    jobs::GenerationBackendIdentity,
};
use generation_supervisor_core::{
    SupervisorError,
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, BaselinePolicy, BatchQualityThresholds,
        ConfigurationBinding, GenerationQualityContract, GeneratorIdentity, MonitoringPolicy,
        MonitoringScope, PromptRevisionKind, PromptRevisionPolicy, ProtectedPromptField,
        RevisionApprovalPolicy, RowQualityThresholds, SupervisorBudgets,
    },
    decision::{
        DeterministicQualityDecision, QualityFailureKind, SupervisorDecisionState,
        SupervisorIssueCode, SystemicPauseDecision,
    },
    lifecycle::{
        ChildKind, ChildOutcome, ChildOutcomeState, ChildReservation, SupervisorRunEvent,
        SupervisorRunState, SupervisorUsage,
    },
    observation::{
        AssessmentEvidence, BatchQualityObservation, QualityScope, QualityWindowKind,
        RowCriterionFailure, RowQualityObservation,
    },
    revision::{
        AdvisorRuntimeIdentity, AdvisorUsage, DiagnosisCause, ExpectedImprovement,
        PromptGuidanceVersion, PromptRevisionActivation, PromptRevisionAuthorization,
        PromptRevisionProposal, PromptRevisionReview, RevisionReviewDecision, SupervisorDiagnosis,
    },
};
use uuid::Uuid;

fn bp(value: u16) -> BasisPoints {
    BasisPoints::new(value).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap()
}

fn contract() -> GenerationQualityContract {
    let generator = GeneratorIdentity::create(
        GenerationBackendIdentity {
            name: "generator".into(),
            model: "writer-v1".into(),
            endpoint: None,
        },
        "generation-v1",
        "generator-config",
    )
    .unwrap();
    let evaluator = EvaluatorIdentity::new(
        "judge",
        "judge-v1",
        "quality-v1",
        "judge-config",
        EvaluatorIndependence::Primary,
        EvaluatorExecutionLocation::LocalProcess,
    )
    .unwrap();
    let row_thresholds = RowQualityThresholds {
        minimum_assigned_label_score: bp(7_000),
        minimum_label_margin: bp(1_000),
        minimum_dimension_score: bp(7_000),
        minimum_difficulty_score: None,
        minimum_authenticity_score: None,
        minimum_strategy_score: None,
        maximum_label_leakage_risk: bp(2_000),
        maximum_shortcut_risk: bp(2_000),
        minimum_evaluator_confidence: bp(7_000),
    };
    let quality_policy = QualityPolicy::new(
        None,
        QualityThresholds {
            minimum_assigned_label_score: row_thresholds.minimum_assigned_label_score,
            minimum_label_margin: row_thresholds.minimum_label_margin,
            minimum_dimension_adherence_score: row_thresholds.minimum_dimension_score,
            minimum_authenticity_score: row_thresholds.minimum_authenticity_score,
            maximum_label_leakage_risk: row_thresholds.maximum_label_leakage_risk,
            maximum_shortcut_risk: row_thresholds.maximum_shortcut_risk,
            minimum_evaluator_confidence: row_thresholds.minimum_evaluator_confidence,
            borderline_margin: bp(500),
        },
        InvalidEvaluatorOutputPolicy::Quarantine,
        BorderlineReviewPolicy::None,
        AuditBudgets {
            maximum_rows_per_batch: 10,
            maximum_evaluator_requests: 20,
            maximum_attempts_per_request: 2,
            maximum_input_tokens: 100_000,
            maximum_output_tokens: 50_000,
            maximum_total_tokens: 150_000,
            maximum_cost_microusd: None,
        },
        EvaluatorEgressPolicy::LocalOnly,
        AuditMode::FullPopulation,
    )
    .unwrap();
    GenerationQualityContract::create(
        Uuid::from_u128(1),
        ArtifactBinding::new(Uuid::from_u128(2), "dataset-fp").unwrap(),
        ArtifactBinding::new(Uuid::from_u128(3), "plan-fp").unwrap(),
        AcceptedCoverageBinding {
            accepted_rows: 0,
            fingerprint: "coverage-fp".into(),
        },
        Some(ArtifactBinding::new(Uuid::from_u128(4), "semantic-fp").unwrap()),
        None,
        ConfigurationBinding::new("construction-fp").unwrap(),
        None,
        generator,
        evaluator,
        GeneratorEvaluatorRelationship::IndependentBackend,
        quality_policy,
        row_thresholds,
        BatchQualityThresholds {
            minimum_qualified_rate: bp(7_500),
            maximum_borderline_rate: bp(2_000),
            maximum_quarantined_rate: bp(2_500),
            maximum_invalid_rate: bp(1_000),
            maximum_exact_duplicate_rate: bp(0),
            maximum_normalized_duplicate_rate: bp(1_000),
            maximum_template_repetition_rate: bp(10_000),
            maximum_qualified_rate_drop: bp(1_000),
            maximum_shortcut_concentration: bp(2_000),
            required_patterns: BTreeSet::new(),
        },
        MonitoringPolicy {
            initial_canary_rows_per_scope: 5,
            revision_canary_rows_per_scope: 5,
            rolling_window_rows_per_scope: 10,
            minimum_evidence_rows_per_scope: 3,
            baseline_minimum_rows_per_scope: 3,
            scope: MonitoringScope::CellAndStrategy,
            baseline_policy: BaselinePolicy::InitialCanary,
            systemic_pause_minimum_scopes: 2,
        },
        SupervisorBudgets {
            maximum_generation_segments: 4,
            maximum_generated_rows: 100,
            maximum_quality_audits: 4,
            maximum_evaluator_requests: 20,
            maximum_evaluator_attempts: 30,
            maximum_evaluator_input_tokens: 400_000,
            maximum_evaluator_output_tokens: 200_000,
            maximum_evaluator_total_tokens: 600_000,
            maximum_prompt_revisions: 2,
            maximum_revision_canaries: 2,
            maximum_pi_model_turns: 5,
            maximum_pi_tool_calls: 10,
            maximum_pi_input_tokens: 10_000,
            maximum_pi_output_tokens: 2_000,
            maximum_retries_per_external_call: 2,
            maximum_duration_seconds: 600,
            maximum_cost_microunits: Some(10_000),
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
        now(),
    )
    .unwrap()
}

#[test]
fn sealed_acceptance_artifacts_cannot_enter_the_supervision_contract() {
    let mut value = contract();
    value.semantic_context =
        Some(ArtifactBinding::sealed_acceptance(Uuid::new_v4(), "sealed-evidence").unwrap());
    let error = value
        .validate()
        .expect_err("sealed evidence must be rejected");
    assert!(matches!(error, SupervisorError::Validation(_)));

    let mut value = contract();
    value.dataset.exposure =
        generation_supervisor_core::contract::ArtifactExposure::SealedAcceptance;
    let error = value
        .validate()
        .expect_err("sealed dataset binding must be rejected");
    assert!(matches!(error, SupervisorError::Validation(_)));
}

#[test]
fn shared_generator_evaluator_identity_cannot_claim_independence() {
    let mut value = contract();
    value.evaluator = EvaluatorIdentity::new(
        value.generator.backend.name.clone(),
        value.generator.backend.model.clone(),
        "quality-v1",
        "same-model-evaluator-config",
        EvaluatorIndependence::Primary,
        EvaluatorExecutionLocation::LocalProcess,
    )
    .unwrap();
    value.generator_evaluator_relationship = GeneratorEvaluatorRelationship::IndependentBackend;
    value.fingerprint = value.reproduce_fingerprint().unwrap();
    assert!(matches!(
        value.validate(),
        Err(SupervisorError::Validation(_))
    ));
}

fn evidence(contract: &GenerationQualityContract, score: u16) -> AssessmentEvidence {
    AssessmentEvidence {
        assessment_id: Uuid::new_v4(),
        assessment_fingerprint: format!("assessment-{score}-{}", Uuid::new_v4()),
        evaluator_fingerprint: contract.evaluator.fingerprint.clone(),
        generator_relationship: contract.generator_evaluator_relationship,
        provider_verdict: if score >= 7_000 {
            QualityVerdict::Qualified
        } else {
            QualityVerdict::Quarantined
        },
        assigned_label_score: bp(score),
        strongest_competing_label: Some("other-label".into()),
        assigned_label_margin: i32::from(score.saturating_sub(5_000)),
        assigned_dimension_scores: BTreeMap::new(),
        difficulty_score: None,
        authenticity_score: None,
        strategy_score: None,
        label_leakage_risk: bp(500),
        shortcut_risk: bp(500),
        confidence: bp(9_000),
        issue_codes: if score >= 7_000 {
            vec![]
        } else {
            vec![QualityIssueCode::InsufficientContext]
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn row_observation(
    contract: &GenerationQualityContract,
    run_id: Uuid,
    prompt_id: Uuid,
    index: u128,
    text: &str,
    score: u16,
    structural_status: ValidationStatus,
) -> RowQualityObservation {
    let row = GeneratedRow {
        id: Uuid::from_u128(1_000 + index),
        dataset_id: contract.dataset.id,
        plan_id: contract.plan.id,
        generation_job_id: Uuid::from_u128(20),
        cell_key: "billing/easy".into(),
        text: text.into(),
        normalized_text: text.to_lowercase(),
        label: "billing".into(),
        dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
        fields: BTreeMap::new(),
        construction: None,
        generator_backend: "generator".into(),
        generator_model: "writer-v1".into(),
        created_at: now(),
        validation_status: structural_status,
        validation_errors: if structural_status == ValidationStatus::Rejected {
            vec!["empty text".into()]
        } else {
            vec![]
        },
        generation_metadata: serde_json::Value::Null,
    };
    RowQualityObservation::create(
        Uuid::from_u128(2_000 + index),
        contract,
        run_id,
        Uuid::from_u128(21),
        Uuid::from_u128(22),
        prompt_id,
        "prompt-fp",
        &row,
        None,
        None,
        false,
        false,
        BTreeSet::new(),
        (structural_status == ValidationStatus::Accepted).then(|| evidence(contract, score)),
        now(),
    )
    .unwrap()
}

fn window(
    contract: &GenerationQualityContract,
    run_id: Uuid,
    id: u128,
    kind: QualityWindowKind,
    scores: &[u16],
    created_at: DateTime<Utc>,
) -> BatchQualityObservation {
    let prompt_id = Uuid::from_u128(30);
    let rows = scores
        .iter()
        .enumerate()
        .map(|(index, score)| {
            row_observation(
                contract,
                run_id,
                prompt_id,
                id * 100 + u128::try_from(index).unwrap(),
                &format!("Billing request {id} number {index}"),
                *score,
                ValidationStatus::Accepted,
            )
        })
        .collect::<Vec<_>>();
    BatchQualityObservation::aggregate(
        Uuid::from_u128(id),
        Uuid::from_u128(id + 10_000),
        contract,
        run_id,
        prompt_id,
        "prompt-fp",
        kind,
        u32::try_from(id).unwrap(),
        QualityScope::CellStrategy {
            cell_key: "billing/easy".into(),
            directive_id: None,
        },
        &rows,
        created_at,
    )
    .unwrap()
    .observation
}

#[test]
fn persisted_row_facts_detect_immediate_weakness() {
    let contract = contract();
    let run_id = Uuid::from_u128(40);
    let weak = window(
        &contract,
        run_id,
        100,
        QualityWindowKind::InitialCanary,
        &[9_000, 4_000, 4_000, 4_000, 4_000],
        now(),
    );
    let decision =
        DeterministicQualityDecision::evaluate(Uuid::new_v4(), &contract, &weak, None, now())
            .unwrap();
    assert_eq!(decision.state, SupervisorDecisionState::PauseForDiagnosis);
    assert_eq!(
        decision.failure_kind,
        Some(QualityFailureKind::ImmediateWeakness)
    );
    assert!(
        decision
            .issues
            .contains(&SupervisorIssueCode::WeakQualifiedRate)
    );
    assert!(
        decision
            .issues
            .contains(&SupervisorIssueCode::AssignedLabelWeakness)
    );
}

#[test]
fn later_quality_drop_is_longitudinal_drift_even_above_absolute_floor() {
    let contract = contract();
    let run_id = Uuid::from_u128(41);
    let baseline = window(
        &contract,
        run_id,
        101,
        QualityWindowKind::InitialCanary,
        &[9_000; 10],
        now(),
    );
    let later = window(
        &contract,
        run_id,
        102,
        QualityWindowKind::Rolling,
        &[
            9_000, 9_000, 9_000, 9_000, 9_000, 9_000, 9_000, 9_000, 4_000, 4_000,
        ],
        now() + Duration::minutes(1),
    );
    assert_eq!(later.rates.qualified.get(), 8_000);
    let decision = DeterministicQualityDecision::evaluate(
        Uuid::new_v4(),
        &contract,
        &later,
        Some(&baseline),
        now() + Duration::minutes(1),
    )
    .unwrap();
    assert_eq!(
        decision.failure_kind,
        Some(QualityFailureKind::LongitudinalDrift)
    );
    assert!(
        decision
            .issues
            .contains(&SupervisorIssueCode::QualifiedRateDrift)
    );
}

#[test]
fn minimum_support_prevents_premature_drift_claim() {
    let contract = contract();
    let run_id = Uuid::from_u128(42);
    let tiny = window(
        &contract,
        run_id,
        103,
        QualityWindowKind::Rolling,
        &[4_000, 4_000],
        now(),
    );
    let decision =
        DeterministicQualityDecision::evaluate(Uuid::new_v4(), &contract, &tiny, None, now())
            .unwrap();
    assert_eq!(
        decision.state,
        SupervisorDecisionState::InsufficientEvidence
    );
    assert_eq!(decision.failure_kind, None);
}

#[test]
fn semantic_quality_failures_remain_independently_visible() {
    let mut contract = contract();
    contract.authenticity_context =
        Some(ArtifactBinding::new(Uuid::from_u128(4_199), "authenticity-fp").unwrap());
    contract.strategy_context =
        Some(ArtifactBinding::new(Uuid::from_u128(4_198), "strategy-fp").unwrap());
    contract.row_thresholds.minimum_difficulty_score = Some(bp(7_000));
    contract.row_thresholds.minimum_authenticity_score = Some(bp(7_000));
    contract.row_thresholds.minimum_strategy_score = Some(bp(7_000));
    let mut policy_thresholds = contract.quality_policy.thresholds.clone();
    policy_thresholds.minimum_authenticity_score = Some(bp(7_000));
    contract.quality_policy = QualityPolicy::new(
        None,
        policy_thresholds,
        contract.quality_policy.invalid_output_policy,
        contract.quality_policy.borderline_review_policy,
        contract.quality_policy.budgets.clone(),
        contract.quality_policy.egress_policy,
        contract.quality_policy.audit_mode,
    )
    .unwrap();
    contract.fingerprint = contract.reproduce_fingerprint().unwrap();
    contract.validate().unwrap();

    let failures = |kind: RowCriterionFailure| {
        let mut row = row_observation(
            &contract,
            Uuid::from_u128(4_200),
            Uuid::from_u128(4_201),
            77 + kind as u128,
            "A concrete customer situation with enough context",
            9_000,
            ValidationStatus::Accepted,
        );
        if kind == RowCriterionFailure::StrategyAdherence {
            row.strategy_directive_id = Some(Uuid::from_u128(4_202));
            row.strategy_assignment_fingerprint = Some("strategy-assignment".into());
        }
        let evidence = row.assessment.as_mut().unwrap();
        evidence.provider_verdict = QualityVerdict::Qualified;
        evidence.difficulty_score = Some(bp(9_000));
        evidence.authenticity_score = Some(bp(9_000));
        evidence.strategy_score = Some(bp(9_000));
        evidence.issue_codes.clear();
        match kind {
            RowCriterionFailure::DifficultyAdherence => {
                evidence.difficulty_score = Some(bp(4_000));
            }
            RowCriterionFailure::AuthenticityAdherence => {
                evidence.authenticity_score = Some(bp(4_000));
            }
            RowCriterionFailure::StrategyAdherence => {
                evidence.strategy_score = Some(bp(4_000));
            }
            RowCriterionFailure::LabelLeakage => evidence.label_leakage_risk = bp(4_000),
            RowCriterionFailure::ShortcutRisk => evidence.shortcut_risk = bp(4_000),
            _ => unreachable!("case is limited to independently scored semantic criteria"),
        }
        row.criterion_failures(&contract)
    };

    for expected in [
        RowCriterionFailure::DifficultyAdherence,
        RowCriterionFailure::AuthenticityAdherence,
        RowCriterionFailure::StrategyAdherence,
        RowCriterionFailure::LabelLeakage,
        RowCriterionFailure::ShortcutRisk,
    ] {
        assert_eq!(failures(expected), BTreeSet::from([expected]));
    }
}

#[test]
fn pause_is_scoped_until_explicit_systemic_threshold_is_crossed() {
    let contract = contract();
    let run_id = Uuid::from_u128(43);
    let weak = window(
        &contract,
        run_id,
        104,
        QualityWindowKind::Rolling,
        &[4_000; 5],
        now(),
    );
    let first =
        DeterministicQualityDecision::evaluate(Uuid::from_u128(501), &contract, &weak, None, now())
            .unwrap();
    let one =
        SystemicPauseDecision::evaluate(&contract, run_id, std::slice::from_ref(&first)).unwrap();
    assert!(!one.global_pause);
    let mut second_window = weak;
    second_window.id = Uuid::from_u128(105);
    second_window.scope = QualityScope::CellStrategy {
        cell_key: "billing/hard".into(),
        directive_id: None,
    };
    second_window.fingerprint = second_window.reproduce_fingerprint().unwrap();
    let second = DeterministicQualityDecision::evaluate(
        Uuid::from_u128(502),
        &contract,
        &second_window,
        None,
        now(),
    )
    .unwrap();
    let systemic = SystemicPauseDecision::evaluate(&contract, run_id, &[first, second]).unwrap();
    assert!(systemic.global_pause);
}

#[test]
fn guidance_revision_needs_exact_review_and_successful_canary() {
    let contract = contract();
    let run_id = Uuid::from_u128(44);
    let weak = window(
        &contract,
        run_id,
        106,
        QualityWindowKind::InitialCanary,
        &[4_000; 5],
        now(),
    );
    let decision =
        DeterministicQualityDecision::evaluate(Uuid::from_u128(601), &contract, &weak, None, now())
            .unwrap();
    let runtime = AdvisorRuntimeIdentity {
        runtime: "pi".into(),
        model: "advisor-v1".into(),
        protocol_version: "pi-jsonl-v1".into(),
        capability_set_version: 1,
        configuration_fingerprint: "pi-config".into(),
    };
    let diagnosis = SupervisorDiagnosis::create(
        Uuid::from_u128(602),
        run_id,
        &decision,
        DiagnosisCause::PromptGuidance,
        "The prompt rewards generic one-line examples.",
        true,
        runtime.clone(),
        AdvisorUsage {
            model_turns: 1,
            tool_calls: 3,
            input_tokens: 100,
            output_tokens: 40,
            cost_microunits: Some(100),
        },
        now(),
    )
    .unwrap();
    let parent = PromptGuidanceVersion::initial(
        Uuid::from_u128(603),
        run_id,
        "base-prompt-fp",
        "protected-fp",
        vec!["Write realistic support requests.".into()],
        now(),
    )
    .unwrap();
    let proposal = PromptRevisionProposal::create(
        Uuid::from_u128(604),
        &contract,
        &decision,
        &diagnosis,
        &parent,
        1,
        BTreeSet::from([decision.scope.clone()]),
        vec![
            "Vary sentence structure and include concrete situational detail without naming the label."
                .into(),
        ],
        vec![ExpectedImprovement {
            metric: "qualified_rate".into(),
            minimum_delta_basis_points: 1_000,
        }],
        runtime,
        AdvisorUsage::default(),
        now(),
    )
    .unwrap();
    assert!(matches!(
        PromptRevisionAuthorization::authorize(&contract, &proposal, &parent, None, now()),
        Err(SupervisorError::InvalidTransition(_))
    ));
    let review = PromptRevisionReview::create(
        Uuid::from_u128(605),
        &proposal,
        None,
        RevisionReviewDecision::Approve,
        "operator",
        "The patch changes guidance only.",
        now(),
    )
    .unwrap();
    let (authorization, candidate) =
        PromptRevisionAuthorization::authorize(&contract, &proposal, &parent, Some(&review), now())
            .unwrap();
    assert_eq!(
        candidate.protected_fields_fingerprint,
        parent.protected_fields_fingerprint
    );

    let mut canary_window = window(
        &contract,
        run_id,
        107,
        QualityWindowKind::RevisionCanary,
        &[9_000; 5],
        now() + Duration::minutes(2),
    );
    canary_window.prompt_version_id = candidate.id;
    canary_window.prompt_version_fingerprint = candidate.fingerprint.clone();
    canary_window.fingerprint = canary_window.reproduce_fingerprint().unwrap();
    let canary_decision = DeterministicQualityDecision::evaluate(
        Uuid::from_u128(606),
        &contract,
        &canary_window,
        None,
        now() + Duration::minutes(2),
    )
    .unwrap();
    assert_eq!(
        canary_decision.state,
        SupervisorDecisionState::RevisionPassed
    );

    let mut failed_window = window(
        &contract,
        run_id,
        108,
        QualityWindowKind::RevisionCanary,
        &[4_000; 5],
        now() + Duration::minutes(2),
    );
    failed_window.prompt_version_id = candidate.id;
    failed_window.prompt_version_fingerprint = candidate.fingerprint.clone();
    failed_window.fingerprint = failed_window.reproduce_fingerprint().unwrap();
    let failed_decision = DeterministicQualityDecision::evaluate(
        Uuid::from_u128(608),
        &contract,
        &failed_window,
        None,
        now() + Duration::minutes(2),
    )
    .unwrap();
    assert_eq!(
        failed_decision.state,
        SupervisorDecisionState::RevisionFailed
    );
    assert!(matches!(
        PromptRevisionActivation::create(
            Uuid::from_u128(609),
            &candidate,
            &authorization,
            &failed_decision,
            now() + Duration::minutes(2),
        ),
        Err(SupervisorError::InvalidTransition(_))
    ));

    PromptRevisionActivation::create(
        Uuid::from_u128(607),
        &candidate,
        &authorization,
        &canary_decision,
        now() + Duration::minutes(2),
    )
    .unwrap();
}

#[test]
fn every_finite_budget_and_external_intent_fails_closed() {
    let contract = contract();
    let full = SupervisorUsage {
        generation_segments: 4,
        generated_rows: 100,
        quality_audits: 4,
        evaluator_requests: 20,
        evaluator_attempts: 30,
        evaluator_input_tokens: 400_000,
        evaluator_output_tokens: 200_000,
        evaluator_total_tokens: 600_000,
        prompt_revisions: 2,
        revision_canaries: 2,
        pi_model_turns: 5,
        pi_tool_calls: 10,
        pi_input_tokens: 10_000,
        pi_output_tokens: 2_000,
        external_retries: 2,
        elapsed_seconds: 600,
        cost_microunits: 10_000,
    };
    assert!(full.reserve(&SupervisorUsage::default(), &contract).is_ok());
    for delta in [
        SupervisorUsage {
            generation_segments: 1,
            ..Default::default()
        },
        SupervisorUsage {
            generated_rows: 1,
            ..Default::default()
        },
        SupervisorUsage {
            evaluator_total_tokens: 1,
            ..Default::default()
        },
        SupervisorUsage {
            pi_tool_calls: 1,
            ..Default::default()
        },
        SupervisorUsage {
            cost_microunits: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            full.reserve(&delta, &contract),
            Err(SupervisorError::BudgetExhausted(_))
        ));
    }

    let reservation = ChildReservation::create(
        Uuid::from_u128(700),
        Uuid::from_u128(701),
        ChildKind::PiModelTurn,
        "diagnosis:1:turn:1",
        Uuid::from_u128(702),
        1,
        None,
        "request-fp",
        now(),
    )
    .unwrap()
    .with_reserved_usage(SupervisorUsage {
        pi_model_turns: 1,
        ..Default::default()
    })
    .unwrap();
    let interrupted = ChildOutcome::record(
        Uuid::from_u128(703),
        &reservation,
        ChildOutcomeState::Interrupted,
        None,
        Some("process outcome uncertain".into()),
        now(),
    )
    .unwrap();
    assert_eq!(interrupted.reservation_id, reservation.id);
}

#[test]
fn lifecycle_is_an_append_only_validated_chain() {
    let run_id = Uuid::from_u128(800);
    let running = SupervisorRunEvent::transition(
        Uuid::from_u128(801),
        run_id,
        None,
        SupervisorRunState::Queued,
        SupervisorRunState::Running,
        "operator started run",
        now(),
    )
    .unwrap();
    let generating = SupervisorRunEvent::transition(
        Uuid::from_u128(802),
        run_id,
        Some(&running),
        SupervisorRunState::Running,
        SupervisorRunState::Generating,
        "reserved finite generation segment",
        now(),
    )
    .unwrap();
    assert_eq!(
        SupervisorRunEvent::verify_chain(run_id, &[running, generating]).unwrap(),
        SupervisorRunState::Generating
    );
}
