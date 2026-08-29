use axum::{
    Json,
    extract::{Query, State},
};
use generation_core::{
    domain::{GeneratedRow, ValidationStatus},
    ports::{RowQuery, RowStore},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct RowsQuery {
    dataset_id: Option<Uuid>,
    job_id: Option<Uuid>,
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

const fn default_limit() -> u32 {
    100
}

pub async fn list_rows(
    State(state): State<AppState>,
    Query(query): Query<RowsQuery>,
) -> Result<Json<Vec<GeneratedRow>>, ApiError> {
    let status = match query.status.as_deref() {
        None => None,
        Some("accepted") => Some(ValidationStatus::Accepted),
        Some("rejected") => Some(ValidationStatus::Rejected),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown row status: {other}"
            )));
        }
    };
    Ok(Json(
        state
            .store
            .list_rows(RowQuery {
                dataset_id: query.dataset_id,
                job_id: query.job_id,
                status,
                limit: query.limit,
                offset: query.offset,
            })
            .await
            .map_err(ApiError::internal)?,
    ))
}
