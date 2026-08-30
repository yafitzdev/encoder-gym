//! OpenAI-compatible implementation of the core generation-backend port.

use std::time::Duration;

use generation_core::{
    domain::{GenerationRequest, GenerationResult, UsageMetadata},
    parsing::parse_generated_candidates,
    ports::{BoxFuture, GenerationBackend, GenerationBackendError},
};
use reqwest::{Client, StatusCode, Url, header::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone)]
pub struct OpenAICompatibleBackend {
    client: Client,
    endpoint: Url,
    api_key: Option<String>,
    model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendProbe {
    pub backend: &'static str,
    pub base_url: String,
    pub model: String,
    pub available_model_count: usize,
}

impl OpenAICompatibleBackend {
    pub async fn probe(
        base_url: &str,
        api_key: Option<String>,
        model: &str,
    ) -> Result<BackendProbe, GenerationBackendError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(GenerationBackendError::Configuration(
                "model must not be empty".into(),
            ));
        }
        let base_url = base_url.trim().trim_end_matches('/');
        let endpoint = Url::parse(&format!("{base_url}/models")).map_err(|error| {
            GenerationBackendError::Configuration(format!("invalid base URL: {error}"))
        })?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| GenerationBackendError::Configuration(error.to_string()))?;
        let mut request = client.get(endpoint);
        if let Some(api_key) = api_key.filter(|key| !key.trim().is_empty()) {
            request = request.bearer_auth(api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| GenerationBackendError::Request(error.to_string()))?;
        let status = response.status();
        let retry_after_milliseconds = retry_after_milliseconds(response.headers());
        let raw = response
            .text()
            .await
            .map_err(|error| GenerationBackendError::Request(error.to_string()))?;
        if !status.is_success() {
            return Err(provider_failure(status, &raw, retry_after_milliseconds));
        }
        let models: ModelList = serde_json::from_str(&raw).map_err(|error| {
            GenerationBackendError::InvalidResponse(format!(
                "model-list response is not valid JSON: {error}"
            ))
        })?;
        if !models.data.iter().any(|available| available.id == model) {
            return Err(GenerationBackendError::Configuration(format!(
                "configured model is not available from the provider: {model}"
            )));
        }
        Ok(BackendProbe {
            backend: "openai-compatible",
            base_url: base_url.into(),
            model: model.into(),
            available_model_count: models.data.len(),
        })
    }

    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Result<Self, GenerationBackendError> {
        let model = model.into().trim().to_owned();
        if model.is_empty() {
            return Err(GenerationBackendError::Configuration(
                "model must not be empty".into(),
            ));
        }
        let base_url = base_url.trim().trim_end_matches('/');
        let endpoint = Url::parse(&format!("{base_url}/chat/completions")).map_err(|error| {
            GenerationBackendError::Configuration(format!("invalid base URL: {error}"))
        })?;
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| GenerationBackendError::Configuration(error.to_string()))?;
        let api_key = api_key.filter(|key| !key.trim().is_empty());

        Ok(Self {
            client,
            endpoint,
            api_key,
            model,
        })
    }

    fn request_body(&self, request: &GenerationRequest) -> Result<Value, GenerationBackendError> {
        let mut body = Map::from_iter([
            ("model".into(), Value::String(self.model.clone())),
            (
                "messages".into(),
                json!([
                    {"role": "system", "content": request.system_prompt},
                    {"role": "user", "content": request.user_prompt}
                ]),
            ),
            ("response_format".into(), json!({"type": "json_object"})),
        ]);

        insert_optional(&mut body, "temperature", request.parameters.temperature)?;
        insert_optional(&mut body, "max_tokens", request.parameters.max_tokens)?;
        insert_optional(&mut body, "seed", request.parameters.seed)?;
        for (key, value) in &request.parameters.extra {
            if [
                "model",
                "messages",
                "response_format",
                "temperature",
                "max_tokens",
                "seed",
            ]
            .contains(&key.as_str())
            {
                return Err(GenerationBackendError::Configuration(format!(
                    "generation parameter may not override reserved field: {key}"
                )));
            }
            body.insert(key.clone(), value.clone());
        }
        Ok(Value::Object(body))
    }

    fn normalize_response(raw: &str) -> Result<GenerationResult, GenerationBackendError> {
        let response: ChatCompletionResponse = serde_json::from_str(raw).map_err(|error| {
            GenerationBackendError::InvalidResponse(format!(
                "response envelope is not valid JSON: {error}"
            ))
        })?;
        let choice = response.choices.into_iter().next().ok_or_else(|| {
            GenerationBackendError::InvalidResponse("response contained no choices".into())
        })?;
        let rows = parse_generated_candidates(&choice.message.content)
            .map_err(|error| GenerationBackendError::InvalidResponse(error.to_string()))?;
        let usage = response.usage.map(|usage| UsageMetadata {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        });

        Ok(GenerationResult {
            rows,
            usage,
            backend_metadata: json!({
                "request_id": response.id,
                "response_model": response.model,
                "finish_reason": choice.finish_reason,
            }),
            errors: vec![],
        })
    }
}

