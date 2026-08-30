use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use dataset_core::{domain::SourceProvenance, ports::AcceptedRowSource};
use generation_core::{
    construction::{
        FieldDefinition, FieldRecipe, FieldSourceKind, FieldValueType, RowConstructionPlan,
    },
    coverage::calculate_coverage,
    domain::{
        DatasetDefinition, DimensionDefinition, GeneratedCandidate, GenerationParameters,
        GenerationRequest, GenerationResult, ValidationStatus,
    },
    jobs::{
        GenerationAttempt, GenerationAttemptKind, GenerationAttemptState, GenerationJob, JobRunner,
        JobRunnerPolicy, JobState,
    },
    planning::equal_target_plan,
    ports::{
        BoxFuture, DatasetStore, GenerationBackend, GenerationBackendError,
        GenerationExecutionStore, JobStore, PlanStore, RowQuery, RowStore,
    },
    validation::ValidationPipeline,
};
use generation_test_support::{
    FakeGenerationBackend, persist_test_generation_execution,
    persist_test_generation_execution_with_construction,
};
use synthetic_data_sqlite::SqliteStore;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy)]
enum BackendBehavior {
    Valid,
    EmptyText,
    DuplicateText,
}

struct ScriptedBackend {
    failures_remaining: AtomicUsize,
    row_sequence: AtomicU64,
    behavior: BackendBehavior,
}

impl ScriptedBackend {
    fn new(failures: usize, behavior: BackendBehavior) -> Self {
        Self {
            failures_remaining: AtomicUsize::new(failures),
            row_sequence: AtomicU64::new(0),
            behavior,
        }
    }
}

impl GenerationBackend for ScriptedBackend {
    fn name(&self) -> &str {
        "scripted"
    }

    fn model(&self) -> &str {
        "test-v1"
    }

    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            if self
                .failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(GenerationBackendError::Request(
                    "temporary test failure".into(),
                ));
            }

            let rows = (0..request.requested_count)
                .map(|_| {
                    let sequence = self.row_sequence.fetch_add(1, Ordering::SeqCst);
                    let text = match self.behavior {
                        BackendBehavior::Valid => format!("valid generated row {sequence}"),
                        BackendBehavior::EmptyText => " ".into(),
                        BackendBehavior::DuplicateText => "same normalized text".into(),
                    };
                    GeneratedCandidate {
                        text,
                        label: request.target.label.clone(),
                        dimensions: request.target.dimensions.clone(),
                        fields: BTreeMap::new(),
                        construction: None,
                    }
                })
                .collect();

            Ok(GenerationResult {
                rows,
                usage: None,
                backend_metadata: serde_json::json!({"scripted": true}),
                errors: vec![],
            })
        })
    }
}

struct BlockingBackend {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl GenerationBackend for BlockingBackend {
    fn name(&self) -> &str {
        "blocking-test"
    }

    fn model(&self) -> &str {
        "test-v1"
    }

    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            self.started.notify_one();
            self.release.notified().await;
            Ok(GenerationResult {
                rows: vec![GeneratedCandidate {
                    text: "row generated before cancellation was observed".into(),
                    label: request.target.label,
                    dimensions: request.target.dimensions,
                    fields: BTreeMap::new(),
                    construction: None,
                }],
                usage: None,
                backend_metadata: serde_json::json!({"blocking": true}),
                errors: vec![],
            })
        })
    }
}

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("runner.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

#[tokio::test]
async fn fake_backend_fills_every_cell_and_persists_progress() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into(), "fraud".into()],
        vec![
            DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                .expect("valid dimension"),
        ],
    )
    .expect("valid dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let plan = equal_target_plan(&dataset, 3).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "deterministic-v1", 12);
    let policy = JobRunnerPolicy {
        batch_size: 2,
        max_request_retries: 1,
        max_attempt_multiplier: 2,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &job, &plan, &policy)
        .await
        .expect("execution");

    let runner = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        policy,
        ValidationPipeline::standard(None),
    );
    let completed = runner
        .run(job.id, GenerationParameters::default())
        .await
        .expect("runner succeeds");

    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.accepted_rows, 12);
    assert_eq!(completed.rejected_rows, 0);
    let coverage = calculate_coverage(&plan, &store.cell_counts(plan.id).await.expect("counts"));
    assert!(coverage.iter().all(|cell| cell.accepted == 3));
    assert!(coverage.iter().all(|cell| cell.remaining == 0));
}

