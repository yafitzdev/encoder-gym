//! Immutable contract authorizing one finite supervised generation run.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use dataset_quality_core::{
    assessment::{EvaluatorIdentity, GeneratorEvaluatorRelationship},
    policy::BasisPoints,
};
use generation_core::jobs::GenerationBackendIdentity;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{SupervisorError, fingerprint, required};

pub const GENERATION_QUALITY_CONTRACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBinding {
    pub id: Uuid,
    pub fingerprint: String,
}

impl ArtifactBinding {
    pub fn new(id: Uuid, fingerprint: impl Into<String>) -> Result<Self, SupervisorError> {
        if id.is_nil() {
            return Err(SupervisorError::Validation(
                "artifact binding id must not be nil".into(),
            ));
        }
        Ok(Self {
            id,
            fingerprint: required(fingerprint, "artifact binding fingerprint")?,
        })
    }

    pub fn validate(&self, field: &str) -> Result<(), SupervisorError> {
        if self.id.is_nil() || required(self.fingerprint.clone(), field)? != self.fingerprint {
            return Err(SupervisorError::Validation(format!(
                "{field} must contain a non-nil id and normalized fingerprint"
            )));
        }
        Ok(())
    }
}

/// Pins the coverage from which the finite run starts. New segments always
/// target absolute remaining coverage rather than adding an inferred delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCoverageBinding {
    pub accepted_rows: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorIdentity {
    pub backend: GenerationBackendIdentity,
    pub protocol_version: String,
    pub configuration_fingerprint: String,
    pub fingerprint: String,
}

