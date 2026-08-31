//! Bounded chat-completions HTTP transport and redacted status handling.

use chrono::{DateTime, Utc};
use dataset_quality_core::ports::{QualityEvaluationError, QualityEvaluationErrorKind};
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER},
};
use serde_json::{Map, Value};

const MAX_METADATA_IDENTIFIER_CHARACTERS: usize = 128;
const MAX_RETRY_AFTER_MILLIS: u64 = 30_000;

#[derive(Clone)]
pub(crate) struct OpenAICompatibleTransport {
    pub(crate) client: Client,
    pub(crate) endpoint: Url,
    pub(crate) api_key: Option<String>,
    pub(crate) now: fn() -> DateTime<Utc>,
}

impl OpenAICompatibleTransport {
    pub(crate) async fn send(&self, body: Vec<u8>) -> Result<Response, QualityEvaluationError> {
        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        request
            .send()
            .await
            .map_err(|error| classify_transport_error(&error))
    }

    pub(crate) fn current_time(&self) -> DateTime<Utc> {
        (self.now)()
    }
}

pub(crate) enum ReadResponseError {
    TooLarge,
    Transport,
}

pub(crate) async fn read_bounded_response(
    response: &mut Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ReadResponseError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(ReadResponseError::TooLarge);
    }
    let mut raw = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| ReadResponseError::Transport)?;
        let Some(chunk) = chunk else {
            return Ok(raw);
        };
        if raw.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(ReadResponseError::TooLarge);
        }
        raw.extend_from_slice(&chunk);
    }
}

fn classify_transport_error(error: &reqwest::Error) -> QualityEvaluationError {
    let message = if error.is_timeout() {
        "provider transport timed out"
    } else if error.is_connect() {
        "provider transport could not connect"
    } else if error.is_body() || error.is_decode() {
        "provider transport body failed"
    } else {
        "provider transport request failed"
    };
    transport_error(message)
}

pub(crate) fn provider_failure(
    status: StatusCode,
    raw: &[u8],
    headers: &HeaderMap,
    api_key: Option<&str>,
    now: DateTime<Utc>,
) -> QualityEvaluationError {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            QualityEvaluationErrorKind::Authentication
        }
        StatusCode::TOO_MANY_REQUESTS => QualityEvaluationErrorKind::RateLimit,
        status
            if status.is_server_error()
                || matches!(status, StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_EARLY) =>
        {
            QualityEvaluationErrorKind::Provider
        }
        _ => QualityEvaluationErrorKind::Configuration,
    };
    let purpose = match kind {
        QualityEvaluationErrorKind::Authentication => "provider rejected authentication",
        QualityEvaluationErrorKind::RateLimit => "provider rate limited the request",
        QualityEvaluationErrorKind::Provider => "provider could not complete the request",
        _ => "provider rejected the evaluator request configuration",
    };
    let details = safe_provider_error_details(raw, api_key);
    let mut message = format!("HTTP {}: {purpose}", status.as_u16());
    if !details.is_empty() {
        message.push_str(" (");
        message.push_str(&details.join(", "));
        message.push(')');
    }
    let mut error = QualityEvaluationError::new(kind, message);
    if let Some(retry_after_millis) = retry_after_millis(headers, now) {
        error = error.with_retry_after_millis(retry_after_millis);
    }
    error
}

fn safe_provider_error_details(raw: &[u8], api_key: Option<&str>) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<Value>(raw) else {
        return Vec::new();
    };
    let Some(error) = value.get("error") else {
        return Vec::new();
    };
    [("code", "code"), ("type", "type"), ("param", "parameter")]
        .into_iter()
        .filter_map(|(field, rendered)| {
            error
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| safe_identifier(value, api_key))
                .map(|value| format!("{rendered}={value}"))
        })
        .collect()
}

fn retry_after_millis(headers: &HeaderMap, now: DateTime<Utc>) -> Option<u64> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.saturating_mul(1_000).min(MAX_RETRY_AFTER_MILLIS));
    }
    let retry_at = DateTime::parse_from_str(value, "%a, %d %b %Y %H:%M:%S GMT")
        .or_else(|_| DateTime::parse_from_rfc2822(value))
        .ok()?
        .with_timezone(&Utc);
    if retry_at <= now {
        return Some(0);
    }
    u64::try_from(retry_at.signed_duration_since(now).num_milliseconds())
        .ok()
        .map(|delay| delay.min(MAX_RETRY_AFTER_MILLIS))
}

pub(crate) fn safe_metadata(
    request_id: Option<&str>,
    response_model: Option<&str>,
    finish_reason: Option<&str>,
    api_key: Option<&str>,
) -> Value {
    let mut metadata = Map::new();
    if let Some(value) = request_id.and_then(|value| safe_identifier(value, api_key)) {
        metadata.insert("request_id".into(), Value::String(value));
    }
    if let Some(value) = response_model.and_then(|value| safe_identifier(value, api_key)) {
        metadata.insert("response_model".into(), Value::String(value));
    }
    if let Some(value) = finish_reason.and_then(|value| safe_identifier(value, api_key)) {
        metadata.insert("finish_reason".into(), Value::String(value));
    }
    Value::Object(metadata)
}

fn safe_identifier(value: &str, api_key: Option<&str>) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_METADATA_IDENTIFIER_CHARACTERS
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:/-".contains(character))
        || value.to_ascii_lowercase().contains("sk-")
        || api_key.is_some_and(|secret| !secret.is_empty() && value.contains(secret))
    {
        return None;
    }
    Some(value.to_owned())
}

pub(crate) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub(crate) fn transport_error(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Transport, message)
}
