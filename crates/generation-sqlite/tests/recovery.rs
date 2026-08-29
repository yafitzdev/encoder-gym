use std::{sync::Arc, time::Duration};

use analysis_core::{domain::AnalysisReport, ports::AnalysisStore};
use artifact_core::{ArtifactKind, ProvenanceNode, ProvenanceStore};
use dataset_core::{
    domain::{SnapshotSplit, SplitConfiguration, SplitRatios},
    ports::{AcceptedRowSource, SnapshotStore},
    splitting::build_snapshot,
};
use evaluation_core::{
    domain::{EvaluationRun, EvaluationRunState},
    ports::EvaluationStore,
};
use generation_core::{
    domain::{DatasetDefinition, GenerationParameters},
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy, JobState},
    planning::equal_target_plan,
    ports::{DatasetStore, JobStore, PlanStore, RowStore},
    validation::ValidationPipeline,
};
use generation_test_support::FakeGenerationBackend;
use optimization_core::{
    domain::{OptimizationProposal, ProposalApplication},
    ports::OptimizationStore,
};
use recovery_core::{RecoveryState, RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{TrainingCheckpoint, TrainingConfiguration, TrainingRun, TrainingRunState},
    ports::TrainingStore,
};

#[tokio::test]
async fn detects_dead_owners_preserves_live_work_and_resumes_generation_from_coverage() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("recovery.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let dataset = DatasetDefinition::new(
        "support",
        "classify support messages",
        vec!["billing".into(), "fraud".into()],
        vec![],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persists");
    let plan = equal_target_plan(&dataset, 1).expect("plan");
    store.create_plan(&plan).await.expect("plan persists");
    let mut generation = GenerationJob::queued(
        dataset.id,
        plan.id,
        "fake",
        "deterministic-v1",
        plan.total_target_count(),
    );
    store.create_job(&generation).await.expect("job persists");
    generation.transition(JobState::Running).expect("running");
    store.save_job(&generation).await.expect("running persists");
    store
        .acquire_execution_lease(WorkflowKind::Generation, generation.id)
        .await
        .expect("lease");

    assert!(
        store
            .detect_interrupted_workflows()
            .await
            .expect("detect")
            .is_empty(),
        "a workflow owned by this live process must not be interrupted"
    );
    store
        .release_execution_lease(WorkflowKind::Generation, generation.id)
        .await
        .expect("release");
    let detected = store
        .detect_interrupted_workflows()
        .await
        .expect("detect dead owner");
    assert_eq!(detected.len(), 1);
    assert!(detected[0].resumable_in_place);
    assert_eq!(
        store
            .get_job(generation.id)
            .await
            .expect("job query")
            .expect("job")
            .state,
        JobState::Failed
    );

    store
        .prepare_generation_resume(generation.id)
        .await
        .expect("prepare resume");
    JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        JobRunnerPolicy {
            batch_size: 1,
            max_request_retries: 0,
            max_attempt_multiplier: 1,
            retry_delay: Duration::ZERO,
        },
        ValidationPipeline::standard(None),
    )
    .run(generation.id, GenerationParameters::default())
    .await
    .expect("resume succeeds");
    store
        .resolve_recovery(
            WorkflowKind::Generation,
            generation.id,
            RecoveryState::Resumed,
            None,
        )
        .await
        .expect("resolve recovery");
    assert_eq!(
        store
            .dataset_cell_counts(dataset.id)
            .await
            .expect("coverage")
            .values()
            .map(|counts| counts.accepted)
            .sum::<u32>(),
        2,
        "resume must stop at persisted per-cell targets"
    );

    let source_rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("source rows");
    let (snapshot, members) = build_snapshot(
        dataset.id,
        "recovery-source",
        None,
        SplitConfiguration::new(SplitRatios::new(1.0, 0.0, 0.0).expect("ratios"), 7),
        source_rows,
    )
    .expect("snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot persists");
    let mut training = TrainingRun::queued(
        snapshot.id,
        "hashing-linear",
        "hashing-linear-v1",
        TrainingConfiguration::default(),
    )
    .expect("training run");
    store
        .create_training_run(&training)
        .await
        .expect("training persists");
    training
        .transition(TrainingRunState::Running)
        .expect("training running");
    store
        .save_training_run(&training)
        .await
        .expect("training state");
    let checkpoint = TrainingCheckpoint {
        id: uuid::Uuid::new_v4(),
        run_id: training.id,
        epoch: 1,
        artifact_path: "artifacts/interrupted/model.bin".into(),
        artifact_checksum: "sha256:test".into(),
        artifact_size_bytes: 0,
        model_format: "hashing-linear-v1".into(),
        training_loss: 1.0,
        validation_loss: None,
        is_final: false,
        created_at: chrono::Utc::now(),
    };
    store
        .create_checkpoint(&checkpoint)
        .await
        .expect("checkpoint metadata");
    let mut evaluation = EvaluationRun::queued(
        checkpoint.id,
        snapshot.id,
        SnapshotSplit::Train,
        "sha256:fixture",
    );
    store
        .create_evaluation_run(&evaluation)
        .await
        .expect("evaluation persists");
    evaluation
        .transition(EvaluationRunState::Running)
        .expect("evaluation running");
    store
        .save_evaluation_run(&evaluation)
        .await
        .expect("evaluation state");

    let detected = store
        .detect_interrupted_workflows()
        .await
        .expect("detect interrupted training and evaluation");
    assert_eq!(detected.len(), 2);
    assert!(detected.iter().all(|record| !record.resumable_in_place));
    let history = store
        .list_recovery_records(true)
        .await
        .expect("recovery history");
    assert_eq!(history.len(), 3);
    assert_eq!(
        history
            .iter()
            .filter(|record| record.state == RecoveryState::Pending)
            .count(),
        2
    );

    let report = AnalysisReport {
        id: uuid::Uuid::new_v4(),
        evaluation_run_id: evaluation.id,
        minimum_support: 1,
        prediction_count: 0,
        error_count: 0,
        findings: vec![],
        errors: vec![],
        protocol: analysis_core::protocol::AnalysisProtocol::default(),
        protocol_fingerprint: "sha256:fixture-protocol".into(),
        source_identity: Some(analysis_core::domain::AnalysisSourceIdentity::legacy(
            evaluation.id,
            0,
        )),
        finding_evidence: std::collections::BTreeMap::new(),
        comparison_diagnosis: None,
        fingerprint: "sha256:fixture-analysis".into(),
        created_at: chrono::Utc::now(),
    };
    store
        .create_analysis_report(&report)
        .await
        .expect("analysis report");
    let proposal = OptimizationProposal {
        id: uuid::Uuid::new_v4(),
        analysis_report_id: report.id,
        dataset_id: dataset.id,
        additional_example_budget: 2,
        minimum_support: 1,
        protocol: None,
        protocol_fingerprint: String::new(),
        source_identity: None,
        evidence_fingerprint: String::new(),
        decision_evidence: None,
        allocation: None,
        decision_cells: Vec::new(),
        normalized_recommendations: Vec::new(),
        training_candidate_set: None,
        review_only_recommendations: Vec::new(),
        rebase_lineage: None,
        recommendations: vec![],
        fingerprint: "sha256:fixture-proposal".into(),
        created_at: chrono::Utc::now(),
    };
    store
        .create_optimization_proposal(&proposal)
        .await
        .expect("proposal");
    let derived_plan = equal_target_plan(&dataset, 2).expect("derived plan");
    let application = ProposalApplication {
        proposal_id: proposal.id,
        generation_plan_id: derived_plan.id,
        approval_review_id: None,
        approval_fingerprint: None,
        selected_recommendation_ids: Vec::new(),
        verified_coverage_fingerprint: None,
        applied_at: chrono::Utc::now(),
    };
    let first = store
        .apply_proposal_plan(&derived_plan, &application)
        .await
        .expect("first application");
    let second = store
        .apply_proposal_plan(&derived_plan, &application)
        .await
        .expect("idempotent repeated application");
    assert_eq!(first, second);
    let plan_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM generation_plans WHERE dataset_id = ?")
            .bind(dataset.id)
            .fetch_one(store.pool())
            .await
            .expect("plan count");
    assert_eq!(plan_count, 2, "repeated application must not add a plan");

    let configuration_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_configurations \
         (id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(configuration_id)
    .bind("sha256:fixture-configuration")
    .bind(dataset.id)
    .bind(plan.id)
    .bind("{\"version\":1}")
    .bind(chrono::Utc::now())
    .execute(store.pool())
    .await
    .expect("configuration provenance");
    let trace = store
        .trace_provenance(ArtifactKind::OptimizationProposal, proposal.id)
        .await
        .expect("trace query")
        .expect("trace exists");
    let mut kinds = std::collections::BTreeSet::new();
    collect_kinds(&trace, &mut kinds);
    for expected in [
        ArtifactKind::OptimizationProposal,
        ArtifactKind::AnalysisReport,
        ArtifactKind::EvaluationRun,
        ArtifactKind::Checkpoint,
        ArtifactKind::TrainingRun,
        ArtifactKind::Snapshot,
        ArtifactKind::Dataset,
        ArtifactKind::ProjectConfiguration,
        ArtifactKind::GenerationJob,
        ArtifactKind::GenerationPlan,
    ] {
        assert!(kinds.contains(expected.as_str()), "missing {expected:?}");
    }
}

fn collect_kinds(node: &ProvenanceNode, kinds: &mut std::collections::BTreeSet<String>) {
    kinds.insert(node.kind.as_str().to_owned());
    for parent in &node.parents {
        collect_kinds(parent, kinds);
    }
}
