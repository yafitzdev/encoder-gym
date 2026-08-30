use std::collections::BTreeMap;

use chrono::Utc;
use generation_core::{
    domain::{
        DatasetDefinition, GeneratedRow, GenerationParameters, GenerationRequest, ValidationStatus,
    },
    jobs::{
        GenerationAttempt, GenerationAttemptState, GenerationBackendIdentity,
        GenerationExecutionPolicy, GenerationExecutionSpec, GenerationJob,
    },
    planning::equal_target_plan,
    ports::{DatasetStore, GenerationExecutionStore, JobStore, PlanStore, RowQuery, RowStore},
    prompting::PromptBuilder,
};
use synthetic_data_sqlite::SqliteStore;

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("execution-ledger.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

#[tokio::test]
async fn execution_attempt_rows_and_job_counters_commit_as_one_fact() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new("support", "classify", vec!["billing".into()], vec![])
        .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");
    let plan = equal_target_plan(&dataset, 2).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "deterministic-v1", 2);
    let spec = GenerationExecutionSpec::new(
        job.id,
        dataset.id,
        plan.id,
        generation_core::planning::calculate_generation_needs(&plan, &BTreeMap::new()),
        GenerationBackendIdentity {
            name: job.backend_name.clone(),
            model: job.backend_model.clone(),
            endpoint: None,
        },
        GenerationParameters::default(),
        GenerationExecutionPolicy {
            batch_size: 2,
            max_request_retries: 1,
            max_attempt_multiplier: 2,
            retry_delay_milliseconds: 0,
        },
        PromptBuilder::template_identity().expect("template"),
        "sha256:empty-semantic-context",
    )
    .expect("spec");
    store
        .create_generation_execution(&job, &spec)
        .await
        .expect("execution");
    assert_eq!(
        store
            .get_generation_execution_spec(job.id)
            .await
            .expect("spec query")
            .expect("spec"),
        spec
    );

    let request = GenerationRequest {
        system_prompt: "system".into(),
        user_prompt: "user".into(),
        target: plan.cells[0].cell.clone(),
        requested_count: 2,
        parameters: GenerationParameters::default(),
        construction: None,
    };
    let mut attempt = GenerationAttempt::start(job.id, 1, 0, &request).expect("attempt");
    store
        .start_generation_attempt(&attempt)
        .await
        .expect("attempt starts");
    let rows = vec![
        generated_row(&job, &plan, "accepted", true),
        generated_row(&job, &plan, "rejected", false),
    ];
    attempt
        .succeed(
            2,
            2,
            1,
            1,
            None,
            serde_json::json!({"request_id": "local-1"}),
            vec![],
        )
        .expect("success");
    let reconciled = store
        .finish_generation_attempt(&attempt, &rows)
        .await
        .expect("atomic finish");
    assert_eq!(reconciled.generated_rows, 2);
    assert_eq!(reconciled.accepted_rows, 1);
    assert_eq!(reconciled.rejected_rows, 1);
    assert_eq!(
        store
            .list_rows(RowQuery {
                job_id: Some(job.id),
                limit: 10,
                ..RowQuery::default()
            })
            .await
            .expect("rows")
            .len(),
        2
    );
    assert_eq!(
        store
            .list_generation_attempts(job.id)
            .await
            .expect("attempts")[0]
            .state,
        GenerationAttemptState::Succeeded
    );

    let mut rolled_back = GenerationAttempt::start(job.id, 2, 0, &request).expect("second attempt");
    store
        .start_generation_attempt(&rolled_back)
        .await
        .expect("second attempt starts");
    let duplicate = generated_row(&job, &plan, "duplicate primary key", true);
    let invalid_batch = vec![duplicate.clone(), duplicate];
    rolled_back
        .succeed(2, 2, 2, 0, None, serde_json::json!({}), vec![])
        .expect("terminal outcome");
    assert!(
        store
            .finish_generation_attempt(&rolled_back, &invalid_batch)
            .await
            .is_err(),
        "a late row failure must abort the entire batch transaction"
    );
    let after_rollback = store
        .list_generation_attempts(job.id)
        .await
        .expect("attempts after rollback");
    assert_eq!(after_rollback[1].state, GenerationAttemptState::Started);
    assert_eq!(
        store
            .get_job(job.id)
            .await
            .expect("job")
            .expect("job")
            .generated_rows,
        2
    );
}

#[tokio::test]
async fn open_provider_calls_become_durable_unknown_outcomes_on_recovery() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new("support", "classify", vec!["billing".into()], vec![])
        .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");
    let plan = equal_target_plan(&dataset, 1).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "deterministic-v1", 1);
    let spec = GenerationExecutionSpec::new(
        job.id,
        dataset.id,
        plan.id,
        generation_core::planning::calculate_generation_needs(&plan, &BTreeMap::new()),
        GenerationBackendIdentity {
            name: job.backend_name.clone(),
            model: job.backend_model.clone(),
            endpoint: None,
        },
        GenerationParameters::default(),
        GenerationExecutionPolicy {
            batch_size: 1,
            max_request_retries: 0,
            max_attempt_multiplier: 1,
            retry_delay_milliseconds: 0,
        },
        PromptBuilder::template_identity().expect("template"),
        "sha256:empty-semantic-context",
    )
    .expect("spec");
    store
        .create_generation_execution(&job, &spec)
        .await
        .expect("execution");
    let request = GenerationRequest {
        system_prompt: "system".into(),
        user_prompt: "user".into(),
        target: plan.cells[0].cell.clone(),
        requested_count: 1,
        parameters: GenerationParameters::default(),
        construction: None,
    };
    let attempt = GenerationAttempt::start(job.id, 1, 0, &request).expect("attempt");
    store
        .start_generation_attempt(&attempt)
        .await
        .expect("attempt starts");

    let interrupted = store
        .interrupt_open_generation_attempts(job.id)
        .await
        .expect("interrupt open attempts");
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].state, GenerationAttemptState::Interrupted);
    assert_eq!(
        store
            .get_job(job.id)
            .await
            .expect("job")
            .expect("job")
            .failed_requests,
        1
    );
}

fn generated_row(
    job: &GenerationJob,
    plan: &generation_core::domain::GenerationPlan,
    text: &str,
    accepted: bool,
) -> GeneratedRow {
    GeneratedRow {
        id: uuid::Uuid::new_v4(),
        dataset_id: job.dataset_id,
        plan_id: job.plan_id,
        generation_job_id: job.id,
        cell_key: plan.cells[0].cell.key(),
        text: text.into(),
        normalized_text: text.into(),
        label: plan.cells[0].cell.label.clone(),
        dimensions: BTreeMap::new(),
        fields: BTreeMap::new(),
        construction: None,
        generator_backend: job.backend_name.clone(),
        generator_model: job.backend_model.clone(),
        created_at: Utc::now(),
        validation_status: if accepted {
            ValidationStatus::Accepted
        } else {
            ValidationStatus::Rejected
        },
        validation_errors: if accepted {
            vec![]
        } else {
            vec!["test rejection".into()]
        },
        generation_metadata: serde_json::json!({}),
    }
}
