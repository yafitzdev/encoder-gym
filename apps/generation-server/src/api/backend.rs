use axum::{Json, extract::State};
use chrono::Utc;
use generation_core::{
    domain::{BackendConfiguration, GenerationParameters},
    ports::BackendConfigurationStore,
};
use serde::Deserialize;

use crate::{error::ApiError, state::AppState};

#[derive(Deserialize)]
pub struct BackendInput {
    base_url: String,
    model: String,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    seed: Option<u64>,
    api_key: Option<String>,
}

pub async fn configure_backend(
    State(state): State<AppState>,
    Json(input): Json<BackendInput>,
) -> Result<Json<BackendConfiguration>, ApiError> {
    if input.base_url.trim().is_empty() || input.model.trim().is_empty() {
        return Err(ApiError::bad_request("base_url and model are required"));
    }
    if let Some(api_key) = input.api_key.filter(|key| !key.trim().is_empty()) {
        *state.api_key.write().await = Some(api_key);
    }
    let configuration = BackendConfiguration {
        name: "openai-compatible".into(),
        base_url: Some(input.base_url.trim_end_matches('/').to_owned()),
        model: input.model.trim().to_owned(),
        parameters: GenerationParameters {
            temperature: input.temperature,
            max_tokens: input.max_tokens,
            seed: input.seed,
            ..GenerationParameters::default()
        },
        updated_at: Utc::now(),
    };
    state
        .store
        .save_backend_configuration(&configuration)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(configuration))
}

pub async fn get_backend(
    State(state): State<AppState>,
) -> Result<Json<Option<BackendConfiguration>>, ApiError> {
    Ok(Json(
        state
            .store
            .get_backend_configuration("openai-compatible")
            .await
            .map_err(ApiError::internal)?,
    ))
}