#[tokio::test]
async fn deterministic_construction_completes_without_calling_the_backend() {
    let (_directory, store) = store().await;
    let (_dataset, plan, job) = persisted_single_cell_job(&store, 2).await;
    let policy = JobRunnerPolicy::default();
    let construction = RowConstructionPlan::new(
        17,
        vec![
            FieldDefinition {
                name: "ticket_id".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::Sequence {
                    prefix: "T-".into(),
                    start: 100,
                    step: 1,
                    width: Some(4),
                },
            },
            FieldDefinition {
                name: "text".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::Template {
                    template: "support ticket ${ticket_id}".into(),
                },
            },
        ],
    )
    .expect("construction plan");
    persist_test_generation_execution_with_construction(
        &store,
        &job,
        &plan,
        &policy,
        construction.clone(),
    )
    .await
    .expect("execution");
    let backend = Arc::new(ScriptedBackend::new(0, BackendBehavior::Valid));
    let completed = JobRunner::new(
        Arc::new(store.clone()),
        backend.clone(),
        policy,
        ValidationPipeline::standard(None),
    )
    .run(job.id, GenerationParameters::default())
    .await
    .expect("deterministic construction succeeds");

    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(backend.row_sequence.load(Ordering::SeqCst), 0);
    let rows = store
        .list_rows(RowQuery {
            job_id: Some(job.id),
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rows");
    assert_eq!(rows[0].text, "support ticket T-0100");
    assert_eq!(rows[0].fields["ticket_id"], serde_json::json!("T-0100"));
    assert_eq!(
        rows[0].construction.as_ref().expect("trace").fields["text"].source,
        FieldSourceKind::Deterministic
    );
    let attempts = store
        .list_generation_attempts(job.id)
        .await
        .expect("attempts");
    assert_eq!(
        attempts[0].kind,
        GenerationAttemptKind::DeterministicConstruction
    );
    assert_eq!(attempts[0].backend_metadata["provider_called"], false);
    let source_rows = store
        .list_accepted_source_rows(job.dataset_id)
        .await
        .expect("accepted source rows");
    assert_eq!(source_rows[0].fields["ticket_id"], "T-0100");
    assert!(matches!(
        &source_rows[0].provenance,
        SourceProvenance::Generated {
            construction_plan_fingerprint: Some(fingerprint),
            ..
        } if fingerprint == &construction.fingerprint
    ));
}

#[tokio::test]
async fn hybrid_construction_merges_llm_text_with_trusted_deterministic_fields() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new(
        "support",
        "classify",
        vec!["billing".into()],
        vec![DimensionDefinition::new("style", vec!["messy".into()]).expect("dimension")],
    )
    .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");
    let plan = equal_target_plan(&dataset, 2).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let job = GenerationJob::queued(dataset.id, plan.id, "scripted", "test-v1", 2);
    let policy = JobRunnerPolicy::default();
    let construction = RowConstructionPlan::new(
        0,
        vec![
            FieldDefinition {
                name: "text".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::Llm {
                    instruction: "Generate a support request.".into(),
                },
            },
            FieldDefinition {
                name: "label_copy".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::CellLabel,
            },
            FieldDefinition {
                name: "style_copy".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::CellDimension {
                    name: "style".into(),
                },
            },
        ],
    )
    .expect("construction plan");
    persist_test_generation_execution_with_construction(
        &store,
        &job,
        &plan,
        &policy,
        construction.clone(),
    )
    .await
    .expect("execution");
    let backend = Arc::new(ScriptedBackend::new(0, BackendBehavior::Valid));
    let completed = JobRunner::new(
        Arc::new(store.clone()),
        backend.clone(),
        policy,
        ValidationPipeline::standard(None),
    )
    .run(job.id, GenerationParameters::default())
    .await
    .expect("hybrid generation succeeds");

    assert_eq!(completed.accepted_rows, 2);
    assert_eq!(backend.row_sequence.load(Ordering::SeqCst), 2);
    let rows = store
        .list_rows(RowQuery {
            job_id: Some(job.id),
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rows");
    assert!(rows.iter().all(|row| row.label == "billing"));
    assert!(rows.iter().all(|row| row.dimensions["style"] == "messy"));
    assert!(rows.iter().all(|row| row.fields["label_copy"] == "billing"));
    assert!(rows.iter().all(|row| row.fields["style_copy"] == "messy"));
    assert!(rows.iter().all(|row| {
        let trace = row.construction.as_ref().expect("trace");
        trace.plan_fingerprint == construction.fingerprint
            && trace.fields["text"].source == FieldSourceKind::Llm
            && trace.fields["label_copy"].source == FieldSourceKind::Deterministic
    }));
}

#[tokio::test]
async fn interrupted_deterministic_batches_replay_without_consuming_external_attempt_budget() {
    let (_directory, store) = store().await;
    let (_dataset, plan, job) = persisted_single_cell_job(&store, 1).await;
    let policy = JobRunnerPolicy {
        batch_size: 1,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    let construction = RowConstructionPlan::new(
        0,
        vec![FieldDefinition {
            name: "text".into(),
            value_type: FieldValueType::String,
            recipe: FieldRecipe::Template {
                template: "row ${row_index}".into(),
            },
        }],
    )
    .expect("construction");
    persist_test_generation_execution_with_construction(
        &store,
        &job,
        &plan,
        &policy,
        construction.clone(),
    )
    .await
    .expect("execution");
    let prepared = construction
        .compile()
        .expect("compile")
        .prepare(plan.cells[0].cell.clone(), 0, 1)
        .expect("prepare");
    let attempt = GenerationAttempt::start_deterministic(job.id, 1, &prepared).expect("attempt");
    store
        .start_generation_attempt(&attempt)
        .await
        .expect("open deterministic attempt");
    store
        .interrupt_open_generation_attempts(job.id)
        .await
        .expect("interrupt attempt");

    let completed = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(ScriptedBackend::new(0, BackendBehavior::Valid)),
        policy,
        ValidationPipeline::standard(None),
    )
    .run(job.id, GenerationParameters::default())
    .await
    .expect("deterministic replay succeeds");

    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.accepted_rows, 1);
    let attempts = store
        .list_generation_attempts(job.id)
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].state, GenerationAttemptState::Interrupted);
    assert_eq!(attempts[1].state, GenerationAttemptState::Succeeded);
    let rows = store
        .list_rows(RowQuery {
            job_id: Some(job.id),
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rows");
    assert_eq!(rows[0].text, "row 0");
}

#[tokio::test]
async fn later_plans_treat_targets_as_absolute_dataset_coverage() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new("support", "classify", vec!["billing".into()], vec![])
        .expect("valid dataset");
    store.create_dataset(&dataset).await.expect("dataset");

    let initial_plan = equal_target_plan(&dataset, 2).expect("initial plan");
    store
        .create_plan(&initial_plan)
        .await
        .expect("initial plan");
    let initial_job =
        GenerationJob::queued(dataset.id, initial_plan.id, "fake", "deterministic-v1", 2);
    persist_test_generation_execution(
        &store,
        &initial_job,
        &initial_plan,
        &JobRunnerPolicy::default(),
    )
    .await
    .expect("execution");
    JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        JobRunnerPolicy::default(),
        ValidationPipeline::standard(None),
    )
    .run(initial_job.id, GenerationParameters::default())
    .await
    .expect("initial generation");

    let absolute_plan = equal_target_plan(&dataset, 5).expect("absolute plan");
    store
        .create_plan(&absolute_plan)
        .await
        .expect("absolute plan");
    let additional_job =
        GenerationJob::queued(dataset.id, absolute_plan.id, "scripted", "test-v1", 3);
    persist_test_generation_execution(
        &store,
        &additional_job,
        &absolute_plan,
        &JobRunnerPolicy::default(),
    )
    .await
    .expect("execution");
    let completed = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(ScriptedBackend::new(0, BackendBehavior::Valid)),
        JobRunnerPolicy::default(),
        ValidationPipeline::standard(None),
    )
    .run(additional_job.id, GenerationParameters::default())
    .await
    .expect("additional generation");

    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.accepted_rows, 3);
    let coverage = calculate_coverage(
        &absolute_plan,
        &store
            .dataset_cell_counts(dataset.id)
            .await
            .expect("dataset counts"),
    );
    assert_eq!(coverage[0].accepted, 5);
    assert_eq!(coverage[0].remaining, 0);
    assert_eq!(
        store
            .cell_counts(absolute_plan.id)
            .await
            .expect("plan counts")[&absolute_plan.cells[0].cell.key()]
            .accepted,
        3
    );
}