impl GenerationBackend for OpenAICompatibleBackend {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            let body = self.request_body(&request)?;
            let mut builder = self.client.post(self.endpoint.clone()).json(&body);
            if let Some(api_key) = &self.api_key {
                builder = builder.bearer_auth(api_key);
            }
            let response = builder
                .send()
                .await
                .map_err(|error| GenerationBackendError::Request(error.to_string()))?;
            let status = response.status();
            let retry_after_milliseconds = retry_after_milliseconds(response.headers());
            let raw = response
                .text()
                .await
                .map_err(|error| GenerationBackendError::Request(error.to_string()))?;
            if !status.is_success() {
                return Err(provider_failure(status, &raw, retry_after_milliseconds));
            }
            Self::normalize_response(&raw)
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModelList {
    data: Vec<AvailableModel>,
}

#[derive(Debug, Deserialize)]
struct AvailableModel {
    id: String,
}

fn insert_optional<T: serde::Serialize>(
    body: &mut Map<String, Value>,
    key: &str,
    value: Option<T>,
) -> Result<(), GenerationBackendError> {
    if let Some(value) = value {
        body.insert(
            key.to_owned(),
            serde_json::to_value(value).map_err(|error| {
                GenerationBackendError::Configuration(format!(
                    "could not encode generation parameter {key}: {error}"
                ))
            })?,
        );
    }
    Ok(())
}

fn provider_error(status: StatusCode, raw: &str) -> String {
    let error = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let code = error
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .map(safe_identifier);
    let kind = error
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .map(safe_identifier);
    let parameter = error
        .as_ref()
        .and_then(|value| value.get("param"))
        .and_then(Value::as_str)
        .map(safe_identifier);
    let details = [
        code.map(|value| format!("code={value}")),
        kind.map(|value| format!("type={value}")),
        parameter.map(|value| format!("parameter={value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    };
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        format!("HTTP {status}{suffix}: provider rejected authentication")
    } else {
        format!("HTTP {status}{suffix}: provider returned a non-success response")
    }
}

fn provider_failure(
    status: StatusCode,
    raw: &str,
    retry_after_milliseconds: Option<u64>,
) -> GenerationBackendError {
    let message = provider_error(status, raw);
    if status == StatusCode::TOO_MANY_REQUESTS {
        GenerationBackendError::RateLimited {
            message,
            retry_after_milliseconds,
        }
    } else if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_EARLY
        || status.is_server_error()
    {
        GenerationBackendError::Request(message)
    } else {
        GenerationBackendError::Rejected(message)
    }
}

fn retry_after_milliseconds(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        .checked_mul(1_000)
}

fn safe_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "._-".contains(*character))
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
    use generation_core::domain::{GenerationCell, GenerationParameters, GenerationRequest};
    use generation_core::ports::{GenerationBackend, GenerationBackendError};
    use reqwest::StatusCode;
    use serde_json::json;
    use tokio::{net::TcpListener, sync::mpsc};

    use super::{OpenAICompatibleBackend, provider_error, provider_failure};

    fn backend() -> OpenAICompatibleBackend {
        OpenAICompatibleBackend::new("http://localhost:8080/v1/", None, "test-model")
            .expect("valid backend")
    }

    #[test]
    fn request_contains_normalized_prompts_and_parameters() {
        let body = backend()
            .request_body(&GenerationRequest {
                system_prompt: "system instructions".into(),
                user_prompt: "user instructions".into(),
                target: GenerationCell {
                    label: "billing".into(),
                    dimensions: BTreeMap::new(),
                },
                requested_count: 2,
                parameters: GenerationParameters {
                    temperature: Some(0.2),
                    max_tokens: Some(500),
                    seed: Some(7),
                    extra: BTreeMap::from([("top_p".into(), json!(0.9))]),
                },
                construction: None,
            })
            .expect("valid request");

        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "system instructions");
        let temperature = body["temperature"].as_f64().expect("numeric temperature");
        assert!((temperature - 0.2).abs() < 0.000_001);
        assert_eq!(body["top_p"], 0.9);
    }

