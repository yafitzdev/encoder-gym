//! Reproducible, non-secret evaluator configuration.

use std::net::IpAddr;

use dataset_quality_core::{
    assessment::EvaluatorIndependence,
    ports::{QualityEvaluationError, QualityEvaluationErrorKind},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};

const CONFIGURATION_SCHEMA_VERSION: u32 = 3;
pub(crate) const BACKEND_NAME: &str = "openai-compatible";
const RESPONSE_FORMAT_VERSION: &str = "chat-completions-json-object-v1";
const MAX_MODEL_CHARACTERS: usize = 256;
const MAX_PROTOCOL_CHARACTERS: usize = 128;
const MAX_PROMPT_VERSION_CHARACTERS: usize = 128;
const MAX_CONFIGURED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MIN_CONFIGURED_RESPONSE_BYTES: usize = 1_024;
const MAX_TIMEOUT_MILLIS: u64 = 10 * 60 * 1_000;
const MAX_CONFIGURED_OUTPUT_TOKENS_PER_REQUEST: u64 = 1_000_000;

pub const DEFAULT_MAXIMUM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAXIMUM_CONTENT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT_MILLIS: u64 = 120_000;
/// Keeps an aggregate audit budget from becoming an implausibly large
/// provider-local `max_tokens` value for a small audit. Operators embedding
/// this adapter may lower or raise the ceiling within the validated range.
pub const DEFAULT_MAXIMUM_OUTPUT_TOKENS_PER_REQUEST: u64 = 16_384;

/// Optional declared pricing used to produce conservative integer cost usage.
/// Prices are micro-US-dollars per one million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenPricing {
    pub input_microusd_per_million_tokens: u64,
    pub output_microusd_per_million_tokens: u64,
}

impl TokenPricing {
    pub(crate) fn validate(self) -> Result<(), QualityEvaluationError> {
        if self.input_microusd_per_million_tokens == 0
            || self.output_microusd_per_million_tokens == 0
        {
            return Err(configuration_error(
                "declared evaluator token prices must both be positive",
            ));
        }
        Ok(())
    }

    pub(crate) fn cost_microusd(self, input_tokens: u64, output_tokens: u64) -> Option<u64> {
        let numerator = u128::from(input_tokens)
            .checked_mul(u128::from(self.input_microusd_per_million_tokens))?
            .checked_add(
                u128::from(output_tokens)
                    .checked_mul(u128::from(self.output_microusd_per_million_tokens))?,
            )?;
        let rounded_up = numerator.checked_add(999_999)? / 1_000_000;
        u64::try_from(rounded_up).ok()
    }
}

/// Operator configuration. It deliberately contains no API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAICompatibleEvaluatorConfig {
    pub base_url: String,
    pub model: String,
    pub protocol_version: String,
    pub independence: EvaluatorIndependence,
    /// Temperature in thousandths, bounded to the OpenAI-compatible 0..=2
    /// range without putting floating-point values into fingerprints.
    pub temperature_thousandths: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    pub timeout_millis: u64,
    pub maximum_output_tokens_per_request: u64,
    pub maximum_response_bytes: usize,
    pub maximum_content_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_pricing: Option<TokenPricing>,
}

impl OpenAICompatibleEvaluatorConfig {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        protocol_version: impl Into<String>,
        independence: EvaluatorIndependence,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            protocol_version: protocol_version.into(),
            independence,
            temperature_thousandths: 0,
            seed: None,
            timeout_millis: DEFAULT_TIMEOUT_MILLIS,
            maximum_output_tokens_per_request: DEFAULT_MAXIMUM_OUTPUT_TOKENS_PER_REQUEST,
            maximum_response_bytes: DEFAULT_MAXIMUM_RESPONSE_BYTES,
            maximum_content_bytes: DEFAULT_MAXIMUM_CONTENT_BYTES,
            token_pricing: None,
        }
    }
}

/// Exact non-secret material bound by the evaluator identity fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NonSecretEvaluatorConfiguration {
    pub schema_version: u32,
    pub backend: String,
    pub endpoint_origin: String,
    pub endpoint_path: String,
    pub model: String,
    pub protocol_version: String,
    pub prompt_version: String,
    pub prompt_policy_fingerprint: String,
    pub response_format_version: String,
    pub temperature_thousandths: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    pub timeout_millis: u64,
    pub maximum_output_tokens_per_request: u64,
    pub maximum_response_bytes: usize,
    pub maximum_content_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_pricing: Option<TokenPricing>,
}

impl NonSecretEvaluatorConfiguration {
    pub fn reproduce_fingerprint(&self) -> Result<String, QualityEvaluationError> {
        artifact_core::fingerprint(self)
            .map_err(|_| configuration_error("could not fingerprint evaluator configuration"))
    }
}

pub(crate) struct ResolvedConfiguration {
    pub(crate) endpoint: Url,
    pub(crate) independence: EvaluatorIndependence,
    pub(crate) public: NonSecretEvaluatorConfiguration,
}