#[tokio::test]
async fn a_pre_requested_cancellation_stops_without_generation() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new("support", "classify", vec!["billing".into()], vec![])
        .expect("valid dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let plan = equal_target_plan(&dataset, 3).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "deterministic-v1", 3);
    persist_test_generation_execution(&store, &job, &plan, &JobRunnerPolicy::default())
        .await
        .expect("execution");
    store
        .request_job_cancellation(job.id)
        .await
        .expect("cancellation persisted");

    let runner = JobRunner::new(
        Arc::new(store),
        Arc::new(FakeGenerationBackend::default()),
        JobRunnerPolicy::default(),
        ValidationPipeline::standard(None),
    );
    let cancelled = runner
        .run(job.id, GenerationParameters::default())
        .await
        .expect("runner returns cancelled job");
    assert_eq!(cancelled.state, JobState::Cancelled);
    assert_eq!(cancelled.generated_rows, 0);
}

#[tokio::test]
async fn cancellation_requested_while_running_is_observed_between_batches() {
    let (_directory, store) = store().await;
    let (_dataset, plan, mut job) = persisted_single_cell_job(&store, 2).await;
    job.backend_name = "blocking-test".into();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let policy = JobRunnerPolicy {
        batch_size: 1,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &job, &plan, &policy)
        .await
        .expect("execution");
    let runner = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(BlockingBackend {
            started: started.clone(),
            release: release.clone(),
        }),
        policy,
        ValidationPipeline::standard(None),
    );
    let job_id = job.id;
    let task =
        tokio::spawn(async move { runner.run(job_id, GenerationParameters::default()).await });

    started.notified().await;
    assert!(
        store
            .request_job_cancellation(job_id)
            .await
            .expect("cancellation persisted")
    );
    release.notify_one();
    let cancelled = task
        .await
        .expect("runner task joins")
        .expect("runner returns a job");

    assert_eq!(cancelled.state, JobState::Cancelled);
    assert!(cancelled.cancel_requested);
    assert_eq!(cancelled.generated_rows, 1);
    assert_eq!(cancelled.accepted_rows, 1);
}