    #[test]
    fn normalizes_chat_completion_content_and_usage() {
        let raw = json!({
            "id": "request-1",
            "model": "served-model",
            "choices": [{
                "message": {
                    "content": "{\"rows\":[{\"text\":\"charged twice\",\"label\":\"billing\",\"dimensions\":{}}]}"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 8,
                "total_tokens": 18
            }
        })
        .to_string();

        let result = OpenAICompatibleBackend::normalize_response(&raw).expect("valid response");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].label, "billing");
        assert_eq!(result.usage.expect("usage").total_tokens, Some(18));
        assert_eq!(result.backend_metadata["request_id"], "request-1");
    }

    #[test]
    fn normalizes_hybrid_rows_without_requiring_provider_owned_dimensions() {
        let raw = json!({
            "id": "request-hybrid",
            "model": "served-model",
            "choices": [{
                "message": {
                    "content": "{\"rows\":[{\"text\":\"charged twice\",\"fields\":{\"rationale\":\"duplicate charge\"}}]}"
                },
                "finish_reason": "stop"
            }]
        })
        .to_string();

        let result = OpenAICompatibleBackend::normalize_response(&raw).expect("hybrid response");
        assert_eq!(result.rows[0].text, "charged twice");
        assert_eq!(result.rows[0].label, "");
        assert!(result.rows[0].dimensions.is_empty());
        assert_eq!(result.rows[0].fields["rationale"], "duplicate charge");
    }

    #[test]
    fn rejects_reserved_parameter_overrides() {
        let request = GenerationRequest {
            system_prompt: "system".into(),
            user_prompt: "user".into(),
            target: GenerationCell {
                label: "billing".into(),
                dimensions: BTreeMap::new(),
            },
            requested_count: 1,
            parameters: GenerationParameters {
                extra: BTreeMap::from([("model".into(), json!("other"))]),
                ..GenerationParameters::default()
            },
            construction: None,
        };
        assert!(backend().request_body(&request).is_err());
    }

    #[test]
    fn provider_errors_keep_codes_but_never_persist_response_messages() {
        let error = provider_error(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{
                "error": {
                    "message": "Incorrect API key provided: sk-secret-fragment",
                    "type": "invalid_request_error",
                    "code": "invalid_api_key",
                    "param": null
                }
            }"#,
        );

        assert!(error.contains("401 Unauthorized"));
        assert!(error.contains("code=invalid_api_key"));
        assert!(error.contains("provider rejected authentication"));
        assert!(!error.contains("sk-secret-fragment"));
        assert!(!error.contains("Incorrect API key"));
    }

