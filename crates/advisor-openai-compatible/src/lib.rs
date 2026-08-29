//! OpenAI-compatible transport for bounded workflow advisory requests.

use std::time::Duration;

use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::json;
use workflow_core::advisor::{
    AdvisorError, AdvisorPrompt, AdvisorTransportResult, AdvisorUsage, AnalysisAdvisor, BoxFuture,
};

#[derive(Clone)]
pub struct OpenAICompatibleAdvisor {
    client: Client,
    endpoint: Url,
    api_key: Option<String>,
}

impl OpenAICompatibleAdvisor {
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self, AdvisorError> {
        let endpoint = Url::parse(&format!(
            "{}/chat/completions",
            base_url.trim().trim_end_matches('/')
        ))
        .map_err(|error| AdvisorError::Request(format!("invalid base URL: {error}")))?;
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| AdvisorError::Request(error.to_string()))?;
        Ok(Self {
            client,
            endpoint,
            api_key: api_key.filter(|value| !value.trim().is_empty()),
        })
    }
}

impl AnalysisAdvisor for OpenAICompatibleAdvisor {
    fn backend_name(&self) -> &str {
        "openai-compatible"
    }

    fn generate(
        &self,
        prompt: AdvisorPrompt,
    ) -> BoxFuture<'_, Result<AdvisorTransportResult, AdvisorError>> {
        Box::pin(async move {
            let body = json!({
                "model": prompt.model,
                "messages": [
                    {"role": "system", "content": prompt.system_prompt},
                    {"role": "user", "content": prompt.user_prompt}
                ],
                "response_format": {"type": "json_object"},
                "temperature": prompt.temperature,
                "max_tokens": prompt.maximum_output_tokens,
            });
            let mut request = self.client.post(self.endpoint.clone()).json(&body);
            if let Some(api_key) = &self.api_key {
                request = request.bearer_auth(api_key);
            }
            let response = request
                .send()
                .await
                .map_err(|error| AdvisorError::Request(error.to_string()))?;
            let status = response.status();
            let raw = response
                .text()
                .await
                .map_err(|error| AdvisorError::Request(error.to_string()))?;
            if !status.is_success() {
                return Err(AdvisorError::Request(format!(
                    "HTTP {status}: {}",
                    raw.chars().take(2_000).collect::<String>()
                )));
            }
            let response: ChatCompletionResponse = serde_json::from_str(&raw)
                .map_err(|error| AdvisorError::InvalidResponse(error.to_string()))?;
            let choice =
                response.choices.into_iter().next().ok_or_else(|| {
                    AdvisorError::InvalidResponse("response has no choices".into())
                })?;
            Ok(AdvisorTransportResult {
                content: choice.message.content,
                usage: response
                    .usage
                    .map_or_else(AdvisorUsage::default, |usage| AdvisorUsage {
                        input_tokens: usage.prompt_tokens,
                        output_tokens: usage.completion_tokens,
                        total_tokens: usage.total_tokens,
                    }),
                metadata: json!({
                    "request_id": response.id,
                    "response_model": response.model,
                    "finish_reason": choice.finish_reason,
                }),
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}
