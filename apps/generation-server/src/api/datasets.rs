use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, DimensionDefinition, ValidationStatus},
    export::{to_csv, to_jsonl},
    ports::{DatasetStore, RowQuery, RowStore},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct CreateDatasetInput {
    name: String,
    task_description: String,
    labels: Vec<String>,
    #[serde(default)]
    dimensions: Vec<DimensionDefinition>,
}

pub async fn create_dataset(
    State(state): State<AppState>,
    Json(input): Json<CreateDatasetInput>,
) -> Result<impl IntoResponse, ApiError> {
    let dataset = DatasetDefinition::new(
        input.name,
        input.task_description,
        input.labels,
        input.dimensions,
    )
    .map_err(ApiError::bad_request)?;
    state
        .store
        .create_dataset(&dataset)
        .await
        .map_err(ApiError::internal)?;
    Ok((axum::http::StatusCode::CREATED, Json(dataset)))
}

pub async fn list_datasets(
    State(state): State<AppState>,
) -> Result<Json<Vec<DatasetDefinition>>, ApiError> {
    Ok(Json(
        state
            .store
            .list_datasets()
            .await
            .map_err(ApiError::internal)?,
    ))
}

pub async fn get_dataset(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DatasetDefinition>, ApiError> {
    state
        .store
        .get_dataset(id)
        .await
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("dataset"))
}

pub async fn dataset_cells(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<generation_core::domain::GenerationCell>>, ApiError> {
    let dataset = state
        .store
        .get_dataset(id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("dataset"))?;
    Ok(Json(expand_generation_cells(&dataset)))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    #[serde(default = "default_export_format")]
    format: String,
}

fn default_export_format() -> String {
    "jsonl".into()
}

pub async fn export_dataset(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    if state
        .store
        .get_dataset(id)
        .await
        .map_err(ApiError::internal)?
        .is_none()
    {
        return Err(ApiError::not_found("dataset"));
    }
    let rows = load_all_accepted_rows(&state, id).await?;
    let (content_type, extension, body) = match query.format.as_str() {
        "jsonl" => (
            "application/x-ndjson",
            "jsonl",
            to_jsonl(&rows).map_err(ApiError::internal)?,
        ),
        "csv" => (
            "text/csv; charset=utf-8",
            "csv",
            to_csv(&rows).map_err(ApiError::internal)?,
        ),
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported export format: {other}"
            )));
        }
    };
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=dataset-{id}.{extension}"))
            .map_err(ApiError::internal)?,
    );
    Ok(response)
}

async fn load_all_accepted_rows(
    state: &AppState,
    dataset_id: Uuid,
) -> Result<Vec<generation_core::domain::GeneratedRow>, ApiError> {
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let page = state
            .store
            .list_rows(RowQuery {
                dataset_id: Some(dataset_id),
                status: Some(ValidationStatus::Accepted),
                limit: 10_000,
                offset,
                ..RowQuery::default()
            })
            .await
            .map_err(ApiError::internal)?;
        let page_size = page.len();
        rows.extend(page);
        if page_size < 10_000 {
            break;
        }
        offset = offset.saturating_add(10_000);
    }
    Ok(rows)
}
