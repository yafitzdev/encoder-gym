use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use encoder_experiment_core::domain::{BackendIdentity, EvidenceRole, ExternalArtifactIdentity};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderRepairError, canonical_sha256,
    diagnosis::{ComparativeDiagnosis, RepairArtifactBinding},
    fingerprint,
    proposal::{RepairProposal, RepairProposalApplication, RepairProposalReview},
    required,
};

pub const NATIVE_DELTA_CANDIDATE_SET_SCHEMA_VERSION: u32 = 2;
pub const NATIVE_DELTA_REPORT_SCHEMA_VERSION: u32 = 1;
pub const NATIVE_DELTA_REVIEW_SCHEMA_VERSION: u32 = 1;
pub const NATIVE_DELTA_SELECTION_SCHEMA_VERSION: u32 = 1;

/// One native row represented only by identities and adapter-owned validation facts.
/// Native text, labels, tool identifiers, and payload never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRepairRowEvidence {
    pub row_id_hash: String,
    pub content_fingerprint: String,
    pub normalized_content_fingerprint: String,
    pub source_identity_fingerprint: String,
    pub group_identity_fingerprint: String,
    pub lineage_identity_fingerprint: String,
    pub target_key: String,
    pub recipe_fingerprint: String,
    pub generator_fingerprint: String,
    pub seed: u64,
    pub project_revision: String,
    pub task_valid: bool,
    pub task_validation_reasons: Vec<String>,
    pub exact_duplicate_matches: u32,
    pub normalized_duplicate_matches: u32,
    pub source_contamination_matches: u32,
    pub group_contamination_matches: u32,
    pub lineage_contamination_matches: u32,
    pub fingerprint: String,
}

impl NativeRepairRowEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        row_id_hash: impl Into<String>,
        content_fingerprint: impl Into<String>,
        normalized_content_fingerprint: impl Into<String>,
        source_identity_fingerprint: impl Into<String>,
        group_identity_fingerprint: impl Into<String>,
        lineage_identity_fingerprint: impl Into<String>,
        target_key: impl Into<String>,
        recipe_fingerprint: impl Into<String>,
        generator_fingerprint: impl Into<String>,
        seed: u64,
        project_revision: impl Into<String>,
        task_valid: bool,
        mut task_validation_reasons: Vec<String>,
        exact_duplicate_matches: u32,
        normalized_duplicate_matches: u32,
        source_contamination_matches: u32,
        group_contamination_matches: u32,
        lineage_contamination_matches: u32,
    ) -> Result<Self, EncoderRepairError> {
        task_validation_reasons.sort();
        task_validation_reasons.dedup();
        let mut value = Self {
            row_id_hash: row_id_hash.into(),
            content_fingerprint: content_fingerprint.into(),
            normalized_content_fingerprint: normalized_content_fingerprint.into(),
            source_identity_fingerprint: source_identity_fingerprint.into(),
            group_identity_fingerprint: group_identity_fingerprint.into(),
            lineage_identity_fingerprint: lineage_identity_fingerprint.into(),
            target_key: required(target_key, "native repair target key")?,
            recipe_fingerprint: recipe_fingerprint.into(),
            generator_fingerprint: generator_fingerprint.into(),
            seed,
            project_revision: required(project_revision, "native repair project revision")?,
            task_valid,
            task_validation_reasons,
            exact_duplicate_matches,
            normalized_duplicate_matches,
            source_contamination_matches,
            group_contamination_matches,
            lineage_contamination_matches,
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
                "native repair row evidence fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub const fn included(&self) -> bool {
        self.task_valid
            && self.exact_duplicate_matches == 0
            && self.normalized_duplicate_matches == 0
            && self.source_contamination_matches == 0
            && self.group_contamination_matches == 0
            && self.lineage_contamination_matches == 0
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if [
            &self.row_id_hash,
            &self.content_fingerprint,
            &self.normalized_content_fingerprint,
            &self.source_identity_fingerprint,
            &self.group_identity_fingerprint,
            &self.lineage_identity_fingerprint,
            &self.recipe_fingerprint,
            &self.generator_fingerprint,
        ]
        .iter()
        .any(|value| !canonical_sha256(value))
            || self.seed == 0
            || self.target_key.is_empty()
            || self.target_key.trim() != self.target_key
            || self.project_revision.is_empty()
            || self.project_revision.trim() != self.project_revision
            || self.task_valid != self.task_validation_reasons.is_empty()
            || self
                .task_validation_reasons
                .iter()
                .any(|value| value.is_empty() || value.trim() != value)
            || self
                .task_validation_reasons
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "native repair row evidence is incomplete or contradictory".into(),
            ));
        }
        Ok(())
    }
}

