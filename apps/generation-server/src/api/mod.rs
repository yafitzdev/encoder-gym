mod backend;
mod datasets;
mod jobs;
mod plans;
mod rows;

use axum::Json;

pub use backend::{configure_backend, get_backend};
pub use datasets::{create_dataset, dataset_cells, export_dataset, get_dataset, list_datasets};
pub use jobs::{cancel_job, get_job, start_job};
pub use plans::{coverage, create_plan, get_plan};
pub use rows::list_rows;

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}
