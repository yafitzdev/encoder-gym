//! Deterministic fakes and fixtures shared by application integration tests.

pub use generation_fake::FakeGenerationBackend;

use generation_core::{
    construction::RowConstructionPlan,
    domain::{GenerationParameters, GenerationPlan},
    jobs::{
        GenerationBackendIdentity, GenerationExecutionPolicy, GenerationExecutionSpec,
        GenerationJob, JobRunnerPolicy,
    },
    planning::calculate_generation_needs,
    ports::{GenerationExecutionStore, RowStore, StoreError},
    prompting::PromptBuilder,
};

/// Persists the immutable execution facts required before a test runner may call a backend.
pub async fn persist_test_generation_execution<S>(
    store: &S,
    job: &GenerationJob,
    plan: &GenerationPlan,
    policy: &JobRunnerPolicy,
) -> Result<GenerationExecutionSpec, StoreError>
where
    S: GenerationExecutionStore + RowStore,
{
    let accepted = store
        .dataset_cell_counts(job.dataset_id)
        .await?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let spec = GenerationExecutionSpec::new(
        job.id,
        job.dataset_id,
        job.plan_id,
        calculate_generation_needs(plan, &accepted),
        GenerationBackendIdentity {
            name: job.backend_name.clone(),
            model: job.backend_model.clone(),
            endpoint: None,
        },
        GenerationParameters::default(),
        GenerationExecutionPolicy::from(policy),
        PromptBuilder::template_identity().map_err(|error| StoreError(error.to_string()))?,
        "none",
    )
    .map_err(|error| StoreError(error.to_string()))?;
    store.create_generation_execution(job, &spec).await?;
    Ok(spec)
}

/// Persists a test execution with an explicit immutable hybrid construction policy.
pub async fn persist_test_generation_execution_with_construction<S>(
    store: &S,
    job: &GenerationJob,
    plan: &GenerationPlan,
    policy: &JobRunnerPolicy,
    construction_plan: RowConstructionPlan,
) -> Result<GenerationExecutionSpec, StoreError>
where
    S: GenerationExecutionStore + RowStore,
{
    let accepted = store
        .dataset_cell_counts(job.dataset_id)
        .await?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let spec = GenerationExecutionSpec::new_with_construction(
        job.id,
        job.dataset_id,
        job.plan_id,
        calculate_generation_needs(plan, &accepted),
        GenerationBackendIdentity {
            name: job.backend_name.clone(),
            model: job.backend_model.clone(),
            endpoint: None,
        },
        GenerationParameters::default(),
        GenerationExecutionPolicy::from(policy),
        PromptBuilder::template_identity().map_err(|error| StoreError(error.to_string()))?,
        "none",
        construction_plan,
    )
    .map_err(|error| StoreError(error.to_string()))?;
    store.create_generation_execution(job, &spec).await?;
    Ok(spec)
}
