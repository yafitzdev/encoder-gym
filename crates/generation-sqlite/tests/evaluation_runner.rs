use std::{sync::Arc, time::Duration};

use analysis_core::{
    analysis::analyze_predictions,
    contract::DiagnosticContract,
    domain::{FindingReviewRecord, FindingReviewState},
    ports::{AnalysisFindingQuery, AnalysisStore, FindingEvidenceQuery, FindingReviewQuery},
    protocol::AnalysisProtocol,
    runner::{reproduce_report_fingerprint, run_analysis},
};
use chrono::Utc;
use dataset_core::{
    domain::{SnapshotSplit, SplitConfiguration, SplitRatios},
    ports::{AcceptedRowSource, SnapshotStore},
    splitting::build_snapshot,
};
use evaluation_core::{
    domain::{
        EvaluationExample, EvaluationProtocol, EvaluationRun, EvaluationRunState,
        EvaluationSourceIdentity,
    },
    metrics::calculate_metrics,
    ports::EvaluationStore,
    runner::EvaluationRunner,
};
use generation_core::{
    domain::{DatasetDefinition, GenerationParameters},
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy},
    planning::equal_target_plan,
    ports::{DatasetStore, PlanStore, RowStore},
    validation::ValidationPipeline,
};
use generation_test_support::{FakeGenerationBackend, persist_test_generation_execution};
use optimization_core::{
    application::approved_proposal_to_plan,
    campaigns::{create_campaign, link_generation_plan},
    domain::ProposalApplication,
    evidence::OptimizationEvidence,
    planning::{
        create_constrained_proposal, create_constrained_proposal_with_training, create_proposal,
        proposal_to_plan, rebase_constrained_proposal,
    },
    ports::{CampaignQuery, OptimizationStore, ProposalReviewQuery},
    protocol::{OptimizationProtocol, RecommendationKind, TrainingCandidateRequest},
    reviews::{ProposalReviewRecord, ProposalReviewState},
    scenarios::create_default_scenario_group,
    training_candidates::{TrainingConfigurationChoice, TrainingConfigurationSpace},
};
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{
        LabelProbability, Prediction, TrainingConfiguration, TrainingExample, TrainingRequest,
        TrainingRun,
    },
    ports::{
        CheckpointSink, Predictor, PredictorLoader, TrainingBackend, TrainingBackendError,
        TrainingStore,
    },
    runner::TrainingRunner,
};
use training_linear::{HashingLinearBackend, HashingLinearPredictorLoader, LocalCheckpointStore};

struct WrongPredictor {
    labels: Vec<String>,
}

impl Predictor for WrongPredictor {
    fn labels(&self) -> &[String] {
        &self.labels
    }

    fn predict(&self, text: &str) -> Result<Prediction, TrainingBackendError> {
        let label = if text.contains(&self.labels[0]) {
            self.labels[1].clone()
        } else {
            self.labels[0].clone()
        };
        Ok(Prediction {
            label: label.clone(),
            confidence: 1.0,
            probabilities: self
                .labels
                .iter()
                .map(|candidate| LabelProbability {
                    label: candidate.clone(),
                    probability: f64::from(candidate == &label),
                })
                .collect(),
        })
    }
}

