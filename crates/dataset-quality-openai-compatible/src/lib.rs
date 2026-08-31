//! OpenAI-compatible adapter for blind dataset-quality assessment.
//!
//! Prompt policy lives in a dedicated, structurally blind component. This module owns non-secret
//! configuration, chat-completions transport, strict response normalization,
//! and the project-owned [`QualityEvaluator`] port implementation.

mod config;
mod normalization;
mod prompt;
mod strict_json;
mod transport;

use std::{fmt, time::Duration};

use chrono::Utc;

use dataset_quality_core::{
    assessment::{BlindEvaluatorRequest, EvaluatorExecutionLocation, EvaluatorIdentity},
    ports::{
        BoxFuture, EvaluatorBatchOutput, QualityEvaluationError, QualityEvaluationErrorKind,
        QualityEvaluator,
    },
};
use prompt::{QualityEvaluatorPrompt, build_production_prompt, production_prompt_identity};
use reqwest::Client;
use serde::Serialize;

use transport::{
    OpenAICompatibleTransport, ReadResponseError, provider_failure, read_bounded_response,
    transport_error,
};

use config::{BACKEND_NAME, ResolvedConfiguration};
pub use config::{
    DEFAULT_MAXIMUM_CONTENT_BYTES, DEFAULT_MAXIMUM_OUTPUT_TOKENS_PER_REQUEST,
    DEFAULT_MAXIMUM_RESPONSE_BYTES, DEFAULT_TIMEOUT_MILLIS, NonSecretEvaluatorConfiguration,
    OpenAICompatibleEvaluatorConfig, TokenPricing,
};
use normalization::normalize_success;

/// Strict OpenAI-compatible quality evaluator.
#[derive(Clone)]
pub struct OpenAICompatibleQualityEvaluator {
    transport: OpenAICompatibleTransport,
    configuration: NonSecretEvaluatorConfiguration,
    identity: EvaluatorIdentity,
}

impl fmt::Debug for OpenAICompatibleQualityEvaluator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAICompatibleQualityEvaluator")
            .field("configuration", &self.configuration)
            .field("identity", &self.identity)
            .field("api_key_configured", &self.transport.api_key.is_some())
            .finish_non_exhaustive()
    }
}

impl OpenAICompatibleQualityEvaluator {
    pub fn new(
        config: OpenAICompatibleEvaluatorConfig,
        api_key: Option<String>,
    ) -> Result<Self, QualityEvaluationError> {
        let api_key = api_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let prompt_identity = production_prompt_identity()?;
        let resolved = ResolvedConfiguration::resolve(
            config,
            &prompt_identity.version,
            prompt_identity.fingerprint,
            api_key.as_deref(),
        )?;
        let client = Client::builder()
            .timeout(Duration::from_millis(resolved.public.timeout_millis))
            .build()
            .map_err(|_| configuration_error("could not construct evaluator HTTP client"))?;
        let configuration_fingerprint = resolved.public.reproduce_fingerprint()?;
        let identity = EvaluatorIdentity::new(
            BACKEND_NAME,
            resolved.public.model.clone(),
            resolved.public.protocol_version.clone(),
            configuration_fingerprint,
            resolved.independence,
            EvaluatorExecutionLocation::ExternalService,
        )
        .map_err(|_| configuration_error("evaluator identity is invalid"))?;
        Ok(Self {
            transport: OpenAICompatibleTransport {
                client,
                endpoint: resolved.endpoint,
                api_key,
                now: Utc::now,
            },
            configuration: resolved.public,
            identity,
        })
    }

    pub fn non_secret_configuration(&self) -> &NonSecretEvaluatorConfiguration {
        &self.configuration
    }

    async fn evaluate_inner(
        &self,
        request: BlindEvaluatorRequest,
    ) -> Result<EvaluatorBatchOutput, QualityEvaluationError> {
        self.validate_request_binding(&request)?;
        let prompt = build_production_prompt(&request)?;
        if prompt.version != self.configuration.prompt_version {
            return Err(configuration_error(
                "prompt builder output does not match its pinned version",
            ));
        }
        let body = self.request_body(&request, &prompt)?;
        self.validate_outbound_budget(&request, &body)?;
        let mut response = self.transport.send(body).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let raw =
            match read_bounded_response(&mut response, self.configuration.maximum_response_bytes)
                .await
            {
                Ok(raw) => raw,
                Err(ReadResponseError::TooLarge) if !status.is_success() => {
                    return Err(provider_failure(
                        status,
                        &[],
                        &headers,
                        self.transport.api_key.as_deref(),
                        self.transport.current_time(),
                    ));
                }
                Err(ReadResponseError::TooLarge) => {
                    return Err(invalid_response(
                        "provider response exceeded the configured envelope size limit",
                    ));
                }
                Err(ReadResponseError::Transport) => {
                    return Err(transport_error("could not read provider response body"));
                }
            };
        if !status.is_success() {
            return Err(provider_failure(
                status,
                &raw,
                &headers,
                self.transport.api_key.as_deref(),
                self.transport.current_time(),
            ));
        }
        normalize_success(
            &request,
            &raw,
            self.configuration.maximum_content_bytes,
            self.configuration.token_pricing,
            self.transport.api_key.as_deref(),
        )
    }

