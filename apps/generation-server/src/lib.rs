mod api;
mod error;
mod state;
mod web;

use axum::{
    Router,
    routing::{get, post},
};
use state::AppState;
use synthetic_data_sqlite::SqliteStore;

pub fn app(store: SqliteStore) -> Router {
    let state = AppState::new(store);
    Router::new()
        .route("/", get(web::index))
        .route("/assets/{*path}", get(web::asset))
        .route("/api/health", get(api::health))
        .route(
            "/api/datasets",
            get(api::list_datasets).post(api::create_dataset),
        )
        .route("/api/datasets/{id}", get(api::get_dataset))
        .route("/api/datasets/{id}/cells", get(api::dataset_cells))
        .route("/api/datasets/{id}/export", get(api::export_dataset))
        .route("/api/plans", post(api::create_plan))
        .route("/api/plans/{id}", get(api::get_plan))
        .route("/api/plans/{id}/coverage", get(api::coverage))
        .route(
            "/api/backend",
            get(api::get_backend).put(api::configure_backend),
        )
        .route("/api/jobs", post(api::start_job))
        .route("/api/jobs/{id}", get(api::get_job))
        .route("/api/jobs/{id}/cancel", post(api::cancel_job))
        .route("/api/rows", get(api::list_rows))
        .with_state(state)
}
