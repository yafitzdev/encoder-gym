use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignStore, ProductionCampaign,
    bind_generation_event, first_campaign_event, prepare_iteration_event, replay_campaign,
    start_run_event,
};
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    journal::{ExperimentEventKind, first_event, replay_experiment},
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition, assess_candidate,
    },
    ports::{ExperimentStore, TrainOutput},
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    diagnosis::{CandidateSuiteOutcome, ComparativeDiagnosis},
    observation::{DevelopmentObservation, DevelopmentObservationSet},
    ports::{NativeRepairQualityStore, NativeRepairTrainingStore, RepairEvidenceStore},
    proposal::{
        CandidateMechanism, NativeRepairQualityPolicy, RepairAction, RepairActionKind,
        RepairBenchmarkBinding, RepairBudget, RepairCandidateHypothesis, RepairContext,
        RepairProposal, RepairProposalApplication, RepairProposalReview, RepairReviewDecision,
        RepairTarget,
    },
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview, NativeDeltaReviewDecision, NativeRepairAuditReference,
        NativeRepairRowEvidence,
    },
    training::NativeRepairTrainingSnapshot,
};
use serde_json::json;
use uuid::Uuid;
use workflow_core::{
    benchmark_bundle::BenchmarkBundle,
    benchmark_generation::{
        BenchmarkFreshnessAuthority, BenchmarkGeneration, BenchmarkGenerationEventKind,
        DevelopmentSuiteAuthority, first_generation_event, replay_benchmark_generation,
    },
    benchmark_qualification::ApprovedBenchmarkQualificationBinding,
    ports::BenchmarkGenerationStore,
};

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn time(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
}

fn project() -> ExternalProjectSnapshot {
    ExternalProjectSnapshot::create(
        "repair persistence",
        EncoderTaskKind::RetrievalRanking,
        "revision",
        digest('a'),
        BackendIdentity::new("nomos", "v1", digest('b')).unwrap(),
        vec![
            ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('1')).unwrap(),
            ExternalArtifactIdentity::new("generic", EvidenceRole::Development, 1, digest('2'))
                .unwrap(),
            ExternalArtifactIdentity::new("retired", EvidenceRole::Development, 1, digest('3'))
                .unwrap(),
            ExternalArtifactIdentity::new("sealed", EvidenceRole::SealedAcceptance, 1, digest('4'))
                .unwrap(),
        ],
        ModelArtifactIdentity::new("baseline", "fake", 1, digest('5')).unwrap(),
        json!({"task":"ranking"}),
        time(0),
    )
    .unwrap()
}

