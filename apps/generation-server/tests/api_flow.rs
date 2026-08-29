use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use synthetic_data_server::app;
use synthetic_data_sqlite::SqliteStore;
use tower::ServiceExt;

async fn test_app() -> (tempfile::TempDir, Router) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("api.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, app(store))
}

async fn json_request(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request_body = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(builder.body(request_body).expect("valid request"))
        .await
        .expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let value = serde_json::from_slice(&bytes).expect("JSON response");
    (status, value)
}

#[tokio::test]
async fn complete_fake_backend_api_workflow() {
    let (_directory, app) = test_app().await;
    let (status, dataset) = json_request(
        &app,
        Method::POST,
        "/api/datasets",
        Some(json!({
            "name": "support",
            "task_description": "Classify support requests",
            "labels": ["billing", "fraud"],
            "dimensions": [{"name": "style", "values": ["clean", "messy"]}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let dataset_id = dataset["id"].as_str().expect("dataset id");

    let (status, cells) = json_request(
        &app,
        Method::GET,
        &format!("/api/datasets/{dataset_id}/cells"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cells.as_array().expect("cells").len(), 4);
    let planned = cells
        .as_array()
        .expect("cells")
        .iter()
        .map(|cell| json!({"cell": cell, "target_count": 2}))
        .collect::<Vec<_>>();

    let (status, plan) = json_request(
        &app,
        Method::POST,
        "/api/plans",
        Some(json!({"dataset_id": dataset_id, "cells": planned})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let plan_id = plan["id"].as_str().expect("plan id");

    let (status, job) = json_request(
        &app,
        Method::POST,
        "/api/jobs",
        Some(json!({"plan_id": plan_id, "backend": "fake", "batch_size": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = job["id"].as_str().expect("job id");

    let completed = loop {
        let (_, current) =
            json_request(&app, Method::GET, &format!("/api/jobs/{job_id}"), None).await;
        if matches!(
            current["state"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            break current;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["accepted_rows"], 8);

    let (_, coverage) = json_request(
        &app,
        Method::GET,
        &format!("/api/plans/{plan_id}/coverage"),
        None,
    )
    .await;
    assert!(
        coverage
            .as_array()
            .expect("coverage")
            .iter()
            .all(|cell| cell["remaining"] == 0)
    );

    let (_, rows) = json_request(
        &app,
        Method::GET,
        &format!("/api/rows?dataset_id={dataset_id}&status=accepted&limit=100"),
        None,
    )
    .await;
    assert_eq!(rows.as_array().expect("rows").len(), 8);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/datasets/{dataset_id}/export?format=csv"))
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("export response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/csv; charset=utf-8"
    );
    let export = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("export body");
    assert_eq!(String::from_utf8_lossy(&export).lines().count(), 9);
}

#[tokio::test]
async fn serves_the_control_surface_and_feature_modules() {
    let (_directory, app) = test_app().await;
    for path in [
        "/",
        "/assets/main.js",
        "/assets/components/coverage-table.js",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK, "path {path}");
    }
}
