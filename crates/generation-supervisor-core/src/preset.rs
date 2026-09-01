//! Outcome-oriented authoring controls for governed generation supervision.
//!
//! Running supervisors never interpret these presets. The compiler resolves
//! them into explicit thresholds, budgets, prompt protection, and non-secret
//! provider profiles before a workflow definition is persisted.

use std::collections::{BTreeMap, BTreeSet};

use dataset_quality_core::policy::{
    AuditBudgets, AuditMode, BasisPoints, BorderlineReviewPolicy, EvaluatorEgressPolicy,
    InvalidEvaluatorOutputPolicy, QualityPolicy, QualityPolicyPresetControls, QualityPreset,
};
use generation_core::domain::GenerationParameters;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    SupervisorError,
    contract::{
        BaselinePolicy, BatchQualityThresholds, MonitoringPolicy, MonitoringScope,
        PromptRevisionKind, PromptRevisionPolicy, ProtectedPromptField, RowQualityThresholds,
        SupervisorBudgets,
    },
    fingerprint, required,
};

pub const SUPERVISION_BLUEPRINT_SCHEMA_VERSION: u32 = 1;
const FAKE_GENERATOR_BACKEND: &str = "supervised-fake";
const FAKE_GENERATOR_MODEL: &str = "repairable-deterministic-v1";
const FAKE_EVALUATOR_BACKEND: &str = "deterministic-fake";
const FAKE_EVALUATOR_MODEL: &str = "blind-lexical-v1";
const DEFAULT_EVALUATOR_PROTOCOL: &str = "quality-evaluator-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisionQualityLevel {
    Economical,
    Balanced,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityImportance {
    Off,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisionProviderBackend {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneratorProfileRequest {
    pub backend: SupervisionProviderBackend,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
    pub extra: BTreeMap<String, Value>,
    pub batch_size: u32,
    pub max_retries: u32,
    pub max_attempt_multiplier: u32,
}

impl Default for GeneratorProfileRequest {
    fn default() -> Self {
        Self {
            backend: SupervisionProviderBackend::Fake,
            base_url: None,
            model: None,
            api_key_env: None,
            temperature: None,
            max_tokens: None,
            seed: Some(42),
            extra: BTreeMap::new(),
            batch_size: 20,
            max_retries: 2,
            max_attempt_multiplier: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluatorProfileRequest {
    pub backend: SupervisionProviderBackend,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub protocol_version: Option<String>,
    pub temperature_thousandths: u16,
    pub seed: i64,
    pub timeout_millis: u64,
    pub maximum_output_tokens_per_request: u64,
    pub maximum_response_bytes: usize,
    pub maximum_content_bytes: usize,
}

impl Default for EvaluatorProfileRequest {
    fn default() -> Self {
        Self {
            backend: SupervisionProviderBackend::Fake,
            base_url: None,
            model: None,
            api_key_env: None,
            protocol_version: None,
            temperature_thousandths: 0,
            seed: 42,
            timeout_millis: 120_000,
            maximum_output_tokens_per_request: 16_384,
            maximum_response_bytes: 2 * 1024 * 1024,
            maximum_content_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisionLimits {
    pub maximum_generated_rows: u64,
    pub maximum_evaluator_requests: u32,
    pub maximum_evaluator_tokens: u64,
    pub maximum_pi_tokens: u64,
    pub maximum_duration_seconds: u64,
    pub maximum_retries_per_external_call: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microunits: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairApprovalRequest {
    Manual,
    Preauthorized {
        maximum_affected_scopes: u32,
        valid_for_seconds: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSupervisionRequest {
    pub quality_level: SupervisionQualityLevel,
    pub authenticity_importance: QualityImportance,
    pub diversity_importance: QualityImportance,
    pub maximum_prompt_repairs: u32,
    pub monitoring_rows_per_scope: u32,
    pub limits: SupervisionLimits,
    pub repair_approval: RepairApprovalRequest,
    #[serde(default)]
    pub generator: GeneratorProfileRequest,
    #[serde(default)]
    pub evaluator: EvaluatorProfileRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedGeneratorProfile {
    pub backend: String,
    pub endpoint: Option<String>,
    pub model: String,
    pub api_key_env: Option<String>,
    pub parameters: GenerationParameters,
    pub batch_size: u32,
    pub max_retries: u32,
    pub max_attempt_multiplier: u32,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedEvaluatorProfile {
    pub backend: String,
    pub endpoint: Option<String>,
    pub model: String,
    pub api_key_env: Option<String>,
    pub protocol_version: String,
    pub temperature_thousandths: u16,
    pub seed: i64,
    pub timeout_millis: u64,
    pub maximum_output_tokens_per_request: u64,
    pub maximum_response_bytes: usize,
    pub maximum_content_bytes: usize,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolvedRepairApproval {
    Manual,
    Preauthorized {
        maximum_affected_scopes: u32,
        valid_for_seconds: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedGenerationSupervision {
    pub schema_version: u32,
    pub quality_level: SupervisionQualityLevel,
    pub authenticity_importance: QualityImportance,
    pub diversity_importance: QualityImportance,
    pub desired_monitoring_rows_per_scope: u32,
    pub quality_policy: QualityPolicy,
    pub row_thresholds: RowQualityThresholds,
    pub batch_thresholds: BatchQualityThresholds,
    pub budgets: SupervisorBudgets,
    pub repair_approval: ResolvedRepairApproval,
    pub revision_policy: PromptRevisionPolicy,
    pub generator: ResolvedGeneratorProfile,
    pub evaluator: ResolvedEvaluatorProfile,
    pub fingerprint: String,
}

impl GenerationSupervisionRequest {
    pub fn compile(self) -> Result<ResolvedGenerationSupervision, SupervisorError> {
        validate_request(&self)?;
        let generator = resolve_generator(self.generator)?;
        let evaluator = resolve_evaluator(self.evaluator)?;
        if (generator.backend == "openai-compatible" || evaluator.backend == "openai-compatible")
            && self.limits.maximum_cost_microunits.is_some()
        {
            return Err(SupervisorError::Validation(
                "external supervision cannot enforce a cost ceiling until provider pricing is pinned; use token/request ceilings"
                    .into(),
            ));
        }

        let evaluate_authenticity = self.authenticity_importance != QualityImportance::Off;
        let evaluator_egress = if evaluator.backend == FAKE_EVALUATOR_BACKEND {
            EvaluatorEgressPolicy::LocalOnly
        } else {
            EvaluatorEgressPolicy::ExternalCandidateText
        };
        let preset = match self.quality_level {
            SupervisionQualityLevel::Economical => QualityPreset::Fast,
            SupervisionQualityLevel::Balanced => QualityPreset::Balanced,
            SupervisionQualityLevel::Strict => QualityPreset::Strict,
        };
        let preset_policy = preset
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: evaluator_egress,
                evaluate_authenticity,
                maximum_cost_microusd: self.limits.maximum_cost_microunits,
            })
            .map_err(|error| SupervisorError::Validation(error.to_string()))?;
        let mut thresholds = preset_policy.thresholds;
        if self.authenticity_importance == QualityImportance::High
            && let Some(value) = thresholds.minimum_authenticity_score
        {
            thresholds.minimum_authenticity_score = Some(bp(value.get().saturating_add(500))?);
        }
        // A generated segment is capped by the generator batch size. Compile
        // the corresponding maximum requests per audit, then split the
        // operator's global token and cost ceilings across the global request
        // count. Reserving several audits can therefore never multiply the
        // authority supplied by the operator.
        let evaluator_requests_per_audit = generator
            .batch_size
            .div_ceil(self.monitoring_rows_per_scope)
            .max(1);
        if evaluator_requests_per_audit > self.limits.maximum_evaluator_requests {
            return Err(SupervisorError::Validation(
                "the evaluator request ceiling cannot cover one maximum-size generation audit"
                    .into(),
            ));
        }
        let evaluator_tokens_per_audit = self
            .limits
            .maximum_evaluator_tokens
            .checked_div(u64::from(self.limits.maximum_evaluator_requests))
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                SupervisorError::Validation(
                    "maximum evaluator tokens must cover every authorized evaluator request".into(),
                )
            })?;
        let evaluator_tokens_per_audit = evaluator_tokens_per_audit
            .checked_mul(u64::from(evaluator_requests_per_audit))
            .ok_or_else(|| {
                SupervisorError::Validation("per-audit evaluator token ceiling overflowed".into())
            })?;
        let evaluator_cost_per_audit = self.limits.maximum_cost_microunits.map(|maximum| {
            maximum / u64::from(self.limits.maximum_evaluator_requests)
                * u64::from(evaluator_requests_per_audit)
        });
        let audit_budgets = AuditBudgets {
            maximum_rows_per_batch: self.monitoring_rows_per_scope,
            maximum_evaluator_requests: evaluator_requests_per_audit,
            maximum_attempts_per_request: self
                .limits
                .maximum_retries_per_external_call
                .checked_add(1)
                .ok_or_else(|| {
                    SupervisorError::Validation("evaluator retry ceiling overflowed".into())
                })?,
            maximum_input_tokens: evaluator_tokens_per_audit,
            maximum_output_tokens: evaluator_tokens_per_audit,
            maximum_total_tokens: evaluator_tokens_per_audit,
            maximum_cost_microusd: evaluator_cost_per_audit,
        };
        let quality_policy = QualityPolicy::new(
            Some(preset),
            thresholds,
            match self.quality_level {
                SupervisionQualityLevel::Strict => InvalidEvaluatorOutputPolicy::FailAudit,
                _ => InvalidEvaluatorOutputPolicy::Quarantine,
            },
            BorderlineReviewPolicy::None,
            audit_budgets,
            evaluator_egress,
            AuditMode::FullPopulation,
        )
        .map_err(|error| SupervisorError::Validation(error.to_string()))?;
        let row_thresholds = RowQualityThresholds {
            minimum_assigned_label_score: quality_policy.thresholds.minimum_assigned_label_score,
            minimum_label_margin: quality_policy.thresholds.minimum_label_margin,
            minimum_dimension_score: quality_policy.thresholds.minimum_dimension_adherence_score,
            minimum_difficulty_score: None,
            minimum_authenticity_score: quality_policy.thresholds.minimum_authenticity_score,
            minimum_strategy_score: None,
            maximum_label_leakage_risk: quality_policy.thresholds.maximum_label_leakage_risk,
            maximum_shortcut_risk: quality_policy.thresholds.maximum_shortcut_risk,
            minimum_evaluator_confidence: quality_policy.thresholds.minimum_evaluator_confidence,
        };
        let batch_thresholds = batch_thresholds(self.quality_level, self.diversity_importance)?;
        let maximum_generation_segments = u32::try_from(self.limits.maximum_generated_rows)
            .map_err(|_| {
                SupervisorError::Validation(
                    "generated-row ceiling is too large for finite segment accounting".into(),
                )
            })?
            .checked_add(self.maximum_prompt_repairs)
            .ok_or_else(|| {
                SupervisorError::Validation("generation segment ceiling overflowed".into())
            })?;
        let evaluator_attempts = self
            .limits
            .maximum_evaluator_requests
            .checked_mul(
                self.limits
                    .maximum_retries_per_external_call
                    .checked_add(1)
                    .ok_or_else(|| {
                        SupervisorError::Validation("evaluator retry ceiling overflowed".into())
                    })?,
            )
            .ok_or_else(|| {
                SupervisorError::Validation("evaluator attempt ceiling overflowed".into())
            })?;
        let maximum_revision_canaries = self.maximum_prompt_repairs.max(1);
        let budgets = SupervisorBudgets {
            maximum_generation_segments: maximum_generation_segments.max(1),
            maximum_generated_rows: self.limits.maximum_generated_rows,
            maximum_quality_audits: maximum_generation_segments.max(1),
            maximum_evaluator_requests: self.limits.maximum_evaluator_requests,
            maximum_evaluator_attempts: evaluator_attempts,
            maximum_evaluator_input_tokens: self.limits.maximum_evaluator_tokens,
            maximum_evaluator_output_tokens: self.limits.maximum_evaluator_tokens,
            maximum_evaluator_total_tokens: self.limits.maximum_evaluator_tokens,
            maximum_prompt_revisions: self.maximum_prompt_repairs,
            maximum_revision_canaries,
            maximum_pi_model_turns: self.maximum_prompt_repairs.saturating_mul(3).max(1),
            maximum_pi_tool_calls: self.maximum_prompt_repairs.saturating_mul(8).max(1),
            maximum_pi_input_tokens: self.limits.maximum_pi_tokens,
            maximum_pi_output_tokens: self.limits.maximum_pi_tokens,
            maximum_retries_per_external_call: self.limits.maximum_retries_per_external_call,
            maximum_duration_seconds: self.limits.maximum_duration_seconds,
            maximum_cost_microunits: self.limits.maximum_cost_microunits,
        };
        let repair_approval = match self.repair_approval {
            RepairApprovalRequest::Manual => ResolvedRepairApproval::Manual,
            RepairApprovalRequest::Preauthorized {
                maximum_affected_scopes,
                valid_for_seconds,
            } => ResolvedRepairApproval::Preauthorized {
                maximum_affected_scopes,
                valid_for_seconds,
            },
        };
        let revision_policy = PromptRevisionPolicy {
            maximum_instructions: 4,
            maximum_characters_per_instruction: 300,
            maximum_total_characters: 800,
            allowed_kind: PromptRevisionKind::ReplaceGenerationGuidance,
            protected_fields: BTreeSet::from([
                ProtectedPromptField::SystemPrompt,
                ProtectedPromptField::OutputSchema,
                ProtectedPromptField::TargetLabel,
                ProtectedPromptField::TargetDimensions,
                ProtectedPromptField::ConstructionGraph,
                ProtectedPromptField::SemanticAuthority,
                ProtectedPromptField::QualityThresholds,
                ProtectedPromptField::Budgets,
                ProtectedPromptField::SafetyInstructions,
            ]),
        };
        let mut resolved = ResolvedGenerationSupervision {
            schema_version: SUPERVISION_BLUEPRINT_SCHEMA_VERSION,
            quality_level: self.quality_level,
            authenticity_importance: self.authenticity_importance,
            diversity_importance: self.diversity_importance,
            desired_monitoring_rows_per_scope: self.monitoring_rows_per_scope,
            quality_policy,
            row_thresholds,
            batch_thresholds,
            budgets,
            repair_approval,
            revision_policy,
            generator,
            evaluator,
            fingerprint: String::new(),
        };
        resolved.fingerprint = resolved.reproduce_fingerprint()?;
        resolved.validate()?;
        Ok(resolved)
    }
}

impl ResolvedGenerationSupervision {
    pub fn monitoring_for_minimum_scope(
        &self,
        minimum_rows_in_scope: u32,
    ) -> Result<MonitoringPolicy, SupervisorError> {
        self.validate()?;
        if minimum_rows_in_scope == 0 {
            return Err(SupervisorError::Validation(
                "supervised generation plan contains an empty scope".into(),
            ));
        }
        let window = self
            .desired_monitoring_rows_per_scope
            .min(minimum_rows_in_scope)
            .max(1);
        let minimum_evidence = window.div_ceil(2).max(1);
        Ok(MonitoringPolicy {
            initial_canary_rows_per_scope: window,
            revision_canary_rows_per_scope: window,
            rolling_window_rows_per_scope: window,
            minimum_evidence_rows_per_scope: minimum_evidence,
            baseline_minimum_rows_per_scope: minimum_evidence,
            scope: MonitoringScope::Cell,
            baseline_policy: BaselinePolicy::InitialCanary,
            systemic_pause_minimum_scopes: 2,
        })
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.schema_version != SUPERVISION_BLUEPRINT_SCHEMA_VERSION
            || self.desired_monitoring_rows_per_scope == 0
            || self.generator.fingerprint != profile_fingerprint(&self.generator)?
            || self.evaluator.fingerprint != profile_fingerprint(&self.evaluator)?
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "resolved generation-supervision blueprint does not reproduce".into(),
            ));
        }
        self.quality_policy
            .verify_integrity()
            .map_err(|error| SupervisorError::Integrity(error.to_string()))?;
        Ok(())
    }
}

fn validate_request(request: &GenerationSupervisionRequest) -> Result<(), SupervisorError> {
    let limits = &request.limits;
    if request.monitoring_rows_per_scope == 0
        || limits.maximum_generated_rows == 0
        || limits.maximum_evaluator_requests == 0
        || limits.maximum_evaluator_tokens == 0
        || limits.maximum_pi_tokens == 0
        || limits.maximum_duration_seconds == 0
    {
        return Err(SupervisorError::Validation(
            "supervision row, request, token, monitoring, and duration ceilings must be positive"
                .into(),
        ));
    }
    if request.generator.max_retries > limits.maximum_retries_per_external_call {
        return Err(SupervisorError::Validation(
            "generator retry policy exceeds the supervision retry ceiling".into(),
        ));
    }
    if let RepairApprovalRequest::Preauthorized {
        maximum_affected_scopes,
        valid_for_seconds,
    } = request.repair_approval
        && (request.maximum_prompt_repairs == 0
            || maximum_affected_scopes == 0
            || valid_for_seconds == 0)
    {
        return Err(SupervisorError::Validation(
            "prompt-repair preauthorization needs repairs, affected scopes, and a finite validity"
                .into(),
        ));
    }
    Ok(())
}

fn resolve_generator(
    request: GeneratorProfileRequest,
) -> Result<ResolvedGeneratorProfile, SupervisorError> {
    if request.batch_size == 0 || request.max_attempt_multiplier == 0 {
        return Err(SupervisorError::Validation(
            "generator batch size and attempt multiplier must be positive".into(),
        ));
    }
    let (backend, endpoint, model, api_key_env) = resolve_common_profile(
        request.backend,
        request.base_url,
        request.model,
        request.api_key_env,
        FAKE_GENERATOR_BACKEND,
        FAKE_GENERATOR_MODEL,
        "generator",
    )?;
    let mut value = ResolvedGeneratorProfile {
        backend,
        endpoint,
        model,
        api_key_env,
        parameters: GenerationParameters {
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            seed: request.seed,
            extra: request.extra,
        },
        batch_size: request.batch_size,
        max_retries: request.max_retries,
        max_attempt_multiplier: request.max_attempt_multiplier,
        fingerprint: String::new(),
    };
    value.fingerprint = profile_fingerprint(&value)?;
    Ok(value)
}

fn resolve_evaluator(
    request: EvaluatorProfileRequest,
) -> Result<ResolvedEvaluatorProfile, SupervisorError> {
    if request.temperature_thousandths > 2_000
        || request.timeout_millis == 0
        || request.maximum_output_tokens_per_request == 0
        || request.maximum_response_bytes < 1_024
        || request.maximum_content_bytes == 0
        || request.maximum_content_bytes > request.maximum_response_bytes
    {
        return Err(SupervisorError::Validation(
            "evaluator temperature, timeout, token, or response bounds are invalid".into(),
        ));
    }
    let (backend, endpoint, model, api_key_env) = resolve_common_profile(
        request.backend,
        request.base_url,
        request.model,
        request.api_key_env,
        FAKE_EVALUATOR_BACKEND,
        FAKE_EVALUATOR_MODEL,
        "evaluator",
    )?;
    if backend == FAKE_EVALUATOR_BACKEND && request.seed < 0 {
        return Err(SupervisorError::Validation(
            "fake evaluator seed must be non-negative".into(),
        ));
    }
    let protocol_version = request
        .protocol_version
        .unwrap_or_else(|| DEFAULT_EVALUATOR_PROTOCOL.to_owned());
    let mut value = ResolvedEvaluatorProfile {
        backend,
        endpoint,
        model,
        api_key_env,
        protocol_version: required(protocol_version, "evaluator protocol version")?,
        temperature_thousandths: request.temperature_thousandths,
        seed: request.seed,
        timeout_millis: request.timeout_millis,
        maximum_output_tokens_per_request: request.maximum_output_tokens_per_request,
        maximum_response_bytes: request.maximum_response_bytes,
        maximum_content_bytes: request.maximum_content_bytes,
        fingerprint: String::new(),
    };
    value.fingerprint = profile_fingerprint(&value)?;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn resolve_common_profile(
    kind: SupervisionProviderBackend,
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    fake_backend: &str,
    fake_model: &str,
    role: &str,
) -> Result<(String, Option<String>, String, Option<String>), SupervisorError> {
    match kind {
        SupervisionProviderBackend::Fake => {
            if base_url.is_some() || api_key_env.is_some() {
                return Err(SupervisorError::Validation(format!(
                    "fake {role} profile cannot contain an endpoint or credential selector"
                )));
            }
            if model.as_deref().is_some_and(|value| value != fake_model) {
                return Err(SupervisorError::Validation(format!(
                    "fake {role} model is fixed to {fake_model}"
                )));
            }
            Ok((fake_backend.into(), None, fake_model.into(), None))
        }
        SupervisionProviderBackend::OpenaiCompatible => {
            let endpoint = required(
                base_url.ok_or_else(|| {
                    SupervisorError::Validation(format!(
                        "OpenAI-compatible {role} profile requires base_url"
                    ))
                })?,
                &format!("{role} endpoint"),
            )?;
            let model = required(
                model.ok_or_else(|| {
                    SupervisorError::Validation(format!(
                        "OpenAI-compatible {role} profile requires model"
                    ))
                })?,
                &format!("{role} model"),
            )?;
            let api_key_env = api_key_env
                .map(|value| validate_api_key_env(value, role))
                .transpose()?;
            Ok((
                "openai-compatible".into(),
                Some(endpoint.trim_end_matches('/').to_owned()),
                model,
                api_key_env,
            ))
        }
    }
}

fn validate_api_key_env(value: String, role: &str) -> Result<String, SupervisorError> {
    let normalized = required(value, &format!("{role} API-key environment variable"))?;
    let valid = normalized.len() <= 128
        && normalized.chars().all(|character| {
            character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
        })
        && normalized
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_uppercase());
    if !valid {
        return Err(SupervisorError::Validation(format!(
            "{role} API-key selector must be an uppercase environment-variable name"
        )));
    }
    Ok(normalized)
}

fn batch_thresholds(
    level: SupervisionQualityLevel,
    diversity: QualityImportance,
) -> Result<BatchQualityThresholds, SupervisorError> {
    let (qualified, borderline, quarantine, invalid, duplicate, repetition, drop, shortcut): (
        u16,
        u16,
        u16,
        u16,
        u16,
        u16,
        u16,
        u16,
    ) = match level {
        SupervisionQualityLevel::Economical => {
            (7_000, 3_000, 2_500, 1_500, 1_000, 3_000, 2_000, 2_500)
        }
        SupervisionQualityLevel::Balanced => (8_000, 2_000, 2_000, 1_000, 500, 2_000, 1_000, 2_000),
        SupervisionQualityLevel::Strict => (9_000, 1_000, 1_000, 500, 0, 1_000, 500, 1_000),
    };
    let diversity_reduction = match diversity {
        QualityImportance::Off => 0,
        QualityImportance::Normal => 250,
        QualityImportance::High => 750,
    };
    Ok(BatchQualityThresholds {
        minimum_qualified_rate: bp(qualified)?,
        maximum_borderline_rate: bp(borderline)?,
        maximum_quarantined_rate: bp(quarantine)?,
        maximum_invalid_rate: bp(invalid)?,
        maximum_exact_duplicate_rate: BasisPoints::ZERO,
        maximum_normalized_duplicate_rate: bp(duplicate.saturating_sub(diversity_reduction))?,
        maximum_template_repetition_rate: bp(repetition.saturating_sub(diversity_reduction))?,
        maximum_qualified_rate_drop: bp(drop)?,
        maximum_shortcut_concentration: bp(shortcut)?,
        required_patterns: BTreeSet::new(),
    })
}

fn bp(value: u16) -> Result<BasisPoints, SupervisorError> {
    BasisPoints::new(value).map_err(|error| SupervisorError::Validation(error.to_string()))
}

fn profile_fingerprint<T: Serialize>(value: &T) -> Result<String, SupervisorError> {
    let mut json = serde_json::to_value(value)
        .map_err(|error| SupervisorError::Fingerprint(error.to_string()))?;
    if let Value::Object(object) = &mut json {
        object.remove("fingerprint");
    }
    fingerprint(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GenerationSupervisionRequest {
        GenerationSupervisionRequest {
            quality_level: SupervisionQualityLevel::Balanced,
            authenticity_importance: QualityImportance::Off,
            diversity_importance: QualityImportance::Normal,
            maximum_prompt_repairs: 2,
            monitoring_rows_per_scope: 20,
            limits: SupervisionLimits {
                maximum_generated_rows: 1_000,
                maximum_evaluator_requests: 100,
                maximum_evaluator_tokens: 100_000,
                maximum_pi_tokens: 20_000,
                maximum_duration_seconds: 3_600,
                maximum_retries_per_external_call: 2,
                maximum_cost_microunits: None,
            },
            repair_approval: RepairApprovalRequest::Manual,
            generator: GeneratorProfileRequest::default(),
            evaluator: EvaluatorProfileRequest::default(),
        }
    }

    #[test]
    fn outcome_controls_compile_to_explicit_reproducible_authority() {
        let resolved = request().compile().expect("resolved supervision");
        resolved.validate().expect("valid blueprint");
        assert_eq!(resolved.generator.backend, FAKE_GENERATOR_BACKEND);
        assert_eq!(resolved.evaluator.backend, FAKE_EVALUATOR_BACKEND);
        assert_eq!(resolved.budgets.maximum_prompt_revisions, 2);
        assert_eq!(
            resolved.quality_policy.preset,
            Some(QualityPreset::Balanced)
        );
        assert!(resolved.row_thresholds.minimum_authenticity_score.is_none());
    }

    #[test]
    fn small_plan_scopes_get_attainable_explicit_windows() {
        let resolved = request().compile().expect("resolved supervision");
        let monitoring = resolved
            .monitoring_for_minimum_scope(3)
            .expect("small monitoring policy");
        assert_eq!(monitoring.initial_canary_rows_per_scope, 3);
        assert_eq!(monitoring.minimum_evidence_rows_per_scope, 2);
    }

    #[test]
    fn high_importance_tightens_authenticity_and_diversity() {
        let mut high = request();
        high.authenticity_importance = QualityImportance::High;
        high.diversity_importance = QualityImportance::High;
        let high = high.compile().expect("high importance");
        let mut normal = request();
        normal.authenticity_importance = QualityImportance::Normal;
        normal.diversity_importance = QualityImportance::Normal;
        let normal = normal.compile().expect("normal importance");
        assert!(
            high.row_thresholds.minimum_authenticity_score
                > normal.row_thresholds.minimum_authenticity_score
        );
        assert!(
            high.batch_thresholds.maximum_template_repetition_rate
                < normal.batch_thresholds.maximum_template_repetition_rate
        );
    }

    #[test]
    fn separate_external_profiles_require_safe_non_secret_selectors() {
        let mut value = request();
        value.generator = GeneratorProfileRequest {
            backend: SupervisionProviderBackend::OpenaiCompatible,
            base_url: Some("https://generator.example/v1/".into()),
            model: Some("generator-model".into()),
            api_key_env: Some("GENERATOR_API_KEY".into()),
            ..GeneratorProfileRequest::default()
        };
        value.evaluator = EvaluatorProfileRequest {
            backend: SupervisionProviderBackend::OpenaiCompatible,
            base_url: Some("https://evaluator.example/v1".into()),
            model: Some("judge-model".into()),
            api_key_env: Some("EVALUATOR_API_KEY".into()),
            ..EvaluatorProfileRequest::default()
        };
        let resolved = value.compile().expect("separate profiles");
        assert_ne!(resolved.generator.endpoint, resolved.evaluator.endpoint);
        assert_ne!(resolved.generator.model, resolved.evaluator.model);

        let mut invalid = request();
        invalid.generator.backend = SupervisionProviderBackend::OpenaiCompatible;
        invalid.generator.base_url = Some("https://example.invalid/v1".into());
        invalid.generator.model = Some("model".into());
        invalid.generator.api_key_env = Some("sk-not-an-env-name".into());
        assert!(invalid.compile().is_err());
    }

    #[test]
    fn finite_preauthorization_requires_an_actual_repair_budget() {
        let mut value = request();
        value.maximum_prompt_repairs = 0;
        value.repair_approval = RepairApprovalRequest::Preauthorized {
            maximum_affected_scopes: 1,
            valid_for_seconds: 60,
        };
        assert!(value.compile().is_err());
    }
}