/// One payload-free member of the exact contamination-audit population.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRepairAuditReference {
    pub role: EvidenceRole,
    pub key: String,
    pub bytes: u64,
    pub fingerprint: String,
    pub row_count: u64,
    pub identity_set_fingerprint: String,
    pub evidence_fingerprint: String,
}

impl NativeRepairAuditReference {
    pub fn create(
        role: EvidenceRole,
        key: impl Into<String>,
        bytes: u64,
        fingerprint: impl Into<String>,
        row_count: u64,
        identity_set_fingerprint: impl Into<String>,
    ) -> Result<Self, EncoderRepairError> {
        let mut value = Self {
            role,
            key: required(key, "native audit reference key")?,
            bytes,
            fingerprint: fingerprint.into(),
            row_count,
            identity_set_fingerprint: identity_set_fingerprint.into(),
            evidence_fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.evidence_fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.evidence_fingerprint {
            return Err(EncoderRepairError::Integrity(
                "native audit reference fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.evidence_fingerprint.clear();
        fingerprint(&value)
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if !matches!(
            self.role,
            EvidenceRole::Training | EvidenceRole::Development | EvidenceRole::SealedAcceptance
        ) || self.key.is_empty()
            || self.key.trim() != self.key
            || self.bytes == 0
            || self.row_count == 0
            || !canonical_sha256(&self.fingerprint)
            || !canonical_sha256(&self.identity_set_fingerprint)
            || !self.evidence_fingerprint.is_empty()
                && !canonical_sha256(&self.evidence_fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "native audit reference is incomplete or has an unsupported role".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDeltaCandidateSet {
    pub schema_version: u32,
    pub id: Uuid,
    pub evidence_fingerprint: String,
    pub proposal: RepairArtifactBinding,
    pub application: RepairArtifactBinding,
    pub execution_project_id: Uuid,
    pub execution_project_fingerprint: String,
    pub adapter: BackendIdentity,
    pub delta_artifact: ExternalArtifactIdentity,
    pub audit_references: Vec<NativeRepairAuditReference>,
    pub rows: Vec<NativeRepairRowEvidence>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl NativeDeltaCandidateSet {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        proposal: &RepairProposal,
        diagnosis: &ComparativeDiagnosis,
        approval: &RepairProposalReview,
        approval_predecessor: Option<&RepairProposalReview>,
        application: &RepairProposalApplication,
        adapter: BackendIdentity,
        delta_artifact: ExternalArtifactIdentity,
        mut audit_references: Vec<NativeRepairAuditReference>,
        mut rows: Vec<NativeRepairRowEvidence>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        proposal.validate_integrity(diagnosis)?;
        approval.validate_against(proposal, approval_predecessor)?;
        application.validate_against(proposal, approval)?;
        if created_at < application.created_at || created_at > proposal.expires_at {
            return Err(EncoderRepairError::Validation(
                "native delta candidate set must follow application and precede proposal expiry"
                    .into(),
            ));
        }
        adapter
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        delta_artifact
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        audit_references.sort_by(|left, right| {
            left.role
                .cmp(&right.role)
                .then_with(|| left.key.cmp(&right.key))
        });
        rows.sort_by(|left, right| left.row_id_hash.cmp(&right.row_id_hash));
        let mut value = Self {
            schema_version: NATIVE_DELTA_CANDIDATE_SET_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            evidence_fingerprint: String::new(),
            proposal: RepairArtifactBinding {
                id: proposal.id,
                fingerprint: proposal.fingerprint.clone(),
            },
            application: RepairArtifactBinding {
                id: application.id,
                fingerprint: application.fingerprint.clone(),
            },
            execution_project_id: proposal.context.execution_project.id,
            execution_project_fingerprint: proposal.context.execution_project.fingerprint.clone(),
            adapter,
            delta_artifact,
            audit_references,
            rows,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields(proposal)?;
        value.evidence_fingerprint = value.reproduce_evidence_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_against(&self, proposal: &RepairProposal) -> Result<(), EncoderRepairError> {
        self.validate_fields(proposal)?;
        if self.reproduce_evidence_fingerprint()? != self.evidence_fingerprint
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "native delta candidate set fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_evidence_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "proposal": self.proposal,
            "application": self.application,
            "execution_project_id": self.execution_project_id,
            "execution_project_fingerprint": self.execution_project_fingerprint,
            "adapter": self.adapter,
            "delta_artifact": self.delta_artifact,
            "audit_references": self.audit_references,
            "rows": self.rows,
        }))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "evidence_fingerprint": self.evidence_fingerprint,
            "proposal": self.proposal,
            "application": self.application,
            "execution_project_id": self.execution_project_id,
            "execution_project_fingerprint": self.execution_project_fingerprint,
            "adapter": self.adapter,
            "delta_artifact": self.delta_artifact,
            "audit_references": self.audit_references,
            "rows": self.rows,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self, proposal: &RepairProposal) -> Result<(), EncoderRepairError> {
        if self.schema_version != NATIVE_DELTA_CANDIDATE_SET_SCHEMA_VERSION
            || self.id.is_nil()
            || !self.evidence_fingerprint.is_empty()
                && !canonical_sha256(&self.evidence_fingerprint)
            || self.proposal.id != proposal.id
            || self.proposal.fingerprint != proposal.fingerprint
            || self.application.id.is_nil()
            || !canonical_sha256(&self.application.fingerprint)
            || self.execution_project_id != proposal.context.execution_project.id
            || self.execution_project_fingerprint != proposal.context.execution_project.fingerprint
            || self.delta_artifact.role != EvidenceRole::Training
            || self.rows.is_empty()
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "native delta candidate set is incomplete or foreign".into(),
            ));
        }
        self.adapter
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        self.delta_artifact
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        let mut audit_keys = BTreeSet::new();
        let mut audit_roles = BTreeSet::new();
        for reference in &self.audit_references {
            reference.validate_integrity()?;
            if !audit_keys.insert(reference.key.as_str()) {
                return Err(EncoderRepairError::Validation(
                    "native delta audit scope repeats an artifact".into(),
                ));
            }
            audit_roles.insert(reference.role);
        }
        if !audit_roles.contains(&EvidenceRole::Training)
            || !audit_roles.contains(&EvidenceRole::Development)
            || !audit_roles.contains(&EvidenceRole::SealedAcceptance)
            || self.audit_references.windows(2).any(|pair| {
                (pair[0].role, pair[0].key.as_str()) >= (pair[1].role, pair[1].key.as_str())
            })
        {
            return Err(EncoderRepairError::Validation(
                "native delta audit scope must canonically cover training, development, and sealed evidence"
                    .into(),
            ));
        }
        let audited_training = self
            .audit_references
            .iter()
            .filter(|reference| reference.role == EvidenceRole::Training)
            .map(|reference| {
                (
                    reference.key.as_str(),
                    reference.bytes,
                    reference.fingerprint.as_str(),
                )
            })
            .collect::<Vec<_>>();
        let expected_training = proposal
            .context
            .base_training_inputs
            .iter()
            .map(|reference| {
                (
                    reference.key.as_str(),
                    reference.bytes,
                    reference.fingerprint.as_str(),
                )
            })
            .collect::<Vec<_>>();
        if audited_training != expected_training {
            return Err(EncoderRepairError::Validation(
                "native delta audit scope does not exactly cover base training inputs".into(),
            ));
        }
        let targets = proposal
            .targets
            .iter()
            .map(|value| (value.key.as_str(), value.absolute_row_target))
            .collect::<BTreeMap<_, _>>();
        let mut observed = BTreeMap::<&str, u64>::new();
        let mut identities = BTreeSet::new();
        let mut content = BTreeSet::new();
        let mut normalized_content = BTreeSet::new();
        for row in &self.rows {
            row.validate_integrity()?;
            if row.project_revision != proposal.context.execution_project.source_revision
                || !targets.contains_key(row.target_key.as_str())
                || !identities.insert(row.row_id_hash.as_str())
                || !content.insert(row.content_fingerprint.as_str())
                || !normalized_content.insert(row.normalized_content_fingerprint.as_str())
            {
                return Err(EncoderRepairError::Validation(
                    "native delta contains a duplicate, foreign target, or wrong project revision"
                        .into(),
                ));
            }
            *observed.entry(&row.target_key).or_default() += 1;
        }
        if observed.len() != targets.len()
            || targets
                .iter()
                .any(|(key, expected)| observed.get(key).copied() != Some(*expected))
            || self
                .rows
                .windows(2)
                .any(|pair| pair[0].row_id_hash >= pair[1].row_id_hash)
        {
            return Err(EncoderRepairError::Validation(
                "native delta does not exactly satisfy every proposal row target".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDeltaQualityCounts {
    pub candidate_rows: u64,
    pub included_rows: u64,
    pub excluded_rows: u64,
    pub invalid_rows: u64,
    pub exact_duplicate_rows: u64,
    pub normalized_duplicate_rows: u64,
    pub source_contamination_rows: u64,
    pub group_contamination_rows: u64,
    pub lineage_contamination_rows: u64,
}

impl NativeDeltaQualityCounts {
    fn from_rows(rows: &[NativeRepairRowEvidence]) -> Self {
        let mut counts = Self {
            candidate_rows: rows.len() as u64,
            ..Self::default()
        };
        for row in rows {
            if row.included() {
                counts.included_rows += 1;
            } else {
                counts.excluded_rows += 1;
            }
            counts.invalid_rows += u64::from(!row.task_valid);
            counts.exact_duplicate_rows += u64::from(row.exact_duplicate_matches > 0);
            counts.normalized_duplicate_rows += u64::from(row.normalized_duplicate_matches > 0);
            counts.source_contamination_rows += u64::from(row.source_contamination_matches > 0);
            counts.group_contamination_rows += u64::from(row.group_contamination_matches > 0);
            counts.lineage_contamination_rows += u64::from(row.lineage_contamination_matches > 0);
        }
        counts
    }

    fn passes(&self, proposal: &RepairProposal) -> bool {
        let policy = &proposal.quality_policy;
        self.invalid_rows <= policy.maximum_invalid_rows
            && self.exact_duplicate_rows <= policy.maximum_exact_duplicates
            && self.normalized_duplicate_rows <= policy.maximum_normalized_duplicates
            && self.source_contamination_rows <= policy.maximum_source_contamination
            && self.group_contamination_rows <= policy.maximum_group_contamination
            && self.lineage_contamination_rows <= policy.maximum_lineage_contamination
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDeltaQualityReport {
    pub schema_version: u32,
    pub id: Uuid,
    pub candidate_set: RepairArtifactBinding,
    pub proposal: RepairArtifactBinding,
    pub policy_fingerprint: String,
    pub counts: NativeDeltaQualityCounts,
    pub eligible: bool,
    pub failure_reasons: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl NativeDeltaQualityReport {
    pub fn create(
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        candidate_set.validate_against(proposal)?;
        proposal.quality_policy.validate_integrity()?;
        if created_at < candidate_set.created_at || created_at > proposal.expires_at {
            return Err(EncoderRepairError::Validation(
                "native delta quality report must follow its candidate set before proposal expiry"
                    .into(),
            ));
        }
        let counts = NativeDeltaQualityCounts::from_rows(&candidate_set.rows);
        let mut failure_reasons = failure_reasons(&counts, proposal);
        failure_reasons.sort();
        let eligible = failure_reasons.is_empty();
        let mut value = Self {
            schema_version: NATIVE_DELTA_REPORT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            candidate_set: RepairArtifactBinding {
                id: candidate_set.id,
                fingerprint: candidate_set.fingerprint.clone(),
            },
            proposal: candidate_set.proposal.clone(),
            policy_fingerprint: proposal.quality_policy.fingerprint.clone(),
            counts,
            eligible,
            failure_reasons,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_against(proposal, candidate_set)?;
        Ok(value)
    }

    pub fn validate_against(
        &self,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
    ) -> Result<(), EncoderRepairError> {
        candidate_set.validate_against(proposal)?;
        let counts = NativeDeltaQualityCounts::from_rows(&candidate_set.rows);
        let mut reasons = failure_reasons(&counts, proposal);
        reasons.sort();
        if self.schema_version != NATIVE_DELTA_REPORT_SCHEMA_VERSION
            || self.id.is_nil()
            || self.candidate_set.id != candidate_set.id
            || self.candidate_set.fingerprint != candidate_set.fingerprint
            || self.proposal != candidate_set.proposal
            || self.policy_fingerprint != proposal.quality_policy.fingerprint
            || self.counts != counts
            || self.eligible != reasons.is_empty()
            || self.failure_reasons != reasons
            || self.created_at < candidate_set.created_at
            || self.created_at > proposal.expires_at
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "native delta quality report does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn failure_reasons(counts: &NativeDeltaQualityCounts, proposal: &RepairProposal) -> Vec<String> {
    let policy = &proposal.quality_policy;
    let mut reasons = Vec::new();
    for (name, observed, maximum) in [
        (
            "invalid_rows",
            counts.invalid_rows,
            policy.maximum_invalid_rows,
        ),
        (
            "exact_duplicate_rows",
            counts.exact_duplicate_rows,
            policy.maximum_exact_duplicates,
        ),
        (
            "normalized_duplicate_rows",
            counts.normalized_duplicate_rows,
            policy.maximum_normalized_duplicates,
        ),
        (
            "source_contamination_rows",
            counts.source_contamination_rows,
            policy.maximum_source_contamination,
        ),
        (
            "group_contamination_rows",
            counts.group_contamination_rows,
            policy.maximum_group_contamination,
        ),
        (
            "lineage_contamination_rows",
            counts.lineage_contamination_rows,
            policy.maximum_lineage_contamination,
        ),
    ] {
        if observed > maximum {
            reasons.push(format!("{name}={observed} exceeds maximum {maximum}"));
        }
    }
    if counts.included_rows + counts.excluded_rows != counts.candidate_rows
        || counts.passes(proposal) != reasons.is_empty()
    {
        reasons.push("quality counts do not reconcile".into());
    }
    reasons
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDeltaReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDeltaReview {
    pub schema_version: u32,
    pub id: Uuid,
    pub report: RepairArtifactBinding,
    pub candidate_set: RepairArtifactBinding,
    pub proposal: RepairArtifactBinding,
    pub predecessor: Option<RepairArtifactBinding>,
    pub decision: NativeDeltaReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl NativeDeltaReview {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        predecessor: Option<&Self>,
        decision: NativeDeltaReviewDecision,
        reviewer: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        report.validate_against(proposal, candidate_set)?;
        if decision == NativeDeltaReviewDecision::Approve && !report.eligible {
            return Err(EncoderRepairError::Validation(
                "ineligible native delta cannot be approved".into(),
            ));
        }
        let mut value = Self {
            schema_version: NATIVE_DELTA_REVIEW_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            report: RepairArtifactBinding {
                id: report.id,
                fingerprint: report.fingerprint.clone(),
            },
            candidate_set: report.candidate_set.clone(),
            proposal: report.proposal.clone(),
            predecessor: predecessor.map(|value| RepairArtifactBinding {
                id: value.id,
                fingerprint: value.fingerprint.clone(),
            }),
            decision,
            reviewer: required(reviewer, "native delta reviewer")?,
            reason: required(reason, "native delta review reason")?,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_against(proposal, candidate_set, report, predecessor)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_against(
        &self,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        predecessor: Option<&Self>,
    ) -> Result<(), EncoderRepairError> {
        report.validate_against(proposal, candidate_set)?;
        if self.schema_version != NATIVE_DELTA_REVIEW_SCHEMA_VERSION
            || self.id.is_nil()
            || self.report.id != report.id
            || self.report.fingerprint != report.fingerprint
            || self.candidate_set != report.candidate_set
            || self.proposal != report.proposal
            || self.predecessor.as_ref().map(|value| value.id) != predecessor.map(|value| value.id)
            || self
                .predecessor
                .as_ref()
                .map(|value| value.fingerprint.as_str())
                != predecessor.map(|value| value.fingerprint.as_str())
            || self.decision == NativeDeltaReviewDecision::Approve && !report.eligible
            || self.created_at < report.created_at
            || self.created_at > proposal.expires_at
            || predecessor.is_some_and(|value| self.created_at < value.created_at)
            || !self.fingerprint.is_empty() && self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "native delta review is stale, foreign, or changed".into(),
            ));
        }
        required(self.reviewer.clone(), "native delta reviewer")?;
        required(self.reason.clone(), "native delta review reason")?;
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDeltaSelectionEntry {
    pub row_id_hash: String,
    pub row_fingerprint: String,
    pub target_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedNativeDeltaSelection {
    pub schema_version: u32,
    pub id: Uuid,
    pub specification_fingerprint: String,
    pub proposal: RepairArtifactBinding,
    pub application: RepairArtifactBinding,
    pub candidate_set: RepairArtifactBinding,
    pub report: RepairArtifactBinding,
    pub approval: RepairArtifactBinding,
    pub delta_artifact: ExternalArtifactIdentity,
    pub entries: Vec<NativeDeltaSelectionEntry>,
    pub excluded_rows: u64,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ApprovedNativeDeltaSelection {
    pub fn create(
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        approval.validate_against(proposal, candidate_set, report, approval_predecessor)?;
        if approval.decision != NativeDeltaReviewDecision::Approve || !report.eligible {
            return Err(EncoderRepairError::Validation(
                "native delta selection requires the current eligible approval".into(),
            ));
        }
        if created_at < approval.created_at {
            return Err(EncoderRepairError::Validation(
                "native delta selection cannot predate its approval".into(),
            ));
        }
        let entries = candidate_set
            .rows
            .iter()
            .filter(|row| row.included())
            .map(|row| NativeDeltaSelectionEntry {
                row_id_hash: row.row_id_hash.clone(),
                row_fingerprint: row.fingerprint.clone(),
                target_key: row.target_key.clone(),
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Err(EncoderRepairError::Validation(
                "native delta selection cannot be empty".into(),
            ));
        }
        let specification_fingerprint = fingerprint(&serde_json::json!({
            "proposal": report.proposal,
            "candidate_set": report.candidate_set,
            "report": {"id": report.id, "fingerprint": report.fingerprint},
            "approval": {"id": approval.id, "fingerprint": approval.fingerprint},
            "delta_artifact": candidate_set.delta_artifact,
            "entries": entries,
        }))?;
        let mut value = Self {
            schema_version: NATIVE_DELTA_SELECTION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            specification_fingerprint,
            proposal: report.proposal.clone(),
            application: candidate_set.application.clone(),
            candidate_set: report.candidate_set.clone(),
            report: approval.report.clone(),
            approval: RepairArtifactBinding {
                id: approval.id,
                fingerprint: approval.fingerprint.clone(),
            },
            delta_artifact: candidate_set.delta_artifact.clone(),
            entries,
            excluded_rows: report.counts.excluded_rows,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_against(
            proposal,
            candidate_set,
            report,
            approval,
            approval_predecessor,
        )?;
        Ok(value)
    }

    pub fn validate_against(
        &self,
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
    ) -> Result<(), EncoderRepairError> {
        let expected = Self::create_unchecked(
            proposal,
            candidate_set,
            report,
            approval,
            approval_predecessor,
            self.id,
            self.created_at,
        )?;
        if *self != expected {
            return Err(EncoderRepairError::Integrity(
                "approved native delta selection does not reproduce".into(),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn create_unchecked(
        proposal: &RepairProposal,
        candidate_set: &NativeDeltaCandidateSet,
        report: &NativeDeltaQualityReport,
        approval: &NativeDeltaReview,
        approval_predecessor: Option<&NativeDeltaReview>,
        id: Uuid,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        approval.validate_against(proposal, candidate_set, report, approval_predecessor)?;
        if approval.decision != NativeDeltaReviewDecision::Approve || !report.eligible {
            return Err(EncoderRepairError::Validation(
                "native delta selection requires the current eligible approval".into(),
            ));
        }
        if id.is_nil() || created_at < approval.created_at || created_at > proposal.expires_at {
            return Err(EncoderRepairError::Validation(
                "native delta selection identity or timestamp is invalid".into(),
            ));
        }
        let entries = candidate_set
            .rows
            .iter()
            .filter(|row| row.included())
            .map(|row| NativeDeltaSelectionEntry {
                row_id_hash: row.row_id_hash.clone(),
                row_fingerprint: row.fingerprint.clone(),
                target_key: row.target_key.clone(),
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Err(EncoderRepairError::Validation(
                "native delta selection cannot be empty".into(),
            ));
        }
        let specification_fingerprint = fingerprint(&serde_json::json!({
            "proposal": report.proposal,
            "candidate_set": report.candidate_set,
            "report": {"id": report.id, "fingerprint": report.fingerprint},
            "approval": {"id": approval.id, "fingerprint": approval.fingerprint},
            "delta_artifact": candidate_set.delta_artifact,
            "entries": entries,
        }))?;
        let mut value = Self {
            schema_version: NATIVE_DELTA_SELECTION_SCHEMA_VERSION,
            id,
            specification_fingerprint,
            proposal: report.proposal.clone(),
            application: candidate_set.application.clone(),
            candidate_set: report.candidate_set.clone(),
            report: approval.report.clone(),
            approval: RepairArtifactBinding {
                id: approval.id,
                fingerprint: approval.fingerprint.clone(),
            },
            delta_artifact: candidate_set.delta_artifact.clone(),
            entries,
            excluded_rows: report.counts.excluded_rows,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    #[test]
    fn row_evidence_is_payload_free_and_fails_closed_on_any_contamination() {
        let clean = NativeRepairRowEvidence::create(
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('5'),
            digest('6'),
            "target",
            digest('7'),
            digest('8'),
            1,
            "revision",
            true,
            vec![],
            0,
            0,
            0,
            0,
            0,
        )
        .unwrap();
        assert!(clean.included());
        let contaminated = NativeRepairRowEvidence::create(
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('5'),
            digest('6'),
            "target",
            digest('7'),
            digest('8'),
            1,
            "revision",
            true,
            vec![],
            0,
            0,
            0,
            1,
            0,
        )
        .unwrap();
        assert!(!contaminated.included());
        assert!(!serde_json::to_string(&clean).unwrap().contains("text"));
    }
}
