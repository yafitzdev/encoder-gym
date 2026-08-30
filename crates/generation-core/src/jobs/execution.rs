//! Immutable generation runtime specifications and provider-request facts.

use artifact_core::{FingerprintError, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    construction::{ConstructionError, PreparedConstructionBatch, RowConstructionPlan},
    domain::{GenerationCell, GenerationParameters, GenerationRequest, UsageMetadata},
    planning::GenerationNeed,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationBackendIdentity {
    pub name: String,
    pub model: String,
    /// Non-secret endpoint identity. Credentials must never be stored here.
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplateIdentity {
    pub name: String,
    pub version: u32,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationExecutionPolicy {
    pub batch_size: u32,
    pub max_request_retries: u32,
    pub max_attempt_multiplier: u32,
    pub retry_delay_milliseconds: u64,
}

impl GenerationExecutionPolicy {
    pub fn validate(&self) -> Result<(), GenerationExecutionError> {
        if self.batch_size == 0 {
            return Err(GenerationExecutionError::InvalidPolicy(
                "batch size must be greater than zero".into(),
            ));
        }
        if self.max_attempt_multiplier == 0 {
            return Err(GenerationExecutionError::InvalidPolicy(
                "maximum attempt multiplier must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationExecutionSpec {
    pub job_id: Uuid,
    pub dataset_id: Uuid,
    pub plan_id: Uuid,
    pub initial_needs: Vec<GenerationNeed>,
    pub backend: GenerationBackendIdentity,
    pub parameters: GenerationParameters,
    pub policy: GenerationExecutionPolicy,
    pub prompt_template: PromptTemplateIdentity,
    pub semantic_context_fingerprint: String,
    #[serde(default)]
    pub construction_plan: Option<RowConstructionPlan>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationExecutionSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        job_id: Uuid,
        dataset_id: Uuid,
        plan_id: Uuid,
        initial_needs: Vec<GenerationNeed>,
        backend: GenerationBackendIdentity,
        parameters: GenerationParameters,
        policy: GenerationExecutionPolicy,
        prompt_template: PromptTemplateIdentity,
        semantic_context_fingerprint: impl Into<String>,
    ) -> Result<Self, GenerationExecutionError> {
        Self::new_with_construction(
            job_id,
            dataset_id,
            plan_id,
            initial_needs,
            backend,
            parameters,
            policy,
            prompt_template,
            semantic_context_fingerprint,
            RowConstructionPlan::llm_text_default()?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_construction(
        job_id: Uuid,
        dataset_id: Uuid,
        plan_id: Uuid,
        initial_needs: Vec<GenerationNeed>,
        backend: GenerationBackendIdentity,
        parameters: GenerationParameters,
        policy: GenerationExecutionPolicy,
        prompt_template: PromptTemplateIdentity,
        semantic_context_fingerprint: impl Into<String>,
        construction_plan: RowConstructionPlan,
    ) -> Result<Self, GenerationExecutionError> {
        policy.validate()?;
        construction_plan.compile()?;
        let mut need_keys = std::collections::BTreeSet::new();
        for need in &initial_needs {
            if need.remaining_count
                != need
                    .planned
                    .target_count
                    .saturating_sub(need.accepted_count)
            {
                return Err(GenerationExecutionError::InvalidIdentity(format!(
                    "initial need has inconsistent coverage: {}",
                    need.planned.cell.key()
                )));
            }
            if !need_keys.insert(need.planned.cell.key()) {
                return Err(GenerationExecutionError::InvalidIdentity(
                    "initial needs repeat a generation cell".into(),
                ));
            }
        }
        let backend = GenerationBackendIdentity {
            name: required(backend.name, "backend name")?,
            model: required(backend.model, "backend model")?,
            endpoint: backend
                .endpoint
                .map(|value| required(value, "backend endpoint"))
                .transpose()?,
        };
        let prompt_template = PromptTemplateIdentity {
            name: required(prompt_template.name, "prompt template name")?,
            version: prompt_template.version,
            fingerprint: required(prompt_template.fingerprint, "prompt template fingerprint")?,
        };
        if prompt_template.version == 0 {
            return Err(GenerationExecutionError::InvalidIdentity(
                "prompt template version must be greater than zero".into(),
            ));
        }
        let semantic_context_fingerprint = required(
            semantic_context_fingerprint.into(),
            "semantic context fingerprint",
        )?;
        let mut value = Self {
            job_id,
            dataset_id,
            plan_id,
            initial_needs,
            backend,
            parameters,
            policy,
            prompt_template,
            semantic_context_fingerprint,
            construction_plan: Some(construction_plan),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, GenerationExecutionError> {
        if let Some(construction_plan) = &self.construction_plan {
            fingerprint(&(
                self.job_id,
                self.dataset_id,
                self.plan_id,
                &self.initial_needs,
                &self.backend,
                &self.parameters,
                &self.policy,
                &self.prompt_template,
                &self.semantic_context_fingerprint,
                construction_plan,
                self.created_at,
            ))
            .map_err(Into::into)
        } else {
            // Compatibility with execution specifications persisted before the
            // hybrid construction plan became a pinned runtime input.
            fingerprint(&(
                self.job_id,
                self.dataset_id,
                self.plan_id,
                &self.initial_needs,
                &self.backend,
                &self.parameters,
                &self.policy,
                &self.prompt_template,
                &self.semantic_context_fingerprint,
                self.created_at,
            ))
            .map_err(Into::into)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationAttemptState {
    Started,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationAttemptKind {
    #[default]
    ProviderRequest,
    DeterministicConstruction,
}

impl GenerationAttemptState {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Started)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationAttemptFailureKind {
    Configuration,
    Request,
    InvalidResponse,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationAttempt {
    pub id: Uuid,
    pub job_id: Uuid,
    pub sequence: u64,
    #[serde(default)]
    pub kind: GenerationAttemptKind,
    pub cell: GenerationCell,
    pub requested_count: u32,
    pub retry_index: u32,
    pub request_fingerprint: String,
    pub state: GenerationAttemptState,
    pub provider_returned_rows: u32,
    pub persisted_rows: u32,
    pub accepted_rows: u32,
    pub rejected_rows: u32,
    pub usage: Option<UsageMetadata>,
    pub backend_metadata: serde_json::Value,
    pub backend_errors: Vec<String>,
    pub failure_kind: Option<GenerationAttemptFailureKind>,
    pub failure_message: Option<String>,
    pub retryable: Option<bool>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome_fingerprint: Option<String>,
}

impl GenerationAttempt {
    pub fn start(
        job_id: Uuid,
        sequence: u64,
        retry_index: u32,
        request: &GenerationRequest,
    ) -> Result<Self, GenerationExecutionError> {
        if sequence == 0 {
            return Err(GenerationExecutionError::InvalidAttempt(
                "attempt sequence must be greater than zero".into(),
            ));
        }
        if request.requested_count == 0 {
            return Err(GenerationExecutionError::InvalidAttempt(
                "attempt requested count must be greater than zero".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            job_id,
            sequence,
            kind: GenerationAttemptKind::ProviderRequest,
            cell: request.target.clone(),
            requested_count: request.requested_count,
            retry_index,
            request_fingerprint: fingerprint(request)?,
            state: GenerationAttemptState::Started,
            provider_returned_rows: 0,
            persisted_rows: 0,
            accepted_rows: 0,
            rejected_rows: 0,
            usage: None,
            backend_metadata: serde_json::Value::Null,
            backend_errors: vec![],
            failure_kind: None,
            failure_message: None,
            retryable: None,
            started_at: Utc::now(),
            finished_at: None,
            outcome_fingerprint: None,
        })
    }

    pub fn start_deterministic(
        job_id: Uuid,
        sequence: u64,
        batch: &PreparedConstructionBatch,
    ) -> Result<Self, GenerationExecutionError> {
        if sequence == 0 {
            return Err(GenerationExecutionError::InvalidAttempt(
                "attempt sequence must be greater than zero".into(),
            ));
        }
        let requested_count = batch.requested_count();
        if requested_count == 0 || batch.requires_llm() {
            return Err(GenerationExecutionError::InvalidAttempt(
                "deterministic attempt requires a non-empty batch without LLM fields".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            job_id,
            sequence,
            kind: GenerationAttemptKind::DeterministicConstruction,
            cell: batch.target.clone(),
            requested_count,
            retry_index: 0,
            request_fingerprint: fingerprint(batch)?,
            state: GenerationAttemptState::Started,
            provider_returned_rows: 0,
            persisted_rows: 0,
            accepted_rows: 0,
            rejected_rows: 0,
            usage: None,
            backend_metadata: serde_json::Value::Null,
            backend_errors: vec![],
            failure_kind: None,
            failure_message: None,
            retryable: None,
            started_at: Utc::now(),
            finished_at: None,
            outcome_fingerprint: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn succeed(
        &mut self,
        provider_returned_rows: u32,
        persisted_rows: u32,
        accepted_rows: u32,
        rejected_rows: u32,
        usage: Option<UsageMetadata>,
        backend_metadata: serde_json::Value,
        backend_errors: Vec<String>,
    ) -> Result<(), GenerationExecutionError> {
        self.ensure_started()?;
        if accepted_rows.saturating_add(rejected_rows) != persisted_rows
            || persisted_rows > provider_returned_rows
        {
            return Err(GenerationExecutionError::InvalidAttempt(
                "attempt row counts are inconsistent".into(),
            ));
        }
        self.state = GenerationAttemptState::Succeeded;
        self.provider_returned_rows = provider_returned_rows;
        self.persisted_rows = persisted_rows;
        self.accepted_rows = accepted_rows;
        self.rejected_rows = rejected_rows;
        self.usage = usage;
        self.backend_metadata = backend_metadata;
        self.backend_errors = backend_errors;
        self.finished_at = Some(Utc::now());
        self.outcome_fingerprint = Some(self.reproduce_outcome_fingerprint()?);
        Ok(())
    }

    pub fn fail(
        &mut self,
        kind: GenerationAttemptFailureKind,
        message: impl Into<String>,
        retryable: bool,
    ) -> Result<(), GenerationExecutionError> {
        self.ensure_started()?;
        self.state = GenerationAttemptState::Failed;
        self.failure_kind = Some(kind);
        self.failure_message = Some(required(message.into(), "attempt failure message")?);
        self.retryable = Some(retryable);
        self.finished_at = Some(Utc::now());
        self.outcome_fingerprint = Some(self.reproduce_outcome_fingerprint()?);
        Ok(())
    }

    pub fn interrupt(&mut self) -> Result<(), GenerationExecutionError> {
        self.ensure_started()?;
        self.state = GenerationAttemptState::Interrupted;
        self.failure_kind = Some(GenerationAttemptFailureKind::Interrupted);
        self.failure_message =
            Some("provider request outcome is unknown after interruption".into());
        self.retryable = Some(false);
        self.finished_at = Some(Utc::now());
        self.outcome_fingerprint = Some(self.reproduce_outcome_fingerprint()?);
        Ok(())
    }

    pub fn reproduce_outcome_fingerprint(&self) -> Result<String, GenerationExecutionError> {
        if !self.state.is_terminal() || self.finished_at.is_none() {
            return Err(GenerationExecutionError::AttemptNotTerminal);
        }
        let mut value = self.clone();
        value.outcome_fingerprint = None;
        fingerprint(&value).map_err(Into::into)
    }

    fn ensure_started(&self) -> Result<(), GenerationExecutionError> {
        if self.state == GenerationAttemptState::Started {
            Ok(())
        } else {
            Err(GenerationExecutionError::AttemptAlreadyTerminal)
        }
    }
}

#[derive(Debug, Error)]
pub enum GenerationExecutionError {
    #[error("invalid generation execution policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid generation execution identity: {0}")]
    InvalidIdentity(String),
    #[error("invalid generation attempt: {0}")]
    InvalidAttempt(String),
    #[error("generation attempt is already terminal")]
    AttemptAlreadyTerminal,
    #[error("generation attempt is not terminal")]
    AttemptNotTerminal,
    #[error("generation execution fingerprint failed: {0}")]
    Fingerprint(#[from] FingerprintError),
    #[error(transparent)]
    Construction(#[from] ConstructionError),
}

fn required(value: String, field: &str) -> Result<String, GenerationExecutionError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(GenerationExecutionError::InvalidIdentity(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        GenerationAttempt, GenerationAttemptFailureKind, GenerationAttemptState,
        GenerationBackendIdentity, GenerationExecutionPolicy, GenerationExecutionSpec,
    };
    use crate::{
        domain::{GenerationCell, GenerationParameters, GenerationRequest},
        jobs::GenerationJob,
        prompting::PromptBuilder,
    };
    use uuid::Uuid;

    #[test]
    fn execution_spec_pins_and_reproduces_every_non_secret_input() {
        let job = GenerationJob::queued(Uuid::new_v4(), Uuid::new_v4(), "fake", "fake-v1", 10);
        let spec = GenerationExecutionSpec::new(
            job.id,
            job.dataset_id,
            job.plan_id,
            vec![],
            GenerationBackendIdentity {
                name: job.backend_name.clone(),
                model: job.backend_model.clone(),
                endpoint: None,
            },
            GenerationParameters {
                temperature: Some(0.4),
                ..GenerationParameters::default()
            },
            GenerationExecutionPolicy {
                batch_size: 5,
                max_request_retries: 2,
                max_attempt_multiplier: 3,
                retry_delay_milliseconds: 500,
            },
            PromptBuilder::template_identity().expect("template identity"),
            "sha256:semantic",
        )
        .expect("execution spec");
        assert_eq!(
            spec.reproduce_fingerprint().expect("fingerprint"),
            spec.fingerprint
        );
        assert_eq!(spec.parameters.temperature, Some(0.4));
    }

    #[test]
    fn attempt_lifecycle_fingerprints_success_and_failure() {
        let request = GenerationRequest {
            system_prompt: "system".into(),
            user_prompt: "user".into(),
            target: GenerationCell {
                label: "billing".into(),
                dimensions: BTreeMap::new(),
            },
            requested_count: 2,
            parameters: GenerationParameters::default(),
            construction: None,
        };
        let mut succeeded =
            GenerationAttempt::start(Uuid::new_v4(), 1, 0, &request).expect("attempt");
        succeeded
            .succeed(2, 2, 1, 1, None, serde_json::json!({}), vec![])
            .expect("success");
        assert_eq!(succeeded.state, GenerationAttemptState::Succeeded);
        assert_eq!(
            succeeded
                .reproduce_outcome_fingerprint()
                .expect("fingerprint"),
            succeeded.outcome_fingerprint.expect("stored fingerprint")
        );

        let mut failed = GenerationAttempt::start(Uuid::new_v4(), 1, 0, &request).expect("attempt");
        failed
            .fail(
                GenerationAttemptFailureKind::Request,
                "temporary failure",
                true,
            )
            .expect("failure");
        assert_eq!(failed.state, GenerationAttemptState::Failed);
        assert_eq!(failed.retryable, Some(true));
    }
}
