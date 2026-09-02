use chrono::{DateTime, TimeZone, Utc};
use std::collections::BTreeMap;

use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignStore, ProductionCampaign,
    SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION, SealedAssessmentExposure, bind_generation_event,
    finalize_iteration_event, first_campaign_event, prepare_iteration_event,
    record_development_event, record_sealed_authorization_event, replay_campaign, start_run_event,
};
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    journal::{ExperimentRunState, ExperimentView, FinalDecision},
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    ports::ExperimentStore,
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde_json::json;
use uuid::Uuid;
use workflow_core::{
    benchmark_bundle::BenchmarkBundle,
    benchmark_generation::{
        BenchmarkFreshnessAuthority, BenchmarkGeneration, BenchmarkGenerationEventKind,
        DevelopmentSuiteAuthority, first_generation_event, prepare_successor_activation,
        replay_benchmark_generation,
    },
    benchmark_qualification::ApprovedBenchmarkQualificationBinding,
    ports::BenchmarkGenerationStore,
};

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn time(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, hour, 0, 0).unwrap()
}

fn project() -> ExternalProjectSnapshot {
    ExternalProjectSnapshot::create(
        "renewable nomos",
        EncoderTaskKind::RetrievalRanking,
        "revision",
        digest('a'),
        BackendIdentity::new("nomos", "v1", digest('b')).unwrap(),
        vec![
            ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('1')).unwrap(),
            ExternalArtifactIdentity::new("development", EvidenceRole::Development, 1, digest('2'))
                .unwrap(),
            ExternalArtifactIdentity::new("sealed", EvidenceRole::SealedAcceptance, 1, digest('3'))
                .unwrap(),
        ],
        ModelArtifactIdentity::new("baseline", "fake", 1, digest('4')).unwrap(),
        json!({"task":"ranking"}),
        time(0),
    )
    .unwrap()
}