fn contract() -> MetricContract {
    MetricContract::create(
        vec![
            MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap(),
            MetricDefinition::new("recall_at_2", MetricDirection::HigherIsBetter).unwrap(),
        ],
        "mrr",
        vec![
            MetricGate::new(
                "mrr",
                EvidenceRole::Development,
                MetricGateCondition::MinimumImprovement { value: 0.0 },
            )
            .unwrap(),
            MetricGate::new(
                "recall_at_2",
                EvidenceRole::Development,
                MetricGateCondition::MaximumRegression { value: 0.0 },
            )
            .unwrap(),
            MetricGate::new(
                "mrr",
                EvidenceRole::SealedAcceptance,
                MetricGateCondition::MaximumRegression { value: 0.0 },
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn authority(
    suite_key: &str,
    suite_fingerprint: String,
    marker: char,
    sealed_suite_id: Uuid,
    sealed_suite_fingerprint: String,
) -> DevelopmentSuiteAuthority {
    let mut bundle = BenchmarkBundle {
        id: Uuid::new_v4(),
        development_suite_id: Uuid::new_v4(),
        development_suite_fingerprint: suite_fingerprint,
        sealed_suite_id: Some(sealed_suite_id),
        sealed_suite_fingerprint: Some(sealed_suite_fingerprint),
        contamination_report_id: Uuid::new_v4(),
        contamination_report_fingerprint: digest(marker),
        created_at: time(0),
        fingerprint: String::new(),
    };
    bundle.fingerprint = bundle.reproduce_fingerprint().unwrap();
    DevelopmentSuiteAuthority {
        suite_key: suite_key.into(),
        bundle: bundle.binding().unwrap(),
        qualification: ApprovedBenchmarkQualificationBinding {
            qualification_id: Uuid::new_v4(),
            qualification_fingerprint: digest('8'),
            review_id: Uuid::new_v4(),
            review_fingerprint: digest('9'),
            benchmark_bundle_id: bundle.id,
            benchmark_bundle_fingerprint: bundle.fingerprint,
        },
        external_evidence: None,
    }
}

fn generation() -> BenchmarkGeneration {
    let sealed_suite_id = Uuid::new_v4();
    let sealed_suite_fingerprint = digest('e');
    let freshness = BenchmarkFreshnessAuthority::create(
        sealed_suite_id,
        sealed_suite_fingerprint.clone(),
        digest('f'),
        digest('0'),
        time(0),
        time(1),
        time(20),
        "reviewer",
        "sealed population independently frozen",
    )
    .unwrap();
    BenchmarkGeneration::create(
        None,
        vec![
            authority(
                "generic",
                digest('6'),
                'c',
                sealed_suite_id,
                sealed_suite_fingerprint.clone(),
            ),
            authority(
                "retired",
                digest('7'),
                'd',
                sealed_suite_id,
                sealed_suite_fingerprint,
            ),
        ],
        freshness,
        time(1),
    )
    .unwrap()
}

fn report(
    project: &ExternalProjectSnapshot,
    contract: &MetricContract,
    model: ModelArtifactIdentity,
    role: EvidenceRole,
    suite_key: &str,
    suite_fingerprint: String,
    metrics: (f64, f64),
) -> EvaluationReport {
    EvaluationReport::create(
        project,
        model,
        role,
        suite_key,
        suite_fingerprint,
        contract,
        BTreeMap::from([("mrr".into(), metrics.0), ("recall_at_2".into(), metrics.1)]),
        3,
        time(2),
    )
    .unwrap()
}

fn protocol(
    project: &ExternalProjectSnapshot,
    generation: &BenchmarkGeneration,
) -> ExperimentProtocol {
    let contract = contract();
    let development_reports = generation
        .development_suites
        .iter()
        .map(|authority| {
            report(
                project,
                &contract,
                project.baseline_model.clone(),
                EvidenceRole::Development,
                &authority.suite_key,
                authority.bundle.development_suite_fingerprint.clone(),
                (0.8, 0.8),
            )
        })
        .collect();
    let sealed_report = report(
        project,
        &contract,
        project.baseline_model.clone(),
        EvidenceRole::SealedAcceptance,
        "sealed",
        generation
            .freshness
            .as_ref()
            .unwrap()
            .sealed_suite_fingerprint
            .clone(),
        (0.8, 0.8),
    );
    let candidate = TrainingCandidate::create(
        project,
        1,
        30,
        BTreeMap::from([("weight".into(), ParameterValue::Number(0.05))]),
    )
    .unwrap();
    ExperimentProtocol::create_multi(
        project,
        contract,
        development_reports,
        sealed_report,
        OptimizationBudget {
            maximum_candidates: 1,
            maximum_training_seconds: 30,
            maximum_development_evaluations: 2,
            maximum_sealed_evaluations: 1,
        },
        30,
        "sealed",
        vec![candidate],
        DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
        time(2),
    )
    .unwrap()
}

struct Fixture {
    store: SqliteExperimentStore,
    project: ExternalProjectSnapshot,
    campaign_id: Uuid,
    run_id: Uuid,
    protocol: ExperimentProtocol,
    generation_id: Uuid,
}

async fn fixture() -> Fixture {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let project = project();
    store.create_project(project.clone()).await.unwrap();
    let generation = generation();
    let first_generation = first_generation_event(&generation, time(1)).unwrap();
    store
        .create_benchmark_generation(&generation, &first_generation)
        .await
        .unwrap();
    let mut generation_events = vec![first_generation];
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let ready = generation_view
        .next_event(
            &generation,
            BenchmarkGenerationEventKind::MarkedReady {
                confirmed_by: "operator".into(),
            },
            time(2),
        )
        .unwrap();
    store
        .append_benchmark_generation_event(&ready)
        .await
        .unwrap();
    generation_events.push(ready);
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let activated = generation_view
        .next_event(
            &generation,
            BenchmarkGenerationEventKind::Activated {
                activated_by: "operator".into(),
            },
            time(3),
        )
        .unwrap();
    store
        .append_benchmark_generation_event(&activated)
        .await
        .unwrap();
    generation_events.push(activated);
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let binding =
        CampaignBenchmarkBinding::from_active_generation(&generation, &generation_view, "sealed")
            .unwrap();

    let campaign = ProductionCampaign::create(
        "repair campaign",
        project.id,
        project.fingerprint.clone(),
        CampaignBudget {
            maximum_iterations: 1,
            maximum_candidates: 1,
            maximum_training_seconds: 30,
            maximum_development_evaluations: 2,
            maximum_sealed_evaluations: 1,
            maximum_backend_operations: 10,
        },
        time(3),
    )
    .unwrap();
    let first_campaign = first_campaign_event(&campaign, time(3)).unwrap();
    store
        .create_campaign(&campaign, &first_campaign)
        .await
        .unwrap();
    let mut campaign_events = vec![first_campaign];
    let campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let bound = bind_generation_event(&campaign, &campaign_view, binding, time(4)).unwrap();
    store.append_campaign_event(&bound).await.unwrap();
    campaign_events.push(bound);

    let protocol = protocol(&project, &generation);
    store.create_protocol(protocol.clone()).await.unwrap();
    let campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let prepared = prepare_iteration_event(&campaign, &campaign_view, &protocol, time(5)).unwrap();
    store.append_campaign_event(&prepared).await.unwrap();
    campaign_events.push(prepared);

    let run_id = Uuid::new_v4();
    let first_experiment = first_event(&protocol, run_id, time(6)).unwrap();
    store.create_run(first_experiment.clone()).await.unwrap();
    let experiment_view = replay_experiment(&project, &protocol, &[first_experiment]).unwrap();
    let campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let started = start_run_event(&campaign, &campaign_view, &experiment_view, time(6)).unwrap();
    store.append_campaign_event(&started).await.unwrap();

    Fixture {
        store,
        project,
        campaign_id: campaign.id,
        run_id,
        protocol,
        generation_id: generation.id,
    }
}

fn observations(ranks: [Option<u32>; 3]) -> Vec<DevelopmentObservation> {
    ranks
        .into_iter()
        .enumerate()
        .map(|(index, rank)| {
            DevelopmentObservation::create(
                format!("row-{index}"),
                digest(char::from(b'a' + u8::try_from(index).unwrap())),
                BTreeMap::from([
                    ("target_capability".into(), "search".into()),
                    (
                        "workflow".into(),
                        if index == 2 { "audit" } else { "lookup" }.into(),
                    ),
                ]),
                false,
                rank,
                rank.map_or(0.0, |value| 1.0 / f64::from(value)),
                Some(0.1),
                Some(digest('f')),
            )
            .unwrap()
        })
        .collect()
}

fn native_rows(
    count: u64,
    project_revision: &str,
    dirty: bool,
    duplicate_normalized_identity: bool,
) -> Vec<NativeRepairRowEvidence> {
    (0..count)
        .map(|index| {
            let row_fingerprint = |offset: u64| format!("sha256:{:064x}", index + offset);
            NativeRepairRowEvidence::create(
                row_fingerprint(1),
                row_fingerprint(101),
                if duplicate_normalized_identity && index == 1 {
                    format!("sha256:{:064x}", 201)
                } else {
                    row_fingerprint(201)
                },
                row_fingerprint(301),
                row_fingerprint(401),
                row_fingerprint(501),
                "supported_failure",
                digest('a'),
                digest('b'),
                index + 1,
                project_revision,
                !dirty || index != 5,
                if dirty && index == 5 {
                    vec!["native task invariant failed".into()]
                } else {
                    vec![]
                },
                u32::from(dirty && index == 0),
                u32::from(dirty && index == 1),
                u32::from(dirty && index == 2),
                u32::from(dirty && index == 3),
                u32::from(dirty && index == 4),
            )
            .unwrap()
        })
        .collect()
}

fn native_audit_references() -> Vec<NativeRepairAuditReference> {
    vec![
        NativeRepairAuditReference::create(
            EvidenceRole::Training,
            "train",
            1,
            digest('1'),
            1,
            digest('5'),
        )
        .unwrap(),
        NativeRepairAuditReference::create(
            EvidenceRole::Development,
            "generic",
            1,
            digest('2'),
            1,
            digest('6'),
        )
        .unwrap(),
        NativeRepairAuditReference::create(
            EvidenceRole::Development,
            "retired",
            1,
            digest('3'),
            1,
            digest('7'),
        )
        .unwrap(),
        NativeRepairAuditReference::create(
            EvidenceRole::SealedAcceptance,
            "sealed",
            1,
            digest('4'),
            1,
            digest('8'),
        )
        .unwrap(),
    ]
}

#[tokio::test]
async fn repair_evidence_round_trips_idempotently_and_deep_verification_detects_tampering() {
    let fixture = fixture().await;
    let observer = BackendIdentity::new("fake-observer", "v1", digest('9')).unwrap();
    let candidate_id = fixture.protocol.candidates[0].id;
    let candidate_model = ModelArtifactIdentity::new("candidate", "fake", 1, digest('8')).unwrap();
    let events = fixture.store.load_events(fixture.run_id).await.unwrap();
    let view = replay_experiment(&fixture.project, &fixture.protocol, &events).unwrap();
    let training_started = view
        .next_event(
            &fixture.protocol,
            ExperimentEventKind::CandidateTrainingStarted { candidate_id },
            time(7),
        )
        .unwrap();
    fixture.store.append_event(training_started).await.unwrap();
    let events = fixture.store.load_events(fixture.run_id).await.unwrap();
    let view = replay_experiment(&fixture.project, &fixture.protocol, &events).unwrap();
    let training_completed = view
        .next_event(
            &fixture.protocol,
            ExperimentEventKind::CandidateTrainingCompleted {
                candidate_id,
                output: TrainOutput {
                    model: candidate_model.clone(),
                    duration_seconds: 1,
                    metadata: json!({"test":true}),
                },
            },
            time(7),
        )
        .unwrap();
    fixture
        .store
        .append_event(training_completed)
        .await
        .unwrap();
    let mut baseline_sets = Vec::new();
    let mut candidate_sets = Vec::new();
    let mut outcomes = Vec::new();

    for (index, baseline) in fixture
        .protocol
        .baseline_development_reports()
        .into_iter()
        .enumerate()
    {
        let candidate = report(
            &fixture.project,
            &fixture.protocol.metric_contract,
            candidate_model.clone(),
            EvidenceRole::Development,
            &baseline.suite_key,
            baseline.suite_fingerprint.clone(),
            if index == 0 {
                (0.81, 0.79)
            } else {
                (0.79, 0.81)
            },
        );
        let assessment = assess_candidate(
            &fixture.project,
            &fixture.protocol.metric_contract,
            baseline,
            &candidate,
            time(7),
        )
        .unwrap();
        let events = fixture.store.load_events(fixture.run_id).await.unwrap();
        let view = replay_experiment(&fixture.project, &fixture.protocol, &events).unwrap();
        let development_completed = view
            .next_event(
                &fixture.protocol,
                ExperimentEventKind::CandidateDevelopmentSuiteCompleted {
                    candidate_id,
                    suite_key: candidate.suite_key.clone(),
                    report: candidate.clone(),
                    assessment: assessment.clone(),
                },
                time(7),
            )
            .unwrap();
        fixture
            .store
            .append_event(development_completed)
            .await
            .unwrap();
        let baseline_set = DevelopmentObservationSet::create(
            &fixture.project,
            fixture.campaign_id,
            fixture.run_id,
            None,
            baseline,
            &fixture.protocol.metric_contract,
            observer.clone(),
            ExternalArtifactIdentity::new(
                format!("baseline-{}", baseline.suite_key),
                EvidenceRole::Development,
                1,
                digest(if index == 0 { '1' } else { '2' }),
            )
            .unwrap(),
            observations([Some(1), Some(2), Some(1)]),
            time(8),
        )
        .unwrap();
        let candidate_set = DevelopmentObservationSet::create(
            &fixture.project,
            fixture.campaign_id,
            fixture.run_id,
            Some(candidate_id),
            &candidate,
            &fixture.protocol.metric_contract,
            observer.clone(),
            ExternalArtifactIdentity::new(
                format!("candidate-{}", baseline.suite_key),
                EvidenceRole::Development,
                1,
                digest(if index == 0 { '3' } else { '4' }),
            )
            .unwrap(),
            observations(if index == 0 {
                [Some(1), Some(1), Some(2)]
            } else {
                [Some(1), Some(3), Some(1)]
            }),
            time(8),
        )
        .unwrap();
        baseline_sets.push(
            fixture
                .store
                .create_observation_set(baseline_set)
                .await
                .unwrap(),
        );
        candidate_sets.push(
            fixture
                .store
                .create_observation_set(candidate_set)
                .await
                .unwrap(),
        );
        outcomes.push(
            CandidateSuiteOutcome::create(
                &fixture.project,
                &fixture.protocol.metric_contract,
                candidate_id,
                baseline,
                &candidate,
                &assessment,
            )
            .unwrap(),
        );
    }

    let duplicate = DevelopmentObservationSet::create(
        &fixture.project,
        fixture.campaign_id,
        fixture.run_id,
        None,
        fixture.protocol.baseline_development_reports()[0],
        &fixture.protocol.metric_contract,
        observer,
        ExternalArtifactIdentity::new(
            "baseline-generic",
            EvidenceRole::Development,
            1,
            digest('1'),
        )
        .unwrap(),
        observations([Some(1), Some(2), Some(1)]),
        time(9),
    )
    .unwrap();
    let existing = fixture
        .store
        .create_observation_set(duplicate)
        .await
        .unwrap();
    assert_eq!(existing.id, baseline_sets[0].id);

    let diagnosis = ComparativeDiagnosis::create(
        &fixture.project,
        fixture.campaign_id,
        fixture.run_id,
        1,
        vec!["workflow".into(), "target_capability".into()],
        &baseline_sets,
        &candidate_sets,
        &outcomes,
        time(10),
    )
    .unwrap();
    let persisted = fixture.store.create_diagnosis(diagnosis).await.unwrap();
    let duplicate = ComparativeDiagnosis::create(
        &fixture.project,
        fixture.campaign_id,
        fixture.run_id,
        1,
        vec!["target_capability".into(), "workflow".into()],
        &baseline_sets,
        &candidate_sets,
        &outcomes,
        time(11),
    )
    .unwrap();
    let idempotent = fixture.store.create_diagnosis(duplicate).await.unwrap();
    assert_eq!(idempotent.id, persisted.id);
    assert_eq!(
        fixture
            .store
            .get_diagnosis(persisted.id)
            .await
            .unwrap()
            .unwrap(),
        persisted
    );

    let generation = fixture
        .store
        .get_benchmark_generation(fixture.generation_id)
        .await
        .unwrap()
        .unwrap();
    let generation_events = fixture
        .store
        .list_benchmark_generation_events(generation.id)
        .await
        .unwrap();
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let benchmark = RepairBenchmarkBinding::create(
        generation.id,
        generation.fingerprint.clone(),
        generation_view.last_sequence,
        generation_view.last_event_fingerprint.clone(),
        generation
            .development_suites
            .iter()
            .map(|value| {
                (
                    value.suite_key.clone(),
                    value.bundle.development_suite_fingerprint.clone(),
                )
            })
            .collect(),
        generation.sealed_suite_id,
        generation.sealed_suite_fingerprint.clone(),
        generation.freshness.as_ref().unwrap().valid_until,
    )
    .unwrap();
    let context =
        RepairContext::create(&persisted, &fixture.project, &fixture.project, benchmark).unwrap();
    let weakness = persisted.weaknesses.first().unwrap();
    let proposal = RepairProposal::create(
        &persisted,
        context,
        vec![RepairTarget {
            key: "supported_failure".into(),
            slice_fingerprint: weakness.slice_fingerprint.clone(),
            slice: weakness.slice.clone(),
            weakness_kind: weakness.kind,
            suite_key: weakness.suite_key.clone(),
            rationale: "test the supported diagnosis with bounded rows".into(),
            absolute_row_target: 12,
        }],
        vec![
            RepairAction {
                key: "generate".into(),
                kind: RepairActionKind::GenerateNativeRows,
                target_keys: vec!["supported_failure".into()],
                parameters: BTreeMap::from([(
                    "recipe".into(),
                    ParameterValue::Text("deterministic-v1".into()),
                )]),
            },
            RepairAction {
                key: "train".into(),
                kind: RepairActionKind::ChangeTrainingConfiguration,
                target_keys: vec!["supported_failure".into()],
                parameters: BTreeMap::from([("epochs".into(), ParameterValue::Integer(1))]),
            },
        ],
        NativeRepairQualityPolicy::create("native-v1", true, 0, 0, 0, 0, 0, 0, true, true).unwrap(),
        RepairBudget {
            maximum_total_rows: 12,
            maximum_rows_per_target: 12,
            maximum_candidates: 1,
            maximum_training_seconds: 30,
            maximum_evaluation_seconds: 30,
            maximum_development_evaluations: 2,
            maximum_external_calls: 0,
            maximum_sealed_uses: 1,
        },
        vec![RepairCandidateHypothesis {
            key: "targeted".into(),
            mechanism: CandidateMechanism::GenuineRetraining,
            hypothesis: "targeted contrasts improve the supported failure".into(),
            action_keys: vec!["generate".into(), "train".into()],
            maximum_training_seconds: 30,
            parameters: BTreeMap::from([("seed".into(), ParameterValue::Integer(7))]),
        }],
        time(19),
        time(11),
    )
    .unwrap();
    let proposal = fixture.store.create_proposal(proposal).await.unwrap();
    let review = RepairProposalReview::create(
        &proposal,
        &persisted,
        None,
        RepairReviewDecision::Approve,
        "operator",
        "bounded repair approved",
        time(12),
    )
    .unwrap();
    let review = fixture.store.append_proposal_review(review).await.unwrap();
    let reservation =
        RepairProposalApplication::reserve(&proposal, &persisted, &review, None, time(13)).unwrap();
    let reservation = fixture
        .store
        .reserve_proposal_application(reservation)
        .await
        .unwrap();
    let duplicate_reservation =
        RepairProposalApplication::reserve(&proposal, &persisted, &review, None, time(13)).unwrap();
    let idempotent = fixture
        .store
        .reserve_proposal_application(duplicate_reservation)
        .await
        .unwrap();
    assert_eq!(reservation.id, idempotent.id);
    let late_proposal_review = RepairProposalReview::create(
        &proposal,
        &persisted,
        Some(&review),
        RepairReviewDecision::RequestRevision,
        "operator",
        "too late after application",
        time(14),
    )
    .unwrap();
    assert!(
        fixture
            .store
            .append_proposal_review(late_proposal_review)
            .await
            .is_err(),
        "application must freeze the exact proposal approval chain"
    );
    assert!(
        NativeDeltaCandidateSet::create(
            &proposal,
            &persisted,
            &review,
            None,
            &reservation,
            BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
            ExternalArtifactIdentity::new(
                "unscoped-delta",
                EvidenceRole::Training,
                12,
                digest('9')
            )
            .unwrap(),
            vec![],
            native_rows(12, &fixture.project.source_revision, false, false),
            time(14),
        )
        .is_err(),
        "a candidate set must retain its exact payload-free audit population"
    );
    let mut foreign_audit_scope = native_audit_references();
    foreign_audit_scope[0] = NativeRepairAuditReference::create(
        EvidenceRole::Training,
        "train",
        1,
        digest('9'),
        1,
        digest('5'),
    )
    .unwrap();
    assert!(
        NativeDeltaCandidateSet::create(
            &proposal,
            &persisted,
            &review,
            None,
            &reservation,
            BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
            ExternalArtifactIdentity::new(
                "foreign-scope-delta",
                EvidenceRole::Training,
                12,
                digest('9')
            )
            .unwrap(),
            foreign_audit_scope,
            native_rows(12, &fixture.project.source_revision, false, false),
            time(14),
        )
        .is_err(),
        "the audit scope must exactly match every proposal base-training input"
    );
    assert!(
        NativeDeltaCandidateSet::create(
            &proposal,
            &persisted,
            &review,
            None,
            &reservation,
            BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
            ExternalArtifactIdentity::new("expired-delta", EvidenceRole::Training, 12, digest('9'))
                .unwrap(),
            native_audit_references(),
            native_rows(12, &fixture.project.source_revision, false, false),
            time(20),
        )
        .is_err(),
        "a candidate set cannot be created after proposal expiry"
    );
    assert!(
        NativeDeltaCandidateSet::create(
            &proposal,
            &persisted,
            &review,
            None,
            &reservation,
            BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
            ExternalArtifactIdentity::new(
                "incomplete-delta",
                EvidenceRole::Training,
                11,
                digest('e')
            )
            .unwrap(),
            native_audit_references(),
            native_rows(11, &fixture.project.source_revision, false, false),
            time(14),
        )
        .is_err(),
        "a candidate set must assess every exact proposal target row"
    );
    assert!(
        NativeDeltaCandidateSet::create(
            &proposal,
            &persisted,
            &review,
            None,
            &reservation,
            BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
            ExternalArtifactIdentity::new(
                "duplicate-normalized-delta",
                EvidenceRole::Training,
                12,
                digest('f'),
            )
            .unwrap(),
            native_audit_references(),
            native_rows(12, &fixture.project.source_revision, false, true),
            time(14),
        )
        .is_err(),
        "normalized identities must be unique inside a candidate delta"
    );
    let dirty_candidate_set = NativeDeltaCandidateSet::create(
        &proposal,
        &persisted,
        &review,
        None,
        &reservation,
        BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
        ExternalArtifactIdentity::new("dirty-delta", EvidenceRole::Training, 12, digest('0'))
            .unwrap(),
        native_audit_references(),
        native_rows(12, &fixture.project.source_revision, true, false),
        time(14),
    )
    .unwrap();
    let dirty_report =
        NativeDeltaQualityReport::create(&proposal, &dirty_candidate_set, time(15)).unwrap();
    assert!(!dirty_report.eligible);
    assert_eq!(dirty_report.counts.invalid_rows, 1);
    assert_eq!(dirty_report.counts.exact_duplicate_rows, 1);
    assert_eq!(dirty_report.counts.normalized_duplicate_rows, 1);
    assert_eq!(dirty_report.counts.source_contamination_rows, 1);
    assert_eq!(dirty_report.counts.group_contamination_rows, 1);
    assert_eq!(dirty_report.counts.lineage_contamination_rows, 1);
    assert!(
        NativeDeltaReview::create(
            &proposal,
            &dirty_candidate_set,
            &dirty_report,
            None,
            NativeDeltaReviewDecision::Approve,
            "operator",
            "unsafe approval",
            time(16),
        )
        .is_err(),
        "an ineligible native delta must not be approvable"
    );
    let rows = native_rows(12, &fixture.project.source_revision, false, false);
    let candidate_set = NativeDeltaCandidateSet::create(
        &proposal,
        &persisted,
        &review,
        None,
        &reservation,
        BackendIdentity::new("fake-native-delta", "v1", digest('c')).unwrap(),
        ExternalArtifactIdentity::new("repair-delta", EvidenceRole::Training, 12, digest('d'))
            .unwrap(),
        native_audit_references(),
        rows,
        time(14),
    )
    .unwrap();
    let candidate_set = fixture
        .store
        .create_native_delta_candidate_set(candidate_set)
        .await
        .unwrap();
    let report = NativeDeltaQualityReport::create(&proposal, &candidate_set, time(15)).unwrap();
    let report = fixture
        .store
        .create_native_delta_report(report)
        .await
        .unwrap();
    assert!(report.eligible);
    let delta_review = NativeDeltaReview::create(
        &proposal,
        &candidate_set,
        &report,
        None,
        NativeDeltaReviewDecision::Approve,
        "operator",
        "all native quality and contamination checks are clean",
        time(16),
    )
    .unwrap();
    let delta_review = fixture
        .store
        .append_native_delta_review(delta_review)
        .await
        .unwrap();
    let selection = ApprovedNativeDeltaSelection::create(
        &proposal,
        &candidate_set,
        &report,
        &delta_review,
        None,
        time(17),
    )
    .unwrap();
    let selection = fixture
        .store
        .create_native_delta_selection(selection)
        .await
        .unwrap();
    let duplicate_selection = ApprovedNativeDeltaSelection::create(
        &proposal,
        &candidate_set,
        &report,
        &delta_review,
        None,
        time(17),
    )
    .unwrap();
    let idempotent_selection = fixture
        .store
        .create_native_delta_selection(duplicate_selection)
        .await
        .unwrap();
    assert_eq!(selection.id, idempotent_selection.id);
    assert_eq!(selection.entries.len(), 12);
    let training_snapshot = NativeRepairTrainingSnapshot::create(
        &fixture.project,
        &proposal,
        &candidate_set,
        &report,
        &delta_review,
        None,
        &selection,
        time(18),
    )
    .unwrap();
    let training_snapshot = fixture
        .store
        .create_native_repair_training_snapshot(training_snapshot)
        .await
        .unwrap();
    let duplicate_training_snapshot = NativeRepairTrainingSnapshot::create(
        &fixture.project,
        &proposal,
        &candidate_set,
        &report,
        &delta_review,
        None,
        &selection,
        time(18),
    )
    .unwrap();
    let idempotent_training_snapshot = fixture
        .store
        .create_native_repair_training_snapshot(duplicate_training_snapshot)
        .await
        .unwrap();
    assert_eq!(training_snapshot.id, idempotent_training_snapshot.id);
    assert_eq!(training_snapshot.base_rows, 1);
    assert_eq!(training_snapshot.delta_rows, 12);
    assert_eq!(training_snapshot.total_rows, 13);
    assert_eq!(
        fixture
            .store
            .get_native_repair_training_snapshot(training_snapshot.id)
            .await
            .unwrap()
            .unwrap(),
        training_snapshot
    );
    assert!(
        sqlx::query(
            "UPDATE encoder_native_repair_training_snapshots SET fingerprint = ? WHERE id = ?",
        )
        .bind(digest('0'))
        .bind(training_snapshot.id)
        .execute(fixture.store.pool())
        .await
        .is_err(),
        "combined training snapshots must be immutable"
    );
    let late_review = NativeDeltaReview::create(
        &proposal,
        &candidate_set,
        &report,
        Some(&delta_review),
        NativeDeltaReviewDecision::Reject,
        "operator",
        "too late to revise the frozen selection",
        time(18),
    )
    .unwrap();
    assert!(
        fixture
            .store
            .append_native_delta_review(late_review)
            .await
            .is_err(),
        "an approved immutable selection must freeze its review chain"
    );
    assert!(
        sqlx::query("UPDATE encoder_native_delta_selections SET fingerprint = ? WHERE id = ?")
            .bind(digest('0'))
            .bind(selection.id)
            .execute(fixture.store.pool())
            .await
            .is_err()
    );
    sqlx::query("DROP TRIGGER encoder_native_delta_selections_no_update")
        .execute(fixture.store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE encoder_native_delta_selections SET fingerprint = ? WHERE id = ?")
        .bind(digest('0'))
        .bind(selection.id)
        .execute(fixture.store.pool())
        .await
        .unwrap();
    assert!(
        fixture
            .store
            .get_native_delta_selection(selection.id)
            .await
            .is_err(),
        "deep reads must detect normalized storage-envelope tampering"
    );
    assert_eq!(
        fixture
            .store
            .get_proposal(proposal.id)
            .await
            .unwrap()
            .unwrap(),
        proposal
    );

    assert!(
        sqlx::query(
            "UPDATE encoder_repair_diagnosis_observation_sets \
             SET observation_set_fingerprint = ? WHERE diagnosis_id = ? AND ordinal = 0",
        )
        .bind(digest('0'))
        .bind(persisted.id)
        .execute(fixture.store.pool())
        .await
        .is_err()
    );
    sqlx::query("DROP TRIGGER encoder_repair_diagnosis_observation_sets_no_update")
        .execute(fixture.store.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE encoder_repair_diagnosis_observation_sets \
         SET observation_set_fingerprint = ? WHERE diagnosis_id = ? AND ordinal = 0",
    )
    .bind(digest('0'))
    .bind(persisted.id)
    .execute(fixture.store.pool())
    .await
    .unwrap();
    assert!(fixture.store.get_diagnosis(persisted.id).await.is_err());
}
