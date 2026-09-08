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
    journal::{first_event, replay_experiment},
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    ports::ExperimentStore,
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    observation::DevelopmentObservation,
    quality::{NativeRepairAuditReference, NativeRepairRowEvidence},
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

pub(crate) fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

pub(crate) fn time(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
}

pub(crate) fn project() -> ExternalProjectSnapshot {
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

pub(crate) fn contract() -> MetricContract {
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

pub(crate) fn authority(
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

pub(crate) fn generation(valid_until: DateTime<Utc>) -> BenchmarkGeneration {
    let sealed_suite_id = Uuid::new_v4();
    let sealed_suite_fingerprint = digest('e');
    let freshness = BenchmarkFreshnessAuthority::create(
        sealed_suite_id,
        sealed_suite_fingerprint.clone(),
        digest('f'),
        digest('0'),
        time(0),
        time(1),
        valid_until,
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

pub(crate) fn report(
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

pub(crate) fn protocol(
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

pub(crate) struct Fixture {
    pub(crate) store: SqliteExperimentStore,
    pub(crate) project: ExternalProjectSnapshot,
    pub(crate) campaign_id: Uuid,
    pub(crate) run_id: Uuid,
    pub(crate) protocol: ExperimentProtocol,
    pub(crate) generation_id: Uuid,
}

pub(crate) async fn fixture() -> Fixture {
    fixture_in("sqlite::memory:", time(20)).await
}

pub(crate) async fn fixture_in(database_url: &str, valid_until: DateTime<Utc>) -> Fixture {
    let store = SqliteExperimentStore::connect(database_url).await.unwrap();
    let project = project();
    store.create_project(project.clone()).await.unwrap();
    let generation = generation(valid_until);
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

pub(crate) fn observations(ranks: [Option<u32>; 3]) -> Vec<DevelopmentObservation> {
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

pub(crate) fn native_rows(
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

pub(crate) fn native_audit_references() -> Vec<NativeRepairAuditReference> {
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