#[tokio::test]
async fn a_transient_backend_failure_is_retried_and_recorded() {
    let (_directory, store) = store().await;
    let (dataset, plan, job) = persisted_single_cell_job(&store, 1).await;
    let policy = JobRunnerPolicy {
        batch_size: 1,
        max_request_retries: 1,
        max_attempt_multiplier: 2,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &job, &plan, &policy)
        .await
        .expect("execution");
    let runner = JobRunner::new(
        Arc::new(store),
        Arc::new(ScriptedBackend::new(1, BackendBehavior::Valid)),
        policy,
        ValidationPipeline::standard(None),
    );

    let completed = runner
        .run(job.id, GenerationParameters::default())
        .await
        .expect("runner retries and succeeds");

    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.failed_requests, 1);
    assert_eq!(completed.accepted_rows, 1);
    assert_eq!(completed.dataset_id, dataset.id);
    assert_eq!(completed.plan_id, plan.id);
}

#[tokio::test]
async fn invalid_rows_are_persisted_as_rejected_and_leave_coverage_incomplete() {
    let (_directory, store) = store().await;
    let (_dataset, plan, job) = persisted_single_cell_job(&store, 1).await;
    let policy = JobRunnerPolicy {
        batch_size: 1,
        max_request_retries: 0,
        max_attempt_multiplier: 2,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &job, &plan, &policy)
        .await
        .expect("execution");
    let runner = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(ScriptedBackend::new(0, BackendBehavior::EmptyText)),
        policy,
        ValidationPipeline::standard(None),
    );

    let failed = runner
        .run(job.id, GenerationParameters::default())
        .await
        .expect("validation failure is a job result");

    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(failed.generated_rows, 2);
    assert_eq!(failed.accepted_rows, 0);
    assert_eq!(failed.rejected_rows, 2);
    let rejected = store
        .list_rows(RowQuery {
            job_id: Some(job.id),
            status: Some(ValidationStatus::Rejected),
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rejected rows load");
    assert_eq!(rejected.len(), 2);
    assert!(rejected.iter().all(|row| {
        row.validation_errors
            .iter()
            .any(|error| error.contains("empty_text"))
    }));
    let coverage = calculate_coverage(&plan, &store.cell_counts(plan.id).await.expect("counts"));
    assert_eq!(coverage[0].attempted, 2);
    assert_eq!(coverage[0].rejected, 2);
    assert_eq!(coverage[0].remaining, 1);
}

#[tokio::test]
async fn normalized_duplicates_are_rejected_without_hiding_the_accepted_row() {
    let (_directory, store) = store().await;
    let (_dataset, plan, job) = persisted_single_cell_job(&store, 2).await;
    let policy = JobRunnerPolicy {
        batch_size: 2,
        max_request_retries: 0,
        max_attempt_multiplier: 1,
        retry_delay: Duration::ZERO,
    };
    persist_test_generation_execution(&store, &job, &plan, &policy)
        .await
        .expect("execution");
    let runner = JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(ScriptedBackend::new(0, BackendBehavior::DuplicateText)),
        policy,
        ValidationPipeline::standard(None),
    );

    let failed = runner
        .run(job.id, GenerationParameters::default())
        .await
        .expect("duplicate exhaustion is a job result");

    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(failed.accepted_rows, 1);
    assert_eq!(failed.rejected_rows, 1);
    let rows = store
        .list_rows(RowQuery {
            job_id: Some(job.id),
            limit: 10,
            ..RowQuery::default()
        })
        .await
        .expect("rows load");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|row| row.validation_status == ValidationStatus::Accepted)
    );
    assert!(rows.iter().any(|row| {
        row.validation_status == ValidationStatus::Rejected
            && row
                .validation_errors
                .iter()
                .any(|error| error.contains("duplicate_text"))
    }));
}

async fn persisted_single_cell_job(
    store: &SqliteStore,
    target_count: u32,
) -> (
    DatasetDefinition,
    generation_core::domain::GenerationPlan,
    GenerationJob,
) {
    let dataset = DatasetDefinition::new("support", "classify", vec!["billing".into()], vec![])
        .expect("valid dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let plan = equal_target_plan(&dataset, target_count).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    let job = GenerationJob::queued(
        dataset.id,
        plan.id,
        "scripted",
        "test-v1",
        u64::from(target_count),
    );
    (dataset, plan, job)
}
