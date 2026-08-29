use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

pub async fn asset(Path(path): Path<String>) -> Response {
    let (content_type, body) = match path.as_str() {
        "styles.css" => (
            "text/css; charset=utf-8",
            include_str!("../static/styles.css"),
        ),
        "api.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/api.js"),
        ),
        "main.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/main.js"),
        ),
        "components/dataset-form.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/dataset-form.js"),
        ),
        "components/dimension-editor.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/dimension-editor.js"),
        ),
        "components/backend-config.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/backend-config.js"),
        ),
        "components/generation-controls.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/generation-controls.js"),
        ),
        "components/generation-progress.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/generation-progress.js"),
        ),
        "components/coverage-table.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/coverage-table.js"),
        ),
        "components/generated-rows-table.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../static/components/generated-rows-table.js"),
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}
