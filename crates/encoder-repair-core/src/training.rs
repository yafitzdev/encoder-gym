use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use encoder_experiment_core::domain::{
    EvidenceRole, ExternalProjectSnapshot, ParameterValue, TrainingCandidate,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderRepairError, canonical_sha256,
    diagnosis::RepairArtifactBinding,
    fingerprint,
    proposal::{CandidateMechanism, ProjectBinding, RepairActionKind, RepairProposal},
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview,
    },
    required,
};

pub const NATIVE_REPAIR_TRAINING_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const REPAIR_SNAPSHOT_ID_PARAMETER: &str = "repair_snapshot_id";
pub const REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER: &str = "repair_snapshot_fingerprint";
pub const REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER: &str =
    "repair_snapshot_specification_fingerprint";
pub const REPAIR_COMBINED_MEMBERSHIP_PARAMETER: &str = "repair_combined_membership_fingerprint";
pub const REPAIR_INPUTS_FINGERPRINT_PARAMETER: &str = "repair_inputs_fingerprint";
pub const REPAIR_SELECTION_ID_PARAMETER: &str = "repair_selection_id";
pub const REPAIR_SELECTION_FINGERPRINT_PARAMETER: &str = "repair_selection_fingerprint";
pub const REPAIR_DELTA_KEY_PARAMETER: &str = "repair_delta_key";
pub const REPAIR_DELTA_BYTES_PARAMETER: &str = "repair_delta_bytes";
pub const REPAIR_DELTA_FINGERPRINT_PARAMETER: &str = "repair_delta_fingerprint";
pub const REPAIR_BASE_ROWS_PARAMETER: &str = "repair_base_rows";
pub const REPAIR_DELTA_ROWS_PARAMETER: &str = "repair_delta_rows";
pub const REPAIR_TOTAL_ROWS_PARAMETER: &str = "repair_total_rows";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairTrainingInputKind {
    BaseTraining,
    ApprovedDelta,
}

/// One immutable member of a logical combined training population.
///
/// This manifest deliberately references content-addressed sources instead of copying them into
/// another mutable aggregate file. Membership fingerprints bind the exact native row identities
/// without moving row payload into the provider-neutral core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairTrainingInput {
    pub kind: RepairTrainingInputKind,
    pub key: String,
    pub bytes: u64,
    pub fingerprint: String,
    pub row_count: u64,
    pub membership_fingerprint: String,
}

impl RepairTrainingInput {
    fn validate(&self) -> Result<(), EncoderRepairError> {
        required(self.key.clone(), "repair training snapshot input key")?;
        if self.bytes == 0
            || self.row_count == 0
            || !canonical_sha256(&self.fingerprint)
            || !canonical_sha256(&self.membership_fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "repair training snapshot input is incomplete".into(),
            ));
        }
        Ok(())
    }
}

