//! Fail-closed provider response normalization.

use std::collections::{BTreeMap, BTreeSet};

use dataset_quality_core::{
    assessment::{
        BlindEvaluatorRequest, MAX_ISSUE_CODES, MAX_RATIONALE_CHARACTERS, RowAssessmentDraft,
    },
    lifecycle::ProviderUsage,
    ports::{EvaluatorBatchOutput, QualityEvaluationError, QualityEvaluationErrorKind},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    TokenPricing, strict_json,
    transport::{contains_bytes, safe_metadata},
};

pub(crate) fn normalize_success(
    request: &BlindEvaluatorRequest,
    raw: &[u8],
    maximum_content_bytes: usize,
    pricing: Option<TokenPricing>,
    api_key: Option<&str>,
) -> Result<EvaluatorBatchOutput, QualityEvaluationError> {
    if api_key.is_some_and(|secret| contains_bytes(raw, secret.as_bytes())) {
        let (usage, metadata) = recover_observed_evidence(raw, pricing, api_key);
        return Err(invalid_response(
            "provider response contained credential material and was discarded",
        )
        .with_observed_evidence(usage, metadata));
    }
    let response_value = strict_json::parse_slice(raw).map_err(|_| {
        let (usage, metadata) = recover_observed_evidence(raw, pricing, api_key);
        invalid_response("provider response envelope does not match the strict schema")
            .with_observed_evidence(usage, metadata)
    })?;
    if api_key.is_some_and(|secret| json_value_contains_secret(&response_value, secret)) {
        let (usage, metadata) = recover_observed_evidence(raw, pricing, api_key);
        return Err(invalid_response(
            "provider response contained credential material and was discarded",
        )
        .with_observed_evidence(usage, metadata));
    }
    let response =
        serde_json::from_value::<ChatCompletionResponse>(response_value).map_err(|_| {
            let (usage, metadata) = recover_observed_evidence(raw, pricing, api_key);
            invalid_response("provider response envelope does not match the strict schema")
                .with_observed_evidence(usage, metadata)
        })?;
    let metadata = safe_metadata(
        response.id.as_deref(),
        response.model.as_deref(),
        response
            .choices
            .first()
            .and_then(|choice| choice.finish_reason.as_deref()),
        api_key,
    );
    let usage = match normalize_usage(response.usage, pricing) {
        Ok(usage) => usage,
        Err(error) => {
            let observed_usage = observed_usage_from_chat(response.usage, pricing);
            return Err(error.with_observed_evidence(observed_usage, metadata));
        }
    };
    let attach_evidence =
        |error: QualityEvaluationError| error.with_observed_evidence(usage, metadata.clone());
    if response.choices.len() != 1 {
        return Err(attach_evidence(invalid_response(
            "provider response must contain exactly one choice",
        )));
    }
    let Some(choice) = response.choices.into_iter().next() else {
        return Err(attach_evidence(invalid_response(
            "provider response must contain exactly one choice",
        )));
    };
    if choice
        .message
        .role
        .as_deref()
        .is_some_and(|role| role != "assistant")
        || choice.message.refusal.is_some()
        || choice.finish_reason.as_deref() != Some("stop")
    {
        return Err(attach_evidence(invalid_response(
            "provider choice was not a completed assistant JSON response",
        )));
    }
    if choice.message.content.len() > maximum_content_bytes {
        return Err(attach_evidence(invalid_response(
            "provider assessment content exceeded the configured size limit",
        )));
    }
    let assessment_value =
        strict_json::parse_slice(choice.message.content.as_bytes()).map_err(|_| {
            attach_evidence(invalid_response(
                "provider assessment content does not match the strict schema",
            ))
        })?;
    if api_key.is_some_and(|secret| json_value_contains_secret(&assessment_value, secret)) {
        return Err(attach_evidence(invalid_response(
            "provider response contained credential material and was discarded",
        )));
    }
    let envelope =
        serde_json::from_value::<AssessmentEnvelope>(assessment_value).map_err(|_| {
            attach_evidence(invalid_response(
                "provider assessment content does not match the strict schema",
            ))
        })?;
    let assessments = decode_and_normalize_assessments(request, envelope.assessments)
        .map_err(|message| attach_evidence(invalid_response(message)))?;
    Ok(EvaluatorBatchOutput {
        assessments,
        usage,
        metadata,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatCompletionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "object")]
    _object: Option<String>,
    #[serde(default, rename = "created")]
    _created: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
    #[serde(default, rename = "system_fingerprint")]
    _system_fingerprint: Option<String>,
    #[serde(default, rename = "service_tier")]
    _service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatChoice {
    #[serde(default, rename = "index")]
    _index: Option<u32>,
    message: ChatResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default, rename = "logprobs")]
    _logprobs: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatResponseMessage {
    #[serde(default)]
    role: Option<String>,
    content: String,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    #[serde(default, rename = "prompt_tokens_details")]
    _prompt_tokens_details: Option<PromptTokenDetails>,
    #[serde(default, rename = "completion_tokens_details")]
    _completion_tokens_details: Option<CompletionTokenDetails>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptTokenDetails {
    #[serde(default, rename = "cached_tokens")]
    _cached_tokens: Option<u64>,
    #[serde(default, rename = "audio_tokens")]
    _audio_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionTokenDetails {
    #[serde(default, rename = "reasoning_tokens")]
    _reasoning_tokens: Option<u64>,
    #[serde(default, rename = "audio_tokens")]
    _audio_tokens: Option<u64>,
    #[serde(default, rename = "accepted_prediction_tokens")]
    _accepted_prediction_tokens: Option<u64>,
    #[serde(default, rename = "rejected_prediction_tokens")]
    _rejected_prediction_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssessmentEnvelope {
    assessments: Vec<Value>,
}

fn decode_and_normalize_assessments(
    request: &BlindEvaluatorRequest,
    values: Vec<Value>,
) -> Result<Vec<RowAssessmentDraft>, &'static str> {
    let mut expected_fields = BTreeSet::from([
        "source_row_id",
        "source_row_fingerprint",
        "label_scores",
        "dimension_scores",
        "label_leakage_risk",
        "shortcut_risk",
        "confidence",
        "issue_codes",
        "rationale",
    ]);
    if request.guidance.authenticity.is_some() {
        expected_fields.insert("authenticity_score");
    }
    let mut assessments = Vec::with_capacity(values.len());
    for value in values {
        let Some(object) = value.as_object() else {
            return Err("provider assessment entries must be JSON objects");
        };
        let actual_fields = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
        if actual_fields != expected_fields {
            return Err("provider assessment fields do not exactly match the requested schema");
        }
        assessments.push(
            serde_json::from_value::<RowAssessmentDraft>(value)
                .map_err(|_| "provider assessment does not match the strict field schema")?,
        );
    }
    normalize_assessments(request, assessments)
}

fn normalize_assessments(
    request: &BlindEvaluatorRequest,
    mut assessments: Vec<RowAssessmentDraft>,
) -> Result<Vec<RowAssessmentDraft>, &'static str> {
    if assessments.len() != request.rows.len() {
        return Err("provider must return exactly one assessment for every requested row");
    }
    let expected_rows = request
        .rows
        .iter()
        .map(|row| (row.source_row_id, row.source_row_fingerprint.as_str()))
        .collect::<BTreeMap<_, _>>();
    let expected_labels = request
        .allowed_labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_dimensions = request
        .allowed_dimensions
        .iter()
        .map(|(name, values)| {
            (
                name.as_str(),
                values.iter().map(String::as_str).collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for assessment in &mut assessments {
        let Some(expected_fingerprint) = expected_rows.get(&assessment.source_row_id) else {
            return Err("provider returned an assessment for an unrequested row");
        };
        if !seen.insert(assessment.source_row_id)
            || assessment.source_row_fingerprint != *expected_fingerprint
        {
            return Err("provider row identity or source fingerprint does not match the request");
        }
        let actual_labels = assessment
            .label_scores
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_labels != expected_labels {
            return Err("provider label score map is incomplete or contains unknown labels");
        }
        let actual_dimension_names = assessment
            .dimension_scores
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_dimension_names != expected_dimensions.keys().copied().collect() {
            return Err(
                "provider dimension score map is incomplete or contains unknown dimensions",
            );
        }
        for (dimension, expected_values) in &expected_dimensions {
            let actual_values = assessment.dimension_scores[*dimension]
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if &actual_values != expected_values {
                return Err(
                    "provider dimension-value score map is incomplete or contains unknown values",
                );
            }
        }
        if assessment.authenticity_score.is_some() != request.guidance.authenticity.is_some() {
            return Err("provider authenticity score presence does not match the request");
        }
        if assessment.issue_codes.len() > MAX_ISSUE_CODES {
            return Err("provider returned too many issue codes");
        }
        assessment.issue_codes.sort_unstable();
        if assessment
            .issue_codes
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err("provider returned duplicate issue codes");
        }
        let rationale = assessment.rationale.trim();
        if rationale.is_empty() || rationale.chars().count() > MAX_RATIONALE_CHARACTERS {
            return Err("provider rationale is empty or exceeds its size limit");
        }
        assessment.rationale = rationale.to_owned();
    }
    if seen.len() != expected_rows.len() {
        return Err("provider assessment row set does not exactly match the request");
    }
    assessments.sort_by_key(|assessment| assessment.source_row_id);
    Ok(assessments)
}

fn normalize_usage(
    usage: ChatUsage,
    pricing: Option<TokenPricing>,
) -> Result<ProviderUsage, QualityEvaluationError> {
    if usage.total_tokens != usage.prompt_tokens.saturating_add(usage.completion_tokens) {
        return Err(invalid_response("provider token usage does not reconcile"));
    }
    let cost_microusd = calculate_cost(usage.prompt_tokens, usage.completion_tokens, pricing)?;
    Ok(ProviderUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cost_microusd,
    })
}

fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    pricing: Option<TokenPricing>,
) -> Result<u64, QualityEvaluationError> {
    let Some(pricing) = pricing else {
        return Ok(0);
    };
    pricing
        .cost_microusd(input_tokens, output_tokens)
        .ok_or_else(|| invalid_response("provider usage cost exceeds the supported integer range"))
}

fn json_value_contains_secret(value: &Value, secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    match value {
        Value::String(value) => value.contains(secret),
        Value::Array(values) => values
            .iter()
            .any(|value| json_value_contains_secret(value, secret)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(secret) || json_value_contains_secret(value, secret)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn observed_usage_from_chat(usage: ChatUsage, pricing: Option<TokenPricing>) -> ProviderUsage {
    conservative_observed_usage(
        usage.prompt_tokens,
        usage.completion_tokens,
        Some(usage.total_tokens),
        pricing,
    )
}

fn recover_observed_evidence(
    raw: &[u8],
    pricing: Option<TokenPricing>,
    api_key: Option<&str>,
) -> (ProviderUsage, Value) {
    let Ok(value) = serde_json::from_slice::<Value>(raw) else {
        return (ProviderUsage::default(), Value::Null);
    };
    let metadata = safe_metadata(
        value.get("id").and_then(Value::as_str),
        value.get("model").and_then(Value::as_str),
        value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str),
        api_key,
    );
    let usage = value.get("usage").and_then(|usage| {
        let input = usage.get("prompt_tokens")?.as_u64()?;
        let output = usage.get("completion_tokens")?.as_u64()?;
        let reported_total = usage.get("total_tokens").and_then(Value::as_u64);
        Some(conservative_observed_usage(
            input,
            output,
            reported_total,
            pricing,
        ))
    });
    (usage.unwrap_or_default(), metadata)
}

fn conservative_observed_usage(
    input_tokens: u64,
    output_tokens: u64,
    reported_total_tokens: Option<u64>,
    pricing: Option<TokenPricing>,
) -> ProviderUsage {
    let Some(component_total) = input_tokens.checked_add(output_tokens) else {
        return ProviderUsage::default();
    };
    let total_tokens =
        reported_total_tokens.map_or(component_total, |reported| reported.max(component_total));
    let reconciled_output_tokens = total_tokens.saturating_sub(input_tokens);
    let Ok(cost_microusd) = calculate_cost(input_tokens, reconciled_output_tokens, pricing) else {
        return ProviderUsage::default();
    };
    ProviderUsage {
        input_tokens,
        output_tokens: reconciled_output_tokens,
        total_tokens,
        cost_microusd,
    }
}

fn invalid_response(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::InvalidResponse, message)
}
