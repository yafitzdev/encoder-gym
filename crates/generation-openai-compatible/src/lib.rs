//! OpenAI-compatible implementation of the core generation-backend port.

use std::time::Duration;

use generation_core::{
    domain::{GenerationRequest, GenerationResult, UsageMetadata},
    parsing::parse_generated_candidates,
    ports::{BoxFuture, GenerationBackend, GenerationBackendError},
};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Clone)]
pub struct OpenAICompatibleBackend {
    client: Client,
    endpoint: Url,
    api_key: Option<String>,
    model: String,
}

impl OpenAICompatibleBackend {
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
            if body.contains_key(key) {
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
            let raw = response
                .text()
                .await
                .map_err(|error| GenerationBackendError::Request(error.to_string()))?;
            if !status.is_success() {
                return Err(GenerationBackendError::Request(format!(
                    "HTTP {status}: {}",
                    truncate(&raw, 2_000)
                )));
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

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
    use generation_core::domain::{GenerationCell, GenerationParameters, GenerationRequest};
    use generation_core::ports::GenerationBackend;
    use serde_json::json;
    use tokio::{net::TcpListener, sync::mpsc};

    use super::OpenAICompatibleBackend;

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
        };
        assert!(backend().request_body(&request).is_err());
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
}
