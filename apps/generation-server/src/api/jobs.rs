use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use generation_core::{
    domain::GenerationParameters,
    jobs::{
        GenerationBackendIdentity, GenerationExecutionPolicy, GenerationExecutionSpec,
        GenerationJob, JobRunner, JobRunnerPolicy,
    },
    planning::calculate_generation_needs,
    ports::{
        BackendConfigurationStore, DatasetStore, GenerationBackend, JobStore, PlanStore, RowStore,
    },
    prompting::PromptBuilder,
    validation::ValidationPipeline,
};
use generation_fake::FakeGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
use semantic_catalog::{
    GenerationSemanticAssignment, SemanticCatalogStore, SemanticDatasetSchema, resolve_semantics,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct StartJobInput {
    plan_id: Uuid,
    #[serde(default = "default_backend")]
    backend: String,
    #[serde(default = "default_batch_size")]
    batch_size: u32,
    #[serde(default = "default_retries")]
    max_retries: u32,
    #[serde(default = "default_attempt_multiplier")]
    max_attempt_multiplier: u32,
}

fn default_backend() -> String {
    "fake".into()
}

const fn default_batch_size() -> u32 {
    20
}

const fn default_retries() -> u32 {
    3
}

const fn default_attempt_multiplier() -> u32 {
    3
}

pub async fn start_job(
    State(state): State<AppState>,
    Json(input): Json<StartJobInput>,
) -> Result<impl IntoResponse, ApiError> {
    let plan = state
        .store
        .get_plan(input.plan_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("plan"))?;
    let accepted = state
        .store
        .dataset_cell_counts(plan.dataset_id)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let initial_needs = calculate_generation_needs(&plan, &accepted);
    let requested_rows = initial_needs
        .iter()
        .map(|need| u64::from(need.remaining_count))
        .sum();
    let dataset = state
        .store
        .get_dataset(plan.dataset_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("dataset"))?;
    let bindings = state
        .store
        .current_semantic_bindings(dataset.id)
        .await
        .map_err(ApiError::internal)?;
    let mut profiles = BTreeMap::new();
    for profile_id in bindings.iter().filter_map(|binding| binding.profile_id) {
        let profile = state
            .store
            .get_semantic_profile(profile_id)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::bad_request(format!("semantic profile missing: {profile_id}"))
            })?;
        profiles.insert(profile_id, profile);
    }
    let semantic_context = resolve_semantics(
        &SemanticDatasetSchema {
            dataset_id: dataset.id,
            labels: dataset.labels,
            dimensions: dataset
                .dimensions
                .into_iter()
                .map(|dimension| (dimension.name, dimension.values))
                .collect(),
        },
        &bindings,
        &profiles,
    )
    .map_err(ApiError::bad_request)?;

    let (backend, parameters, endpoint): (
        Arc<dyn GenerationBackend>,
        GenerationParameters,
        Option<String>,
    ) = match input.backend.as_str() {
        "fake" => (
            Arc::new(FakeGenerationBackend::default()),
            GenerationParameters::default(),
            None,
        ),
        "openai-compatible" => {
            let configuration = state
                .store
                .get_backend_configuration("openai-compatible")
                .await
                .map_err(ApiError::internal)?
                .ok_or_else(|| ApiError::bad_request("configure the backend first"))?;
            let base_url = configuration
                .base_url
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("backend base URL is missing"))?;
            let backend = OpenAICompatibleBackend::new(
                base_url,
                state.api_key.read().await.clone(),
                configuration.model.clone(),
            )
            .map_err(ApiError::bad_request)?;
            (
                Arc::new(backend),
                configuration.parameters,
                Some(base_url.trim().trim_end_matches('/').to_owned()),
            )
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported generation backend: {other}"
            )));
        }
    };

    let job = GenerationJob::queued(
        plan.dataset_id,
        plan.id,
        backend.name(),
        backend.model(),
        requested_rows,
    );
    let policy = JobRunnerPolicy {
        batch_size: input.batch_size,
        max_request_retries: input.max_retries,
        max_attempt_multiplier: input.max_attempt_multiplier,
        retry_delay: Duration::from_millis(500),
    };
    let execution = GenerationExecutionSpec::new(
        job.id,
        job.dataset_id,
        job.plan_id,
        initial_needs,
        GenerationBackendIdentity {
            name: backend.name().into(),
            model: backend.model().into(),
            endpoint,
        },
        parameters.clone(),
        GenerationExecutionPolicy::from(&policy),
        PromptBuilder::template_identity().map_err(ApiError::internal)?,
        semantic_context.fingerprint.clone(),
    )
    .map_err(ApiError::bad_request)?;
    let semantics = GenerationSemanticAssignment::new(job.id, semantic_context.clone())
        .map_err(ApiError::internal)?;
    state
        .store
        .create_generation_execution_bundle(&job, &execution, &semantics)
        .await
        .map_err(ApiError::internal)?;

    let runner = JobRunner::new(
        Arc::new(state.store.clone()),
        backend,
        policy,
        ValidationPipeline::standard(None),
    )
    .with_semantic_context(semantic_context);
    let worker = state.worker.clone();
    let job_id = job.id;
    tokio::spawn(async move {
        let Ok(_permit) = worker.acquire_owned().await else {
            return;
        };
        if let Err(error) = runner.run(job_id, parameters).await {
            tracing::error!(%job_id, %error, "generation worker stopped unexpectedly");
        }
    });

    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<GenerationJob>, ApiError> {
    state
        .store
        .get_job(id)
        .await
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job"))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = state
        .store
        .request_job_cancellation(id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({"cancel_requested": changed})))
}
