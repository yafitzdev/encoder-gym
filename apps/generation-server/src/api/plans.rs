use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use generation_core::{
    coverage::{CellCoverage, calculate_coverage},
    domain::{GenerationPlan, PlannedCell},
    planning::{equal_target_plan, explicit_target_plan},
    ports::{DatasetStore, PlanStore, RowStore},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct CreatePlanInput {
    dataset_id: Uuid,
    target_per_cell: Option<u32>,
    cells: Option<Vec<PlannedCell>>,
}

pub async fn create_plan(
    State(state): State<AppState>,
    Json(input): Json<CreatePlanInput>,
) -> Result<impl IntoResponse, ApiError> {
    let dataset = state
        .store
        .get_dataset(input.dataset_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("dataset"))?;
    let plan = match (input.target_per_cell, input.cells) {
        (Some(target), None) => equal_target_plan(&dataset, target),
        (None, Some(cells)) => explicit_target_plan(&dataset, cells),
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of target_per_cell or cells",
            ));
        }
    }
    .map_err(ApiError::bad_request)?;
    state
        .store
        .create_plan(&plan)
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(plan)))
}

pub async fn get_plan(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<GenerationPlan>, ApiError> {
    state
        .store
        .get_plan(id)
        .await
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("plan"))
}

pub async fn coverage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CellCoverage>>, ApiError> {
    let plan = state
        .store
        .get_plan(id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("plan"))?;
    let counts = state
        .store
        .dataset_cell_counts(plan.dataset_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(calculate_coverage(&plan, &counts)))
}
