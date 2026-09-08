use std::collections::BTreeMap;

use chrono::Utc;
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
        NativeDeltaReview, NativeDeltaReviewDecision,
    },
    training::NativeRepairTrainingSnapshot,
};
use serde_json::json;
use workflow_core::{
    benchmark_generation::replay_benchmark_generation, ports::BenchmarkGenerationStore,
};

#[allow(dead_code)]
#[path = "../../../../crates/encoder-experiment-sqlite/tests/support/repair_fixture.rs"]
mod repair_fixture;
use repair_fixture::*;

pub(crate) struct Seed {
    pub(crate) fixture: Fixture,
    pub(crate) training_snapshot: NativeRepairTrainingSnapshot,
}

pub(crate) async fn seed(database_url: &str) -> Seed {
    let valid_until = Utc::now() + chrono::Duration::days(1);
    let fixture = fixture_in(database_url, valid_until).await;
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
        valid_until,
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
    Seed {
        fixture,
        training_snapshot,
    }
}
