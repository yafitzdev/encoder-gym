//! Deterministic cross-cohort leakage checks and explicit overrides.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotMember;
use generation_core::deduplication::normalize_text;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::governance::{CohortDisposition, CohortRoleDecision, EvaluationCohort};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContaminationPolicy {
    pub max_source_overlap: u64,
    pub max_exact_text_overlap: u64,
    pub max_normalized_text_overlap: u64,
    pub max_group_overlap: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CohortContaminationInput {
    pub cohort: EvaluationCohort,
    pub role: CohortRoleDecision,
    pub members: Vec<ContaminationMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContaminationMember {
    pub snapshot_member_id: Uuid,
    pub snapshot_id: Uuid,
    pub source_row_id: Uuid,
    pub text: String,
    pub group_id: Option<String>,
}

impl ContaminationMember {
    pub fn from_snapshot_member(member: &SnapshotMember, group_dimension: Option<&str>) -> Self {
        Self {
            snapshot_member_id: member.id,
            snapshot_id: member.snapshot_id,
            source_row_id: member.source_row_id,
            text: member.text.clone(),
            group_id: group_dimension.and_then(|name| member.dimensions.get(name).cloned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContaminationKind {
    SourceRow,
    ExactText,
    NormalizedText,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContaminationFinding {
    pub kind: ContaminationKind,
    pub left_cohort_id: Uuid,
    pub right_cohort_id: Uuid,
    pub left_member_id: Uuid,
    pub right_member_id: Uuid,
    /// A fingerprint of the overlapping value; raw text is never persisted here.
    pub evidence_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContaminationStatus {
    Clean,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContaminationReport {
    pub id: Uuid,
    pub cohort_ids: Vec<Uuid>,
    pub cohort_fingerprints: BTreeMap<Uuid, String>,
    pub role_decision_fingerprints: BTreeMap<Uuid, String>,
    pub group_dimension: Option<String>,
    pub policy: ContaminationPolicy,
    pub counts: BTreeMap<ContaminationKind, u64>,
    pub findings: Vec<ContaminationFinding>,
    pub status: ContaminationStatus,
    pub reasons: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ContaminationReport {
    pub fn reproduce_fingerprint(&self) -> Result<String, ContaminationError> {
        report_fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContaminationOverride {
    pub id: Uuid,
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub reason: String,
    pub approved_by: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ContaminationOverride {
    pub fn new(
        report: &ContaminationReport,
        reason: impl Into<String>,
        approved_by: impl Into<String>,
    ) -> Result<Self, ContaminationError> {
        validate_report(report)?;
        if report.status != ContaminationStatus::Blocked {
            return Err(ContaminationError::CleanOverride);
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            reason: required(reason.into(), "override reason")?,
            approved_by: required(approved_by.into(), "override approver")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = override_fingerprint(&value)?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ContaminationError> {
        override_fingerprint(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContaminationError {
    #[error("contamination check requires at least one cohort")]
    TooFewCohorts,
    #[error("cohort appears more than once: {0}")]
    DuplicateCohort(Uuid),
    #[error("cohort role does not match cohort {0}")]
    RoleMismatch(Uuid),
    #[error("cohort or role evidence fingerprint mismatch: {0}")]
    EvidenceFingerprint(Uuid),
    #[error("retired cohort cannot enter a benchmark suite: {0}")]
    RetiredCohort(Uuid),
    #[error("member {member_id} does not belong to cohort snapshot {snapshot_id}")]
    MemberSnapshot { member_id: Uuid, snapshot_id: Uuid },
    #[error("member {member_id} has no value for required group dimension {dimension}")]
    MissingGroup { member_id: Uuid, dimension: String },
    #[error("{0} must not be empty")]
    Empty(&'static str),
    #[error("a clean report cannot be overridden")]
    CleanOverride,
    #[error("artifact fingerprint mismatch")]
    FingerprintMismatch,
    #[error("could not fingerprint contamination artifact: {0}")]
    Fingerprint(String),
}

pub fn check_contamination(
    mut inputs: Vec<CohortContaminationInput>,
    group_dimension: Option<String>,
    policy: ContaminationPolicy,
) -> Result<ContaminationReport, ContaminationError> {
    if inputs.is_empty() {
        return Err(ContaminationError::TooFewCohorts);
    }
    let group_dimension = group_dimension
        .map(|value| required(value, "group dimension"))
        .transpose()?;
    inputs.sort_by_key(|input| input.cohort.id);
    let mut seen = BTreeSet::new();
    for input in &inputs {
        if !seen.insert(input.cohort.id) {
            return Err(ContaminationError::DuplicateCohort(input.cohort.id));
        }
        if input
            .cohort
            .reproduce_fingerprint()
            .map_err(|_| ContaminationError::EvidenceFingerprint(input.cohort.id))?
            != input.cohort.fingerprint
            || input
                .role
                .reproduce_fingerprint()
                .map_err(|_| ContaminationError::EvidenceFingerprint(input.cohort.id))?
                != input.role.fingerprint
        {
            return Err(ContaminationError::EvidenceFingerprint(input.cohort.id));
        }
        if input.role.cohort_id != input.cohort.id {
            return Err(ContaminationError::RoleMismatch(input.cohort.id));
        }
        if input.role.disposition == CohortDisposition::Retired {
            return Err(ContaminationError::RetiredCohort(input.cohort.id));
        }
        for member in &input.members {
            if member.snapshot_member_id.is_nil() || member.snapshot_id != input.cohort.snapshot_id
            {
                return Err(ContaminationError::MemberSnapshot {
                    member_id: member.snapshot_member_id,
                    snapshot_id: input.cohort.snapshot_id,
                });
            }
            if let Some(dimension) = &group_dimension {
                if member
                    .group_id
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(ContaminationError::MissingGroup {
                        member_id: member.snapshot_member_id,
                        dimension: dimension.clone(),
                    });
                }
            }
        }
    }

    let mut findings = Vec::new();
    for left_index in 0..inputs.len() {
        for right_index in (left_index + 1)..inputs.len() {
            compare_pair(&inputs[left_index], &inputs[right_index], &mut findings)?;
        }
    }
    findings.sort_by_key(|finding| {
        (
            finding.kind,
            finding.left_cohort_id,
            finding.right_cohort_id,
            finding.left_member_id,
            finding.right_member_id,
        )
    });
    let counts = ContaminationKind::ALL
        .into_iter()
        .map(|kind| {
            (
                kind,
                findings
                    .iter()
                    .filter(|finding| finding.kind == kind)
                    .count() as u64,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let limits = [
        (ContaminationKind::SourceRow, policy.max_source_overlap),
        (ContaminationKind::ExactText, policy.max_exact_text_overlap),
        (
            ContaminationKind::NormalizedText,
            policy.max_normalized_text_overlap,
        ),
        (ContaminationKind::Group, policy.max_group_overlap),
    ];
    let reasons = limits
        .into_iter()
        .filter_map(|(kind, limit)| {
            let observed = counts[&kind];
            (observed > limit)
                .then(|| format!("{kind:?} overlap {observed} exceeds configured maximum {limit}"))
        })
        .collect::<Vec<_>>();
    let status = if reasons.is_empty() {
        ContaminationStatus::Clean
    } else {
        ContaminationStatus::Blocked
    };
    let mut report = ContaminationReport {
        id: Uuid::new_v4(),
        cohort_ids: inputs.iter().map(|input| input.cohort.id).collect(),
        cohort_fingerprints: inputs
            .iter()
            .map(|input| (input.cohort.id, input.cohort.fingerprint.clone()))
            .collect(),
        role_decision_fingerprints: inputs
            .iter()
            .map(|input| (input.cohort.id, input.role.fingerprint.clone()))
            .collect(),
        group_dimension,
        policy,
        counts,
        findings,
        status,
        reasons,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    report.fingerprint = report_fingerprint(&report)?;
    Ok(report)
}

impl ContaminationKind {
    const ALL: [Self; 4] = [
        Self::SourceRow,
        Self::ExactText,
        Self::NormalizedText,
        Self::Group,
    ];
}

fn compare_pair(
    left: &CohortContaminationInput,
    right: &CohortContaminationInput,
    findings: &mut Vec<ContaminationFinding>,
) -> Result<(), ContaminationError> {
    for left_member in &left.members {
        for right_member in &right.members {
            if left_member.source_row_id == right_member.source_row_id {
                add_finding(
                    ContaminationKind::SourceRow,
                    left,
                    right,
                    left_member,
                    right_member,
                    &left_member.source_row_id.to_string(),
                    findings,
                )?;
            }
            if left_member.text == right_member.text {
                add_finding(
                    ContaminationKind::ExactText,
                    left,
                    right,
                    left_member,
                    right_member,
                    &left_member.text,
                    findings,
                )?;
            }
            let left_normalized = normalize_text(&left_member.text);
            let right_normalized = normalize_text(&right_member.text);
            if left_normalized == right_normalized {
                add_finding(
                    ContaminationKind::NormalizedText,
                    left,
                    right,
                    left_member,
                    right_member,
                    &left_normalized,
                    findings,
                )?;
            }
            if let (Some(left_group), Some(right_group)) =
                (&left_member.group_id, &right_member.group_id)
            {
                if left_group == right_group {
                    add_finding(
                        ContaminationKind::Group,
                        left,
                        right,
                        left_member,
                        right_member,
                        left_group,
                        findings,
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_finding(
    kind: ContaminationKind,
    left: &CohortContaminationInput,
    right: &CohortContaminationInput,
    left_member: &ContaminationMember,
    right_member: &ContaminationMember,
    evidence: &str,
    findings: &mut Vec<ContaminationFinding>,
) -> Result<(), ContaminationError> {
    findings.push(ContaminationFinding {
        kind,
        left_cohort_id: left.cohort.id,
        right_cohort_id: right.cohort.id,
        left_member_id: left_member.snapshot_member_id,
        right_member_id: right_member.snapshot_member_id,
        evidence_fingerprint: artifact_core::fingerprint(&evidence)
            .map_err(|error| ContaminationError::Fingerprint(error.to_string()))?,
    });
    Ok(())
}

fn validate_report(report: &ContaminationReport) -> Result<(), ContaminationError> {
    if report.reproduce_fingerprint()? != report.fingerprint {
        return Err(ContaminationError::FingerprintMismatch);
    }
    Ok(())
}

fn report_fingerprint(report: &ContaminationReport) -> Result<String, ContaminationError> {
    artifact_core::fingerprint(&serde_json::json!({
        "cohort_ids": report.cohort_ids,
        "cohort_fingerprints": report.cohort_fingerprints,
        "role_decision_fingerprints": report.role_decision_fingerprints,
        "group_dimension": report.group_dimension,
        "policy": report.policy,
        "counts": report.counts,
        "findings": report.findings,
        "status": report.status,
        "reasons": report.reasons,
    }))
    .map_err(|error| ContaminationError::Fingerprint(error.to_string()))
}

fn override_fingerprint(value: &ContaminationOverride) -> Result<String, ContaminationError> {
    artifact_core::fingerprint(&serde_json::json!({
        "report_id": value.report_id,
        "report_fingerprint": value.report_fingerprint,
        "reason": value.reason,
        "approved_by": value.approved_by,
    }))
    .map_err(|error| ContaminationError::Fingerprint(error.to_string()))
}

fn required(value: String, field: &'static str) -> Result<String, ContaminationError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(ContaminationError::Empty(field))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use dataset_core::domain::{SnapshotMember, SnapshotSplit, SourceProvenance};

    use super::*;
    use crate::governance::{CohortOrigin, CohortRole};

    fn input(role: CohortRole, text: &str, source: Uuid, group: &str) -> CohortContaminationInput {
        let cohort = EvaluationCohort::new(
            format!("{role:?}"),
            Uuid::new_v4(),
            format!("sha256:{role:?}"),
            SnapshotSplit::Test,
            CohortOrigin::InternalSnapshot,
        )
        .expect("cohort");
        let decision = CohortRoleDecision::initial(&cohort, role, "fixture").expect("role");
        let member = SnapshotMember {
            id: Uuid::new_v4(),
            snapshot_id: cohort.snapshot_id,
            source_row_id: source,
            split: cohort.split,
            text: text.into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("account".into(), group.into())]),
            fields: BTreeMap::new(),
            source_provenance: SourceProvenance::Generated {
                generation_job_id: Uuid::nil(),
                backend: "fake".into(),
                model: "fake-v1".into(),
                construction_plan_fingerprint: None,
            },
            source_created_at: Utc::now(),
        };
        CohortContaminationInput {
            cohort,
            role: decision,
            members: vec![ContaminationMember::from_snapshot_member(
                &member,
                Some("account"),
            )],
        }
    }

    #[test]
    fn detects_source_exact_normalized_and_group_overlap_without_persisting_text() {
        let source = Uuid::new_v4();
        let report = check_contamination(
            vec![
                input(CohortRole::Training, "Charged   twice", source, "a-1"),
                input(CohortRole::SealedAcceptance, "charged twice", source, "a-1"),
            ],
            Some("account".into()),
            ContaminationPolicy::default(),
        )
        .expect("report");
        assert_eq!(report.status, ContaminationStatus::Blocked);
        assert_eq!(report.counts[&ContaminationKind::SourceRow], 1);
        assert_eq!(report.counts[&ContaminationKind::ExactText], 0);
        assert_eq!(report.counts[&ContaminationKind::NormalizedText], 1);
        assert_eq!(report.counts[&ContaminationKind::Group], 1);
        assert!(
            !serde_json::to_string(&report)
                .expect("JSON")
                .contains("charged twice")
        );
        assert_eq!(
            report.reproduce_fingerprint().expect("fingerprint"),
            report.fingerprint
        );
    }

    #[test]
    fn thresholds_and_explicit_override_control_eligibility() {
        let source = Uuid::new_v4();
        let report = check_contamination(
            vec![
                input(CohortRole::Training, "one", source, "a"),
                input(CohortRole::Development, "two", Uuid::new_v4(), "b"),
            ],
            None,
            ContaminationPolicy {
                max_source_overlap: 1,
                ..ContaminationPolicy::default()
            },
        )
        .expect("report");
        assert_eq!(report.status, ContaminationStatus::Clean);
        assert_eq!(
            ContaminationOverride::new(&report, "ignore", "operator"),
            Err(ContaminationError::CleanOverride)
        );

        let blocked = check_contamination(
            vec![
                input(CohortRole::Training, "one", source, "a"),
                input(CohortRole::Development, "ONE", source, "b"),
            ],
            None,
            ContaminationPolicy::default(),
        )
        .expect("blocked report");
        let override_record =
            ContaminationOverride::new(&blocked, "known fixture overlap", "operator")
                .expect("override");
        assert_eq!(override_record.report_id, blocked.id);
    }

    #[test]
    fn rejects_missing_group_values_and_mismatched_member_snapshots() {
        let mut missing_group = input(CohortRole::Development, "one", Uuid::new_v4(), "account-1");
        missing_group.members[0].group_id = None;
        let member_id = missing_group.members[0].snapshot_member_id;
        assert_eq!(
            check_contamination(
                vec![missing_group],
                Some("account".into()),
                ContaminationPolicy::default(),
            ),
            Err(ContaminationError::MissingGroup {
                member_id,
                dimension: "account".into(),
            })
        );

        let mut wrong_snapshot = input(CohortRole::Development, "two", Uuid::new_v4(), "account-2");
        wrong_snapshot.members[0].snapshot_id = Uuid::new_v4();
        let member_id = wrong_snapshot.members[0].snapshot_member_id;
        let snapshot_id = wrong_snapshot.cohort.snapshot_id;
        assert_eq!(
            check_contamination(vec![wrong_snapshot], None, ContaminationPolicy::default(),),
            Err(ContaminationError::MemberSnapshot {
                member_id,
                snapshot_id,
            })
        );
    }

    #[test]
    fn rejects_tampered_cohort_and_role_evidence() {
        let mut tampered = input(CohortRole::Development, "one", Uuid::new_v4(), "account-1");
        tampered.cohort.name = "tampered".into();
        assert_eq!(
            check_contamination(vec![tampered.clone()], None, ContaminationPolicy::default()),
            Err(ContaminationError::EvidenceFingerprint(tampered.cohort.id))
        );

        let mut tampered = input(CohortRole::Development, "two", Uuid::new_v4(), "account-2");
        tampered.role.reason = "tampered".into();
        assert_eq!(
            check_contamination(vec![tampered.clone()], None, ContaminationPolicy::default()),
            Err(ContaminationError::EvidenceFingerprint(tampered.cohort.id))
        );
    }
}