/// Immutable, row-free manifest for one approved base-plus-delta training population.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRepairTrainingSnapshot {
    pub schema_version: u32,
    pub id: Uuid,
    pub specification_fingerprint: String,
    pub proposal: RepairArtifactBinding,
    pub application: RepairArtifactBinding,
    pub candidate_set: RepairArtifactBinding,
    pub report: RepairArtifactBinding,
    pub approval: RepairArtifactBinding,
    pub selection: RepairArtifactBinding,
    pub execution_project: ProjectBinding,
    pub baseline_model_fingerprint: String,
    pub inputs: Vec<RepairTrainingInput>,
    pub base_rows: u64,
    pub delta_rows: u64,
    pub total_rows: u64,
    pub combined_membership_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl NativeRepairTrainingSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project: &ExternalProjectSnapshot,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
        selection: &ApprovedNativeDeltaSelection,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        Self::create_unchecked(
            project,
            proposal,
            candidate_set,
            report,
            approval,
            approval_predecessor,
            selection,
            Uuid::new_v4(),
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn validate_against(
        &self,
        project: &ExternalProjectSnapshot,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
        selection: &ApprovedNativeDeltaSelection,
    ) -> Result<(), EncoderRepairError> {
        let expected = Self::create_unchecked(
            project,
            proposal,
            candidate_set,
            report,
            approval,
            approval_predecessor,
            selection,
            self.id,
            self.created_at,
        )?;
        if *self != expected {
            return Err(EncoderRepairError::Integrity(
                "native repair training snapshot does not reproduce".into(),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn create_unchecked(
        project: &ExternalProjectSnapshot,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
        selection: &ApprovedNativeDeltaSelection,
        id: Uuid,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        proposal.context.execution_project.verify(project)?;
        selection.validate_against(
            proposal,
            candidate_set,
            report,
            approval,
            approval_predecessor,
        )?;
        if id.is_nil()
            || created_at < selection.created_at
            || created_at > proposal.expires_at
            || selection.excluded_rows != 0
            || selection.entries.len() != candidate_set.rows.len()
        {
            return Err(EncoderRepairError::Validation(
                "repair training snapshot requires a current, complete, zero-exclusion selection"
                    .into(),
            ));
        }

        let mut inputs = candidate_set
            .audit_references
            .iter()
            .filter(|reference| reference.role == EvidenceRole::Training)
            .map(|reference| RepairTrainingInput {
                kind: RepairTrainingInputKind::BaseTraining,
                key: reference.key.clone(),
                bytes: reference.bytes,
                fingerprint: reference.fingerprint.clone(),
                row_count: reference.row_count,
                membership_fingerprint: reference.identity_set_fingerprint.clone(),
            })
            .collect::<Vec<_>>();
        let delta_membership_fingerprint = fingerprint(&selection.entries)?;
        inputs.push(RepairTrainingInput {
            kind: RepairTrainingInputKind::ApprovedDelta,
            key: selection.delta_artifact.key.clone(),
            bytes: selection.delta_artifact.bytes,
            fingerprint: selection.delta_artifact.fingerprint.clone(),
            row_count: selection.entries.len() as u64,
            membership_fingerprint: delta_membership_fingerprint,
        });
        inputs.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.key.cmp(&right.key))
        });

        let mut keys = BTreeSet::new();
        let mut base_rows = 0_u64;
        let mut delta_rows = 0_u64;
        for input in &inputs {
            input.validate()?;
            if !keys.insert(input.key.as_str()) {
                return Err(EncoderRepairError::Validation(
                    "repair training snapshot repeats an input".into(),
                ));
            }
            match input.kind {
                RepairTrainingInputKind::BaseTraining => {
                    base_rows = base_rows.checked_add(input.row_count).ok_or_else(|| {
                        EncoderRepairError::Validation(
                            "repair base training row count overflowed".into(),
                        )
                    })?;
                }
                RepairTrainingInputKind::ApprovedDelta => {
                    delta_rows = delta_rows.checked_add(input.row_count).ok_or_else(|| {
                        EncoderRepairError::Validation(
                            "repair delta training row count overflowed".into(),
                        )
                    })?;
                }
            }
        }
        if base_rows == 0 || delta_rows == 0 {
            return Err(EncoderRepairError::Validation(
                "repair training snapshot requires base and approved-delta rows".into(),
            ));
        }
        let total_rows = base_rows.checked_add(delta_rows).ok_or_else(|| {
            EncoderRepairError::Validation("repair total training row count overflowed".into())
        })?;
        let execution_project = ProjectBinding::from_project(project)?;
        let baseline_model_fingerprint = project.baseline_model.fingerprint.clone();
        let combined_membership_fingerprint = fingerprint(&serde_json::json!({
            "execution_project": execution_project,
            "baseline_model_fingerprint": baseline_model_fingerprint,
            "inputs": inputs,
            "base_rows": base_rows,
            "delta_rows": delta_rows,
            "total_rows": total_rows,
        }))?;
        let selection_binding = RepairArtifactBinding {
            id: selection.id,
            fingerprint: selection.fingerprint.clone(),
        };
        let specification_fingerprint = fingerprint(&serde_json::json!({
            "selection": selection_binding,
            "selection_specification_fingerprint": selection.specification_fingerprint,
            "execution_project": execution_project,
            "baseline_model_fingerprint": baseline_model_fingerprint,
            "inputs": inputs,
            "combined_membership_fingerprint": combined_membership_fingerprint,
        }))?;
        let mut value = Self {
            schema_version: NATIVE_REPAIR_TRAINING_SNAPSHOT_SCHEMA_VERSION,
            id,
            specification_fingerprint,
            proposal: selection.proposal.clone(),
            application: selection.application.clone(),
            candidate_set: selection.candidate_set.clone(),
            report: selection.report.clone(),
            approval: selection.approval.clone(),
            selection: selection_binding,
            execution_project,
            baseline_model_fingerprint,
            inputs,
            base_rows,
            delta_rows,
            total_rows,
            combined_membership_fingerprint,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_fields()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    /// Compile the proposal's predeclared genuine-retraining hypotheses into ordinary immutable
    /// experiment candidates. Repair lineage is carried only as normalized parameters so the
    /// generic experiment runner does not acquire repair-specific business logic.
    pub fn compile_training_candidates(
        &self,
        project: &ExternalProjectSnapshot,
        proposal: &RepairProposal,
    ) -> Result<Vec<TrainingCandidate>, EncoderRepairError> {
        self.validate_integrity()?;
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        self.execution_project.verify(project)?;
        if self.proposal.id != proposal.id
            || self.proposal.fingerprint != proposal.fingerprint
            || self.created_at > proposal.expires_at
        {
            return Err(EncoderRepairError::Integrity(
                "repair training snapshot does not belong to the proposal".into(),
            ));
        }
        let delta = self
            .inputs
            .iter()
            .find(|input| input.kind == RepairTrainingInputKind::ApprovedDelta)
            .ok_or_else(|| {
                EncoderRepairError::Integrity(
                    "repair training snapshot has no approved delta input".into(),
                )
            })?;
        let delta_bytes = i64::try_from(delta.bytes).map_err(|_| {
            EncoderRepairError::Validation("repair delta byte count exceeds candidate range".into())
        })?;
        let inputs_fingerprint = fingerprint(&self.inputs)?;
        let reserved = [
            REPAIR_SNAPSHOT_ID_PARAMETER,
            REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER,
            REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER,
            REPAIR_COMBINED_MEMBERSHIP_PARAMETER,
            REPAIR_INPUTS_FINGERPRINT_PARAMETER,
            REPAIR_SELECTION_ID_PARAMETER,
            REPAIR_SELECTION_FINGERPRINT_PARAMETER,
            REPAIR_DELTA_KEY_PARAMETER,
            REPAIR_DELTA_BYTES_PARAMETER,
            REPAIR_DELTA_FINGERPRINT_PARAMETER,
            REPAIR_BASE_ROWS_PARAMETER,
            REPAIR_DELTA_ROWS_PARAMETER,
            REPAIR_TOTAL_ROWS_PARAMETER,
        ];
        let mut compiled = Vec::with_capacity(proposal.candidates.len());
        for (index, hypothesis) in proposal.candidates.iter().enumerate() {
            if hypothesis.mechanism != CandidateMechanism::GenuineRetraining
                || hypothesis.parameters.keys().any(|key| {
                    reserved.contains(&key.as_str())
                        || key == "strategy"
                            && hypothesis.parameters.get(key)
                                != Some(&ParameterValue::Text("fine_tune".into()))
                })
            {
                return Err(EncoderRepairError::Validation(
                    "repair training snapshot supports only predeclared genuine fine-tuning".into(),
                ));
            }
            let training_actions = hypothesis
                .action_keys
                .iter()
                .filter_map(|key| proposal.actions.iter().find(|action| action.key == *key))
                .filter(|action| action.kind == RepairActionKind::ChangeTrainingConfiguration)
                .collect::<Vec<_>>();
            if training_actions.len() != 1
                || training_actions[0].parameters != hypothesis.parameters
            {
                return Err(EncoderRepairError::Validation(
                    "repair candidate parameters must exactly match one declared training action"
                        .into(),
                ));
            }
            let mut parameters = hypothesis.parameters.clone();
            parameters.insert("strategy".into(), ParameterValue::Text("fine_tune".into()));
            parameters.insert(
                REPAIR_SNAPSHOT_ID_PARAMETER.into(),
                ParameterValue::Text(self.id.to_string()),
            );
            parameters.insert(
                REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(self.fingerprint.clone()),
            );
            parameters.insert(
                REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER.into(),
                ParameterValue::Text(self.specification_fingerprint.clone()),
            );
            parameters.insert(
                REPAIR_COMBINED_MEMBERSHIP_PARAMETER.into(),
                ParameterValue::Text(self.combined_membership_fingerprint.clone()),
            );
            parameters.insert(
                REPAIR_INPUTS_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(inputs_fingerprint.clone()),
            );
            parameters.insert(
                REPAIR_SELECTION_ID_PARAMETER.into(),
                ParameterValue::Text(self.selection.id.to_string()),
            );
            parameters.insert(
                REPAIR_SELECTION_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(self.selection.fingerprint.clone()),
            );
            parameters.insert(
                REPAIR_DELTA_KEY_PARAMETER.into(),
                ParameterValue::Text(delta.key.clone()),
            );
            parameters.insert(
                REPAIR_DELTA_BYTES_PARAMETER.into(),
                ParameterValue::Integer(delta_bytes),
            );
            parameters.insert(
                REPAIR_DELTA_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(delta.fingerprint.clone()),
            );
            for (key, count) in [
                (REPAIR_BASE_ROWS_PARAMETER, self.base_rows),
                (REPAIR_DELTA_ROWS_PARAMETER, self.delta_rows),
                (REPAIR_TOTAL_ROWS_PARAMETER, self.total_rows),
            ] {
                parameters.insert(
                    key.into(),
                    ParameterValue::Integer(i64::try_from(count).map_err(|_| {
                        EncoderRepairError::Validation(
                            "repair training row count exceeds candidate range".into(),
                        )
                    })?),
                );
            }
            let sequence = u32::try_from(index + 1).map_err(|_| {
                EncoderRepairError::Validation("repair candidate sequence overflowed".into())
            })?;
            compiled.push(
                TrainingCandidate::create(
                    project,
                    sequence,
                    hypothesis.maximum_training_seconds,
                    parameters,
                )
                .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?,
            );
        }
        if compiled.is_empty()
            || compiled.len()
                > usize::try_from(proposal.budget.maximum_candidates).map_err(|_| {
                    EncoderRepairError::Validation(
                        "repair candidate budget exceeds platform range".into(),
                    )
                })?
        {
            return Err(EncoderRepairError::Validation(
                "repair proposal has no finite compilable training candidates".into(),
            ));
        }
        Ok(compiled)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "native repair training snapshot fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if self.schema_version != NATIVE_REPAIR_TRAINING_SNAPSHOT_SCHEMA_VERSION
            || self.id.is_nil()
            || !canonical_sha256(&self.specification_fingerprint)
            || !canonical_sha256(&self.baseline_model_fingerprint)
            || !canonical_sha256(&self.combined_membership_fingerprint)
            || !canonical_sha256(&self.fingerprint)
            || self.inputs.is_empty()
            || self.base_rows == 0
            || self.delta_rows == 0
            || self.total_rows != self.base_rows.checked_add(self.delta_rows).unwrap_or(0)
        {
            return Err(EncoderRepairError::Validation(
                "native repair training snapshot is incomplete".into(),
            ));
        }
        self.execution_project.validate()?;
        for binding in [
            &self.proposal,
            &self.application,
            &self.candidate_set,
            &self.report,
            &self.approval,
            &self.selection,
        ] {
            binding.validate()?;
        }
        for input in &self.inputs {
            input.validate()?;
        }
        if self.inputs.windows(2).any(|pair| {
            (pair[0].kind, pair[0].key.as_str()) >= (pair[1].kind, pair[1].key.as_str())
        }) {
            return Err(EncoderRepairError::Validation(
                "repair training snapshot inputs are not canonical".into(),
            ));
        }
        Ok(())
    }
}