fn protocol(
    project: &ExternalProjectSnapshot,
    generation: &BenchmarkGeneration,
) -> ExperimentProtocol {
    let contract = MetricContract::create(
        vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
        "mrr",
        vec![
            MetricGate::new(
                "mrr",
                EvidenceRole::Development,
                MetricGateCondition::MinimumImprovement { value: 0.0 },
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
    .unwrap();
    let development_authority = &generation.development_suites[0];
    let baseline_report = |role, suite_key: &str, suite_fingerprint: String| {
        EvaluationReport::create(
            project,
            project.baseline_model.clone(),
            role,
            suite_key,
            suite_fingerprint,
            &contract,
            BTreeMap::from([("mrr".into(), 0.8)]),
            100,
            time(3),
        )
        .unwrap()
    };
    let candidate = TrainingCandidate::create(
        project,
        1,
        60,
        BTreeMap::from([("weight".into(), ParameterValue::Number(0.05))]),
    )
    .unwrap();
    let development_reports = vec![baseline_report(
        EvidenceRole::Development,
        &development_authority.suite_key,
        development_authority
            .bundle
            .development_suite_fingerprint
            .clone(),
    )];
    let sealed_report = baseline_report(
        EvidenceRole::SealedAcceptance,
        "sealed",
        generation
            .freshness
            .as_ref()
            .unwrap()
            .sealed_suite_fingerprint
            .clone(),
    );
    ExperimentProtocol::create_multi(
        project,
        contract,
        development_reports,
        sealed_report,
        OptimizationBudget {
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
        },
        60,
        "sealed",
        vec![candidate],
        DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
        time(3),
    )
    .unwrap()
}

fn experiment_view(
    protocol: &ExperimentProtocol,
    run_id: Uuid,
    state: ExperimentRunState,
    selected_candidate_id: Option<Uuid>,
    final_decision: Option<FinalDecision>,
    sequence: u32,
) -> ExperimentView {
    ExperimentView {
        run_id,
        protocol_id: protocol.id,
        state,
        candidates: BTreeMap::new(),
        selected_candidate_id,
        sealed_authorized_by: None,
        sealed_report: None,
        sealed_assessment: None,
        final_decision,
        failure_reason: None,
        last_sequence: sequence,
        last_event_fingerprint: digest('f'),
        updated_at: time(sequence),
    }
}

fn generation(
    predecessor: Option<&BenchmarkGeneration>,
    suite_character: char,
) -> BenchmarkGeneration {
    let sealed_suite_id = Uuid::new_v4();
    let mut bundle = BenchmarkBundle {
        id: Uuid::new_v4(),
        development_suite_id: Uuid::new_v4(),
        development_suite_fingerprint: digest(suite_character),
        sealed_suite_id: Some(sealed_suite_id),
        sealed_suite_fingerprint: Some(digest('5')),
        contamination_report_id: Uuid::new_v4(),
        contamination_report_fingerprint: digest('6'),
        created_at: time(0),
        fingerprint: String::new(),
    };
    bundle.fingerprint = bundle.reproduce_fingerprint().unwrap();
    let authority = DevelopmentSuiteAuthority {
        suite_key: "development".into(),
        bundle: bundle.binding().unwrap(),
        qualification: ApprovedBenchmarkQualificationBinding {
            qualification_id: Uuid::new_v4(),
            qualification_fingerprint: digest('7'),
            review_id: Uuid::new_v4(),
            review_fingerprint: digest('8'),
            benchmark_bundle_id: bundle.id,
            benchmark_bundle_fingerprint: bundle.fingerprint,
        },
        external_evidence: None,
    };
    let freshness = BenchmarkFreshnessAuthority::create(
        sealed_suite_id,
        digest('5'),
        digest('9'),
        digest('a'),
        time(0),
        time(1),
        time(10),
        "reviewer",
        "successor population was independently frozen",
    )
    .unwrap();
    BenchmarkGeneration::create(predecessor, vec![authority], freshness, time(1)).unwrap()
}

async fn create_active_generation(
    store: &SqliteExperimentStore,
    generation: &BenchmarkGeneration,
) -> Vec<workflow_core::benchmark_generation::BenchmarkGenerationEvent> {
    let first = first_generation_event(generation, time(1)).unwrap();
    store
        .create_benchmark_generation(generation, &first)
        .await
        .unwrap();
    let mut events = vec![first];
    let mut view = replay_benchmark_generation(generation, &events).unwrap();
    let ready = view
        .next_event(
            generation,
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
    events.push(ready);
    view = replay_benchmark_generation(generation, &events).unwrap();
    let active = view
        .next_event(
            generation,
            BenchmarkGenerationEventKind::Activated {
                activated_by: "operator".into(),
            },
            time(3),
        )
        .unwrap();
    store
        .append_benchmark_generation_event(&active)
        .await
        .unwrap();
    events.push(active);
    events
}

#[tokio::test]
async fn generation_and_campaign_journals_round_trip_and_conflicts_fail() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let project = project();
    store.create_project(project.clone()).await.unwrap();
    let generation = generation(None, 'c');
    let events = create_active_generation(&store, &generation).await;
    let generation_view = replay_benchmark_generation(&generation, &events).unwrap();
    let binding =
        CampaignBenchmarkBinding::from_active_generation(&generation, &generation_view, "sealed")
            .unwrap();
    let campaign = ProductionCampaign::create(
        "renewable campaign",
        project.id,
        project.fingerprint,
        CampaignBudget {
            maximum_iterations: 1,
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
            maximum_backend_operations: 10,
        },
        time(3),
    )
    .unwrap();
    let first = first_campaign_event(&campaign, time(3)).unwrap();
    store.create_campaign(&campaign, &first).await.unwrap();
    let campaign_events = store.list_campaign_events(campaign.id).await.unwrap();
    let view = replay_campaign(&campaign, &campaign_events).unwrap();
    let bound = bind_generation_event(&campaign, &view, binding.clone(), time(4)).unwrap();
    let conflicting = bind_generation_event(&campaign, &view, binding, time(4)).unwrap();
    store.append_campaign_event(&bound).await.unwrap();
    assert!(store.append_campaign_event(&conflicting).await.is_err());
    assert_eq!(
        store
            .get_benchmark_generation(generation.id)
            .await
            .unwrap()
            .unwrap(),
        generation
    );
    assert_eq!(
        store.get_campaign(campaign.id).await.unwrap().unwrap(),
        campaign
    );
}

#[tokio::test]
async fn successor_activation_updates_both_journals_atomically() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let predecessor = generation(None, 'c');
    let mut predecessor_events = create_active_generation(&store, &predecessor).await;
    let predecessor_view = replay_benchmark_generation(&predecessor, &predecessor_events).unwrap();
    let exhausted = predecessor_view
        .next_event(
            &predecessor,
            BenchmarkGenerationEventKind::Exhausted {
                reason: "iteration completed".into(),
            },
            time(4),
        )
        .unwrap();
    store
        .append_benchmark_generation_event(&exhausted)
        .await
        .unwrap();
    predecessor_events.push(exhausted);
    let predecessor_view = replay_benchmark_generation(&predecessor, &predecessor_events).unwrap();

    let successor = generation(Some(&predecessor), 'd');
    let first = first_generation_event(&successor, time(4)).unwrap();
    store
        .create_benchmark_generation(&successor, &first)
        .await
        .unwrap();
    let mut successor_events = vec![first];
    let successor_view = replay_benchmark_generation(&successor, &successor_events).unwrap();
    let ready = successor_view
        .next_event(
            &successor,
            BenchmarkGenerationEventKind::MarkedReady {
                confirmed_by: "operator".into(),
            },
            time(5),
        )
        .unwrap();
    store
        .append_benchmark_generation_event(&ready)
        .await
        .unwrap();
    successor_events.push(ready);
    let successor_view = replay_benchmark_generation(&successor, &successor_events).unwrap();
    let activation = prepare_successor_activation(
        &predecessor,
        &predecessor_view,
        &successor,
        &successor_view,
        "operator",
        time(6),
    )
    .unwrap();
    store
        .activate_successor_generation(
            &activation.predecessor_superseded,
            &activation.successor_activated,
        )
        .await
        .unwrap();
    let predecessor_reloaded = store
        .list_benchmark_generation_events(predecessor.id)
        .await
        .unwrap();
    let successor_reloaded = store
        .list_benchmark_generation_events(successor.id)
        .await
        .unwrap();
    assert_eq!(
        replay_benchmark_generation(&predecessor, &predecessor_reloaded)
            .unwrap()
            .state,
        workflow_core::benchmark_generation::BenchmarkGenerationState::Superseded
    );
    assert!(
        replay_benchmark_generation(&successor, &successor_reloaded)
            .unwrap()
            .is_adaptive_eligible()
    );
}

#[tokio::test]
async fn sealed_iteration_finalization_rolls_back_both_journals_on_conflict() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let project = project();
    store.create_project(project.clone()).await.unwrap();
    let generation = generation(None, 'c');
    let generation_events = create_active_generation(&store, &generation).await;
    let generation_view = replay_benchmark_generation(&generation, &generation_events).unwrap();
    let binding =
        CampaignBenchmarkBinding::from_active_generation(&generation, &generation_view, "sealed")
            .unwrap();
    let protocol = protocol(&project, &generation);
    let campaign = ProductionCampaign::create(
        "atomic finalization",
        project.id,
        project.fingerprint.clone(),
        CampaignBudget {
            maximum_iterations: 1,
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
            maximum_backend_operations: 10,
        },
        time(3),
    )
    .unwrap();
    let first = first_campaign_event(&campaign, time(3)).unwrap();
    store.create_campaign(&campaign, &first).await.unwrap();
    let mut campaign_events = vec![first.clone()];
    let mut campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let bound = bind_generation_event(&campaign, &campaign_view, binding.clone(), time(4)).unwrap();
    store.append_campaign_event(&bound).await.unwrap();
    campaign_events.push(bound);
    campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let prepared = prepare_iteration_event(&campaign, &campaign_view, &protocol, time(5)).unwrap();
    store.append_campaign_event(&prepared).await.unwrap();
    campaign_events.push(prepared);
    campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let run_id = Uuid::new_v4();
    let ready = experiment_view(&protocol, run_id, ExperimentRunState::Ready, None, None, 1);
    let started = start_run_event(&campaign, &campaign_view, &ready, time(6)).unwrap();
    store.append_campaign_event(&started).await.unwrap();
    campaign_events.push(started);
    campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let candidate_id = protocol.candidates[0].id;
    let development = experiment_view(
        &protocol,
        run_id,
        ExperimentRunState::AwaitingSealedAuthorization,
        Some(candidate_id),
        None,
        5,
    );
    let developed =
        record_development_event(&campaign, &campaign_view, &development, time(7)).unwrap();
    store.append_campaign_event(&developed).await.unwrap();
    campaign_events.push(developed);
    campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();
    let authorized = experiment_view(
        &protocol,
        run_id,
        ExperimentRunState::SealedAuthorized,
        Some(candidate_id),
        None,
        6,
    );
    let authorization = record_sealed_authorization_event(
        &campaign,
        &campaign_view,
        &authorized,
        "operator",
        time(8),
    )
    .unwrap();
    store.append_campaign_event(&authorization).await.unwrap();
    campaign_events.push(authorization);
    campaign_view = replay_campaign(&campaign, &campaign_events).unwrap();

    let completed = experiment_view(
        &protocol,
        run_id,
        ExperimentRunState::Completed,
        Some(candidate_id),
        Some(FinalDecision::RetainBaseline),
        9,
    );
    let mut exposure = SealedAssessmentExposure {
        schema_version: SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        campaign_fingerprint: campaign.fingerprint.clone(),
        generation_id: generation.id,
        generation_fingerprint: generation.fingerprint.clone(),
        iteration: 1,
        experiment_run_id: run_id,
        experiment_protocol_id: protocol.id,
        experiment_protocol_fingerprint: protocol.fingerprint.clone(),
        candidate_id,
        sealed_suite_id: binding.sealed_suite_id,
        sealed_suite_fingerprint: binding.sealed_suite_fingerprint.clone(),
        report_id: Uuid::new_v4(),
        report_fingerprint: digest('d'),
        assessment_id: Uuid::new_v4(),
        assessment_fingerprint: digest('e'),
        authorized_by: "operator".into(),
        created_at: time(9),
        fingerprint: String::new(),
    };
    exposure.fingerprint = exposure.reproduce_fingerprint().unwrap();
    let terminal = generation_view
        .next_event(
            &generation,
            BenchmarkGenerationEventKind::IterationConsumed {
                experiment_run_id: run_id,
                experiment_protocol_fingerprint: protocol.fingerprint.clone(),
                sealed_exposure_id: exposure.id,
                sealed_exposure_fingerprint: exposure.fingerprint.clone(),
                final_decision_fingerprint: completed.last_event_fingerprint.clone(),
            },
            time(9),
        )
        .unwrap();
    let finalized = finalize_iteration_event(
        &campaign,
        &campaign_view,
        &completed,
        Some(&terminal),
        Some(exposure),
        time(10),
    )
    .unwrap();

    let generation_count = generation_events.len();
    let campaign_count = campaign_events.len();
    let mut colliding_finalized = finalized.clone();
    colliding_finalized.id = first.id;
    colliding_finalized.fingerprint = colliding_finalized.reproduce_fingerprint().unwrap();
    assert!(
        store
            .append_iteration_finalization(&terminal, &colliding_finalized)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_benchmark_generation_events(generation.id)
            .await
            .unwrap()
            .len(),
        generation_count
    );
    assert_eq!(
        store.list_campaign_events(campaign.id).await.unwrap().len(),
        campaign_count
    );

    store
        .append_iteration_finalization(&terminal, &finalized)
        .await
        .unwrap();
    let generation_view = replay_benchmark_generation(
        &generation,
        &store
            .list_benchmark_generation_events(generation.id)
            .await
            .unwrap(),
    )
    .unwrap();
    let campaign_view = replay_campaign(
        &campaign,
        &store.list_campaign_events(campaign.id).await.unwrap(),
    )
    .unwrap();
    assert_eq!(
        generation_view.state,
        workflow_core::benchmark_generation::BenchmarkGenerationState::Exhausted
    );
    assert_eq!(
        campaign_view.state,
        encoder_campaign_core::CampaignState::RenewalRequired
    );
}

#[tokio::test]
async fn immutable_rows_reject_sql_tampering() {
    let store = SqliteExperimentStore::connect("sqlite::memory:")
        .await
        .unwrap();
    let generation = generation(None, 'c');
    let events = create_active_generation(&store, &generation).await;
    assert!(
        sqlx::query(
            "UPDATE encoder_benchmark_generation_events SET artifact_json = '{}' WHERE id = ?"
        )
        .bind(events[0].id)
        .execute(store.pool())
        .await
        .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM encoder_benchmark_generations WHERE id = ?")
            .bind(generation.id)
            .execute(store.pool())
            .await
            .is_err()
    );
}
