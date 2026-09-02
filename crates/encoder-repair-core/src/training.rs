use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use encoder_experiment_core::domain::{EvidenceRole, ExternalProjectSnapshot};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderRepairError, canonical_sha256,
    diagnosis::RepairArtifactBinding,
    fingerprint,
    proposal::{ProjectBinding, RepairProposal},
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview,
    },
    required,
};

pub const NATIVE_REPAIR_TRAINING_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
