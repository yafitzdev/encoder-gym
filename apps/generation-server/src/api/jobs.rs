use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use generation_core::{
    domain::GenerationParameters,
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy},
    planning::calculate_generation_needs,
    ports::{BackendConfigurationStore, GenerationBackend, JobStore, PlanStore, RowStore},
    validation::ValidationPipeline,
};
use generation_fake::FakeGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
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
    let requested_rows = calculate_generation_needs(&plan, &accepted)
        .iter()
        .map(|need| u64::from(need.remaining_count))
        .sum();

    let (backend, parameters): (Arc<dyn GenerationBackend>, GenerationParameters) =
        match input.backend.as_str() {
            "fake" => (
                Arc::new(FakeGenerationBackend::default()),
                GenerationParameters::default(),
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
                (Arc::new(backend), configuration.parameters)
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
    state
        .store
        .create_job(&job)
        .await
        .map_err(ApiError::internal)?;

    let runner = JobRunner::new(
        Arc::new(state.store.clone()),
        backend,
        JobRunnerPolicy {
            batch_size: input.batch_size,
            max_request_retries: input.max_retries,
            max_attempt_multiplier: input.max_attempt_multiplier,
            retry_delay: Duration::from_millis(500),
        },
        ValidationPipeline::standard(None),
    );
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