    #[test]
    fn classifies_retryable_and_permanent_http_failures() {
        let unauthorized = provider_failure(StatusCode::UNAUTHORIZED, "{}", None);
        assert!(!unauthorized.is_retryable());
        assert!(matches!(unauthorized, GenerationBackendError::Rejected(_)));

        let rate_limited = provider_failure(StatusCode::TOO_MANY_REQUESTS, "{}", Some(2_000));
        assert!(rate_limited.is_retryable());
        assert_eq!(rate_limited.retry_after_milliseconds(), Some(2_000));

        let unavailable = provider_failure(StatusCode::SERVICE_UNAVAILABLE, "{}", None);
        assert!(unavailable.is_retryable());
    }

    #[tokio::test]
    async fn sends_a_chat_completion_and_normalizes_the_http_response() {
        type RecordedRequest = (HeaderMap, serde_json::Value);

        async fn handler(
            State(sender): State<mpsc::UnboundedSender<RecordedRequest>>,
            headers: HeaderMap,
            Json(body): Json<serde_json::Value>,
        ) -> Json<serde_json::Value> {
            sender.send((headers, body)).expect("test receiver is open");
            Json(json!({
                "id": "request-over-http",
                "model": "served-model",
                "choices": [{
                    "message": {
                        "content": "{\"rows\":[{\"text\":\"charged twice\",\"label\":\"billing\",\"dimensions\":{}}]}"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {"total_tokens": 14}
            }))
        }

        let (sender, mut receiver) = mpsc::unbounded_channel::<RecordedRequest>();
        let app = Router::new()
            .route("/v1/chat/completions", post(handler))
            .with_state(sender);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local mock server");
        let address = listener.local_addr().expect("local address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server remains available");
        });
        let backend = OpenAICompatibleBackend::new(
            &format!("http://{address}/v1"),
            Some("test-secret".into()),
            "requested-model",
        )
        .expect("valid backend");
        let request = GenerationRequest {
            system_prompt: "system instructions".into(),
            user_prompt: "user instructions".into(),
            target: GenerationCell {
                label: "billing".into(),
                dimensions: BTreeMap::new(),
            },
            requested_count: 1,
            parameters: GenerationParameters {
                temperature: Some(0.3),
                ..GenerationParameters::default()
            },
            construction: None,
        };

        let result = backend
            .generate(request)
            .await
            .expect("HTTP generation succeeds");
        let (headers, body) = receiver.recv().await.expect("request was recorded");

        assert_eq!(
            headers
                .get("authorization")
                .expect("authorization header")
                .to_str()
                .expect("text header"),
            "Bearer test-secret"
        );
        assert_eq!(body["model"], "requested-model");
        assert_eq!(body["messages"][1]["content"], "user instructions");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].text, "charged twice");
        assert_eq!(result.backend_metadata["request_id"], "request-over-http");
        server.abort();
    }

    #[tokio::test]
    async fn probe_authenticates_and_requires_the_configured_model() {
        async fn handler(headers: HeaderMap) -> Json<serde_json::Value> {
            assert_eq!(
                headers
                    .get("authorization")
                    .expect("authorization header")
                    .to_str()
                    .expect("text header"),
                "Bearer test-secret"
            );
            Json(json!({
                "object": "list",
                "data": [{"id": "requested-model"}, {"id": "other-model"}]
            }))
        }

        let app = Router::new().route("/v1/models", axum::routing::get(handler));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local mock server");
        let address = listener.local_addr().expect("local address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server remains available");
        });

        let probe = OpenAICompatibleBackend::probe(
            &format!("http://{address}/v1/"),
            Some("test-secret".into()),
            "requested-model",
        )
        .await
        .expect("probe succeeds");
        assert_eq!(probe.model, "requested-model");
        assert_eq!(probe.available_model_count, 2);
        server.abort();
    }
}