    fn validate_request_binding(
        &self,
        request: &BlindEvaluatorRequest,
    ) -> Result<(), QualityEvaluationError> {
        if request.evaluator_identity_fingerprint != self.identity.fingerprint
            || request.evaluator_protocol_version != self.identity.protocol_version
            || request.budget.maximum_input_tokens == 0
            || request.budget.maximum_output_tokens == 0
            || request.budget.maximum_total_tokens == 0
            || request.budget.maximum_total_tokens
                > request
                    .budget
                    .maximum_input_tokens
                    .saturating_add(request.budget.maximum_output_tokens)
            || (request.budget.maximum_cost_microusd.is_some()
                && self.configuration.token_pricing.is_none())
        {
            return Err(configuration_error(
                "evaluator request does not bind this evaluator identity or a valid finite budget",
            ));
        }
        Ok(())
    }

    fn request_body(
        &self,
        request: &BlindEvaluatorRequest,
        prompt: &QualityEvaluatorPrompt,
    ) -> Result<Vec<u8>, QualityEvaluationError> {
        let maximum_output_tokens = self.maximum_output_tokens(request);
        let messages = [
            ChatMessage {
                role: "system",
                content: &prompt.system_message,
            },
            ChatMessage {
                role: "user",
                content: &prompt.user_message,
            },
        ];
        serde_json::to_vec(&ChatCompletionRequest {
            model: &self.configuration.model,
            messages: &messages,
            response_format: ResponseFormat {
                kind: "json_object",
            },
            temperature: f64::from(self.configuration.temperature_thousandths) / 1_000.0,
            max_tokens: maximum_output_tokens,
            seed: self.configuration.seed,
        })
        .map_err(|_| configuration_error("could not encode evaluator HTTP request"))
    }

    fn validate_outbound_budget(
        &self,
        request: &BlindEvaluatorRequest,
        serialized_body: &[u8],
    ) -> Result<(), QualityEvaluationError> {
        // A one-token-per-transmitted-byte ceiling is deliberately stricter
        // than ordinary byte-fallback tokenizers and includes JSON/message
        // framing. It is evaluated over the exact bytes sent by transport.
        let maximum_input_tokens = u64::try_from(serialized_body.len())
            .map_err(|_| configuration_error("evaluator request body is too large"))?;
        let maximum_output_tokens = self.maximum_output_tokens(request);
        let maximum_total_tokens = maximum_input_tokens
            .checked_add(maximum_output_tokens)
            .ok_or_else(|| configuration_error("evaluator request token bound overflowed"))?;
        if maximum_input_tokens > request.budget.maximum_input_tokens
            || maximum_total_tokens > request.budget.maximum_total_tokens
        {
            return Err(configuration_error(
                "serialized evaluator request cannot fit its finite token budget",
            ));
        }

        if let Some(maximum_cost_microusd) = request.budget.maximum_cost_microusd {
            let pricing = self.configuration.token_pricing.ok_or_else(|| {
                configuration_error("finite evaluator cost budget requires declared token pricing")
            })?;
            let maximum_cost = pricing
                .cost_microusd(maximum_input_tokens, maximum_output_tokens)
                .ok_or_else(|| configuration_error("evaluator request cost bound overflowed"))?;
            if maximum_cost > maximum_cost_microusd {
                return Err(configuration_error(
                    "serialized evaluator request cannot fit its finite cost budget",
                ));
            }
        }
        Ok(())
    }

    fn maximum_output_tokens(&self, request: &BlindEvaluatorRequest) -> u64 {
        request
            .budget
            .maximum_output_tokens
            .min(self.configuration.maximum_output_tokens_per_request)
    }
}

impl QualityEvaluator for OpenAICompatibleQualityEvaluator {
    fn identity(&self) -> EvaluatorIdentity {
        self.identity.clone()
    }

    fn evaluate(
        &self,
        request: BlindEvaluatorRequest,
    ) -> BoxFuture<'_, Result<EvaluatorBatchOutput, QualityEvaluationError>> {
        Box::pin(async move { self.evaluate_inner(request).await })
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage<'a>],
    response_format: ResponseFormat<'a>,
    temperature: f64,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

fn configuration_error(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Configuration, message)
}

