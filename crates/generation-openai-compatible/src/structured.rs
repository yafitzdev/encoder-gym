use generation_core::{
    domain::UsageMetadata,
    ports::{BoxFuture, GenerationBackendError},
    structured::{
        StructuredGenerationBackend, StructuredGenerationRequest, StructuredGenerationResult,
    },
};
use serde_json::json;

use crate::{
    ChatCompletionResponse, OpenAICompatibleBackend, provider_failure, retry_after_milliseconds,
};

impl StructuredGenerationBackend for OpenAICompatibleBackend {
    fn model(&self) -> &str {
        &self.model
    }

    fn generate_structured(
        &self,
        request: StructuredGenerationRequest,
    ) -> BoxFuture<'_, Result<StructuredGenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            request.validate()?;
            let body = json!({
                "model":self.model,
                "messages":[{"role":"system","content":request.system_prompt},{"role":"user","content":request.user_prompt}],
                "max_tokens":request.maximum_output_tokens,
                "response_format":{"type":"json_object"},
            });
            let mut builder = self.client.post(self.endpoint.clone()).json(&body);
            if let Some(key) = &self.api_key {
                builder = builder.bearer_auth(key);
            }
            // No hidden transport retry: one invocation consumes one host reservation.
            let mut response = builder.send().await.map_err(|_| {
                GenerationBackendError::Request("Data generation connection failed".into())
            })?;
            let status = response.status();
            let retry_after = retry_after_milliseconds(response.headers());
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                GenerationBackendError::Request("Data generation response interrupted".into())
            })? {
                if bytes.len() + chunk.len() > 2_097_152 {
                    return Err(GenerationBackendError::InvalidResponse(
                        "Data generation response exceeds 2 MiB".into(),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            if !status.is_success() {
                return Err(provider_failure(
                    status,
                    &String::from_utf8_lossy(&bytes),
                    retry_after,
                ));
            }
            let envelope: ChatCompletionResponse =
                serde_json::from_slice(&bytes).map_err(|_| {
                    GenerationBackendError::InvalidResponse(
                        "Data generation envelope is invalid".into(),
                    )
                })?;
            let mut choices = envelope.choices;
            if choices.len() != 1 {
                return Err(GenerationBackendError::InvalidResponse(
                    "Expected one data generation response".into(),
                ));
            }
            let choice = choices.pop().expect("one choice");
            Ok(StructuredGenerationResult {
                content: choice.message.content,
                usage: envelope.usage.map(|usage| UsageMetadata {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                }),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use tokio::{net::TcpListener, sync::mpsc};

    fn request() -> StructuredGenerationRequest {
        StructuredGenerationRequest {
            system_prompt: "Task-owned schema".into(),
            user_prompt: "Generate a native example".into(),
            maximum_output_tokens: 128,
        }
    }

    #[tokio::test]
    async fn selected_generator_receives_exact_request_and_preserves_invalid_output_usage() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new().route("/v1/chat/completions", post(|State(sender):State<mpsc::UnboundedSender<(HeaderMap,serde_json::Value)>>, headers:HeaderMap, Json(body):Json<serde_json::Value>| async move {
            sender.send((headers,body)).unwrap();
            Json(json!({"choices":[{"message":{"content":"not valid task JSON"}}],"usage":{"prompt_tokens":18,"completion_tokens":4}}))
        })).with_state(sender);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let backend =
            OpenAICompatibleBackend::new(&endpoint, Some("fixture-key".into()), "generator-pro")
                .unwrap();
        let result = backend.generate_structured(request()).await.unwrap();
        let (headers, body) = receiver.recv().await.unwrap();
        assert_eq!(headers["authorization"], "Bearer fixture-key");
        assert_eq!(body["model"], "generator-pro");
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(result.content, "not valid task JSON");
        assert_eq!(result.usage.unwrap().input_tokens, Some(18));
        assert!(receiver.try_recv().is_err());
        server.abort();
    }

    #[tokio::test]
    async fn rejection_does_not_retry_or_echo_provider_body() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let sender = sender.clone();
                async move {
                    sender.send(()).unwrap();
                    (StatusCode::TOO_MANY_REQUESTS, "private prompt and secret")
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend = OpenAICompatibleBackend::new(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            None,
            "generator",
        )
        .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let error = backend.generate_structured(request()).await.unwrap_err();
        assert!(error.is_retryable());
        assert!(!error.to_string().contains("private"));
        receiver.recv().await.unwrap();
        assert!(receiver.try_recv().is_err());
        server.abort();
    }
}