#[tokio::test]
async fn evaluates_a_checkpoint_and_persists_reproducible_metrics() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("evaluation.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");

    let dataset = DatasetDefinition::new(
        "support",
        "classify support messages",
        vec!["billing".into(), "account".into()],
        vec![],
    )
    .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");
    let plan = equal_target_plan(&dataset, 10).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let generation_job = GenerationJob::queued(
        dataset.id,
        plan.id,
        "fake",
        "deterministic-v1",
        plan.total_target_count(),
    );
    let generation_policy = JobRunnerPolicy {
        batch_size: 5,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &generation_job, &plan, &generation_policy)
        .await
        .expect("execution");
    JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        generation_policy,
        ValidationPipeline::standard(None),
    )
    .run(generation_job.id, GenerationParameters::default())
    .await
    .expect("generate");

    let rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("source rows");
    let (snapshot, members) = build_snapshot(
        dataset.id,
        "baseline",
        None,
        SplitConfiguration::new(SplitRatios::new(0.7, 0.1, 0.2).expect("ratios"), 7),
        rows,
    )
    .expect("snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot persists");

    let configuration = TrainingConfiguration {
        feature_dimension: 128,
        epochs: 5,
        learning_rate: 0.2,
        l2: 0.0,
        checkpoint_every: 5,
        seed: 11,
    };
    let backend = HashingLinearBackend;
    let training_run = TrainingRun::queued(
        snapshot.id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
    )
    .expect("training run");
    store
        .create_training_run(&training_run)
        .await
        .expect("training run persists");
    let request = TrainingRequest::in_memory(
        training_run.id,
        snapshot.id,
        dataset.labels.clone(),
        members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Train)
            .map(|member| TrainingExample {
                snapshot_member_id: member.id,
                text: member.text.clone(),
                label: member.label.clone(),
            })
            .collect(),
        vec![],
        configuration,
    );
    let artifacts = Arc::new(LocalCheckpointStore::new(
        directory.path().join("artifacts"),
    ));
    TrainingRunner::new(
        Arc::new(store.clone()),
        Arc::new(HashingLinearBackend),
        artifacts.clone(),
    )
    .run(training_run.id, request)
    .await
    .expect("train");
    let checkpoint = store
        .list_checkpoints(training_run.id)
        .await
        .expect("checkpoints")
        .pop()
        .expect("final checkpoint");
    let bytes = artifacts.read(&checkpoint.artifact_path).expect("artifact");
    let predictor = HashingLinearPredictorLoader
        .load(&bytes)
        .expect("predictor");

    let evaluation_run = EvaluationRun::queued(
        checkpoint.id,
        snapshot.id,
        SnapshotSplit::Test,
        "sha256:fixture",
    );
    store
        .create_evaluation_run(&evaluation_run)
        .await
        .expect("evaluation run persists");
    let examples = members
        .iter()
        .filter(|member| member.split == SnapshotSplit::Test)
        .map(|member| EvaluationExample {
            snapshot_member_id: member.id,
            source_row_id: member.source_row_id,
            text: member.text.clone(),
            expected_label: member.label.clone(),
            dimensions: member.dimensions.clone(),
        })
        .collect::<Vec<_>>();
    let expected_count = examples.len() as u64;
    let completed = EvaluationRunner::new(
        Arc::new(store.clone()),
        Arc::new(store.clone()),
        Arc::from(predictor),
    )
    .run(evaluation_run.id, dataset.labels.clone())
    .await
    .expect("evaluate");

    assert_eq!(completed.state, EvaluationRunState::Completed);
    assert_eq!(completed.example_count, expected_count);
    let predictions = store
        .list_predictions(evaluation_run.id)
        .await
        .expect("predictions");
    assert_eq!(predictions.len() as u64, expected_count);
    assert_eq!(
        completed.metrics,
        Some(calculate_metrics(&dataset.labels, &predictions))
    );
    assert_eq!(
        store
            .get_evaluation_run(evaluation_run.id)
            .await
            .expect("read run"),
        Some(completed)
    );

    let report = analyze_predictions(evaluation_run.id, &predictions, 1).expect("analysis");
    store
        .create_analysis_report(&report)
        .await
        .expect("analysis report persists");
    assert_eq!(
        store
            .get_analysis_report(report.id)
            .await
            .expect("read analysis report"),
        Some(report)
    );

    let error_protocol = EvaluationProtocol {
        split: SnapshotSplit::Test,
        ..EvaluationProtocol::default()
    };
    let error_source = EvaluationSourceIdentity {
        checkpoint_checksum: checkpoint.artifact_checksum.clone(),
        checkpoint_model_format: checkpoint.model_format.clone(),
        base_model_fingerprint: None,
        tokenizer_fingerprint: None,
        snapshot_fingerprint: snapshot.fingerprint.clone(),
        cohort_fingerprint: "sha256:fixture-error-cohort".into(),
        labels: dataset.labels.clone(),
    };
    let test_count = members
        .iter()
        .filter(|member| member.split == SnapshotSplit::Test)
        .count() as u64;
    let error_run = EvaluationRun::queued_with_protocol(
        checkpoint.id,
        snapshot.id,
        error_protocol,
        error_source,
        test_count,
        "sha256:fixture-error",
    )
    .expect("error evaluation run");
    store
        .create_evaluation_run(&error_run)
        .await
        .expect("error evaluation run persists");
    EvaluationRunner::new(
        Arc::new(store.clone()),
        Arc::new(store.clone()),
        Arc::new(WrongPredictor {
            labels: dataset.labels.clone(),
        }),
    )
    .run(error_run.id, dataset.labels.clone())
    .await
    .expect("error evaluation");
    let error_report = run_analysis(&store, &store, error_run.id, AnalysisProtocol::default())
        .await
        .expect("bounded analysis");
    assert_eq!(error_report.error_count, error_report.prediction_count);
    assert!(error_report.errors.is_empty());
    assert_eq!(
        reproduce_report_fingerprint(&error_report).expect("fingerprint"),
        error_report.fingerprint
    );
    let findings = store
        .query_analysis_findings(AnalysisFindingQuery {
            analysis_report_id: error_report.id,
            kind: None,
            minimum_support: None,
            minimum_error_count: None,
            sort: Default::default(),
            limit: 100,
            offset: 0,
        })
        .await
        .expect("normalized findings");
    assert_eq!(findings, error_report.findings);
    let first = findings.first().expect("finding");
    let evidence = store
        .query_finding_evidence(FindingEvidenceQuery {
            analysis_report_id: error_report.id,
            finding_key: first.key.clone(),
            category: None,
            limit: 100,
            offset: 0,
        })
        .await
        .expect("normalized evidence");
    assert_eq!(evidence, error_report.finding_evidence[&first.key]);
    let review = FindingReviewRecord {
        id: uuid::Uuid::new_v4(),
        analysis_report_id: error_report.id,
        finding_key: first.key.clone(),
        state: FindingReviewState::CandidateForMoreData,
        note: Some("collect more examples".into()),
        resolution_evaluation_run_id: None,
        resolution_comparison_id: None,
        created_at: Utc::now(),
    };
    store
        .append_finding_review(&review)
        .await
        .expect("append review");
    assert_eq!(
        store
            .query_finding_reviews(FindingReviewQuery {
                analysis_report_id: Some(error_report.id),
                state: None,
                limit: 10,
                offset: 0,
            })
            .await
            .expect("review history"),
        vec![review]
    );

    let accepted = store
        .dataset_cell_counts(dataset.id)
        .await
        .expect("accepted coverage")
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let protocol = OptimizationProtocol::legacy(5, 1);
    let diagnostics = DiagnosticContract::try_from(&error_report).expect("diagnostics");
    let decision_evidence = OptimizationEvidence::new(
        diagnostics,
        &dataset,
        snapshot.id,
        snapshot.fingerprint.clone(),
        &accepted,
        &protocol,
        None,
        true,
    )
    .expect("optimization evidence");
    let constrained = create_constrained_proposal(&decision_evidence, &protocol)
        .expect("constrained optimization proposal");
    store
        .create_optimization_proposal(&constrained)
        .await
        .expect("constrained proposal persists");
    assert_eq!(
        store
            .get_optimization_proposal(constrained.id)
            .await
            .expect("read constrained proposal"),
        Some(constrained.clone())
    );
    let normalized_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimization_recommendations WHERE proposal_id = ?",
    )
    .bind(constrained.id)
    .fetch_one(store.pool())
    .await
    .expect("normalized recommendation count");
    assert_eq!(
        normalized_count,
        i64::try_from(constrained.normalized_recommendations.len()).expect("count")
    );
    let completed_training = store
        .get_training_run(training_run.id)
        .await
        .expect("completed baseline lookup")
        .expect("completed baseline");
    let mut candidate_configuration = completed_training.configuration.clone();
    candidate_configuration.learning_rate = 0.1;
    let training_space = TrainingConfigurationSpace::new(
        &completed_training,
        &checkpoint,
        snapshot.fingerprint.clone(),
        None,
        vec![TrainingConfigurationChoice {
            configuration: candidate_configuration,
            transformer_configuration: None,
        }],
    )
    .expect("training configuration space");
    let mut training_protocol = protocol.clone();
    training_protocol.recommendation_kinds = vec![
        RecommendationKind::DataGeneration,
        RecommendationKind::TrainingConfiguration,
    ];
    training_protocol.training_candidates = Some(TrainingCandidateRequest {
        maximum_candidates: 1,
    });
    let training_protocol = training_protocol.normalize().expect("training protocol");
    let training_evidence = decision_evidence
        .rebind_protocol(&training_protocol, Some(training_space.fingerprint.clone()))
        .expect("training-bound evidence");
    let training_proposal = create_constrained_proposal_with_training(
        &training_evidence,
        &training_protocol,
        Some(&training_space),
    )
    .expect("proposal with training candidates");
    store
        .create_optimization_proposal(&training_proposal)
        .await
        .expect("training proposal persists");
    let training_candidate_id = training_proposal
        .training_candidate_set
        .as_ref()
        .expect("training candidate set")
        .candidates[0]
        .id
        .clone();
    let training_review = ProposalReviewRecord::new(
        &training_proposal,
        ProposalReviewState::AcceptedTrainingExperimentCandidate,
        vec![training_candidate_id],
        Some("run separately".into()),
        None,
        None,
    )
    .expect("training candidate approval");
    store
        .append_proposal_review(&training_review)
        .await
        .expect("training candidate review persists");
    let training_candidate_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimization_training_candidates WHERE proposal_id = ?",
    )
    .bind(training_proposal.id)
    .fetch_one(store.pool())
    .await
    .expect("training candidate count");
    assert_eq!(training_candidate_count, 1);
    let training_selection_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimization_proposal_review_training_selections \
         WHERE review_id = ?",
    )
    .bind(training_review.id)
    .fetch_one(store.pool())
    .await
    .expect("training selection count");
    assert_eq!(training_selection_count, 1);
    let mut advisory_protocol = protocol.clone();
    advisory_protocol.recommendation_kinds = vec![
        RecommendationKind::DataGeneration,
        RecommendationKind::ReviewOnly,
    ];
    let advisory_protocol = advisory_protocol.normalize().expect("advisory protocol");
    let advisory_evidence = decision_evidence
        .rebind_protocol(&advisory_protocol, None)
        .expect("advisory-bound evidence");
    let advisory_proposal = create_constrained_proposal(&advisory_evidence, &advisory_protocol)
        .expect("proposal with advisory recommendations");
    store
        .create_optimization_proposal(&advisory_proposal)
        .await
        .expect("advisory proposal persists");
    let advisory_review = ProposalReviewRecord::new(
        &advisory_proposal,
        ProposalReviewState::AcceptedReviewOnlyCandidates,
        vec![advisory_proposal.review_only_recommendations[0].id.clone()],
        Some("inspect label boundary".into()),
        None,
        None,
    )
    .expect("advisory review");
    store
        .append_proposal_review(&advisory_review)
        .await
        .expect("advisory review persists");
    let advisory_selection_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimization_proposal_review_advisory_selections \
         WHERE review_id = ?",
    )
    .bind(advisory_review.id)
    .fetch_one(store.pool())
    .await
    .expect("advisory selection count");
    assert_eq!(advisory_selection_count, 1);
    let mut refreshed_accepted = accepted.clone();
    *refreshed_accepted
        .entry(constrained.normalized_recommendations[0].cell.key())
        .or_default() += 1;
    let refreshed_evidence = OptimizationEvidence::new(
        decision_evidence.diagnostics.clone(),
        &dataset,
        snapshot.id,
        snapshot.fingerprint.clone(),
        &refreshed_accepted,
        &protocol,
        None,
        true,
    )
    .expect("refreshed optimization evidence");
    let rebased =
        rebase_constrained_proposal(&constrained, &refreshed_evidence).expect("rebased proposal");
    store
        .create_optimization_proposal(&rebased)
        .await
        .expect("rebased proposal persists");
    assert_eq!(
        store
            .get_optimization_proposal(rebased.id)
            .await
            .expect("read rebased proposal"),
        Some(rebased.clone())
    );
    let rebase_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimization_proposal_rebases WHERE proposal_id = ?",
    )
    .bind(rebased.id)
    .fetch_one(store.pool())
    .await
    .expect("normalized rebase count");
    assert_eq!(rebase_count, 1);
    let scenario_group =
        create_default_scenario_group(&decision_evidence, &protocol).expect("scenario group");
    store
        .create_scenario_group(&scenario_group)
        .await
        .expect("scenario group persists");
    assert_eq!(
        store
            .get_scenario_group(scenario_group.id)
            .await
            .expect("read scenario group"),
        Some(scenario_group)
    );
    let approval = ProposalReviewRecord::new(
        &constrained,
        ProposalReviewState::PartiallyAccepted,
        vec![constrained.normalized_recommendations[0].id.clone()],
        Some("bounded first experiment".into()),
        None,
        None,
    )
    .expect("proposal approval");
    store
        .append_proposal_review(&approval)
        .await
        .expect("review persists");
    assert_eq!(
        store
            .query_proposal_reviews(ProposalReviewQuery {
                proposal_id: Some(constrained.id),
                state: None,
                limit: 10,
                offset: 0,
            })
            .await
            .expect("review history"),
        vec![approval.clone()]
    );
    let (approved_plan, approved_application) =
        approved_proposal_to_plan(&constrained, &approval, &dataset, &accepted)
            .expect("approved plan application");
    let (left_application, right_application) = tokio::join!(
        store.apply_proposal_plan(&approved_plan, &approved_application),
        store.apply_proposal_plan(&approved_plan, &approved_application),
    );
    assert_eq!(
        left_application.expect("first concurrent application"),
        approved_application
    );
    assert_eq!(
        right_application.expect("second concurrent application"),
        approved_application
    );
    assert_eq!(
        store
            .apply_proposal_plan(&approved_plan, &approved_application)
            .await
            .expect("approved application is idempotent"),
        approved_application
    );
    let campaign = create_campaign(&constrained, &approval).expect("optimization campaign");
    store
        .create_campaign(&campaign)
        .await
        .expect("campaign persists");
    let plan_link = link_generation_plan(&campaign, &approved_plan, &approved_application)
        .expect("compatible plan link");
    store
        .append_campaign_link(&plan_link)
        .await
        .expect("campaign link persists");
    assert_eq!(
        store
            .get_campaign(campaign.id)
            .await
            .expect("campaign lookup"),
        Some(campaign.clone())
    );
    assert_eq!(
        store
            .query_campaigns(CampaignQuery {
                proposal_id: Some(constrained.id),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("campaign query"),
        vec![campaign]
    );
    assert_eq!(
        store
            .list_campaign_links(plan_link.campaign_id)
            .await
            .expect("campaign links"),
        vec![plan_link]
    );
    let proposal =
        create_proposal(&error_report, &dataset, &accepted, 5, 1).expect("optimization proposal");
    assert_eq!(proposal.allocated_count(), 5);
    store
        .create_optimization_proposal(&proposal)
        .await
        .expect("proposal persists");
    let plan = proposal_to_plan(&proposal, &dataset).expect("proposal plan");
    store.create_plan(&plan).await.expect("plan persists");
    let application = ProposalApplication {
        proposal_id: proposal.id,
        generation_plan_id: plan.id,
        approval_review_id: None,
        approval_fingerprint: None,
        selected_recommendation_ids: Vec::new(),
        verified_coverage_fingerprint: None,
        applied_at: Utc::now(),
    };
    store
        .record_proposal_application(&application)
        .await
        .expect("application persists");
    assert_eq!(
        store
            .get_optimization_proposal(proposal.id)
            .await
            .expect("read proposal"),
        Some(proposal)
    );
    assert_eq!(
        store
            .get_proposal_application(application.proposal_id)
            .await
            .expect("read application"),
        Some(application)
    );
    assert_eq!(
        generation_core::planning::calculate_generation_needs(&plan, &accepted)
            .iter()
            .map(|need| u64::from(need.remaining_count))
            .sum::<u64>(),
        5
    );
}
