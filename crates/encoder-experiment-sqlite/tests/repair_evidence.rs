use std::collections::BTreeMap;

use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget,
    optimization::{
        OptimizationArtifactBinding, OptimizationEventKind, OptimizationLaunchStore,
        ProductionOptimizationDefinition, ProductionOptimizationRun, first_optimization_event,
        replay_optimization,
    },
};
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EvidenceRole, ExternalArtifactIdentity, ModelArtifactIdentity,
        ParameterValue,
    },
    journal::{ExperimentEventKind, replay_experiment},
    metrics::assess_candidate,
    ports::{ExperimentStore, TrainOutput},
};
use encoder_repair_core::{
    diagnosis::{CandidateSuiteOutcome, ComparativeDiagnosis},
    observation::DevelopmentObservationSet,
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
    },
    training::{NativeRepairTrainingSnapshot, REPAIR_DELTA_FINGERPRINT_PARAMETER},
};
use serde_json::json;
use workflow_core::{
    benchmark_generation::{BenchmarkGenerationEventKind, replay_benchmark_generation},
    ports::BenchmarkGenerationStore,
};

#[path = "support/repair_fixture.rs"]
mod fixture_support;
use fixture_support::*;

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
                parameters: BTreeMap::from([("seed".into(), ParameterValue::Integer(7))]),
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
    assert_eq!(
        fixture
            .store
            .native_delta_selection_ids_for_project(fixture.project.id)
            .await
            .unwrap(),
        vec![selection.id]
    );
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
    let compiled_candidates = training_snapshot
        .compile_training_candidates(&fixture.project, &proposal)
        .unwrap();
    assert_eq!(compiled_candidates.len(), 1);
    assert_eq!(
        compiled_candidates[0]
            .parameters
            .get(REPAIR_DELTA_FINGERPRINT_PARAMETER),
        Some(&ParameterValue::Text(digest('d')))
    );
    assert_eq!(
        fixture
            .store
            .get_native_repair_training_snapshot(training_snapshot.id)
            .await
            .unwrap()
            .unwrap(),
        training_snapshot
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
    let benchmark =
        CampaignBenchmarkBinding::from_active_generation(&generation, &generation_view, "sealed")
            .unwrap();
    let definition = ProductionOptimizationDefinition::create(
        "bounded repair",
        digest('e'),
        &fixture.project,
        OptimizationArtifactBinding::new(proposal.id, proposal.fingerprint.clone()).unwrap(),
        OptimizationArtifactBinding::new(selection.id, selection.fingerprint.clone()).unwrap(),
        OptimizationArtifactBinding::new(
            training_snapshot.id,
            training_snapshot.fingerprint.clone(),
        )
        .unwrap(),
        training_snapshot.specification_fingerprint.clone(),
        benchmark,
        &fixture.protocol,
        compiled_candidates,
        CampaignBudget {
            maximum_iterations: 1,
            maximum_candidates: 1,
            maximum_training_seconds: 30,
            maximum_development_evaluations: 2,
            maximum_sealed_evaluations: 1,
            maximum_backend_operations: 4,
        },
        30,
        time(18),
    )
    .unwrap();
    let optimization_run = ProductionOptimizationRun::create(&definition, time(18)).unwrap();
    let optimization_first = first_optimization_event(&optimization_run, time(18)).unwrap();
    fixture
        .store
        .create_optimization(
            definition.clone(),
            optimization_run.clone(),
            optimization_first,
        )
        .await
        .unwrap();
    let (reloaded_definition, reloaded_run) = fixture
        .store
        .find_optimization_by_manifest(digest('e'))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded_definition, definition);
    assert_eq!(reloaded_run, optimization_run);
    let events = fixture
        .store
        .list_optimization_events(optimization_run.id)
        .await
        .unwrap();
    let view = replay_optimization(&optimization_run, &events).unwrap();
    let attached = view
        .next_event(
            &optimization_run,
            OptimizationEventKind::CampaignAttached {
                campaign_fingerprint: digest('f'),
            },
            time(19),
        )
        .unwrap();
    fixture
        .store
        .append_optimization_event(attached.clone())
        .await
        .unwrap();
    assert!(
        fixture
            .store
            .append_optimization_event(attached)
            .await
            .is_err(),
        "the same journal reservation cannot be spent twice"
    );
    let events = fixture
        .store
        .list_optimization_events(optimization_run.id)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    let generation_events = fixture
        .store
        .list_benchmark_generation_events(generation.id)
        .await
        .unwrap();
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let exhausted = generation_view
        .next_event(
            &generation,
            BenchmarkGenerationEventKind::Exhausted {
                reason: "test the immutable optimization after terminal generation state".into(),
            },
            time(19),
        )
        .unwrap();
    fixture
        .store
        .append_benchmark_generation_event(&exhausted)
        .await
        .unwrap();
    assert!(
        fixture
            .store
            .get_optimization_run(optimization_run.id)
            .await
            .unwrap()
            .is_some(),
        "terminal generation state must not make historical optimization evidence unreadable"
    );
    assert!(
        sqlx::query(
            "UPDATE encoder_production_optimization_definitions SET fingerprint = ? WHERE id = ?",
        )
        .bind(digest('0'))
        .bind(definition.id)
        .execute(fixture.store.pool())
        .await
        .is_err(),
        "optimization definitions must be immutable"
    );
    assert!(
        sqlx::query(
            "UPDATE encoder_production_optimization_events SET fingerprint = ? \
             WHERE run_id = ? AND sequence = 1",
        )
        .bind(digest('0'))
        .bind(optimization_run.id)
        .execute(fixture.store.pool())
        .await
        .is_err(),
        "optimization journals must be append-only"
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
    sqlx::query("DROP TRIGGER encoder_production_optimization_definitions_no_update")
        .execute(fixture.store.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE encoder_production_optimization_definitions SET artifact_json = '{}' WHERE id = ?",
    )
    .bind(definition.id)
    .execute(fixture.store.pool())
    .await
    .unwrap();
    assert!(
        fixture
            .store
            .get_optimization_run(optimization_run.id)
            .await
            .is_err(),
        "optimization storage-envelope tampering must fail closed"
    );
}
