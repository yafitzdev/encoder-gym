use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use encoder_experiment_core::domain::{EvidenceRole, ExternalProjectSnapshot, ParameterValue};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderRepairError, canonical_sha256,
    diagnosis::{ComparativeDiagnosis, RepairArtifactBinding, WeaknessKind},
    fingerprint, required,
};

pub const REPAIR_PROPOSAL_SCHEMA_VERSION: u32 = 1;
pub const REPAIR_PROPOSAL_REVIEW_SCHEMA_VERSION: u32 = 1;
pub const REPAIR_PROPOSAL_APPLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBinding {
    pub id: Uuid,
    pub fingerprint: String,
    pub source_revision: String,
    pub source_fingerprint: String,
}

impl ProjectBinding {
    pub fn from_project(project: &ExternalProjectSnapshot) -> Result<Self, EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        Ok(Self {
            id: project.id,
            fingerprint: project.fingerprint.clone(),
            source_revision: project.source_revision.clone(),
            source_fingerprint: project.source_fingerprint.clone(),
        })
    }

    pub fn verify(&self, project: &ExternalProjectSnapshot) -> Result<(), EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if *self != Self::from_project(project)? {
            return Err(EncoderRepairError::Integrity(
                "repair project binding is stale or foreign".into(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), EncoderRepairError> {
        if self.id.is_nil()
            || !canonical_sha256(&self.fingerprint)
            || self.source_revision.is_empty()
            || self.source_revision.trim() != self.source_revision
            || !canonical_sha256(&self.source_fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair project binding is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingInputBinding {
    pub key: String,
    pub bytes: u64,
    pub fingerprint: String,
}

impl TrainingInputBinding {
    fn from_project(project: &ExternalProjectSnapshot) -> Vec<Self> {
        project
            .inputs
            .iter()
            .filter(|input| input.role == EvidenceRole::Training)
            .map(|input| Self {
                key: input.key.clone(),
                bytes: input.bytes,
                fingerprint: input.fingerprint.clone(),
            })
            .collect()
    }

    fn validate(&self) -> Result<(), EncoderRepairError> {
        required(self.key.clone(), "training input key")?;
        if self.bytes == 0 || !canonical_sha256(&self.fingerprint) {
            return Err(EncoderRepairError::Validation(
                "repair training input binding is invalid".into(),
            ));
        }
        Ok(())
    }
}

/// Row-free binding to one active renewable benchmark generation.
///
/// Core does not depend on workflow orchestration. The application boundary
/// constructs this value from a deeply replayed generation journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairBenchmarkBinding {
    pub generation_id: Uuid,
    pub generation_fingerprint: String,
    pub journal_sequence: u32,
    pub journal_head_fingerprint: String,
    pub development_suite_fingerprints: BTreeMap<String, String>,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub valid_until: DateTime<Utc>,
    pub fingerprint: String,
}

impl RepairBenchmarkBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        generation_id: Uuid,
        generation_fingerprint: impl Into<String>,
        journal_sequence: u32,
        journal_head_fingerprint: impl Into<String>,
        development_suite_fingerprints: BTreeMap<String, String>,
        sealed_suite_id: Uuid,
        sealed_suite_fingerprint: impl Into<String>,
        valid_until: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        let mut value = Self {
            generation_id,
            generation_fingerprint: generation_fingerprint.into(),
            journal_sequence,
            journal_head_fingerprint: journal_head_fingerprint.into(),
            development_suite_fingerprints,
            sealed_suite_id,
            sealed_suite_fingerprint: sealed_suite_fingerprint.into(),
            valid_until,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "repair benchmark binding fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_at(&self, at: DateTime<Utc>) -> Result<(), EncoderRepairError> {
        self.validate_integrity()?;
        if at > self.valid_until {
            return Err(EncoderRepairError::Integrity(
                "repair benchmark authority expired".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if self.generation_id.is_nil()
            || self.journal_sequence == 0
            || self.sealed_suite_id.is_nil()
            || self.development_suite_fingerprints.is_empty()
            || [
                &self.generation_fingerprint,
                &self.journal_head_fingerprint,
                &self.sealed_suite_fingerprint,
            ]
            .iter()
            .any(|value| !canonical_sha256(value))
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair benchmark binding is incomplete".into(),
            ));
        }
        for (key, value) in &self.development_suite_fingerprints {
            required(key.clone(), "development suite key")?;
            if !canonical_sha256(value) {
                return Err(EncoderRepairError::Validation(
                    "development suite fingerprint is invalid".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairContext {
    pub diagnosis: RepairArtifactBinding,
    pub source_project: ProjectBinding,
    pub execution_project: ProjectBinding,
    pub source_campaign_id: Uuid,
    pub source_experiment_run_id: Uuid,
    pub base_training_inputs: Vec<TrainingInputBinding>,
    pub baseline_model_fingerprint: String,
    pub benchmark: RepairBenchmarkBinding,
    pub fingerprint: String,
}

impl RepairContext {
    pub fn create(
        diagnosis: &ComparativeDiagnosis,
        source_project: &ExternalProjectSnapshot,
        execution_project: &ExternalProjectSnapshot,
        benchmark: RepairBenchmarkBinding,
    ) -> Result<Self, EncoderRepairError> {
        diagnosis.validate_integrity()?;
        if diagnosis.project_snapshot_id != source_project.id
            || diagnosis.project_snapshot_fingerprint != source_project.fingerprint
        {
            return Err(EncoderRepairError::Validation(
                "repair diagnosis does not belong to the source project".into(),
            ));
        }
        let mut base_training_inputs = TrainingInputBinding::from_project(execution_project);
        base_training_inputs.sort_by(|left, right| left.key.cmp(&right.key));
        let mut value = Self {
            diagnosis: RepairArtifactBinding {
                id: diagnosis.id,
                fingerprint: diagnosis.fingerprint.clone(),
            },
            source_project: ProjectBinding::from_project(source_project)?,
            execution_project: ProjectBinding::from_project(execution_project)?,
            source_campaign_id: diagnosis.source_campaign_id,
            source_experiment_run_id: diagnosis.source_experiment_run_id,
            base_training_inputs,
            baseline_model_fingerprint: execution_project.baseline_model.fingerprint.clone(),
            benchmark,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn verify_against(
        &self,
        diagnosis: &ComparativeDiagnosis,
        source_project: &ExternalProjectSnapshot,
        execution_project: &ExternalProjectSnapshot,
        benchmark: &RepairBenchmarkBinding,
    ) -> Result<(), EncoderRepairError> {
        self.validate_integrity()?;
        let expected = Self::create(
            diagnosis,
            source_project,
            execution_project,
            benchmark.clone(),
        )?;
        if self.fingerprint != expected.fingerprint || *self != expected {
            return Err(EncoderRepairError::Integrity(
                "repair proposal context is stale".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "repair context fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        self.diagnosis.validate()?;
        self.source_project.validate()?;
        self.execution_project.validate()?;
        self.benchmark.validate_integrity()?;
        if self.source_campaign_id.is_nil()
            || self.source_experiment_run_id.is_nil()
            || self.base_training_inputs.is_empty()
            || !canonical_sha256(&self.baseline_model_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair context is incomplete".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        for input in &self.base_training_inputs {
            input.validate()?;
            if !keys.insert(input.key.as_str()) {
                return Err(EncoderRepairError::Validation(
                    "repair context repeats a training input".into(),
                ));
            }
        }
        if self
            .base_training_inputs
            .windows(2)
            .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(EncoderRepairError::Validation(
                "repair training inputs are not canonically ordered".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairActionKind {
    GenerateNativeRows,
    ImportNativeRows,
    ChangeTrainingConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairTarget {
    pub key: String,
    pub slice_fingerprint: String,
    pub slice: BTreeMap<String, String>,
    pub weakness_kind: WeaknessKind,
    pub suite_key: Option<String>,
    pub rationale: String,
    pub absolute_row_target: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAction {
    pub key: String,
    pub kind: RepairActionKind,
    pub target_keys: Vec<String>,
    pub parameters: BTreeMap<String, ParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRepairQualityPolicy {
    pub protocol_version: String,
    pub require_task_validation: bool,
    pub maximum_invalid_rows: u64,
    pub maximum_exact_duplicates: u64,
    pub maximum_normalized_duplicates: u64,
    pub maximum_source_contamination: u64,
    pub maximum_group_contamination: u64,
    pub maximum_lineage_contamination: u64,
    pub require_complete_assessment: bool,
    pub require_human_approval: bool,
    pub fingerprint: String,
}

impl NativeRepairQualityPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        protocol_version: impl Into<String>,
        require_task_validation: bool,
        maximum_invalid_rows: u64,
        maximum_exact_duplicates: u64,
        maximum_normalized_duplicates: u64,
        maximum_source_contamination: u64,
        maximum_group_contamination: u64,
        maximum_lineage_contamination: u64,
        require_complete_assessment: bool,
        require_human_approval: bool,
    ) -> Result<Self, EncoderRepairError> {
        let mut value = Self {
            protocol_version: required(protocol_version, "native quality protocol")?,
            require_task_validation,
            maximum_invalid_rows,
            maximum_exact_duplicates,
            maximum_normalized_duplicates,
            maximum_source_contamination,
            maximum_group_contamination,
            maximum_lineage_contamination,
            require_complete_assessment,
            require_human_approval,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "native repair quality policy changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        required(self.protocol_version.clone(), "native quality protocol")?;
        if !self.require_task_validation
            || !self.require_complete_assessment
            || !self.require_human_approval
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "native repair quality policy must require validation, complete evidence, and review"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairBudget {
    pub maximum_total_rows: u64,
    pub maximum_rows_per_target: u64,
    pub maximum_candidates: u32,
    pub maximum_training_seconds: u64,
    pub maximum_evaluation_seconds: u64,
    pub maximum_development_evaluations: u32,
    pub maximum_external_calls: u32,
    pub maximum_sealed_uses: u32,
}

impl RepairBudget {
    fn validate(&self) -> Result<(), EncoderRepairError> {
        if self.maximum_total_rows == 0
            || self.maximum_rows_per_target == 0
            || self.maximum_rows_per_target > self.maximum_total_rows
            || self.maximum_candidates == 0
            || self.maximum_training_seconds == 0
            || self.maximum_evaluation_seconds == 0
            || self.maximum_development_evaluations < self.maximum_candidates
            || self.maximum_sealed_uses > 1
        {
            return Err(EncoderRepairError::Validation(
                "repair budget must be positive, finite, and allow at most one sealed use".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateMechanism {
    GenuineRetraining,
    InterpolationControl,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairCandidateHypothesis {
    pub key: String,
    pub mechanism: CandidateMechanism,
    pub hypothesis: String,
    pub action_keys: Vec<String>,
    pub maximum_training_seconds: u64,
    pub parameters: BTreeMap<String, ParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposal {
    pub schema_version: u32,
    pub id: Uuid,
    /// Stable identity excluding record UUID and creation timestamp.
    pub specification_fingerprint: String,
    pub context: RepairContext,
    pub targets: Vec<RepairTarget>,
    pub actions: Vec<RepairAction>,
    pub quality_policy: NativeRepairQualityPolicy,
    pub budget: RepairBudget,
    pub candidates: Vec<RepairCandidateHypothesis>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RepairProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        diagnosis: &ComparativeDiagnosis,
        context: RepairContext,
        mut targets: Vec<RepairTarget>,
        mut actions: Vec<RepairAction>,
        quality_policy: NativeRepairQualityPolicy,
        budget: RepairBudget,
        mut candidates: Vec<RepairCandidateHypothesis>,
        expires_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        context.validate_integrity()?;
        if context.diagnosis.id != diagnosis.id
            || context.diagnosis.fingerprint != diagnosis.fingerprint
        {
            return Err(EncoderRepairError::Validation(
                "repair proposal context does not bind the diagnosis".into(),
            ));
        }
        targets.sort_by(|left, right| left.key.cmp(&right.key));
        actions.sort_by(|left, right| left.key.cmp(&right.key));
        candidates.sort_by(|left, right| left.key.cmp(&right.key));
        let mut value = Self {
            schema_version: REPAIR_PROPOSAL_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            specification_fingerprint: String::new(),
            context,
            targets,
            actions,
            quality_policy,
            budget,
            candidates,
            expires_at,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields(diagnosis)?;
        value.specification_fingerprint = value.reproduce_specification_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_against(
        &self,
        diagnosis: &ComparativeDiagnosis,
        context: &RepairContext,
        at: DateTime<Utc>,
    ) -> Result<(), EncoderRepairError> {
        self.validate_fields(diagnosis)?;
        if self.context != *context
            || self.reproduce_specification_fingerprint()? != self.specification_fingerprint
            || self.reproduce_fingerprint()? != self.fingerprint
            || at > self.expires_at
        {
            return Err(EncoderRepairError::Integrity(
                "repair proposal is stale or failed integrity".into(),
            ));
        }
        self.context.benchmark.validate_at(at)
    }

    pub fn validate_integrity(
        &self,
        diagnosis: &ComparativeDiagnosis,
    ) -> Result<(), EncoderRepairError> {
        self.validate_fields(diagnosis)?;
        if self.reproduce_specification_fingerprint()? != self.specification_fingerprint
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "repair proposal fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "context": self.context,
            "targets": self.targets,
            "actions": self.actions,
            "quality_policy": self.quality_policy,
            "budget": self.budget,
            "candidates": self.candidates,
            "expires_at": self.expires_at,
        }))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "specification_fingerprint": self.specification_fingerprint,
            "context": self.context,
            "targets": self.targets,
            "actions": self.actions,
            "quality_policy": self.quality_policy,
            "budget": self.budget,
            "candidates": self.candidates,
            "expires_at": self.expires_at,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self, diagnosis: &ComparativeDiagnosis) -> Result<(), EncoderRepairError> {
        diagnosis.validate_integrity()?;
        self.context.validate_integrity()?;
        self.quality_policy.validate_integrity()?;
        self.budget.validate()?;
        if self.schema_version != REPAIR_PROPOSAL_SCHEMA_VERSION
            || self.id.is_nil()
            || !self.specification_fingerprint.is_empty()
                && !canonical_sha256(&self.specification_fingerprint)
            || self.targets.is_empty()
            || self.actions.is_empty()
            || self.candidates.is_empty()
            || self.created_at >= self.expires_at
            || self.expires_at > self.context.benchmark.valid_until
            || self.candidates.len() > self.budget.maximum_candidates as usize
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair proposal is incomplete or exceeds its authority".into(),
            ));
        }
        validate_targets(&self.targets, diagnosis, &self.budget)?;
        validate_actions(&self.actions, &self.targets)?;
        validate_candidates(&self.candidates, &self.actions, &self.budget)?;
        Ok(())
    }
}

fn validate_targets(
    targets: &[RepairTarget],
    diagnosis: &ComparativeDiagnosis,
    budget: &RepairBudget,
) -> Result<(), EncoderRepairError> {
    let known = diagnosis
        .weaknesses
        .iter()
        .map(|value| (value.slice_fingerprint.as_str(), value))
        .collect::<BTreeMap<_, _>>();
    let mut total = 0_u64;
    for target in targets {
        required(target.key.clone(), "repair target key")?;
        required(target.rationale.clone(), "repair target rationale")?;
        let weakness = known
            .get(target.slice_fingerprint.as_str())
            .ok_or_else(|| {
                EncoderRepairError::Validation(format!(
                    "repair target {} is not present in the diagnosis",
                    target.key
                ))
            })?;
        if target.slice != weakness.slice
            || target.weakness_kind != weakness.kind
            || target.suite_key != weakness.suite_key
            || target.absolute_row_target == 0
            || target.absolute_row_target > budget.maximum_rows_per_target
        {
            return Err(EncoderRepairError::Validation(format!(
                "repair target {} does not match its diagnosed weakness or budget",
                target.key
            )));
        }
        total = total
            .checked_add(target.absolute_row_target)
            .ok_or_else(|| EncoderRepairError::Validation("repair row target overflowed".into()))?;
    }
    if total > budget.maximum_total_rows
        || targets.windows(2).any(|pair| pair[0].key >= pair[1].key)
    {
        return Err(EncoderRepairError::Validation(
            "repair targets are duplicated, unordered, or over budget".into(),
        ));
    }
    Ok(())
}

fn validate_actions(
    actions: &[RepairAction],
    targets: &[RepairTarget],
) -> Result<(), EncoderRepairError> {
    let target_keys = targets
        .iter()
        .map(|value| value.key.as_str())
        .collect::<BTreeSet<_>>();
    for action in actions {
        required(action.key.clone(), "repair action key")?;
        if action.target_keys.is_empty()
            || action.parameters.is_empty()
            || action
                .target_keys
                .iter()
                .any(|key| !target_keys.contains(key.as_str()))
            || action.target_keys.windows(2).any(|pair| pair[0] >= pair[1])
            || action
                .parameters
                .keys()
                .any(|key| key.is_empty() || key.trim() != key)
        {
            return Err(EncoderRepairError::Validation(format!(
                "repair action {} is incomplete or references unknown targets",
                action.key
            )));
        }
    }
    if actions.windows(2).any(|pair| pair[0].key >= pair[1].key)
        || !actions.iter().any(|value| {
            matches!(
                value.kind,
                RepairActionKind::GenerateNativeRows | RepairActionKind::ImportNativeRows
            )
        })
        || !actions
            .iter()
            .any(|value| value.kind == RepairActionKind::ChangeTrainingConfiguration)
    {
        return Err(EncoderRepairError::Validation(
            "repair proposal requires canonical data and training actions".into(),
        ));
    }
    Ok(())
}

fn validate_candidates(
    candidates: &[RepairCandidateHypothesis],
    actions: &[RepairAction],
    budget: &RepairBudget,
) -> Result<(), EncoderRepairError> {
    let action_keys = actions
        .iter()
        .map(|value| value.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut training_seconds = 0_u64;
    for candidate in candidates {
        required(candidate.key.clone(), "repair candidate key")?;
        required(candidate.hypothesis.clone(), "repair candidate hypothesis")?;
        if candidate.action_keys.is_empty()
            || candidate.parameters.is_empty()
            || candidate.maximum_training_seconds == 0
            || candidate
                .action_keys
                .iter()
                .any(|key| !action_keys.contains(key.as_str()))
            || candidate
                .action_keys
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(EncoderRepairError::Validation(format!(
                "repair candidate {} is incomplete",
                candidate.key
            )));
        }
        training_seconds = training_seconds
            .checked_add(candidate.maximum_training_seconds)
            .ok_or_else(|| {
                EncoderRepairError::Validation("repair training time overflowed".into())
            })?;
    }
    if candidates.windows(2).any(|pair| pair[0].key >= pair[1].key)
        || training_seconds > budget.maximum_training_seconds
        || !candidates
            .iter()
            .any(|value| value.mechanism == CandidateMechanism::GenuineRetraining)
    {
        return Err(EncoderRepairError::Validation(
            "repair candidate set must be canonical, bounded, and genuinely retrained".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposalReview {
    pub schema_version: u32,
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub proposal_specification_fingerprint: String,
    pub predecessor: Option<RepairArtifactBinding>,
    pub decision: RepairReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RepairProposalReview {
    pub fn create(
        proposal: &RepairProposal,
        diagnosis: &ComparativeDiagnosis,
        predecessor: Option<&Self>,
        decision: RepairReviewDecision,
        reviewer: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        proposal.validate_integrity(diagnosis)?;
        if predecessor.is_some_and(|value| {
            value.proposal_id != proposal.id || value.proposal_fingerprint != proposal.fingerprint
        }) {
            return Err(EncoderRepairError::Validation(
                "repair review predecessor is foreign".into(),
            ));
        }
        let mut value = Self {
            schema_version: REPAIR_PROPOSAL_REVIEW_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            proposal_specification_fingerprint: proposal.specification_fingerprint.clone(),
            predecessor: predecessor.map(|value| RepairArtifactBinding {
                id: value.id,
                fingerprint: value.fingerprint.clone(),
            }),
            decision,
            reviewer: required(reviewer, "repair reviewer")?,
            reason: required(reason, "repair review reason")?,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields(proposal, predecessor)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_against(
        &self,
        proposal: &RepairProposal,
        predecessor: Option<&Self>,
    ) -> Result<(), EncoderRepairError> {
        self.validate_fields(proposal, predecessor)?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "repair proposal review fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn validate_fields(
        &self,
        proposal: &RepairProposal,
        predecessor: Option<&Self>,
    ) -> Result<(), EncoderRepairError> {
        if self.schema_version != REPAIR_PROPOSAL_REVIEW_SCHEMA_VERSION
            || self.id.is_nil()
            || self.proposal_id != proposal.id
            || self.proposal_fingerprint != proposal.fingerprint
            || self.proposal_specification_fingerprint != proposal.specification_fingerprint
            || self.predecessor.as_ref().map(|value| value.id) != predecessor.map(|value| value.id)
            || self
                .predecessor
                .as_ref()
                .map(|value| value.fingerprint.as_str())
                != predecessor.map(|value| value.fingerprint.as_str())
            || self.created_at < proposal.created_at
            || self.created_at > proposal.expires_at
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair proposal review is stale or incomplete".into(),
            ));
        }
        required(self.reviewer.clone(), "repair reviewer")?;
        required(self.reason.clone(), "repair review reason")?;
        if let Some(value) = &self.predecessor {
            value.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposalApplication {
    pub schema_version: u32,
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub reservation_key: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RepairProposalApplication {
    pub fn reserve(
        proposal: &RepairProposal,
        diagnosis: &ComparativeDiagnosis,
        approval: &RepairProposalReview,
        approval_predecessor: Option<&RepairProposalReview>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        proposal.validate_integrity(diagnosis)?;
        approval.validate_against(proposal, approval_predecessor)?;
        if approval.decision != RepairReviewDecision::Approve || created_at > proposal.expires_at {
            return Err(EncoderRepairError::Validation(
                "only a current exact approval can reserve repair application".into(),
            ));
        }
        let reservation_key = fingerprint(&serde_json::json!({
            "proposal_specification_fingerprint": proposal.specification_fingerprint,
            "approval_fingerprint": approval.fingerprint,
        }))?;
        let mut value = Self {
            schema_version: REPAIR_PROPOSAL_APPLICATION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            approval_id: approval.id,
            approval_fingerprint: approval.fingerprint.clone(),
            reservation_key,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate_against(
        &self,
        proposal: &RepairProposal,
        approval: &RepairProposalReview,
    ) -> Result<(), EncoderRepairError> {
        if self.schema_version != REPAIR_PROPOSAL_APPLICATION_SCHEMA_VERSION
            || self.id.is_nil()
            || self.proposal_id != proposal.id
            || self.proposal_fingerprint != proposal.fingerprint
            || self.approval_id != approval.id
            || self.approval_fingerprint != approval.fingerprint
            || approval.decision != RepairReviewDecision::Approve
            || self.created_at < approval.created_at
            || !canonical_sha256(&self.reservation_key)
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "repair proposal application is foreign or changed".into(),
            ));
        }
        let expected_key = fingerprint(&serde_json::json!({
            "proposal_specification_fingerprint": proposal.specification_fingerprint,
            "approval_fingerprint": approval.fingerprint,
        }))?;
        if self.reservation_key != expected_key {
            return Err(EncoderRepairError::Integrity(
                "repair proposal application reservation key changed".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use encoder_experiment_core::domain::{
        BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, ModelArtifactIdentity,
    };
    use serde_json::json;

    use super::*;
    use crate::diagnosis::{CrossSuiteWeakness, SuiteWeaknessEvidence};

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project(character: char) -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos",
            EncoderTaskKind::RetrievalRanking,
            character.to_string(),
            digest(character),
            BackendIdentity::new("nomos", "v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 10, digest('c'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "development",
                    EvidenceRole::Development,
                    10,
                    digest('d'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    10,
                    digest('e'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 10, digest('f'))
                .unwrap(),
            json!({"task":"retrieval"}),
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
        )
        .unwrap()
    }

    fn diagnosis(source: &ExternalProjectSnapshot) -> ComparativeDiagnosis {
        // Proposal validation needs only an already integrity-valid diagnosis. Constructing
        // complete observation fixtures here would duplicate diagnosis tests, so start from JSON
        // and compute its two owned fingerprints exactly.
        let mut value = ComparativeDiagnosis {
            schema_version: 1,
            id: Uuid::new_v4(),
            derivation_fingerprint: String::new(),
            project_snapshot_id: source.id,
            project_snapshot_fingerprint: source.fingerprint.clone(),
            source_campaign_id: Uuid::new_v4(),
            source_experiment_run_id: Uuid::new_v4(),
            minimum_support: 1,
            slice_dimensions: vec!["workflow".into()],
            observation_sets: vec![RepairArtifactBinding {
                id: Uuid::new_v4(),
                fingerprint: digest('1'),
            }],
            weaknesses: vec![CrossSuiteWeakness {
                slice: BTreeMap::from([("workflow".into(), "bounded_change".into())]),
                slice_fingerprint: digest('2'),
                kind: WeaknessKind::SuiteSpecific,
                suite_key: Some("retired".into()),
                suites: vec![SuiteWeaknessEvidence {
                    suite_key: "retired".into(),
                    eligible_support: 12,
                    expected_abstentions: 0,
                    baseline_top_one_errors: 6,
                    baseline_top_one_error_rate: 0.5,
                }],
                total_baseline_top_one_errors: 6,
            }],
            candidate_comparisons: vec![crate::diagnosis::CandidateSliceComparison {
                candidate_id: Uuid::new_v4(),
                model_fingerprint: digest('3'),
                suite_key: "retired".into(),
                slice: BTreeMap::from([("workflow".into(), "bounded_change".into())]),
                slice_fingerprint: digest('2'),
                eligible_support: 12,
                expected_abstentions: 0,
                baseline_top_one_errors: 6,
                candidate_top_one_errors: 5,
                baseline_top_two_errors: 3,
                candidate_top_two_errors: 2,
                fixed_at_one: 1,
                regressed_at_one: 0,
                persistent_top_one_errors: 5,
                rank_improved: 1,
                rank_worsened: 0,
                baseline_top_one_error_rate: 0.5,
                candidate_top_one_error_rate: 5.0 / 12.0,
            }],
            candidate_tradeoffs: vec![crate::diagnosis::CandidateTradeoff {
                candidate_id: Uuid::new_v4(),
                model_fingerprint: digest('4'),
                passed_suites: vec!["generic".into()],
                failed_suites: vec!["retired".into()],
                outcomes: vec![],
            }],
            created_at: Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
            fingerprint: String::new(),
        };
        value.derivation_fingerprint = value.reproduce_derivation_fingerprint().unwrap();
        value.fingerprint = value.reproduce_fingerprint().unwrap();
        value
    }

    fn benchmark(at: DateTime<Utc>) -> RepairBenchmarkBinding {
        RepairBenchmarkBinding::create(
            Uuid::new_v4(),
            digest('5'),
            3,
            digest('6'),
            BTreeMap::from([
                ("generic".into(), digest('7')),
                ("retired".into(), digest('8')),
            ]),
            Uuid::new_v4(),
            digest('9'),
            at + Duration::days(30),
        )
        .unwrap()
    }

    fn proposal_fixture() -> (ComparativeDiagnosis, RepairProposal, RepairContext) {
        let at = Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap();
        let source = project('a');
        let execution = project('9');
        let diagnosis = diagnosis(&source);
        let context =
            RepairContext::create(&diagnosis, &source, &execution, benchmark(at)).unwrap();
        let policy = NativeRepairQualityPolicy::create(
            "native-repair-v1",
            true,
            0,
            0,
            0,
            0,
            0,
            0,
            true,
            true,
        )
        .unwrap();
        let budget = RepairBudget {
            maximum_total_rows: 100,
            maximum_rows_per_target: 100,
            maximum_candidates: 1,
            maximum_training_seconds: 600,
            maximum_evaluation_seconds: 600,
            maximum_development_evaluations: 2,
            maximum_external_calls: 0,
            maximum_sealed_uses: 1,
        };
        let proposal =
            RepairProposal::create(
                &diagnosis,
                context.clone(),
                vec![RepairTarget {
                    key: "retired_preflight".into(),
                    slice_fingerprint: digest('2'),
                    slice: BTreeMap::from([("workflow".into(), "bounded_change".into())]),
                    weakness_kind: WeaknessKind::SuiteSpecific,
                    suite_key: Some("retired".into()),
                    rationale: "repair the supported retired preflight failure".into(),
                    absolute_row_target: 100,
                }],
                vec![
                    RepairAction {
                        key: "generate_delta".into(),
                        kind: RepairActionKind::GenerateNativeRows,
                        target_keys: vec!["retired_preflight".into()],
                        parameters: BTreeMap::from([(
                            "recipe".into(),
                            ParameterValue::Text("bounded-preflight-v1".into()),
                        )]),
                    },
                    RepairAction {
                        key: "train_delta".into(),
                        kind: RepairActionKind::ChangeTrainingConfiguration,
                        target_keys: vec!["retired_preflight".into()],
                        parameters: BTreeMap::from([("epochs".into(), ParameterValue::Integer(1))]),
                    },
                ],
                policy,
                budget,
                vec![RepairCandidateHypothesis {
                    key: "targeted_triplet".into(),
                    mechanism: CandidateMechanism::GenuineRetraining,
                    hypothesis:
                        "targeted preflight contrasts improve retired without generic regression"
                            .into(),
                    action_keys: vec!["generate_delta".into(), "train_delta".into()],
                    maximum_training_seconds: 600,
                    parameters: BTreeMap::from([("seed".into(), ParameterValue::Integer(7))]),
                }],
                at + Duration::days(20),
                at,
            )
            .unwrap();
        (diagnosis, proposal, context)
    }

    #[test]
    fn proposal_pins_exact_context_and_becomes_stale() {
        let (diagnosis, proposal, context) = proposal_fixture();
        proposal
            .validate_against(&diagnosis, &context, proposal.created_at)
            .unwrap();
        let mut changed = context.clone();
        changed.execution_project.source_revision = "changed".into();
        assert!(
            proposal
                .validate_against(&diagnosis, &changed, proposal.created_at)
                .is_err()
        );
        assert!(
            proposal
                .validate_against(
                    &diagnosis,
                    &context,
                    proposal.expires_at + Duration::seconds(1)
                )
                .is_err()
        );
    }

    #[test]
    fn exact_approval_reserves_one_stable_application_key() {
        let (diagnosis, proposal, _) = proposal_fixture();
        let review = RepairProposalReview::create(
            &proposal,
            &diagnosis,
            None,
            RepairReviewDecision::Approve,
            "operator",
            "bounded experiment approved",
            proposal.created_at + Duration::seconds(1),
        )
        .unwrap();
        let first = RepairProposalApplication::reserve(
            &proposal,
            &diagnosis,
            &review,
            None,
            proposal.created_at + Duration::seconds(2),
        )
        .unwrap();
        let second = RepairProposalApplication::reserve(
            &proposal,
            &diagnosis,
            &review,
            None,
            proposal.created_at + Duration::seconds(2),
        )
        .unwrap();
        assert_eq!(first.reservation_key, second.reservation_key);
        assert_ne!(first.id, second.id);
    }
}
