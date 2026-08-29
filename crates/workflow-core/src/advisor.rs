//! Provider-neutral, bounded advisory analysis over development evidence.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use generation_core::domain::GenerationCell;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const PROMPT_VERSION: &str = "workflow-advisor-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorEgressPolicy {
    AggregateOnly,
    DevelopmentText,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorConfiguration {
    pub backend: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key_env: String,
    pub egress_policy: AdvisorEgressPolicy,
    pub maximum_findings: u16,
    pub maximum_representative_errors: u16,
    pub maximum_actions: u16,
    pub maximum_output_tokens: u32,
    pub temperature: Option<f32>,
}

impl AdvisorConfiguration {
    pub fn validate(&self) -> Result<(), AdvisorError> {
        if self.backend.trim().is_empty()
            || self.model.trim().is_empty()
            || self.api_key_env.trim().is_empty()
            || self.maximum_findings == 0
            || self.maximum_findings > 100
            || self.maximum_representative_errors > 50
            || self.maximum_actions == 0
            || self.maximum_actions > 20
            || self.maximum_output_tokens == 0
            || self.maximum_output_tokens > 100_000
            || self
                .temperature
                .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            return Err(AdvisorError::InvalidConfiguration);
        }
        if self.egress_policy == AdvisorEgressPolicy::DevelopmentText
            && self.maximum_representative_errors == 0
        {
            return Err(AdvisorError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryFinding {
    pub key: String,
    pub fingerprint: String,
    pub kind: String,
    pub support: u64,
    pub error_count: u64,
    pub error_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryExample {
    pub finding_key: String,
    pub text: String,
    pub expected_label: String,
    pub predicted_label: String,
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryRequest {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub workflow_iteration: u32,
    pub task: String,
    pub labels: Vec<String>,
    pub dimensions: BTreeMap<String, Vec<String>>,
    pub analysis_report_id: Uuid,
    pub analysis_report_fingerprint: String,
    pub acceptance_assessment_id: Uuid,
    pub acceptance_assessment_fingerprint: String,
    pub acceptance_state: String,
    pub prediction_count: u64,
    pub error_count: u64,
    pub findings: Vec<AdvisoryFinding>,
    #[serde(default)]
    pub representative_errors: Vec<AdvisoryExample>,
    pub allowed_cells: Vec<GenerationCell>,
    pub allowed_actions: Vec<AdvisoryActionKind>,
    pub remaining_row_budget: u64,
    pub remaining_iteration_budget: u32,
    pub egress_policy: AdvisorEgressPolicy,
}

impl AdvisoryRequest {
    pub fn validate(&self, configuration: &AdvisorConfiguration) -> Result<(), AdvisorError> {
        configuration.validate()?;
        if self.egress_policy == AdvisorEgressPolicy::AggregateOnly
            && !self.representative_errors.is_empty()
        {
            return Err(AdvisorError::RawTextNotAuthorized);
        }
        if self.task.trim().is_empty()
            || self.labels.is_empty()
            || self.findings.len() > usize::from(configuration.maximum_findings)
            || self.allowed_cells.is_empty()
            || self.allowed_actions.is_empty()
            || self.allowed_actions.len() > usize::from(configuration.maximum_actions)
            || self.representative_errors.len()
                > usize::from(configuration.maximum_representative_errors)
            || self.egress_policy != configuration.egress_policy
        {
            return Err(AdvisorError::InvalidRequest);
        }
        if self.findings.iter().any(|finding| {
            finding.key.trim().is_empty()
                || finding.fingerprint.trim().is_empty()
                || !finding.error_rate.is_finite()
                || !(0.0..=1.0).contains(&finding.error_rate)
                || finding.error_count > finding.support
        }) {
            return Err(AdvisorError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisoryActionKind {
    Stop,
    Inspect,
    ConsiderExperiment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryHypothesis {
    pub statement: String,
    pub finding_keys: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryCandidateCell {
    pub cell: GenerationCell,
    pub finding_keys: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryDraft {
    pub interpretation: String,
    pub hypotheses: Vec<AdvisoryHypothesis>,
    pub candidate_cells: Vec<AdvisoryCandidateCell>,
    pub cautions: Vec<String>,
    pub recommended_action: AdvisoryActionKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisorPrompt {
    pub system_prompt: String,
    pub user_prompt: String,
    pub prompt_version: String,
    pub prompt_fingerprint: String,
    pub model: String,
    pub maximum_output_tokens: u32,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisorTransportResult {
    pub content: String,
    pub usage: AdvisorUsage,
    pub metadata: Value,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait AnalysisAdvisor: Send + Sync {
    fn backend_name(&self) -> &str;
    fn generate(
        &self,
        prompt: AdvisorPrompt,
    ) -> BoxFuture<'_, Result<AdvisorTransportResult, AdvisorError>>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryAssessment {
    pub id: Uuid,
    pub request: AdvisoryRequest,
    pub backend: String,
    pub model: String,
    pub prompt_version: String,
    pub prompt_fingerprint: String,
    pub draft: AdvisoryDraft,
    pub usage: AdvisorUsage,
    pub backend_metadata: Value,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AdvisoryAssessment {
    pub fn reproduce_fingerprint(&self) -> Result<String, AdvisorError> {
        assessment_fingerprint(self)
    }
}

pub async fn run_advisor(
    backend: &dyn AnalysisAdvisor,
    configuration: &AdvisorConfiguration,
    request: AdvisoryRequest,
) -> Result<AdvisoryAssessment, AdvisorError> {
    request.validate(configuration)?;
    if backend.backend_name() != configuration.backend {
        return Err(AdvisorError::BackendMismatch);
    }
    let prompt = build_prompt(configuration, &request)?;
    let response = backend.generate(prompt.clone()).await?;
    if response.content.contains("```") {
        return Err(AdvisorError::UnsafeOutput);
    }
    let draft: AdvisoryDraft = serde_json::from_str(&response.content)
        .map_err(|error| AdvisorError::InvalidResponse(error.to_string()))?;
    validate_draft(&draft, &request, configuration)?;
    let mut assessment = AdvisoryAssessment {
        id: Uuid::new_v4(),
        request,
        backend: backend.backend_name().to_owned(),
        model: configuration.model.clone(),
        prompt_version: prompt.prompt_version,
        prompt_fingerprint: prompt.prompt_fingerprint,
        draft,
        usage: response.usage,
        backend_metadata: response.metadata,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    assessment.fingerprint = assessment_fingerprint(&assessment)?;
    Ok(assessment)
}

pub fn build_prompt(
    configuration: &AdvisorConfiguration,
    request: &AdvisoryRequest,
) -> Result<AdvisorPrompt, AdvisorError> {
    request.validate(configuration)?;
    let system_prompt = "You are a bounded encoder-development advisor. Use only supplied development evidence. Return one strict JSON object matching the requested schema. Do not decide benchmark acceptance, allocate row counts, emit commands or code, or claim proven root causes.".to_owned();
    let user_prompt = serde_json::to_string_pretty(&serde_json::json!({
        "task": request.task,
        "labels": request.labels,
        "dimensions": request.dimensions,
        "acceptance_state": request.acceptance_state,
        "prediction_count": request.prediction_count,
        "error_count": request.error_count,
        "findings": request.findings,
        "representative_errors": request.representative_errors,
        "allowed_cells": request.allowed_cells,
        "allowed_actions": request.allowed_actions,
        "remaining_row_budget": request.remaining_row_budget,
        "remaining_iteration_budget": request.remaining_iteration_budget,
        "output_schema": {
            "interpretation": "string",
            "hypotheses": [{"statement": "string", "finding_keys": ["known key"], "confidence": "0..1"}],
            "candidate_cells": [{"cell": "one exact allowed cell", "finding_keys": ["known key"], "rationale": "string"}],
            "cautions": ["string"],
            "recommended_action": "stop | inspect | consider_experiment"
        }
    }))
    .map_err(|error| AdvisorError::InvalidRequestWithReason(error.to_string()))?;
    let prompt_fingerprint = artifact_core::fingerprint(&serde_json::json!({
        "version": PROMPT_VERSION,
        "system": system_prompt,
        "user": user_prompt,
    }))
    .map_err(|error| AdvisorError::InvalidRequestWithReason(error.to_string()))?;
    Ok(AdvisorPrompt {
        system_prompt,
        user_prompt,
        prompt_version: PROMPT_VERSION.into(),
        prompt_fingerprint,
        model: configuration.model.clone(),
        maximum_output_tokens: configuration.maximum_output_tokens,
        temperature: configuration.temperature,
    })
}

fn validate_draft(
    draft: &AdvisoryDraft,
    request: &AdvisoryRequest,
    configuration: &AdvisorConfiguration,
) -> Result<(), AdvisorError> {
    if draft.interpretation.trim().is_empty()
        || draft.hypotheses.len() > usize::from(configuration.maximum_actions)
        || draft.candidate_cells.len() > usize::from(configuration.maximum_actions)
        || draft.cautions.len() > usize::from(configuration.maximum_actions)
        || !request.allowed_actions.contains(&draft.recommended_action)
    {
        return Err(AdvisorError::InvalidResponse(
            "bounded output violated".into(),
        ));
    }
    let known_findings = request
        .findings
        .iter()
        .map(|finding| finding.key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let valid_keys = |keys: &[String]| {
        !keys.is_empty() && keys.iter().all(|key| known_findings.contains(key.as_str()))
    };
    if draft.hypotheses.iter().any(|hypothesis| {
        hypothesis.statement.trim().is_empty()
            || !hypothesis.confidence.is_finite()
            || !(0.0..=1.0).contains(&hypothesis.confidence)
            || !valid_keys(&hypothesis.finding_keys)
    }) || draft.candidate_cells.iter().any(|candidate| {
        candidate.rationale.trim().is_empty()
            || !request.allowed_cells.contains(&candidate.cell)
            || !valid_keys(&candidate.finding_keys)
    }) || draft.cautions.iter().any(|value| value.trim().is_empty())
    {
        return Err(AdvisorError::InvalidResponse(
            "output references unknown or invalid evidence".into(),
        ));
    }
    Ok(())
}

fn assessment_fingerprint(value: &AdvisoryAssessment) -> Result<String, AdvisorError> {
    artifact_core::fingerprint(&serde_json::json!({
        "request": value.request,
        "backend": value.backend,
        "model": value.model,
        "prompt_version": value.prompt_version,
        "prompt_fingerprint": value.prompt_fingerprint,
        "draft": value.draft,
        "usage": value.usage,
        "backend_metadata": value.backend_metadata,
    }))
    .map_err(|error| AdvisorError::InvalidResponse(error.to_string()))
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AdvisorError {
    #[error("advisor configuration is invalid")]
    InvalidConfiguration,
    #[error("advisor request is invalid")]
    InvalidRequest,
    #[error("advisor request is invalid: {0}")]
    InvalidRequestWithReason(String),
    #[error("raw development text egress is not authorized")]
    RawTextNotAuthorized,
    #[error("advisor backend does not match persisted configuration")]
    BackendMismatch,
    #[error("advisor request failed: {0}")]
    Request(String),
    #[error("advisor response is invalid: {0}")]
    InvalidResponse(String),
    #[error("advisor response contains code or command content")]
    UnsafeOutput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct StubAdvisor {
        content: String,
    }

    impl AnalysisAdvisor for StubAdvisor {
        fn backend_name(&self) -> &str {
            "fake"
        }

        fn generate(
            &self,
            _prompt: AdvisorPrompt,
        ) -> BoxFuture<'_, Result<AdvisorTransportResult, AdvisorError>> {
            let content = self.content.clone();
            Box::pin(async move {
                Ok(AdvisorTransportResult {
                    content,
                    usage: AdvisorUsage::default(),
                    metadata: serde_json::json!({}),
                })
            })
        }
    }

    fn configuration() -> AdvisorConfiguration {
        AdvisorConfiguration {
            backend: "fake".into(),
            model: "deterministic-v1".into(),
            base_url: None,
            api_key_env: "SYNTH_ADVISOR_API_KEY".into(),
            egress_policy: AdvisorEgressPolicy::AggregateOnly,
            maximum_findings: 5,
            maximum_representative_errors: 0,
            maximum_actions: 3,
            maximum_output_tokens: 1_000,
            temperature: Some(0.0),
        }
    }

    fn request() -> AdvisoryRequest {
        AdvisoryRequest {
            id: Uuid::new_v4(),
            workflow_run_id: Uuid::new_v4(),
            workflow_iteration: 0,
            task: "classify support requests".into(),
            labels: vec!["billing".into(), "fraud".into()],
            dimensions: BTreeMap::from([("difficulty".into(), vec!["easy".into(), "hard".into()])]),
            analysis_report_id: Uuid::new_v4(),
            analysis_report_fingerprint: "sha256:analysis".into(),
            acceptance_assessment_id: Uuid::new_v4(),
            acceptance_assessment_fingerprint: "sha256:acceptance".into(),
            acceptance_state: "fail".into(),
            prediction_count: 100,
            error_count: 20,
            findings: vec![AdvisoryFinding {
                key: "finding-1".into(),
                fingerprint: "sha256:finding".into(),
                kind: "cell".into(),
                support: 10,
                error_count: 4,
                error_rate: 0.4,
            }],
            representative_errors: Vec::new(),
            allowed_cells: vec![GenerationCell {
                label: "billing".into(),
                dimensions: BTreeMap::from([("difficulty".into(), "hard".into())]),
            }],
            allowed_actions: vec![AdvisoryActionKind::Stop, AdvisoryActionKind::Inspect],
            remaining_row_budget: 100,
            remaining_iteration_budget: 2,
            egress_policy: AdvisorEgressPolicy::AggregateOnly,
        }
    }

    #[tokio::test]
    async fn validates_evidence_references_and_fingerprints_assessment() {
        let request = request();
        let draft = AdvisoryDraft {
            interpretation: "One bounded inspection is warranted.".into(),
            hypotheses: vec![AdvisoryHypothesis {
                statement: "The observed cell may be difficult.".into(),
                finding_keys: vec!["finding-1".into()],
                confidence: 0.5,
            }],
            candidate_cells: Vec::new(),
            cautions: vec!["Adaptive development evidence is not final evidence.".into()],
            recommended_action: AdvisoryActionKind::Inspect,
        };
        let backend = StubAdvisor {
            content: serde_json::to_string(&draft).expect("draft JSON"),
        };
        let assessment = run_advisor(&backend, &configuration(), request)
            .await
            .expect("valid advisory");
        assert_eq!(
            assessment.reproduce_fingerprint().expect("fingerprint"),
            assessment.fingerprint
        );
    }

    #[test]
    fn aggregate_only_rejects_raw_text_before_prompt_construction() {
        let mut request = request();
        request.representative_errors.push(AdvisoryExample {
            finding_key: "finding-1".into(),
            text: "raw development text".into(),
            expected_label: "billing".into(),
            predicted_label: "fraud".into(),
            dimensions: BTreeMap::new(),
        });
        assert_eq!(
            build_prompt(&configuration(), &request),
            Err(AdvisorError::RawTextNotAuthorized)
        );
    }
}