fn invalid_response(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::InvalidResponse, message)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap as AxumHeaderMap, StatusCode, header::RETRY_AFTER},
        response::IntoResponse,
        routing::post,
    };
    use chrono::{DateTime, TimeZone, Utc};
    use dataset_core::domain::{SourceProvenance, SourceRow};
    use dataset_quality_core::{
        assessment::{
            BlindEvaluatorRequest, EvaluatorGuidance, EvaluatorIndependence,
            EvaluatorRequestBudget, RowAssessmentDraft,
        },
        policy::{
            AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, GuidanceReferences},
        ports::{QualityEvaluationErrorKind, QualityEvaluator},
    };
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
    use uuid::Uuid;

    use super::{
        DEFAULT_MAXIMUM_OUTPUT_TOKENS_PER_REQUEST, OpenAICompatibleEvaluatorConfig,
        OpenAICompatibleQualityEvaluator, TokenPricing, build_production_prompt,
    };

    const PROTOCOL_VERSION: &str = "quality-evaluator-v1";
    const API_KEY: &str = "sk-test-secret-never-persist";

    fn fixed_retry_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0)
            .single()
            .expect("fixed retry time")
    }

    type RecordedRequest = (AxumHeaderMap, Value);

    #[derive(Clone)]
    struct MockState {
        responses: Arc<Mutex<VecDeque<MockResponse>>>,
        sender: mpsc::UnboundedSender<RecordedRequest>,
    }

    #[derive(Clone)]
    struct MockResponse {
        status: StatusCode,
        headers: AxumHeaderMap,
        body: Value,
    }

    async fn mock_handler(
        State(state): State<MockState>,
        headers: AxumHeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let _ = state.sender.send((headers, body));
        let response = state
            .responses
            .lock()
            .expect("mock response queue is not poisoned")
            .pop_front()
            .expect("a mock response was configured");
        (response.status, response.headers, Json(response.body))
    }

    async fn spawn_mock(
        responses: Vec<MockResponse>,
        api_key: Option<&str>,
    ) -> (
        OpenAICompatibleQualityEvaluator,
        mpsc::UnboundedReceiver<RecordedRequest>,
        JoinHandle<()>,
    ) {
        spawn_mock_with_pricing(responses, api_key, None).await
    }

    async fn spawn_mock_with_pricing(
        responses: Vec<MockResponse>,
        api_key: Option<&str>,
        pricing: Option<TokenPricing>,
    ) -> (
        OpenAICompatibleQualityEvaluator,
        mpsc::UnboundedReceiver<RecordedRequest>,
        JoinHandle<()>,
    ) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let state = MockState {
            responses: Arc::new(Mutex::new(responses.into())),
            sender,
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback mock evaluator");
        let address = listener.local_addr().expect("mock address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock evaluator remains available");
        });
        let mut config = OpenAICompatibleEvaluatorConfig::new(
            format!("http://{address}/v1"),
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        config.token_pricing = pricing;
        let evaluator = OpenAICompatibleQualityEvaluator::new(config, api_key.map(str::to_owned))
            .expect("valid evaluator");
        (evaluator, receiver, server)
    }

    fn fixture_request(evaluator: &OpenAICompatibleQualityEvaluator) -> BlindEvaluatorRequest {
        fixture_request_with_budget(
            evaluator,
            None,
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 100,
                maximum_total_tokens: 10_100,
                maximum_cost_microusd: None,
            },
        )
    }

    fn fixture_request_with_budget(
        evaluator: &OpenAICompatibleQualityEvaluator,
        plan_maximum_cost_microusd: Option<u64>,
        budget: EvaluatorRequestBudget,
    ) -> BlindEvaluatorRequest {
        let dataset = DatasetDefinition::with_identity(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").expect("dataset ID"),
            "support",
            "Classify support requests",
            vec!["fraud".into(), "billing".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["hard".into(), "easy".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("fixture time"),
        )
        .expect("dataset");
        let row = SourceRow {
            id: Uuid::parse_str("20000000-0000-4000-8000-000000000001").expect("row ID"),
            dataset_id: dataset.id,
            text: "Why did the service charge me twice?".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Generated {
                generation_job_id: Uuid::parse_str("30000000-0000-4000-8000-000000000001")
                    .expect("generation job ID"),
                backend: "generator-private-backend".into(),
                model: "generator-private-model".into(),
                construction_plan_fingerprint: Some("sha256:private-construction".into()),
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("fixture time"),
        };
        let policy = QualityPreset::Fast
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::ExternalCandidateText,
                evaluate_authenticity: false,
                maximum_cost_microusd: plan_maximum_cost_microusd,
            })
            .expect("quality policy");
        let guidance = EvaluatorGuidance::default();
        let guidance_fingerprint = guidance
            .reproduce_fingerprint()
            .expect("guidance fingerprint");
        let plan = AuditPlan::with_identity(
            Uuid::parse_str("40000000-0000-4000-8000-000000000001").expect("plan ID"),
            &dataset,
            policy,
            GuidanceReferences::default(),
            guidance_fingerprint.clone(),
            PROTOCOL_VERSION,
            vec![row.clone()],
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("fixture time"),
        )
        .expect("audit plan");
        BlindEvaluatorRequest::create(
            &plan,
            Uuid::parse_str("50000000-0000-4000-8000-000000000001").expect("request ID"),
            Uuid::parse_str("60000000-0000-4000-8000-000000000001").expect("run ID"),
            Uuid::parse_str("70000000-0000-4000-8000-000000000001").expect("attempt ID"),
            1,
            1,
            &evaluator.identity(),
            vec![row],
            guidance,
            guidance_fingerprint,
            budget,
        )
        .expect("blind evaluator request")
    }

    fn valid_draft(request: &BlindEvaluatorRequest) -> RowAssessmentDraft {
        let row = &request.rows[0];
        RowAssessmentDraft {
            source_row_id: row.source_row_id,
            source_row_fingerprint: row.source_row_fingerprint.clone(),
            label_scores: BTreeMap::from([
                ("billing".into(), BasisPoints::new(9_000).expect("score")),
                ("fraud".into(), BasisPoints::new(1_000).expect("score")),
            ]),
            dimension_scores: BTreeMap::from([(
                "difficulty".into(),
                BTreeMap::from([
                    ("easy".into(), BasisPoints::new(9_000).expect("score")),
                    ("hard".into(), BasisPoints::new(2_000).expect("score")),
                ]),
            )]),
            authenticity_score: None,
            label_leakage_risk: BasisPoints::new(100).expect("score"),
            shortcut_risk: BasisPoints::new(200).expect("score"),
            confidence: BasisPoints::new(9_000).expect("score"),
            issue_codes: Vec::new(),
            rationale: "The candidate clearly describes a duplicate charge.".into(),
        }
    }

    fn success_response(content: String, input: u64, output: u64) -> Value {
        json!({
            "id": "quality-response-1",
            "object": "chat.completion",
            "created": 1_767_225_600_u64,
            "model": "served-quality-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
                "logprobs": null
            }],
            "usage": {
                "prompt_tokens": input,
                "completion_tokens": output,
                "total_tokens": input + output
            },
            "system_fingerprint": "system-fingerprint",
            "service_tier": "default"
        })
    }

    fn mock_success(body: Value) -> MockResponse {
        MockResponse {
            status: StatusCode::OK,
            headers: AxumHeaderMap::new(),
            body,
        }
    }

    fn offline_fixture_request() -> BlindEvaluatorRequest {
        let evaluator = OpenAICompatibleQualityEvaluator::new(
            OpenAICompatibleEvaluatorConfig::new(
                "http://127.0.0.1:9/v1",
                "quality-model",
                PROTOCOL_VERSION,
                EvaluatorIndependence::Primary,
            ),
            None,
        )
        .expect("offline fixture evaluator");
        fixture_request(&evaluator)
    }

    #[tokio::test]
    async fn sends_a_blind_bounded_prompt_and_normalizes_valid_output() {
        let request = offline_fixture_request();
        let content = json!({"assessments": [valid_draft(&request)]}).to_string();
        let (evaluator, mut receiver, server) = spawn_mock(
            vec![mock_success(success_response(content, 11, 7))],
            Some(API_KEY),
        )
        .await;
        let request = fixture_request(&evaluator);
        let output = evaluator
            .evaluate(request.clone())
            .await
            .expect("valid response normalizes");
        let (headers, body) = receiver.recv().await.expect("request captured");

        assert_eq!(headers["authorization"], format!("Bearer {API_KEY}"));
        assert_eq!(body["model"], "quality-model");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"].as_array().expect("messages").len(), 2);
        let user: Value = serde_json::from_str(
            body["messages"][1]["content"]
                .as_str()
                .expect("user prompt"),
        )
        .expect("user prompt is inspectable JSON");
        assert_eq!(user["required_output"]["assessment_count"], 1);
        assert_eq!(
            user["candidate_rows"][0]
                .as_object()
                .expect("candidate row")
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "source_row_fingerprint".into(),
                "source_row_id".into(),
                "text".into(),
            ])
        );
        let rendered_user = body["messages"][1]["content"]
            .as_str()
            .expect("user prompt");
        for forbidden in [
            "generation_job_id",
            "generator-private-backend",
            "generator-private-model",
            "private-construction",
            "audit_plan_id",
            "audit_run_id",
            "plan_item_fingerprint",
        ] {
            assert!(
                !rendered_user.contains(forbidden),
                "blind prompt leaked {forbidden}"
            );
        }
        assert_eq!(output.assessments.len(), 1);
        assert_eq!(output.usage.input_tokens, 11);
        assert_eq!(output.usage.output_tokens, 7);
        assert_eq!(output.usage.total_tokens, 18);
        assert_eq!(output.metadata["request_id"], "quality-response-1");
        server.abort();
    }

    #[test]
    fn outbound_output_tokens_are_capped_by_fingerprinted_configuration() {
        let default_evaluator = OpenAICompatibleQualityEvaluator::new(
            OpenAICompatibleEvaluatorConfig::new(
                "http://127.0.0.1:9/v1",
                "quality-model",
                PROTOCOL_VERSION,
                EvaluatorIndependence::Primary,
            ),
            None,
        )
        .expect("default bounded evaluator");
        let default_request = fixture_request_with_budget(
            &default_evaluator,
            None,
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 1_000_000,
                maximum_total_tokens: 1_010_000,
                maximum_cost_microusd: None,
            },
        );
        let default_prompt =
            build_production_prompt(&default_request).expect("default bounded prompt");
        let default_body: Value = serde_json::from_slice(
            &default_evaluator
                .request_body(&default_request, &default_prompt)
                .expect("default bounded request body"),
        )
        .expect("default request body JSON");
        assert_eq!(
            default_body["max_tokens"],
            DEFAULT_MAXIMUM_OUTPUT_TOKENS_PER_REQUEST
        );

        let mut config = OpenAICompatibleEvaluatorConfig::new(
            "http://127.0.0.1:9/v1",
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        config.maximum_output_tokens_per_request = 37;
        let evaluator =
            OpenAICompatibleQualityEvaluator::new(config.clone(), None).expect("bounded evaluator");
        let request = fixture_request(&evaluator);
        assert!(request.budget.maximum_output_tokens > 37);
        let prompt = build_production_prompt(&request).expect("bounded prompt");
        let body: Value = serde_json::from_slice(
            &evaluator
                .request_body(&request, &prompt)
                .expect("bounded request body"),
        )
        .expect("request body JSON");
        assert_eq!(body["max_tokens"], 37);
        assert_eq!(
            evaluator
                .non_secret_configuration()
                .maximum_output_tokens_per_request,
            37
        );

        config.maximum_output_tokens_per_request = 38;
        let changed = OpenAICompatibleQualityEvaluator::new(config.clone(), None)
            .expect("changed bounded evaluator");
        assert_ne!(
            evaluator.identity().configuration_fingerprint,
            changed.identity().configuration_fingerprint
        );

        config.maximum_output_tokens_per_request = 0;
        let error = OpenAICompatibleQualityEvaluator::new(config, None)
            .expect_err("zero output-token ceiling must be rejected");
        assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
    }

    #[tokio::test]
    async fn usage_overrun_is_observed_for_the_runner_instead_of_hidden() {
        let request = offline_fixture_request();
        let content = json!({"assessments": [valid_draft(&request)]}).to_string();
        let (evaluator, _, server) = spawn_mock(
            vec![mock_success(success_response(content, 20_000, 300))],
            None,
        )
        .await;
        let request = fixture_request(&evaluator);
        let output = evaluator
            .evaluate(request)
            .await
            .expect("adapter reports actual usage for host enforcement");

        assert_eq!(output.usage.total_tokens, 20_300);
        assert!(output.usage.total_tokens > 10_100);
        server.abort();
    }

    #[tokio::test]
    async fn rejects_token_and_total_budget_overruns_before_http_io() {
        let (evaluator, mut receiver, server) =
            spawn_mock(vec![mock_success(json!({"unused": true}))], None).await;
        let budgets = [
            EvaluatorRequestBudget {
                maximum_input_tokens: 100,
                maximum_output_tokens: 100,
                maximum_total_tokens: 200,
                maximum_cost_microusd: None,
            },
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 100,
                maximum_total_tokens: 500,
                maximum_cost_microusd: None,
            },
        ];

        for budget in budgets {
            let request = fixture_request_with_budget(&evaluator, None, budget);
            let error = evaluator
                .evaluate(request)
                .await
                .expect_err("an unprovable token bound must fail before transport");
            assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
            assert_eq!(error.observed_usage.total_tokens, 0);
        }
        assert!(receiver.try_recv().is_err(), "transport must not be called");
        server.abort();
    }

    #[tokio::test]
    async fn finite_cost_budget_requires_positive_pricing_and_authorizes_worst_case() {
        let (unpriced, mut unpriced_receiver, unpriced_server) =
            spawn_mock(vec![mock_success(json!({"unused": true}))], None).await;
        let finite_budget = EvaluatorRequestBudget {
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 100,
            maximum_total_tokens: 10_100,
            maximum_cost_microusd: Some(10_000),
        };
        let error = unpriced
            .evaluate(fixture_request_with_budget(
                &unpriced,
                finite_budget.maximum_cost_microusd,
                finite_budget,
            ))
            .await
            .expect_err("finite cost without pricing is unauthorized");
        assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
        assert!(unpriced_receiver.try_recv().is_err());
        unpriced_server.abort();

        let pricing = TokenPricing {
            input_microusd_per_million_tokens: 1_000_000,
            output_microusd_per_million_tokens: 1_000_000,
        };
        let (priced, mut priced_receiver, priced_server) = spawn_mock_with_pricing(
            vec![mock_success(json!({"unused": true}))],
            None,
            Some(pricing),
        )
        .await;
        let insufficient_cost = EvaluatorRequestBudget {
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 100,
            maximum_total_tokens: 10_100,
            maximum_cost_microusd: Some(1),
        };
        let error = priced
            .evaluate(fixture_request_with_budget(
                &priced,
                insufficient_cost.maximum_cost_microusd,
                insufficient_cost,
            ))
            .await
            .expect_err("worst-case cost beyond the request cap is unauthorized");
        assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
        assert!(priced_receiver.try_recv().is_err());
        priced_server.abort();
    }

    #[test]
    fn rejects_zero_pricing_plaintext_external_egress_and_secret_paths_without_io() {
        let mut zero_pricing = OpenAICompatibleEvaluatorConfig::new(
            "https://provider.example/v1",
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        zero_pricing.token_pricing = Some(TokenPricing {
            input_microusd_per_million_tokens: 0,
            output_microusd_per_million_tokens: 1,
        });
        let error = OpenAICompatibleQualityEvaluator::new(zero_pricing, None)
            .expect_err("zero declared prices are ambiguous");
        assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);

        let plaintext = OpenAICompatibleEvaluatorConfig::new(
            "http://provider.example/v1",
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        for api_key in [None, Some(API_KEY.to_owned())] {
            let error = OpenAICompatibleQualityEvaluator::new(plaintext.clone(), api_key)
                .expect_err("candidate text cannot leave loopback over plaintext HTTP");
            assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
            assert!(!error.to_string().contains(API_KEY));
        }

        let credential_path = OpenAICompatibleEvaluatorConfig::new(
            "https://provider.example/v1/%73%6B%2Dtest-secret-never-persist",
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        let error = OpenAICompatibleQualityEvaluator::new(credential_path, None)
            .expect_err("encoded credential-like endpoint paths are not public identity");
        assert_eq!(error.kind, QualityEvaluationErrorKind::Configuration);
        assert!(!error.to_string().contains(API_KEY));
    }

    #[tokio::test]
    async fn strict_normalization_rejects_unknown_fields_and_preserves_usage() {
        let request = offline_fixture_request();
        let mut draft = serde_json::to_value(valid_draft(&request)).expect("draft JSON");
        draft["membership_decision"] = json!("include");
        draft["replacement_text"] = json!("rewritten candidate");
        let content = json!({"assessments": [draft]}).to_string();
        let (evaluator, _, server) =
            spawn_mock(vec![mock_success(success_response(content, 23, 17))], None).await;
        let request = fixture_request(&evaluator);
        let error = evaluator
            .evaluate(request)
            .await
            .expect_err("unknown and decision fields fail closed");

        assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
        assert_eq!(error.observed_usage.total_tokens, 40);
        assert_eq!(error.metadata["request_id"], "quality-response-1");
        assert!(!error.message.contains("rewritten candidate"));
        server.abort();
    }

    #[tokio::test]
    async fn rejects_json_escaped_credentials_after_decoding_and_preserves_usage() {
        let request = offline_fixture_request();
        let draft = valid_draft(&request);
        let serialized = serde_json::to_string(&draft).expect("draft JSON");
        let rationale = serde_json::to_string(&draft.rationale).expect("rationale JSON");
        let encoded_secret = r#""\u0073\u006b\u002dtest-secret-never-persist""#;
        let encoded_draft = serialized.replacen(&rationale, encoded_secret, 1);
        assert_ne!(encoded_draft, serialized);
        assert!(!encoded_draft.contains(API_KEY));
        let content = format!(r#"{{"assessments":[{encoded_draft}]}}"#);
        let (evaluator, _, server) = spawn_mock(
            vec![mock_success(success_response(content, 13, 8))],
            Some(API_KEY),
        )
        .await;
        let error = evaluator
            .evaluate(fixture_request(&evaluator))
            .await
            .expect_err("decoded provider credential material must fail closed");

        assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
        assert_eq!(error.observed_usage.total_tokens, 21);
        assert!(!error.to_string().contains(API_KEY));
        assert!(!error.metadata.to_string().contains(API_KEY));
        server.abort();
    }

    #[tokio::test]
    async fn rejects_duplicate_members_in_assessments_and_nested_score_maps() {
        let request = offline_fixture_request();
        let draft = valid_draft(&request);
        let serialized = serde_json::to_string(&draft).expect("draft JSON");
        let rationale_value = serde_json::to_string(&draft.rationale).expect("rationale JSON");
        let rationale_field = format!(r#""rationale":{rationale_value}"#);
        let duplicate_rationale = serialized.replacen(
            &rationale_field,
            &format!("{rationale_field},{rationale_field}"),
            1,
        );
        assert_ne!(duplicate_rationale, serialized);
        let duplicate_label_score =
            serialized.replacen(r#""billing":9000"#, r#""billing":9000,"billing":9000"#, 1);
        assert_ne!(duplicate_label_score, serialized);

        let responses = [duplicate_rationale, duplicate_label_score]
            .into_iter()
            .map(|draft| {
                let content = format!(r#"{{"assessments":[{draft}]}}"#);
                mock_success(success_response(content, 17, 9))
            })
            .collect();
        let (evaluator, _, server) = spawn_mock(responses, None).await;
        for _ in 0..2 {
            let error = evaluator
                .evaluate(fixture_request(&evaluator))
                .await
                .expect_err("duplicate JSON members must fail closed");
            assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
            assert_eq!(error.observed_usage.total_tokens, 26);
        }
        server.abort();
    }

    #[tokio::test]
    async fn rejects_extra_rows_wrong_fingerprints_and_incomplete_score_maps() {
        let request = offline_fixture_request();

        let mut extra = valid_draft(&request);
        extra.source_row_id = Uuid::new_v4();
        let mut wrong_fingerprint = valid_draft(&request);
        wrong_fingerprint.source_row_fingerprint = "sha256:wrong".into();
        let mut incomplete = valid_draft(&request);
        incomplete.label_scores.remove("fraud");
        let cases = vec![
            json!({"assessments": [valid_draft(&request), extra]}).to_string(),
            json!({"assessments": [wrong_fingerprint]}).to_string(),
            json!({"assessments": [incomplete]}).to_string(),
        ];
        let responses = cases
            .into_iter()
            .map(|content| mock_success(success_response(content, 10, 10)))
            .collect();
        let (evaluator, _, server) = spawn_mock(responses, None).await;
        for _ in 0..3 {
            let request = fixture_request(&evaluator);
            let error = evaluator
                .evaluate(request)
                .await
                .expect_err("invalid row binding or map shape fails closed");
            assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
            assert_eq!(error.observed_usage.total_tokens, 20);
        }
        server.abort();
    }

    #[tokio::test]
    async fn rejects_unknown_response_envelope_fields_and_multiple_choices() {
        let request = offline_fixture_request();
        let content = json!({"assessments": [valid_draft(&request)]}).to_string();
        let mut unknown = success_response(content.clone(), 10, 5);
        unknown["provider_extension"] = json!(true);
        let mut multiple = success_response(content, 10, 5);
        let choice = multiple["choices"][0].clone();
        multiple["choices"]
            .as_array_mut()
            .expect("choices")
            .push(choice);
        let (evaluator, _, server) =
            spawn_mock(vec![mock_success(unknown), mock_success(multiple)], None).await;

        for _ in 0..2 {
            let error = evaluator
                .evaluate(fixture_request(&evaluator))
                .await
                .expect_err("strict response envelope fails closed");
            assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
            assert_eq!(error.observed_usage.total_tokens, 15);
        }
        server.abort();
    }

    #[tokio::test]
    async fn classifies_http_statuses_and_retry_after_without_raw_messages() {
        let mut retry_headers = AxumHeaderMap::new();
        retry_headers.insert(RETRY_AFTER, "7".parse().expect("header"));
        let mut date_retry_headers = AxumHeaderMap::new();
        date_retry_headers.insert(
            RETRY_AFTER,
            "Mon, 31 Aug 2026 12:00:07 GMT"
                .parse()
                .expect("HTTP-date header"),
        );
        let mut clamped_retry_headers = AxumHeaderMap::new();
        clamped_retry_headers.insert(RETRY_AFTER, "999".parse().expect("header"));
        let provider_error = |status, headers| MockResponse {
            status,
            headers,
            body: json!({
                "error": {
                    "message": format!("raw secret {API_KEY}"),
                    "code": "safe_code",
                    "type": "safe_type",
                    "param": "safe_parameter"
                }
            }),
        };
        let responses = vec![
            provider_error(StatusCode::UNAUTHORIZED, AxumHeaderMap::new()),
            provider_error(StatusCode::BAD_REQUEST, AxumHeaderMap::new()),
            provider_error(StatusCode::TOO_MANY_REQUESTS, retry_headers),
            provider_error(StatusCode::TOO_MANY_REQUESTS, date_retry_headers),
            provider_error(StatusCode::TOO_MANY_REQUESTS, clamped_retry_headers),
            provider_error(StatusCode::SERVICE_UNAVAILABLE, AxumHeaderMap::new()),
        ];
        let (mut evaluator, _, server) = spawn_mock(responses, Some(API_KEY)).await;
        evaluator.transport.now = fixed_retry_time;
        let expected = [
            QualityEvaluationErrorKind::Authentication,
            QualityEvaluationErrorKind::Configuration,
            QualityEvaluationErrorKind::RateLimit,
            QualityEvaluationErrorKind::RateLimit,
            QualityEvaluationErrorKind::RateLimit,
            QualityEvaluationErrorKind::Provider,
        ];
        for (index, expected_kind) in expected.into_iter().enumerate() {
            let error = evaluator
                .evaluate(fixture_request(&evaluator))
                .await
                .expect_err("HTTP failure is classified");
            assert_eq!(error.kind, expected_kind);
            assert!(!error.to_string().contains(API_KEY));
            assert!(!error.to_string().contains("raw secret"));
            match index {
                2 | 3 => assert_eq!(error.retry_after_millis, Some(7_000)),
                4 => assert_eq!(error.retry_after_millis, Some(30_000)),
                _ => assert_eq!(error.retry_after_millis, None),
            }
        }
        assert!(!format!("{evaluator:?}").contains(API_KEY));
        server.abort();
    }

    #[tokio::test]
    async fn transport_failures_are_redacted_and_classified() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve unused address");
        let address = listener.local_addr().expect("unused address");
        drop(listener);
        let evaluator = OpenAICompatibleQualityEvaluator::new(
            OpenAICompatibleEvaluatorConfig::new(
                format!("http://{address}/v1"),
                "quality-model",
                PROTOCOL_VERSION,
                EvaluatorIndependence::Primary,
            ),
            Some(API_KEY.into()),
        )
        .expect("evaluator");
        let error = evaluator
            .evaluate(fixture_request(&evaluator))
            .await
            .expect_err("closed loopback port fails");

        assert_eq!(error.kind, QualityEvaluationErrorKind::Transport);
        assert!(!error.to_string().contains(API_KEY));
    }

    #[tokio::test]
    async fn enforces_content_and_response_envelope_size_limits() {
        let request = offline_fixture_request();
        let content = json!({"assessments": [valid_draft(&request)]}).to_string();

        let (mut content_evaluator, _, content_server) = spawn_mock(
            vec![mock_success(success_response(content.clone(), 10, 10))],
            None,
        )
        .await;
        content_evaluator.configuration.maximum_content_bytes = 32;
        let error = content_evaluator
            .evaluate(fixture_request(&content_evaluator))
            .await
            .expect_err("content limit enforced");
        assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
        assert_eq!(error.observed_usage.total_tokens, 20);
        content_server.abort();

        let oversized = "x".repeat(2_000);
        let (mut envelope_evaluator, _, envelope_server) = spawn_mock(
            vec![mock_success(success_response(oversized, 10, 10))],
            None,
        )
        .await;
        envelope_evaluator.configuration.maximum_response_bytes = 1_024;
        let error = envelope_evaluator
            .evaluate(fixture_request(&envelope_evaluator))
            .await
            .expect_err("envelope limit enforced");
        assert_eq!(error.kind, QualityEvaluationErrorKind::InvalidResponse);
        envelope_server.abort();
    }

    #[test]
    fn identity_pins_non_secret_endpoint_prompt_parameters_and_pricing() {
        let mut config = OpenAICompatibleEvaluatorConfig::new(
            "https://provider.example/v1",
            "quality-model",
            PROTOCOL_VERSION,
            EvaluatorIndependence::Primary,
        );
        config.temperature_thousandths = 125;
        config.seed = Some(7);
        config.token_pricing = Some(TokenPricing {
            input_microusd_per_million_tokens: 100,
            output_microusd_per_million_tokens: 200,
        });
        let left = OpenAICompatibleQualityEvaluator::new(config.clone(), Some(API_KEY.into()))
            .expect("left evaluator");
        let without_key =
            OpenAICompatibleQualityEvaluator::new(config.clone(), None).expect("keyless evaluator");
        assert_eq!(left.identity(), without_key.identity());
        assert_eq!(
            left.non_secret_configuration().endpoint_origin,
            "https://provider.example"
        );
        assert_eq!(
            left.non_secret_configuration().endpoint_path,
            "/v1/chat/completions"
        );
        assert!(
            left.non_secret_configuration()
                .prompt_policy_fingerprint
                .starts_with("sha256:")
        );

        config.seed = Some(8);
        let changed =
            OpenAICompatibleQualityEvaluator::new(config, None).expect("changed evaluator");
        assert_ne!(
            left.identity().configuration_fingerprint,
            changed.identity().configuration_fingerprint
        );
        assert!(!format!("{left:?}").contains(API_KEY));
    }
}