impl ResolvedConfiguration {
    pub(crate) fn resolve(
        config: OpenAICompatibleEvaluatorConfig,
        prompt_version: &str,
        prompt_policy_fingerprint: String,
        api_key: Option<&str>,
    ) -> Result<Self, QualityEvaluationError> {
        let base_url = config.base_url.trim();
        let base = Url::parse(base_url)
            .map_err(|_| configuration_error("evaluator base URL is invalid"))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(configuration_error(
                "evaluator base URL must be an HTTP(S) origin/path without credentials, query, or fragment",
            ));
        }
        let endpoint = Url::parse(&format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .map_err(|_| configuration_error("evaluator chat-completions endpoint is invalid"))?;
        validate_endpoint_security(&endpoint, api_key)?;
        let model = bounded_normalized(
            config.model,
            MAX_MODEL_CHARACTERS,
            "evaluator model is invalid",
        )?;
        let protocol_version = bounded_normalized(
            config.protocol_version,
            MAX_PROTOCOL_CHARACTERS,
            "evaluator protocol version is invalid",
        )?;
        let prompt_version = bounded_normalized(
            prompt_version.to_owned(),
            MAX_PROMPT_VERSION_CHARACTERS,
            "evaluator prompt version is invalid",
        )?;
        let prompt_policy_fingerprint = bounded_normalized(
            prompt_policy_fingerprint,
            128,
            "evaluator prompt policy fingerprint is invalid",
        )?;
        if let Some(pricing) = config.token_pricing {
            pricing.validate()?;
        }
        if config.temperature_thousandths > 2_000
            || config.timeout_millis == 0
            || config.timeout_millis > MAX_TIMEOUT_MILLIS
            || config.maximum_output_tokens_per_request == 0
            || config.maximum_output_tokens_per_request > MAX_CONFIGURED_OUTPUT_TOKENS_PER_REQUEST
            || config.maximum_response_bytes < MIN_CONFIGURED_RESPONSE_BYTES
            || config.maximum_response_bytes > MAX_CONFIGURED_RESPONSE_BYTES
            || config.maximum_content_bytes == 0
            || config.maximum_content_bytes > config.maximum_response_bytes
        {
            return Err(configuration_error(
                "evaluator parameters or response limits are outside supported bounds",
            ));
        }
        let endpoint_origin = endpoint.origin().ascii_serialization();
        let endpoint_path = endpoint.path().to_owned();
        Ok(Self {
            endpoint,
            independence: config.independence,
            public: NonSecretEvaluatorConfiguration {
                schema_version: CONFIGURATION_SCHEMA_VERSION,
                backend: BACKEND_NAME.to_owned(),
                endpoint_origin,
                endpoint_path,
                model,
                protocol_version,
                prompt_version,
                prompt_policy_fingerprint,
                response_format_version: RESPONSE_FORMAT_VERSION.to_owned(),
                temperature_thousandths: config.temperature_thousandths,
                seed: config.seed,
                timeout_millis: config.timeout_millis,
                maximum_output_tokens_per_request: config.maximum_output_tokens_per_request,
                maximum_response_bytes: config.maximum_response_bytes,
                maximum_content_bytes: config.maximum_content_bytes,
                token_pricing: config.token_pricing,
            },
        })
    }
}

fn validate_endpoint_security(
    endpoint: &Url,
    api_key: Option<&str>,
) -> Result<(), QualityEvaluationError> {
    let host = endpoint
        .host_str()
        .ok_or_else(|| configuration_error("evaluator endpoint host is invalid"))?;
    if endpoint.scheme() == "http" && !is_loopback_host(host) {
        return Err(configuration_error(
            "external evaluator endpoints require HTTPS; plaintext HTTP is loopback-only",
        ));
    }

    let decoded_path = decode_endpoint_path(endpoint.path())?;
    if contains_credential_like_path(&decoded_path, api_key) {
        return Err(configuration_error(
            "evaluator endpoint path appears to contain credential material",
        ));
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

fn decode_endpoint_path(path: &str) -> Result<String, QualityEvaluationError> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(high) = bytes.get(index + 1).and_then(|value| hex_value(*value)) else {
                return Err(configuration_error(
                    "evaluator endpoint path encoding is invalid",
                ));
            };
            let Some(low) = bytes.get(index + 2).and_then(|value| hex_value(*value)) else {
                return Err(configuration_error(
                    "evaluator endpoint path encoding is invalid",
                ));
            };
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| configuration_error("evaluator endpoint path encoding is invalid"))
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn contains_credential_like_path(path: &str, api_key: Option<&str>) -> bool {
    if api_key.is_some_and(|secret| !secret.is_empty() && path.contains(secret)) {
        return true;
    }
    let normalized = path.to_ascii_lowercase();
    [
        "sk-",
        "api_key",
        "api-key",
        "apikey",
        "access_token",
        "access-token",
        "bearer_token",
        "bearer-token",
        "secret-",
        "secret_",
    ]
    .into_iter()
    .any(|marker| normalized.contains(marker))
        || normalized
            .split('/')
            .any(|segment| matches!(segment, "token" | "secret" | "credential" | "credentials"))
}

fn bounded_normalized(
    value: String,
    maximum_characters: usize,
    message: &'static str,
) -> Result<String, QualityEvaluationError> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized.chars().count() > maximum_characters
        || normalized != value
    {
        return Err(configuration_error(message));
    }
    Ok(value)
}

fn configuration_error(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Configuration, message)
}