impl GeneratorIdentity {
    pub fn create(
        backend: GenerationBackendIdentity,
        protocol_version: impl Into<String>,
        configuration_fingerprint: impl Into<String>,
    ) -> Result<Self, SupervisorError> {
        let mut value = Self {
            backend,
            protocol_version: required(protocol_version, "generator protocol version")?,
            configuration_fingerprint: required(
                configuration_fingerprint,
                "generator configuration fingerprint",
            )?,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    fn validate_fields(&self) -> Result<(), SupervisorError> {
        required(self.backend.name.clone(), "generator backend")?;
        required(self.backend.model.clone(), "generator model")?;
        required(self.protocol_version.clone(), "generator protocol version")?;
        required(
            self.configuration_fingerprint.clone(),
            "generator configuration fingerprint",
        )?;
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        self.validate_fields()?;
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(SupervisorError::Integrity(
                "generator identity fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowQualityThresholds {
    pub minimum_assigned_label_score: BasisPoints,
    pub minimum_label_margin: BasisPoints,
    pub minimum_dimension_score: BasisPoints,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_difficulty_score: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_authenticity_score: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_strategy_score: Option<BasisPoints>,
    pub maximum_label_leakage_risk: BasisPoints,
    pub maximum_shortcut_risk: BasisPoints,
    pub minimum_evaluator_confidence: BasisPoints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchQualityThresholds {
    pub minimum_qualified_rate: BasisPoints,
    pub maximum_borderline_rate: BasisPoints,
    pub maximum_quarantined_rate: BasisPoints,
    pub maximum_invalid_rate: BasisPoints,
    pub maximum_normalized_duplicate_rate: BasisPoints,
    pub maximum_template_repetition_rate: BasisPoints,
    pub maximum_qualified_rate_drop: BasisPoints,
    pub maximum_shortcut_concentration: BasisPoints,
    #[serde(default)]
    pub required_patterns: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselinePolicy {
    FirstQualifiedWindow,
    InitialCanary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringScope {
    Cell,
    CellAndStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitoringPolicy {
    pub initial_canary_rows_per_scope: u32,
    pub revision_canary_rows_per_scope: u32,
    pub rolling_window_rows_per_scope: u32,
    pub minimum_evidence_rows_per_scope: u32,
    pub baseline_minimum_rows_per_scope: u32,
    pub scope: MonitoringScope,
    pub baseline_policy: BaselinePolicy,
    /// Number of independently failing scopes required for a global pause.
    /// Zero disables inferred global pauses.
    pub systemic_pause_minimum_scopes: u32,
}

impl MonitoringPolicy {
    fn validate(&self) -> Result<(), SupervisorError> {
        let positive = [
            ("initial canary rows", self.initial_canary_rows_per_scope),
            ("revision canary rows", self.revision_canary_rows_per_scope),
            ("rolling window rows", self.rolling_window_rows_per_scope),
            (
                "minimum evidence rows",
                self.minimum_evidence_rows_per_scope,
            ),
            (
                "baseline minimum rows",
                self.baseline_minimum_rows_per_scope,
            ),
        ];
        if let Some((name, _)) = positive.into_iter().find(|(_, value)| *value == 0) {
            return Err(SupervisorError::Validation(format!(
                "{name} must be positive"
            )));
        }
        if self.minimum_evidence_rows_per_scope > self.initial_canary_rows_per_scope
            || self.minimum_evidence_rows_per_scope > self.rolling_window_rows_per_scope
            || self.minimum_evidence_rows_per_scope > self.revision_canary_rows_per_scope
        {
            return Err(SupervisorError::Validation(
                "every observation window must be large enough to collect minimum evidence".into(),
            ));
        }
        if self.baseline_minimum_rows_per_scope > self.initial_canary_rows_per_scope {
            return Err(SupervisorError::Validation(
                "initial canary must be large enough to establish its baseline".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorBudgets {
    pub maximum_generation_segments: u32,
    pub maximum_generated_rows: u64,
    pub maximum_quality_audits: u32,
    pub maximum_evaluator_requests: u32,
    pub maximum_evaluator_attempts: u32,
    pub maximum_prompt_revisions: u32,
    pub maximum_revision_canaries: u32,
    pub maximum_pi_model_turns: u32,
    pub maximum_pi_tool_calls: u32,
    pub maximum_pi_input_tokens: u64,
    pub maximum_pi_output_tokens: u64,
    pub maximum_retries_per_external_call: u32,
    pub maximum_duration_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microunits: Option<u64>,
}

impl SupervisorBudgets {
    fn validate(&self, monitoring: &MonitoringPolicy) -> Result<(), SupervisorError> {
        let positive_u32 = [
            self.maximum_generation_segments,
            self.maximum_quality_audits,
            self.maximum_evaluator_requests,
            self.maximum_evaluator_attempts,
            self.maximum_revision_canaries,
            self.maximum_pi_model_turns,
            self.maximum_pi_tool_calls,
        ];
        if positive_u32.contains(&0)
            || self.maximum_generated_rows == 0
            || self.maximum_duration_seconds == 0
        {
            return Err(SupervisorError::Validation(
                "finite supervisor execution budgets must be positive".into(),
            ));
        }
        if self.maximum_evaluator_attempts < self.maximum_evaluator_requests {
            return Err(SupervisorError::Validation(
                "evaluator attempt budget cannot be lower than request budget".into(),
            ));
        }
        if self.maximum_generated_rows < u64::from(monitoring.initial_canary_rows_per_scope) {
            return Err(SupervisorError::Validation(
                "generated-row budget cannot collect the initial canary".into(),
            ));
        }
        if self.maximum_prompt_revisions > self.maximum_revision_canaries {
            return Err(SupervisorError::Validation(
                "every authorized prompt revision needs a canary budget".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectedPromptField {
    SystemPrompt,
    OutputSchema,
    TargetLabel,
    TargetDimensions,
    ConstructionGraph,
    SemanticAuthority,
    QualityThresholds,
    Budgets,
    SafetyInstructions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRevisionKind {
    ReplaceGenerationGuidance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptRevisionPolicy {
    pub maximum_instructions: u32,
    pub maximum_characters_per_instruction: u32,
    pub maximum_total_characters: u32,
    pub allowed_kind: PromptRevisionKind,
    pub protected_fields: BTreeSet<ProtectedPromptField>,
}

impl PromptRevisionPolicy {
    fn validate(&self) -> Result<(), SupervisorError> {
        if self.maximum_instructions == 0
            || self.maximum_characters_per_instruction == 0
            || self.maximum_total_characters == 0
        {
            return Err(SupervisorError::Validation(
                "prompt revision limits must be positive".into(),
            ));
        }
        let required = BTreeSet::from([
            ProtectedPromptField::SystemPrompt,
            ProtectedPromptField::OutputSchema,
            ProtectedPromptField::TargetLabel,
            ProtectedPromptField::TargetDimensions,
            ProtectedPromptField::ConstructionGraph,
            ProtectedPromptField::SemanticAuthority,
            ProtectedPromptField::QualityThresholds,
            ProtectedPromptField::Budgets,
            ProtectedPromptField::SafetyInstructions,
        ]);
        if !required.is_subset(&self.protected_fields) {
            return Err(SupervisorError::Validation(
                "all trusted prompt fields must be protected".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreauthorizationEnvelope {
    pub revision_kind: PromptRevisionKind,
    pub maximum_affected_scopes: u32,
    pub maximum_instructions: u32,
    pub maximum_total_characters: u32,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RevisionApprovalPolicy {
    ExplicitReview,
    FinitePreauthorization { envelope: PreauthorizationEnvelope },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationQualityContract {
    pub id: Uuid,
    pub schema_version: u32,
    pub dataset: ArtifactBinding,
    pub plan: ArtifactBinding,
    pub starting_coverage: AcceptedCoverageBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_context: Option<ArtifactBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity_context: Option<ArtifactBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub construction_context: Option<ArtifactBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_context: Option<ArtifactBinding>,
    pub generator: GeneratorIdentity,
    pub evaluator: EvaluatorIdentity,
    pub generator_evaluator_relationship: GeneratorEvaluatorRelationship,
    pub row_thresholds: RowQualityThresholds,
    pub batch_thresholds: BatchQualityThresholds,
    pub monitoring: MonitoringPolicy,
    pub budgets: SupervisorBudgets,
    pub approval_policy: RevisionApprovalPolicy,
    pub revision_policy: PromptRevisionPolicy,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationQualityContract {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        dataset: ArtifactBinding,
        plan: ArtifactBinding,
        starting_coverage: AcceptedCoverageBinding,
        semantic_context: Option<ArtifactBinding>,
        authenticity_context: Option<ArtifactBinding>,
        construction_context: Option<ArtifactBinding>,
        strategy_context: Option<ArtifactBinding>,
        generator: GeneratorIdentity,
        evaluator: EvaluatorIdentity,
        generator_evaluator_relationship: GeneratorEvaluatorRelationship,
        row_thresholds: RowQualityThresholds,
        batch_thresholds: BatchQualityThresholds,
        monitoring: MonitoringPolicy,
        budgets: SupervisorBudgets,
        approval_policy: RevisionApprovalPolicy,
        revision_policy: PromptRevisionPolicy,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        let mut value = Self {
            id,
            schema_version: GENERATION_QUALITY_CONTRACT_SCHEMA_VERSION,
            dataset,
            plan,
            starting_coverage,
            semantic_context,
            authenticity_context,
            construction_context,
            strategy_context,
            generator,
            evaluator,
            generator_evaluator_relationship,
            row_thresholds,
            batch_thresholds,
            monitoring,
            budgets,
            approval_policy,
            revision_policy,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    fn validate_fields(&self) -> Result<(), SupervisorError> {
        if self.id.is_nil() || self.schema_version != GENERATION_QUALITY_CONTRACT_SCHEMA_VERSION {
            return Err(SupervisorError::Validation(
                "contract identity or schema version is invalid".into(),
            ));
        }
        self.dataset.validate("dataset fingerprint")?;
        self.plan.validate("plan fingerprint")?;
        required(
            self.starting_coverage.fingerprint.clone(),
            "starting coverage fingerprint",
        )?;
        for (name, binding) in [
            ("semantic context fingerprint", &self.semantic_context),
            (
                "authenticity context fingerprint",
                &self.authenticity_context,
            ),
            (
                "construction context fingerprint",
                &self.construction_context,
            ),
            ("strategy context fingerprint", &self.strategy_context),
        ] {
            if let Some(binding) = binding {
                binding.validate(name)?;
            }
        }
        self.generator.validate()?;
        self.evaluator
            .validate()
            .map_err(|error| SupervisorError::Integrity(error.to_string()))?;
        self.validate_relationship()?;
        if self.row_thresholds.minimum_authenticity_score.is_some()
            && self.authenticity_context.is_none()
        {
            return Err(SupervisorError::Validation(
                "authenticity threshold requires pinned authenticity context".into(),
            ));
        }
        if self.row_thresholds.minimum_strategy_score.is_some() && self.strategy_context.is_none() {
            return Err(SupervisorError::Validation(
                "strategy threshold requires pinned strategy context".into(),
            ));
        }
        self.monitoring.validate()?;
        self.budgets.validate(&self.monitoring)?;
        self.revision_policy.validate()?;
        if let RevisionApprovalPolicy::FinitePreauthorization { envelope } = &self.approval_policy {
            if envelope.maximum_affected_scopes == 0
                || envelope.maximum_instructions == 0
                || envelope.maximum_total_characters == 0
                || envelope.maximum_instructions > self.revision_policy.maximum_instructions
                || envelope.maximum_total_characters > self.revision_policy.maximum_total_characters
                || envelope.revision_kind != self.revision_policy.allowed_kind
                || envelope.expires_at <= self.created_at
            {
                return Err(SupervisorError::Validation(
                    "finite preauthorization exceeds prompt revision policy or is expired".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_relationship(&self) -> Result<(), SupervisorError> {
        let same_backend = self
            .generator
            .backend
            .name
            .eq_ignore_ascii_case(&self.evaluator.backend);
        let same_model = self
            .generator
            .backend
            .model
            .eq_ignore_ascii_case(&self.evaluator.model);
        let actual = if same_backend && same_model {
            GeneratorEvaluatorRelationship::SharedBackendAndModel
        } else if same_backend {
            GeneratorEvaluatorRelationship::SharedBackend
        } else {
            GeneratorEvaluatorRelationship::IndependentBackend
        };
        if self.generator_evaluator_relationship != actual {
            return Err(SupervisorError::Validation(format!(
                "declared generator/evaluator relationship does not match identities; expected {actual:?}"
            )));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        self.validate_fields()?;
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(SupervisorError::Integrity(
                "generation quality contract fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use dataset_quality_core::assessment::{EvaluatorExecutionLocation, EvaluatorIndependence};

    use super::*;

    fn bp(value: u16) -> BasisPoints {
        BasisPoints::new(value).unwrap()
    }

    fn fixture() -> GenerationQualityContract {
        let now = Utc.with_ymd_and_hms(2026, 8, 31, 10, 0, 0).unwrap();
        let generator = GeneratorIdentity::create(
            GenerationBackendIdentity {
                name: "openai-compatible".into(),
                model: "generator-v1".into(),
                endpoint: Some("https://example.invalid/v1".into()),
            },
            "generation-protocol-v1",
            "generator-config-fingerprint",
        )
        .unwrap();
        let evaluator = EvaluatorIdentity::new(
            "independent-quality",
            "judge-v1",
            "quality-protocol-v1",
            "evaluator-config-fingerprint",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::ExternalService,
        )
        .unwrap();
        GenerationQualityContract::create(
            Uuid::new_v4(),
            ArtifactBinding::new(Uuid::new_v4(), "dataset-fp").unwrap(),
            ArtifactBinding::new(Uuid::new_v4(), "plan-fp").unwrap(),
            AcceptedCoverageBinding {
                accepted_rows: 20,
                fingerprint: "coverage-fp".into(),
            },
            Some(ArtifactBinding::new(Uuid::new_v4(), "semantic-fp").unwrap()),
            Some(ArtifactBinding::new(Uuid::new_v4(), "authenticity-fp").unwrap()),
            None,
            Some(ArtifactBinding::new(Uuid::new_v4(), "strategy-fp").unwrap()),
            generator,
            evaluator,
            GeneratorEvaluatorRelationship::IndependentBackend,
            RowQualityThresholds {
                minimum_assigned_label_score: bp(7_500),
                minimum_label_margin: bp(1_000),
                minimum_dimension_score: bp(7_000),
                minimum_difficulty_score: None,
                minimum_authenticity_score: Some(bp(7_000)),
                minimum_strategy_score: Some(bp(6_500)),
                maximum_label_leakage_risk: bp(1_500),
                maximum_shortcut_risk: bp(2_000),
                minimum_evaluator_confidence: bp(7_000),
            },
            BatchQualityThresholds {
                minimum_qualified_rate: bp(8_000),
                maximum_borderline_rate: bp(1_500),
                maximum_quarantined_rate: bp(1_000),
                maximum_invalid_rate: bp(500),
                maximum_normalized_duplicate_rate: bp(500),
                maximum_template_repetition_rate: bp(1_000),
                maximum_qualified_rate_drop: bp(1_000),
                maximum_shortcut_concentration: bp(1_500),
                required_patterns: BTreeSet::from(["boundary".into()]),
            },
            MonitoringPolicy {
                initial_canary_rows_per_scope: 20,
                revision_canary_rows_per_scope: 20,
                rolling_window_rows_per_scope: 50,
                minimum_evidence_rows_per_scope: 10,
                baseline_minimum_rows_per_scope: 15,
                scope: MonitoringScope::CellAndStrategy,
                baseline_policy: BaselinePolicy::InitialCanary,
                systemic_pause_minimum_scopes: 3,
            },
            SupervisorBudgets {
                maximum_generation_segments: 20,
                maximum_generated_rows: 10_000,
                maximum_quality_audits: 20,
                maximum_evaluator_requests: 500,
                maximum_evaluator_attempts: 600,
                maximum_prompt_revisions: 3,
                maximum_revision_canaries: 3,
                maximum_pi_model_turns: 9,
                maximum_pi_tool_calls: 24,
                maximum_pi_input_tokens: 100_000,
                maximum_pi_output_tokens: 20_000,
                maximum_retries_per_external_call: 2,
                maximum_duration_seconds: 3_600,
                maximum_cost_microunits: Some(5_000_000),
            },
            RevisionApprovalPolicy::FinitePreauthorization {
                envelope: PreauthorizationEnvelope {
                    revision_kind: PromptRevisionKind::ReplaceGenerationGuidance,
                    maximum_affected_scopes: 2,
                    maximum_instructions: 4,
                    maximum_total_characters: 1_000,
                    expires_at: now + Duration::hours(1),
                },
            },
            PromptRevisionPolicy {
                maximum_instructions: 8,
                maximum_characters_per_instruction: 500,
                maximum_total_characters: 2_000,
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
            },
            now,
        )
        .unwrap()
    }

    #[test]
    fn contract_reproduces_and_detects_tampering() {
        let contract = fixture();
        contract.validate().unwrap();
        let mut tampered = contract;
        tampered.budgets.maximum_generated_rows += 1;
        assert!(matches!(
            tampered.validate(),
            Err(SupervisorError::Integrity(_))
        ));
    }

    #[test]
    fn relationship_is_derived_from_pinned_identities() {
        let mut contract = fixture();
        contract.evaluator.backend = contract.generator.backend.name.clone();
        contract.evaluator.fingerprint = contract.evaluator.reproduce_fingerprint().unwrap();
        contract.fingerprint = contract.reproduce_fingerprint().unwrap();
        assert!(matches!(
            contract.validate(),
            Err(SupervisorError::Validation(message)) if message.contains("relationship")
        ));
    }

    #[test]
    fn authenticity_threshold_requires_context() {
        let mut contract = fixture();
        contract.authenticity_context = None;
        contract.fingerprint = contract.reproduce_fingerprint().unwrap();
        assert!(matches!(
            contract.validate(),
            Err(SupervisorError::Validation(message)) if message.contains("authenticity")
        ));
    }
}
