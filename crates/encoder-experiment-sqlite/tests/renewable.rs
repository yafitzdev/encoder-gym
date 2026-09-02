use chrono::{DateTime, TimeZone, Utc};
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignStore, ProductionCampaign,
    bind_generation_event, first_campaign_event, replay_campaign,
};
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity,
    },
    ports::ExperimentStore,
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
