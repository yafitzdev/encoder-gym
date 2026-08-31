//! Immutable quality reports, append-only reviews, complete curation manifests,
//! and qualified-snapshot applications.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::{DatasetSnapshot, SnapshotMember, SourceRow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    QualityError,
    assessment::{
        BlindEvaluatorRequest, QualityIssueCode, QualityVerdict, RowQualityAssessment,
        verify_assessment_sequence,
    },
    fingerprint,
    lifecycle::{EvaluatorAttempt, QualityAuditRun, QualityAuditRunState},
    population::{AuditPlan, AuditSelection, CellIdentity, ProvenanceStratum},
    required,
};

pub const DATASET_QUALITY_REPORT_SCHEMA_VERSION: u32 = 1;
pub const CURATION_PROPOSAL_SCHEMA_VERSION: u32 = 1;
pub const CURATION_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CURATION_APPLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub id: Uuid,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnauditedReason {
    SelectedButUnassessed,
    UnselectedReportOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ReportRowVerdict {
    Qualified,
    Borderline,
    Quarantined,
    InvalidEvaluatorOutput,
    Unaudited(UnauditedReason),
}

impl ReportRowVerdict {
    const fn from_assessment(value: QualityVerdict) -> Self {
        match value {
            QualityVerdict::Qualified => Self::Qualified,
            QualityVerdict::Borderline => Self::Borderline,
            QualityVerdict::Quarantined => Self::Quarantined,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetQualityReportRow {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub cell: CellIdentity,
    pub provenance_stratum: ProvenanceStratum,
    pub selection: AuditSelection,
    pub assessment_references: Vec<ArtifactReference>,
    pub assessment_set_fingerprint: String,
    pub invalid_attempt_references: Vec<ArtifactReference>,
    pub invalid_attempt_set_fingerprint: String,
    pub verdict: ReportRowVerdict,
    /// True when repeated assessments disagree. The worst policy verdict is
    /// retained, but curation still requires an explicit review.
    pub conflicting_evidence: bool,
    pub issue_codes: Vec<QualityIssueCode>,
    pub fingerprint: String,
}

impl DatasetQualityReportRow {
    pub fn reproduce_assessment_set_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&self.assessment_references)
    }

    pub fn reproduce_invalid_attempt_set_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&self.invalid_attempt_references)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityCounts {
    pub population_rows: u64,
    pub selected_rows: u64,
    pub assessed_rows: u64,
    pub qualified_rows: u64,
    pub borderline_rows: u64,
    pub quarantined_rows: u64,
    pub invalid_rows: u64,
    pub unaudited_rows: u64,
    pub selected_but_unassessed_rows: u64,
    pub unselected_report_only_rows: u64,
    pub conflicting_rows: u64,
}

impl QualityCounts {
    fn observe(&mut self, row: &DatasetQualityReportRow) {
        self.population_rows += 1;
        if row.selection == AuditSelection::Selected {
            self.selected_rows += 1;
        }
        match row.verdict {
            ReportRowVerdict::Qualified => {
                self.assessed_rows += 1;
                self.qualified_rows += 1;
            }
            ReportRowVerdict::Borderline => {
                self.assessed_rows += 1;
                self.borderline_rows += 1;
            }
            ReportRowVerdict::Quarantined => {
                self.assessed_rows += 1;
                self.quarantined_rows += 1;
            }
            ReportRowVerdict::InvalidEvaluatorOutput => self.invalid_rows += 1,
            ReportRowVerdict::Unaudited(reason) => {
                self.unaudited_rows += 1;
                match reason {
                    UnauditedReason::SelectedButUnassessed => {
                        self.selected_but_unassessed_rows += 1;
                    }
                    UnauditedReason::UnselectedReportOnly => {
                        self.unselected_report_only_rows += 1;
                    }
                }
            }
        }
        if row.conflicting_evidence {
            self.conflicting_rows += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelQualitySummary {
    pub label: String,
    pub counts: QualityCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellQualitySummary {
    pub cell: CellIdentity,
    pub counts: QualityCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceQualitySummary {
    pub provenance_stratum: ProvenanceStratum,
    pub counts: QualityCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueQualitySummary {
    pub issue_code: QualityIssueCode,
    pub affected_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetQualityReport {
    pub id: Uuid,
    pub schema_version: u32,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub run_id: Uuid,
    pub run_specification_fingerprint: String,
    pub dataset_definition_id: Uuid,
    pub dataset_definition_fingerprint: String,
    pub source_set_fingerprint: String,
    pub policy_fingerprint: String,
    pub rows: Vec<DatasetQualityReportRow>,
    pub assessment_set_fingerprint: String,
    pub invalid_attempt_set_fingerprint: String,
    pub totals: QualityCounts,
    pub by_label: Vec<LabelQualitySummary>,
    pub by_cell: Vec<CellQualitySummary>,
    pub by_provenance: Vec<ProvenanceQualitySummary>,
    pub by_issue: Vec<IssueQualitySummary>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DatasetQualityReport {
    pub fn create(
        plan: &AuditPlan,
        run: &QualityAuditRun,
        requests: &[BlindEvaluatorRequest],
        attempts: &[EvaluatorAttempt],
        assessments: &[RowQualityAssessment],
    ) -> Result<Self, QualityError> {
        run.verify_against_evidence(plan, requests, attempts, assessments)?;
        if run.state != QualityAuditRunState::Completed
            || run.plan_id != plan.id
            || run.plan_fingerprint != plan.fingerprint
            || run.source_set_fingerprint != plan.source_set_fingerprint
            || run.policy_fingerprint != plan.policy.fingerprint
            || run.reproduce_specification_fingerprint()? != run.specification_fingerprint
        {
            return Err(QualityError::Integrity(
                "a quality report requires its exact completed audit run and plan".into(),
            ));
        }

        let items = plan
            .items
            .iter()
            .map(|item| (item.source_row_id, item))
            .collect::<BTreeMap<_, _>>();
        if items.len() != plan.items.len() {
            return Err(QualityError::Integrity(
                "audit plan repeats a source row".into(),
            ));
        }

        let mut assessment_ids = BTreeSet::new();
        let mut assessments_by_row = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
        for assessment in assessments {
            assessment.verify_integrity(plan)?;
            if !assessment_ids.insert(assessment.id) {
                return Err(QualityError::Integrity(
                    "report contains a duplicate, non-reproducible, or foreign assessment".into(),
                ));
            }
            let item = items.get(&assessment.source_row_id).ok_or_else(|| {
                QualityError::Integrity(format!(
                    "assessment {} targets a row outside the pinned population",
                    assessment.id
                ))
            })?;
            if item.selection != AuditSelection::Selected
                || assessment.source_row_fingerprint != item.source_row_fingerprint
            {
                return Err(QualityError::Integrity(format!(
                    "assessment {} does not match a selected pinned source row",
                    assessment.id
                )));
            }
            assessments_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(assessment);
        }

        let invalid_attempts_by_row = invalid_attempts_by_row(attempts);

        let distinct_assessed = assessments_by_row
            .keys()
            .filter(|source_row_id| !invalid_attempts_by_row.contains_key(source_row_id))
            .count() as u64;
        if distinct_assessed != run.progress.assessed_rows
            || invalid_attempts_by_row.len() as u64 != run.progress.invalid_rows
        {
            return Err(QualityError::Integrity(
                "report assessment and invalid-row accounting does not match the completed run"
                    .into(),
            ));
        }

        let mut rows = Vec::with_capacity(plan.items.len());
        for item in &plan.items {
            rows.push(report_row(
                plan,
                run,
                item,
                assessments_by_row
                    .remove(&item.source_row_id)
                    .unwrap_or_default(),
                invalid_attempts_by_row
                    .get(&item.source_row_id)
                    .cloned()
                    .unwrap_or_default(),
            )?);
        }
        rows.sort_by_key(|row| row.source_row_id);
        let assessment_set_fingerprint = report_assessment_set_fingerprint(&rows)?;
        let invalid_attempt_set_fingerprint = report_invalid_attempt_set_fingerprint(&rows)?;
        let summaries = summarize_rows(&rows);
        if summaries.totals.assessed_rows != run.progress.assessed_rows
            || summaries.totals.invalid_rows != run.progress.invalid_rows
            || summaries.totals.qualified_rows != run.progress.qualified_rows
            || summaries.totals.borderline_rows != run.progress.borderline_rows
            || summaries.totals.quarantined_rows != run.progress.quarantined_rows
        {
            return Err(QualityError::Integrity(
                "completed audit progress disagrees with immutable assessment verdict evidence"
                    .into(),
            ));
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: DATASET_QUALITY_REPORT_SCHEMA_VERSION,
            plan_id: plan.id,
            plan_fingerprint: plan.fingerprint.clone(),
            run_id: run.id,
            run_specification_fingerprint: run.specification_fingerprint.clone(),
            dataset_definition_id: plan.dataset_schema.dataset_definition_id,
            dataset_definition_fingerprint: plan
                .dataset_schema
                .dataset_definition_fingerprint
                .clone(),
            source_set_fingerprint: plan.source_set_fingerprint.clone(),
            policy_fingerprint: plan.policy.fingerprint.clone(),
            rows,
            assessment_set_fingerprint,
            invalid_attempt_set_fingerprint,
            totals: summaries.totals,
            by_label: summaries.by_label,
            by_cell: summaries.by_cell,
            by_provenance: summaries.by_provenance,
            by_issue: summaries.by_issue,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_against(plan, run, requests, attempts, assessments)?;
        Ok(value)
    }

    /// Verifies a loaded report against the exact durable execution evidence.
    /// A report that was merely modified and rehashed cannot authorize
    /// curation because every row is reconstructed from pinned assessments.
    pub fn verify_against(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        requests: &[BlindEvaluatorRequest],
        attempts: &[EvaluatorAttempt],
        assessments: &[RowQualityAssessment],
    ) -> Result<(), QualityError> {
        self.verify_integrity()?;
        run.verify_against_evidence(plan, requests, attempts, assessments)?;
        if run.state != QualityAuditRunState::Completed
            || self.plan_id != plan.id
            || self.plan_fingerprint != plan.fingerprint
            || self.run_id != run.id
            || self.run_specification_fingerprint != run.specification_fingerprint
            || self.dataset_definition_id != plan.dataset_schema.dataset_definition_id
            || self.dataset_definition_fingerprint
                != plan.dataset_schema.dataset_definition_fingerprint
            || self.source_set_fingerprint != plan.source_set_fingerprint
            || self.policy_fingerprint != plan.policy.fingerprint
        {
            return Err(QualityError::Integrity(
                "dataset quality report is not bound to its exact completed run and plan".into(),
            ));
        }

        let mut assessments_by_row = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
        for assessment in assessments {
            assessments_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(assessment);
        }
        let invalid_attempts_by_row = invalid_attempts_by_row(attempts);
        let mut expected_rows = Vec::with_capacity(plan.items.len());
        for item in &plan.items {
            expected_rows.push(report_row(
                plan,
                run,
                item,
                assessments_by_row
                    .remove(&item.source_row_id)
                    .unwrap_or_default(),
                invalid_attempts_by_row
                    .get(&item.source_row_id)
                    .cloned()
                    .unwrap_or_default(),
            )?);
        }
        expected_rows.sort_by_key(|row| row.source_row_id);
        if !assessments_by_row.is_empty() || self.rows != expected_rows {
            return Err(QualityError::Integrity(
                "dataset quality report rows do not reproduce from durable assessments".into(),
            ));
        }
        Ok(())
    }

    pub fn row(&self, source_row_id: Uuid) -> Option<&DatasetQualityReportRow> {
        self.rows
            .binary_search_by_key(&source_row_id, |row| row.source_row_id)
            .ok()
            .map(|index| &self.rows[index])
    }

    pub fn reproduce_assessment_set_fingerprint(&self) -> Result<String, QualityError> {
        report_assessment_set_fingerprint(&self.rows)
    }

    pub fn reproduce_invalid_attempt_set_fingerprint(&self) -> Result<String, QualityError> {
        report_invalid_attempt_set_fingerprint(&self.rows)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn verify_integrity(&self) -> Result<(), QualityError> {
        let rows_valid = self
            .rows
            .windows(2)
            .all(|pair| pair[0].source_row_id < pair[1].source_row_id)
            && self.rows.iter().all(|row| {
                row.reproduce_fingerprint().ok().as_ref() == Some(&row.fingerprint)
                    && row.reproduce_assessment_set_fingerprint().ok().as_ref()
                        == Some(&row.assessment_set_fingerprint)
                    && row
                        .reproduce_invalid_attempt_set_fingerprint()
                        .ok()
                        .as_ref()
                        == Some(&row.invalid_attempt_set_fingerprint)
                    && report_row_evidence_shape_is_valid(row)
            });
        if self.schema_version != DATASET_QUALITY_REPORT_SCHEMA_VERSION
            || !rows_valid
            || self.reproduce_assessment_set_fingerprint()? != self.assessment_set_fingerprint
            || self.reproduce_invalid_attempt_set_fingerprint()?
                != self.invalid_attempt_set_fingerprint
        {
            return Err(QualityError::Integrity(
                "dataset quality report row identity or evidence failed integrity".into(),
            ));
        }
        let summaries = summarize_rows(&self.rows);
        if self.totals != summaries.totals
            || self.by_label != summaries.by_label
            || self.by_cell != summaries.by_cell
            || self.by_provenance != summaries.by_provenance
            || self.by_issue != summaries.by_issue
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "dataset quality report summaries or fingerprint failed integrity".into(),
            ));
        }
        Ok(())
    }
}

fn report_row(
    plan: &AuditPlan,
    run: &QualityAuditRun,
    item: &crate::population::AuditPlanItem,
    mut assessments: Vec<&RowQualityAssessment>,
    mut invalid_attempts: Vec<&EvaluatorAttempt>,
) -> Result<DatasetQualityReportRow, QualityError> {
    assessments.sort_by_key(|assessment| {
        (
            assessment.request_sequence,
            assessment.attempt_number,
            assessment.id,
        )
    });
    let mut assessment_references = assessments
        .iter()
        .map(|assessment| ArtifactReference {
            id: assessment.id,
            fingerprint: assessment.fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    assessment_references.sort();
    invalid_attempts
        .sort_by_key(|attempt| (attempt.request_sequence, attempt.attempt_number, attempt.id));
    let mut invalid_attempt_references = invalid_attempts
        .iter()
        .map(|attempt| ArtifactReference {
            id: attempt.id,
            fingerprint: attempt.fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    invalid_attempt_references.sort();
    let (verdict, conflicting_evidence) = if !invalid_attempts.is_empty() {
        if invalid_attempts.len() != 1 {
            return Err(QualityError::Integrity(
                "a report row must have at most one terminal invalid-output marker".into(),
            ));
        }
        if !assessments.is_empty() {
            let summary = verify_assessment_sequence(
                plan,
                run.id,
                &run.primary_evaluator,
                &run.independent_reviewers,
                &assessments,
            )?;
            if summary.is_complete() {
                return Err(QualityError::Integrity(
                    "invalid output cannot replace a complete assessment sequence".into(),
                ));
            }
        }
        (ReportRowVerdict::InvalidEvaluatorOutput, false)
    } else if assessments.is_empty() {
        (
            ReportRowVerdict::Unaudited(match item.selection {
                AuditSelection::Selected => UnauditedReason::SelectedButUnassessed,
                AuditSelection::UnselectedReportOnly => UnauditedReason::UnselectedReportOnly,
            }),
            false,
        )
    } else {
        let summary = verify_assessment_sequence(
            plan,
            run.id,
            &run.primary_evaluator,
            &run.independent_reviewers,
            &assessments,
        )?;
        if !summary.is_complete() {
            return Err(QualityError::Integrity(
                "quality report cannot contain an unfinished assessment-review sequence".into(),
            ));
        }
        (
            ReportRowVerdict::from_assessment(summary.effective_verdict),
            summary.conflicting_evidence,
        )
    };
    let issue_codes = assessments
        .iter()
        .flat_map(|assessment| assessment.issue_codes.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let assessment_set_fingerprint = fingerprint(&assessment_references)?;
    let invalid_attempt_set_fingerprint = fingerprint(&invalid_attempt_references)?;
    let mut value = DatasetQualityReportRow {
        source_row_id: item.source_row_id,
        source_row_fingerprint: item.source_row_fingerprint.clone(),
        cell: item.cell.clone(),
        provenance_stratum: item.provenance_stratum.clone(),
        selection: item.selection,
        assessment_references,
        assessment_set_fingerprint,
        invalid_attempt_references,
        invalid_attempt_set_fingerprint,
        verdict,
        conflicting_evidence,
        issue_codes,
        fingerprint: String::new(),
    };
    value.fingerprint = value.reproduce_fingerprint()?;
    Ok(value)
}

fn report_row_evidence_shape_is_valid(row: &DatasetQualityReportRow) -> bool {
    let canonical_assessments = row
        .assessment_references
        .windows(2)
        .all(|pair| pair[0] < pair[1]);
    let canonical_invalid_attempts = row
        .invalid_attempt_references
        .windows(2)
        .all(|pair| pair[0] < pair[1]);
    if !canonical_assessments || !canonical_invalid_attempts {
        return false;
    }
    match row.verdict {
        ReportRowVerdict::Qualified
        | ReportRowVerdict::Borderline
        | ReportRowVerdict::Quarantined => {
            row.selection == AuditSelection::Selected
                && !row.assessment_references.is_empty()
                && row.invalid_attempt_references.is_empty()
        }
        ReportRowVerdict::InvalidEvaluatorOutput => {
            row.selection == AuditSelection::Selected && row.invalid_attempt_references.len() == 1
        }
        ReportRowVerdict::Unaudited(UnauditedReason::SelectedButUnassessed) => {
            row.selection == AuditSelection::Selected
                && row.assessment_references.is_empty()
                && row.invalid_attempt_references.is_empty()
        }
        ReportRowVerdict::Unaudited(UnauditedReason::UnselectedReportOnly) => {
            row.selection == AuditSelection::UnselectedReportOnly
                && row.assessment_references.is_empty()
                && row.invalid_attempt_references.is_empty()
        }
    }
}

fn invalid_attempts_by_row(
    attempts: &[EvaluatorAttempt],
) -> BTreeMap<Uuid, Vec<&EvaluatorAttempt>> {
    let mut result = BTreeMap::<Uuid, Vec<&EvaluatorAttempt>>::new();
    for attempt in attempts
        .iter()
        .filter(|attempt| attempt.state == crate::lifecycle::EvaluatorAttemptState::InvalidResponse)
    {
        for source_row_id in &attempt.invalid_source_row_ids {
            result.entry(*source_row_id).or_default().push(attempt);
        }
    }
    result
}

fn report_assessment_set_fingerprint(
    rows: &[DatasetQualityReportRow],
) -> Result<String, QualityError> {
    let mut references = rows
        .iter()
        .flat_map(|row| {
            row.assessment_references
                .iter()
                .cloned()
                .map(move |assessment| (row.source_row_id, assessment))
        })
        .collect::<Vec<_>>();
    references.sort();
    fingerprint(&references)
}

fn report_invalid_attempt_set_fingerprint(
    rows: &[DatasetQualityReportRow],
) -> Result<String, QualityError> {
    let mut references = rows
        .iter()
        .flat_map(|row| {
            row.invalid_attempt_references
                .iter()
                .cloned()
                .map(move |attempt| (row.source_row_id, attempt))
        })
        .collect::<Vec<_>>();
    references.sort();
    fingerprint(&references)
}

struct ReportSummaries {
    totals: QualityCounts,
    by_label: Vec<LabelQualitySummary>,
    by_cell: Vec<CellQualitySummary>,
    by_provenance: Vec<ProvenanceQualitySummary>,
    by_issue: Vec<IssueQualitySummary>,
}

fn summarize_rows(rows: &[DatasetQualityReportRow]) -> ReportSummaries {
    let mut totals = QualityCounts::default();
    let mut labels = BTreeMap::<String, QualityCounts>::new();
    let mut cells = BTreeMap::<CellIdentity, QualityCounts>::new();
    let mut provenance = BTreeMap::<ProvenanceStratum, QualityCounts>::new();
    let mut issues = BTreeMap::<QualityIssueCode, u64>::new();
    for row in rows {
        totals.observe(row);
        labels
            .entry(row.cell.label.clone())
            .or_default()
            .observe(row);
        cells.entry(row.cell.clone()).or_default().observe(row);
        provenance
            .entry(row.provenance_stratum.clone())
            .or_default()
            .observe(row);
        for issue in &row.issue_codes {
            *issues.entry(*issue).or_default() += 1;
        }
    }
    ReportSummaries {
        totals,
        by_label: labels
            .into_iter()
            .map(|(label, counts)| LabelQualitySummary { label, counts })
            .collect(),
        by_cell: cells
            .into_iter()
            .map(|(cell, counts)| CellQualitySummary { cell, counts })
            .collect(),
        by_provenance: provenance
            .into_iter()
            .map(|(provenance_stratum, counts)| ProvenanceQualitySummary {
                provenance_stratum,
                counts,
            })
            .collect(),
        by_issue: issues
            .into_iter()
            .map(|(issue_code, affected_rows)| IssueQualitySummary {
                issue_code,
                affected_rows,
            })
            .collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowQualityReviewDecision {
    Include,
    Exclude,
    RequestReassessment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowQualityReview {
    pub id: Uuid,
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub source_row_id: Uuid,
    pub report_row_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub decision: RowQualityReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RowQualityReview {
    pub fn create(
        report: &DatasetQualityReport,
        source_row_id: Uuid,
        predecessor: Option<&Self>,
        decision: RowQualityReviewDecision,
        reviewer: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, QualityError> {
        report.verify_integrity()?;
        let row = report.row(source_row_id).ok_or_else(|| {
            QualityError::Validation("row review targets a row outside the report".into())
        })?;
        if let Some(previous) = predecessor {
            if previous.reproduce_fingerprint()? != previous.fingerprint
                || previous.report_id != report.id
                || previous.report_fingerprint != report.fingerprint
                || previous.source_row_id != source_row_id
                || previous.report_row_fingerprint != row.fingerprint
            {
                return Err(QualityError::Integrity(
                    "row-review predecessor does not reproduce or targets another immutable row"
                        .into(),
                ));
            }
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            source_row_id,
            report_row_fingerprint: row.fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            predecessor_fingerprint: predecessor.map(|value| value.fingerprint.clone()),
            decision,
            reviewer: required(reviewer, "row review reviewer")?,
            reason: required(reason, "row review reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationDecision {
    Include,
    Exclude,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationDecisionBasis {
    QualifiedAssessment,
    QuarantinedAssessment,
    InvalidEvaluatorOutput,
    Unaudited,
    ConflictingEvidence,
    BorderlineAssessment,
    HumanIncludeOverride,
    HumanExclude,
    ReassessmentRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationProposalEntry {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub report_row_fingerprint: String,
    pub assessment_references: Vec<ArtifactReference>,
    pub invalid_attempt_references: Vec<ArtifactReference>,
    pub decision: CurationDecision,
    pub basis: CurationDecisionBasis,
    pub applied_row_review_id: Option<Uuid>,
    pub applied_row_review_fingerprint: Option<String>,
    pub reasons: Vec<String>,
    pub fingerprint: String,
}

impl CurationProposalEntry {
    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationCounts {
    pub total_rows: u64,
    pub included_rows: u64,
    pub excluded_rows: u64,
    pub needs_review_rows: u64,
}

impl CurationCounts {
    fn from_entries(entries: &[CurationProposalEntry]) -> Self {
        let mut value = Self {
            total_rows: entries.len() as u64,
            ..Self::default()
        };
        for entry in entries {
            match entry.decision {
                CurationDecision::Include => value.included_rows += 1,
                CurationDecision::Exclude => value.excluded_rows += 1,
                CurationDecision::NeedsReview => value.needs_review_rows += 1,
            }
        }
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationProposal {
    pub id: Uuid,
    pub schema_version: u32,
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub dataset_definition_id: Uuid,
    pub dataset_definition_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub row_review_references: Vec<ArtifactReference>,
    pub row_review_set_fingerprint: String,
    pub entries: Vec<CurationProposalEntry>,
    pub counts: CurationCounts,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CurationProposal {
    pub fn create(
        report: &DatasetQualityReport,
        predecessor: Option<&Self>,
        row_reviews: &[RowQualityReview],
    ) -> Result<Self, QualityError> {
        report.verify_integrity()?;
        let latest = latest_row_reviews(report, row_reviews)?;
        let mut row_review_references = row_reviews
            .iter()
            .map(|review| ArtifactReference {
                id: review.id,
                fingerprint: review.fingerprint.clone(),
            })
            .collect::<Vec<_>>();
        row_review_references.sort();
        if let Some(previous) = predecessor {
            if previous.reproduce_fingerprint()? != previous.fingerprint
                || previous.report_id != report.id
                || previous.report_fingerprint != report.fingerprint
                || !previous
                    .row_review_references
                    .iter()
                    .all(|reference| row_review_references.binary_search(reference).is_ok())
                || previous.row_review_references.len() >= row_review_references.len()
            {
                return Err(QualityError::Integrity(
                    "curation proposal predecessor is foreign, stale, or not extended by reviews"
                        .into(),
                ));
            }
        }
        let entries = report
            .rows
            .iter()
            .map(|row| proposal_entry(row, latest.get(&row.source_row_id).copied()))
            .collect::<Result<Vec<_>, _>>()?;
        let counts = CurationCounts::from_entries(&entries);
        let row_review_set_fingerprint = fingerprint(&row_review_references)?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: CURATION_PROPOSAL_SCHEMA_VERSION,
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            dataset_definition_id: report.dataset_definition_id,
            dataset_definition_fingerprint: report.dataset_definition_fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            predecessor_fingerprint: predecessor.map(|value| value.fingerprint.clone()),
            row_review_references,
            row_review_set_fingerprint,
            entries,
            counts,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_against(report, predecessor, row_reviews)?;
        Ok(value)
    }

    pub fn entry(&self, source_row_id: Uuid) -> Option<&CurationProposalEntry> {
        self.entries
            .binary_search_by_key(&source_row_id, |entry| entry.source_row_id)
            .ok()
            .map(|index| &self.entries[index])
    }

    pub fn reproduce_row_review_set_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&self.row_review_references)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    /// Replays the complete append-only row-review chain and deterministically
    /// reconstructs every proposal entry. A self-consistent proposal hash is
    /// not sufficient: inclusion authority comes from the immutable report or
    /// an exact row-level override.
    pub fn verify_against(
        &self,
        report: &DatasetQualityReport,
        predecessor: Option<&Self>,
        row_reviews: &[RowQualityReview],
    ) -> Result<(), QualityError> {
        self.verify_content(report, row_reviews)?;
        let expected_predecessor = predecessor.map(|value| (value.id, value.fingerprint.as_str()));
        let actual_predecessor = self
            .predecessor_id
            .zip(self.predecessor_fingerprint.as_deref());
        if self.predecessor_id.is_some() != self.predecessor_fingerprint.is_some()
            || actual_predecessor != expected_predecessor
        {
            return Err(QualityError::Integrity(
                "curation proposal predecessor identity does not match".into(),
            ));
        }
        if let Some(previous) = predecessor {
            let prior_reviews = row_reviews
                .iter()
                .filter(|review| {
                    previous
                        .row_review_references
                        .binary_search(&ArtifactReference {
                            id: review.id,
                            fingerprint: review.fingerprint.clone(),
                        })
                        .is_ok()
                })
                .cloned()
                .collect::<Vec<_>>();
            previous.verify_content(report, &prior_reviews)?;
            if !previous
                .row_review_references
                .iter()
                .all(|reference| self.row_review_references.binary_search(reference).is_ok())
                || previous.row_review_references.len() >= self.row_review_references.len()
            {
                return Err(QualityError::Integrity(
                    "curation proposal predecessor is not strictly extended by row reviews".into(),
                ));
            }
        }
        Ok(())
    }

    fn verify_content(
        &self,
        report: &DatasetQualityReport,
        row_reviews: &[RowQualityReview],
    ) -> Result<(), QualityError> {
        report.verify_integrity()?;
        let latest = latest_row_reviews(report, row_reviews)?;
        let expected_entries = report
            .rows
            .iter()
            .map(|row| proposal_entry(row, latest.get(&row.source_row_id).copied()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut expected_review_references = row_reviews
            .iter()
            .map(|review| ArtifactReference {
                id: review.id,
                fingerprint: review.fingerprint.clone(),
            })
            .collect::<Vec<_>>();
        expected_review_references.sort();
        if self.id.is_nil()
            || self.schema_version != CURATION_PROPOSAL_SCHEMA_VERSION
            || self.report_id != report.id
            || self.report_fingerprint != report.fingerprint
            || self.dataset_definition_id != report.dataset_definition_id
            || self.dataset_definition_fingerprint != report.dataset_definition_fingerprint
            || self.row_review_references != expected_review_references
            || self.entries != expected_entries
            || self.counts != CurationCounts::from_entries(&self.entries)
            || self.reproduce_row_review_set_fingerprint()? != self.row_review_set_fingerprint
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "curation proposal is incomplete or failed immutable-report integrity".into(),
            ));
        }
        Ok(())
    }
}

fn proposal_entry(
    row: &DatasetQualityReportRow,
    review: Option<&RowQualityReview>,
) -> Result<CurationProposalEntry, QualityError> {
    let (decision, basis, reasons) = match review {
        Some(value) if value.decision == RowQualityReviewDecision::Include => (
            CurationDecision::Include,
            CurationDecisionBasis::HumanIncludeOverride,
            vec![format!("human inclusion override: {}", value.reason)],
        ),
        Some(value) if value.decision == RowQualityReviewDecision::Exclude => (
            CurationDecision::Exclude,
            CurationDecisionBasis::HumanExclude,
            vec![format!("human exclusion: {}", value.reason)],
        ),
        Some(value) => (
            CurationDecision::NeedsReview,
            CurationDecisionBasis::ReassessmentRequested,
            vec![format!("reassessment requested: {}", value.reason)],
        ),
        None if row.conflicting_evidence => (
            CurationDecision::NeedsReview,
            CurationDecisionBasis::ConflictingEvidence,
            vec!["repeated assessments have conflicting verdicts".into()],
        ),
        None => match row.verdict {
            ReportRowVerdict::Qualified => (
                CurationDecision::Include,
                CurationDecisionBasis::QualifiedAssessment,
                vec!["conservative assessment aggregate is qualified".into()],
            ),
            ReportRowVerdict::Borderline => (
                CurationDecision::NeedsReview,
                CurationDecisionBasis::BorderlineAssessment,
                vec!["conservative assessment aggregate is borderline".into()],
            ),
            ReportRowVerdict::Quarantined => (
                CurationDecision::Exclude,
                CurationDecisionBasis::QuarantinedAssessment,
                vec!["conservative assessment aggregate is quarantined".into()],
            ),
            ReportRowVerdict::InvalidEvaluatorOutput => (
                CurationDecision::Exclude,
                CurationDecisionBasis::InvalidEvaluatorOutput,
                vec!["evaluator output was invalid under the pinned audit policy".into()],
            ),
            ReportRowVerdict::Unaudited(reason) => (
                CurationDecision::Exclude,
                CurationDecisionBasis::Unaudited,
                vec![format!("row is unaudited: {reason:?}").to_lowercase()],
            ),
        },
    };
    let mut value = CurationProposalEntry {
        source_row_id: row.source_row_id,
        source_row_fingerprint: row.source_row_fingerprint.clone(),
        report_row_fingerprint: row.fingerprint.clone(),
        assessment_references: row.assessment_references.clone(),
        invalid_attempt_references: row.invalid_attempt_references.clone(),
        decision,
        basis,
        applied_row_review_id: review.map(|value| value.id),
        applied_row_review_fingerprint: review.map(|value| value.fingerprint.clone()),
        reasons,
        fingerprint: String::new(),
    };
    value.fingerprint = value.reproduce_fingerprint()?;
    Ok(value)
}

fn latest_row_reviews<'a>(
    report: &DatasetQualityReport,
    reviews: &'a [RowQualityReview],
) -> Result<BTreeMap<Uuid, &'a RowQualityReview>, QualityError> {
    let rows = report
        .rows
        .iter()
        .map(|row| (row.source_row_id, row))
        .collect::<BTreeMap<_, _>>();
    let by_id = reviews
        .iter()
        .map(|review| (review.id, review))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != reviews.len() {
        return Err(QualityError::Integrity(
            "row review IDs must be unique".into(),
        ));
    }
    let mut successors = BTreeMap::<Uuid, Uuid>::new();
    let mut roots = BTreeMap::<Uuid, Vec<Uuid>>::new();
    for review in reviews {
        let row = rows.get(&review.source_row_id).ok_or_else(|| {
            QualityError::Integrity("row review targets a row outside its report".into())
        })?;
        if review.reproduce_fingerprint()? != review.fingerprint
            || review.report_id != report.id
            || review.report_fingerprint != report.fingerprint
            || review.report_row_fingerprint != row.fingerprint
        {
            return Err(QualityError::Integrity(
                "row review does not reproduce against its immutable report row".into(),
            ));
        }
        match (review.predecessor_id, &review.predecessor_fingerprint) {
            (None, None) => roots
                .entry(review.source_row_id)
                .or_default()
                .push(review.id),
            (Some(predecessor_id), Some(predecessor_fingerprint)) => {
                let predecessor = by_id.get(&predecessor_id).ok_or_else(|| {
                    QualityError::Integrity("row review predecessor is missing".into())
                })?;
                if predecessor.source_row_id != review.source_row_id
                    || &predecessor.fingerprint != predecessor_fingerprint
                    || successors.insert(predecessor_id, review.id).is_some()
                {
                    return Err(QualityError::Integrity(
                        "row review chain forks or crosses immutable report rows".into(),
                    ));
                }
            }
            _ => {
                return Err(QualityError::Integrity(
                    "row review predecessor identity and fingerprint must appear together".into(),
                ));
            }
        }
    }
    let mut latest = BTreeMap::new();
    for (source_row_id, row_reviews) in group_reviews_by_row(reviews) {
        let row_roots = roots.get(&source_row_id).map(Vec::as_slice).unwrap_or(&[]);
        if row_roots.len() != 1 {
            return Err(QualityError::Integrity(
                "each reviewed row must have exactly one review-chain root".into(),
            ));
        }
        let mut cursor = row_roots[0];
        let mut visited = BTreeSet::new();
        while visited.insert(cursor) {
            let Some(next) = successors.get(&cursor).copied() else {
                break;
            };
            cursor = next;
        }
        if visited.len() != row_reviews.len() {
            return Err(QualityError::Integrity(
                "row review chain is cyclic or disconnected".into(),
            ));
        }
        latest.insert(
            source_row_id,
            *by_id.get(&cursor).expect("visited review exists"),
        );
    }
    Ok(latest)
}

fn group_reviews_by_row(reviews: &[RowQualityReview]) -> BTreeMap<Uuid, Vec<&RowQualityReview>> {
    let mut grouped = BTreeMap::<Uuid, Vec<&RowQualityReview>>::new();
    for review in reviews {
        grouped
            .entry(review.source_row_id)
            .or_default()
            .push(review);
    }
    grouped
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationManifestReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationManifestReview {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub decision: CurationManifestReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CurationManifestReview {
    pub fn create(
        proposal: &CurationProposal,
        predecessor: Option<&Self>,
        decision: CurationManifestReviewDecision,
        reviewer: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, QualityError> {
        if proposal.reproduce_fingerprint()? != proposal.fingerprint {
            return Err(QualityError::Integrity(
                "manifest review requires a reproducible proposal".into(),
            ));
        }
        if let Some(previous) = predecessor
            && (previous.reproduce_fingerprint()? != previous.fingerprint
                || previous.proposal_id != proposal.id
                || previous.proposal_fingerprint != proposal.fingerprint)
        {
            return Err(QualityError::Integrity(
                "manifest-review predecessor targets another proposal".into(),
            ));
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            predecessor_fingerprint: predecessor.map(|value| value.fingerprint.clone()),
            decision,
            reviewer: required(reviewer, "manifest reviewer")?,
            reason: required(reason, "manifest review reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationDisposition {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationManifestMember {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub proposal_entry_fingerprint: String,
    pub disposition: CurationDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedCurationManifest {
    pub id: Uuid,
    pub schema_version: u32,
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub dataset_definition_id: Uuid,
    pub dataset_definition_fingerprint: String,
    /// One disposition for every source row, including excluded rows.
    pub members: Vec<CurationManifestMember>,
    pub complete_member_fingerprint: String,
    pub selected_member_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ApprovedCurationManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        report: &DatasetQualityReport,
        proposal: &CurationProposal,
        proposal_predecessor: Option<&CurationProposal>,
        row_reviews: &[RowQualityReview],
        reviews: &[CurationManifestReview],
    ) -> Result<Self, QualityError> {
        proposal.verify_against(report, proposal_predecessor, row_reviews)?;
        if proposal.counts.needs_review_rows != 0 {
            return Err(QualityError::Validation(
                "a manifest cannot be approved while any row needs review".into(),
            ));
        }
        let latest = latest_manifest_review(proposal, reviews)?;
        if latest.decision != CurationManifestReviewDecision::Approve {
            return Err(QualityError::Validation(
                "only the exact latest explicit proposal approval may create a manifest".into(),
            ));
        }
        let members = proposal
            .entries
            .iter()
            .map(|entry| {
                let disposition = match entry.decision {
                    CurationDecision::Include => CurationDisposition::Include,
                    CurationDecision::Exclude => CurationDisposition::Exclude,
                    CurationDecision::NeedsReview => {
                        return Err(QualityError::Integrity(
                            "unresolved proposal entry reached manifest construction".into(),
                        ));
                    }
                };
                Ok(CurationManifestMember {
                    source_row_id: entry.source_row_id,
                    source_row_fingerprint: entry.source_row_fingerprint.clone(),
                    proposal_entry_fingerprint: entry.fingerprint.clone(),
                    disposition,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let complete_member_fingerprint = fingerprint(&members)?;
        let selected_member_fingerprint = selected_member_fingerprint(&members)?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: CURATION_MANIFEST_SCHEMA_VERSION,
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            approval_id: latest.id,
            approval_fingerprint: latest.fingerprint.clone(),
            dataset_definition_id: report.dataset_definition_id,
            dataset_definition_fingerprint: report.dataset_definition_fingerprint.clone(),
            members,
            complete_member_fingerprint,
            selected_member_fingerprint,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_against(report, proposal, proposal_predecessor, row_reviews, reviews)?;
        Ok(value)
    }

    pub fn selected_source_row_ids(&self) -> Vec<Uuid> {
        let mut values = self
            .members
            .iter()
            .filter(|member| member.disposition == CurationDisposition::Include)
            .map(|member| member.source_row_id)
            .collect::<Vec<_>>();
        values.sort_unstable();
        values
    }

    pub fn reproduce_complete_member_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&self.members)
    }

    pub fn reproduce_selected_member_fingerprint(&self) -> Result<String, QualityError> {
        selected_member_fingerprint(&self.members)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn verify_against<'a>(
        &self,
        report: &DatasetQualityReport,
        proposal: &CurationProposal,
        proposal_predecessor: Option<&CurationProposal>,
        row_reviews: &[RowQualityReview],
        manifest_reviews: &'a [CurationManifestReview],
    ) -> Result<&'a CurationManifestReview, QualityError> {
        proposal.verify_against(report, proposal_predecessor, row_reviews)?;
        let approval = latest_manifest_review(proposal, manifest_reviews)?;
        let complete = self.members.len() == proposal.entries.len()
            && self
                .members
                .iter()
                .zip(&proposal.entries)
                .all(|(member, entry)| {
                    member.source_row_id == entry.source_row_id
                        && member.source_row_fingerprint == entry.source_row_fingerprint
                        && member.proposal_entry_fingerprint == entry.fingerprint
                        && matches!(
                            (member.disposition, entry.decision),
                            (CurationDisposition::Include, CurationDecision::Include)
                                | (CurationDisposition::Exclude, CurationDecision::Exclude)
                        )
                });
        if self.id.is_nil()
            || self.schema_version != CURATION_MANIFEST_SCHEMA_VERSION
            || self.report_id != report.id
            || self.report_fingerprint != report.fingerprint
            || self.proposal_id != proposal.id
            || self.proposal_fingerprint != proposal.fingerprint
            || self.dataset_definition_id != report.dataset_definition_id
            || self.dataset_definition_fingerprint != report.dataset_definition_fingerprint
            || self.approval_id != approval.id
            || self.approval_fingerprint != approval.fingerprint
            || approval.proposal_id != proposal.id
            || approval.proposal_fingerprint != proposal.fingerprint
            || approval.decision != CurationManifestReviewDecision::Approve
            || approval.reproduce_fingerprint()? != approval.fingerprint
            || proposal.counts.needs_review_rows != 0
            || !complete
            || self.reproduce_complete_member_fingerprint()? != self.complete_member_fingerprint
            || self.reproduce_selected_member_fingerprint()? != self.selected_member_fingerprint
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "approved curation manifest is incomplete or failed provenance integrity".into(),
            ));
        }
        Ok(approval)
    }
}

fn selected_member_fingerprint(members: &[CurationManifestMember]) -> Result<String, QualityError> {
    let selected = members
        .iter()
        .filter(|member| member.disposition == CurationDisposition::Include)
        .map(|member| (member.source_row_id, member.source_row_fingerprint.clone()))
        .collect::<Vec<_>>();
    fingerprint(&selected)
}

fn latest_manifest_review<'a>(
    proposal: &CurationProposal,
    reviews: &'a [CurationManifestReview],
) -> Result<&'a CurationManifestReview, QualityError> {
    if reviews.is_empty() {
        return Err(QualityError::Validation(
            "manifest construction requires an explicit human review".into(),
        ));
    }
    let by_id = reviews
        .iter()
        .map(|review| (review.id, review))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != reviews.len() {
        return Err(QualityError::Integrity(
            "manifest review IDs must be unique".into(),
        ));
    }
    let mut root = None;
    let mut successors = BTreeMap::new();
    for review in reviews {
        if review.reproduce_fingerprint()? != review.fingerprint
            || review.proposal_id != proposal.id
            || review.proposal_fingerprint != proposal.fingerprint
        {
            return Err(QualityError::Integrity(
                "manifest review is foreign or non-reproducible".into(),
            ));
        }
        match (review.predecessor_id, &review.predecessor_fingerprint) {
            (None, None) => {
                if root.replace(review.id).is_some() {
                    return Err(QualityError::Integrity(
                        "manifest review chain has multiple roots".into(),
                    ));
                }
            }
            (Some(predecessor_id), Some(predecessor_fingerprint)) => {
                let predecessor = by_id.get(&predecessor_id).ok_or_else(|| {
                    QualityError::Integrity("manifest review predecessor is missing".into())
                })?;
                if &predecessor.fingerprint != predecessor_fingerprint
                    || successors.insert(predecessor_id, review.id).is_some()
                {
                    return Err(QualityError::Integrity(
                        "manifest review chain forks or changes its predecessor".into(),
                    ));
                }
            }
            _ => {
                return Err(QualityError::Integrity(
                    "manifest review predecessor identity and fingerprint must appear together"
                        .into(),
                ));
            }
        }
    }
    let mut cursor =
        root.ok_or_else(|| QualityError::Integrity("manifest review chain has no root".into()))?;
    let mut visited = BTreeSet::new();
    while visited.insert(cursor) {
        let Some(next) = successors.get(&cursor).copied() else {
            break;
        };
        cursor = next;
    }
    if visited.len() != reviews.len() {
        return Err(QualityError::Integrity(
            "manifest review chain is cyclic or disconnected".into(),
        ));
    }
    Ok(*by_id.get(&cursor).expect("visited manifest review exists"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurationApplication {
    pub id: Uuid,
    pub schema_version: u32,
    pub manifest_id: Uuid,
    pub manifest_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub selected_member_fingerprint: String,
    pub snapshot_membership_fingerprint: String,
    pub applied_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CurationApplication {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        manifest: &ApprovedCurationManifest,
        report: &DatasetQualityReport,
        proposal: &CurationProposal,
        proposal_predecessor: Option<&CurationProposal>,
        row_reviews: &[RowQualityReview],
        manifest_reviews: &[CurationManifestReview],
        snapshot: &DatasetSnapshot,
        snapshot_members: &[SnapshotMember],
    ) -> Result<Self, QualityError> {
        let approval = manifest.verify_against(
            report,
            proposal,
            proposal_predecessor,
            row_reviews,
            manifest_reviews,
        )?;
        let snapshot_membership_fingerprint =
            verify_curated_snapshot(manifest, snapshot, snapshot_members)?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: CURATION_APPLICATION_SCHEMA_VERSION,
            manifest_id: manifest.id,
            manifest_fingerprint: manifest.fingerprint.clone(),
            approval_id: approval.id,
            approval_fingerprint: approval.fingerprint.clone(),
            snapshot_id: snapshot.id,
            snapshot_fingerprint: snapshot.fingerprint.clone(),
            selected_member_fingerprint: manifest.selected_member_fingerprint.clone(),
            snapshot_membership_fingerprint,
            applied_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn verify_against(
        &self,
        manifest: &ApprovedCurationManifest,
        report: &DatasetQualityReport,
        proposal: &CurationProposal,
        proposal_predecessor: Option<&CurationProposal>,
        row_reviews: &[RowQualityReview],
        manifest_reviews: &[CurationManifestReview],
        snapshot: &DatasetSnapshot,
        snapshot_members: &[SnapshotMember],
    ) -> Result<(), QualityError> {
        let approval = manifest.verify_against(
            report,
            proposal,
            proposal_predecessor,
            row_reviews,
            manifest_reviews,
        )?;
        let membership_fingerprint = verify_curated_snapshot(manifest, snapshot, snapshot_members)?;
        if self.id.is_nil()
            || self.schema_version != CURATION_APPLICATION_SCHEMA_VERSION
            || self.manifest_id != manifest.id
            || self.manifest_fingerprint != manifest.fingerprint
            || self.approval_id != approval.id
            || self.approval_fingerprint != approval.fingerprint
            || self.snapshot_id != snapshot.id
            || self.snapshot_fingerprint != snapshot.fingerprint
            || self.selected_member_fingerprint != manifest.selected_member_fingerprint
            || self.snapshot_membership_fingerprint != membership_fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "curation application does not reproduce from its complete evidence chain".into(),
            ));
        }
        Ok(())
    }
}

fn verify_curated_snapshot(
    manifest: &ApprovedCurationManifest,
    snapshot: &DatasetSnapshot,
    snapshot_members: &[SnapshotMember],
) -> Result<String, QualityError> {
    dataset_core::splitting::verify_snapshot(snapshot, snapshot_members)
        .map_err(|error| QualityError::Integrity(error.to_string()))?;
    if snapshot.source_dataset_id != manifest.dataset_definition_id
        || snapshot.member_count != snapshot_members.len() as u64
        || snapshot_members
            .iter()
            .any(|member| member.snapshot_id != snapshot.id)
    {
        return Err(QualityError::Integrity(
            "snapshot identity or member count does not match the approved curation".into(),
        ));
    }
    let selected = manifest.selected_source_row_ids();
    let selected_members = manifest
        .members
        .iter()
        .filter(|member| member.disposition == CurationDisposition::Include)
        .map(|member| (member.source_row_id, member))
        .collect::<BTreeMap<_, _>>();
    let mut snapshot_row_ids = snapshot_members
        .iter()
        .map(|member| member.source_row_id)
        .collect::<Vec<_>>();
    snapshot_row_ids.sort_unstable();
    if snapshot_row_ids.windows(2).any(|pair| pair[0] == pair[1]) || snapshot_row_ids != selected {
        return Err(QualityError::Integrity(
            "snapshot membership differs from the approved selected source rows".into(),
        ));
    }
    for member in snapshot_members {
        let expected = selected_members.get(&member.source_row_id).ok_or_else(|| {
            QualityError::Integrity(
                "snapshot contains a row absent from the selected curation members".into(),
            )
        })?;
        let source = SourceRow {
            id: member.source_row_id,
            dataset_id: manifest.dataset_definition_id,
            text: member.text.clone(),
            label: member.label.clone(),
            dimensions: member.dimensions.clone(),
            fields: member.fields.clone(),
            provenance: member.source_provenance.clone(),
            created_at: member.source_created_at,
        };
        if fingerprint(&source)? != expected.source_row_fingerprint {
            return Err(QualityError::Integrity(
                "snapshot member contents differ from the source row approved by curation".into(),
            ));
        }
    }
    let mut membership = snapshot_members
        .iter()
        .map(|member| (member.source_row_id, member.split))
        .collect::<Vec<_>>();
    membership.sort();
    fingerprint(&membership)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::{
        domain::{SourceProvenance, SourceRow, SplitConfiguration, SplitRatios},
        splitting::build_snapshot,
    };
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};

    use super::*;
    use crate::{
        assessment::{
            BlindEvaluatorRequest, EvaluatorExecutionLocation, EvaluatorGuidance,
            EvaluatorIdentity, EvaluatorIndependence, EvaluatorRequestBudget,
        },
        lifecycle::{AuditProgress, EvaluatorAttempt, ProviderUsage},
        policy::{AuditMode, EvaluatorEgressPolicy, QualityPolicyPresetControls, QualityPreset},
        population::GuidanceReferences,
    };

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::with_identity(
            Uuid::from_u128(1),
            "support",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![DimensionDefinition::new("difficulty", vec!["easy".into()]).expect("dimension")],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset")
    }

    fn source_row(dataset_id: Uuid, discriminator: u128) -> SourceRow {
        SourceRow {
            id: Uuid::from_u128(100 + discriminator),
            dataset_id,
            text: format!("Billing request {discriminator}"),
            label: "billing".into(),
            dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Imported {
                import_id: Uuid::from_u128(200),
                source_path: "fixture.jsonl".into(),
                source_row_number: discriminator as u64,
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 2, 0, 0, discriminator as u32)
                .single()
                .expect("time"),
        }
    }

    struct ReportEvidence {
        plan: AuditPlan,
        run: QualityAuditRun,
        requests: Vec<BlindEvaluatorRequest>,
        attempts: Vec<EvaluatorAttempt>,
    }

    fn unaudited_report() -> (DatasetQualityReport, Vec<SourceRow>, ReportEvidence) {
        let dataset = dataset();
        let rows = vec![source_row(dataset.id, 1), source_row(dataset.id, 2)];
        let policy = QualityPreset::Fast
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::DeterministicSampleReportOnly {
                    sample_size: 1,
                    seed: 41,
                    minimum_rows_per_cell: 1,
                },
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("policy");
        let plan = AuditPlan::with_identity(
            Uuid::from_u128(300),
            &dataset,
            policy,
            GuidanceReferences::default(),
            EvaluatorGuidance::default()
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            "quality-v1",
            rows.clone(),
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("plan");
        let evaluator = EvaluatorIdentity::new(
            "fake",
            "deterministic",
            "quality-v1",
            "sha256:fake-config",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator");
        let mut run = QualityAuditRun::queue(&plan, evaluator.clone(), Vec::new()).expect("run");
        run.start(&plan).expect("start");
        let selected_id = plan
            .selected_items()
            .next()
            .expect("selected item")
            .source_row_id;
        let selected_row = rows
            .iter()
            .find(|row| row.id == selected_id)
            .expect("selected source row")
            .clone();
        let guidance = EvaluatorGuidance::default();
        let request = BlindEvaluatorRequest::create(
            &plan,
            Uuid::from_u128(400),
            run.id,
            Uuid::from_u128(401),
            1,
            1,
            &evaluator,
            vec![selected_row],
            guidance.clone(),
            guidance
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            EvaluatorRequestBudget {
                maximum_input_tokens: 1_000,
                maximum_output_tokens: 1_000,
                maximum_total_tokens: 2_000,
                maximum_cost_microusd: None,
            },
        )
        .expect("request");
        let mut attempt =
            EvaluatorAttempt::start(&plan, &run, &request, evaluator.clone()).expect("attempt");
        run.reserve_attempt(&plan, &request, &evaluator)
            .expect("reserve attempt");
        attempt
            .invalidate(
                ProviderUsage::default(),
                vec![selected_id],
                "malformed evaluator row",
                serde_json::Value::Null,
            )
            .expect("invalidate attempt");
        run.reconcile_progress(AuditProgress {
            population_rows: 2,
            selected_rows: 1,
            assessed_rows: 0,
            qualified_rows: 0,
            borderline_rows: 0,
            quarantined_rows: 0,
            invalid_rows: 1,
            pending_review_rows: 0,
        })
        .expect("progress");
        run.complete(&plan).expect("complete");
        let requests = vec![request];
        let attempts = vec![attempt];
        let report =
            DatasetQualityReport::create(&plan, &run, &requests, &attempts, &[]).expect("report");
        (
            report,
            rows,
            ReportEvidence {
                plan,
                run,
                requests,
                attempts,
            },
        )
    }

    #[test]
    fn invalid_and_unevaluated_rows_never_enter_a_proposal_and_sampling_stays_visible() {
        let (report, _, _) = unaudited_report();
        assert_eq!(report.rows.len(), 2);
        assert!(
            report
                .rows
                .iter()
                .any(|row| row.verdict == ReportRowVerdict::InvalidEvaluatorOutput)
        );
        assert!(report.rows.iter().any(|row| {
            row.verdict == ReportRowVerdict::Unaudited(UnauditedReason::UnselectedReportOnly)
        }));
        assert_eq!(report.by_cell.len(), 1);
        assert_eq!(report.by_provenance.len(), 1);

        let proposal = CurationProposal::create(&report, None, &[]).expect("proposal");
        assert_eq!(proposal.counts.included_rows, 0);
        assert_eq!(proposal.counts.excluded_rows, 2);
        assert!(
            proposal
                .entries
                .iter()
                .all(|entry| entry.decision == CurationDecision::Exclude)
        );
    }

    #[test]
    fn rehashed_report_cannot_promote_invalid_evidence_to_qualified() {
        let (mut report, _, evidence) = unaudited_report();
        let row = report
            .rows
            .iter_mut()
            .find(|row| row.verdict == ReportRowVerdict::InvalidEvaluatorOutput)
            .expect("invalid row");
        let forged_reference = row.invalid_attempt_references[0].clone();
        row.assessment_references = vec![forged_reference];
        row.assessment_set_fingerprint = row
            .reproduce_assessment_set_fingerprint()
            .expect("assessment set fingerprint");
        row.invalid_attempt_references.clear();
        row.invalid_attempt_set_fingerprint = row
            .reproduce_invalid_attempt_set_fingerprint()
            .expect("invalid attempt set fingerprint");
        row.verdict = ReportRowVerdict::Qualified;
        row.fingerprint = row.reproduce_fingerprint().expect("row fingerprint");
        report.assessment_set_fingerprint = report
            .reproduce_assessment_set_fingerprint()
            .expect("report assessment fingerprint");
        report.invalid_attempt_set_fingerprint = report
            .reproduce_invalid_attempt_set_fingerprint()
            .expect("report invalid-attempt fingerprint");
        let summaries = summarize_rows(&report.rows);
        report.totals = summaries.totals;
        report.by_label = summaries.by_label;
        report.by_cell = summaries.by_cell;
        report.by_provenance = summaries.by_provenance;
        report.by_issue = summaries.by_issue;
        report.fingerprint = report.reproduce_fingerprint().expect("report fingerprint");

        report
            .verify_integrity()
            .expect("forgery is deliberately self-consistent");
        assert!(
            report
                .verify_against(
                    &evidence.plan,
                    &evidence.run,
                    &evidence.requests,
                    &evidence.attempts,
                    &[],
                )
                .is_err()
        );
    }

    #[test]
    fn row_reviews_form_one_append_only_chain() {
        let (report, _, _) = unaudited_report();
        let row_id = report.rows[0].source_row_id;
        let first = RowQualityReview::create(
            &report,
            row_id,
            None,
            RowQualityReviewDecision::Include,
            "alice",
            "verified against the source system",
        )
        .expect("first review");
        let second = RowQualityReview::create(
            &report,
            row_id,
            Some(&first),
            RowQualityReviewDecision::Exclude,
            "bob",
            "source-system verification was withdrawn",
        )
        .expect("second review");
        let proposal = CurationProposal::create(&report, None, &[first.clone(), second.clone()])
            .expect("proposal");
        assert_eq!(
            proposal.entry(row_id).expect("entry").decision,
            CurationDecision::Exclude
        );

        let fork = RowQualityReview::create(
            &report,
            row_id,
            Some(&first),
            RowQualityReviewDecision::RequestReassessment,
            "carol",
            "forked stale review",
        )
        .expect("locally well-formed review");
        assert!(CurationProposal::create(&report, None, &[first, second, fork]).is_err());
    }

    #[test]
    fn explicit_human_override_can_include_an_unaudited_row() {
        let (report, _, _) = unaudited_report();
        let row_id = report.rows[0].source_row_id;
        let review = RowQualityReview::create(
            &report,
            row_id,
            None,
            RowQualityReviewDecision::Include,
            "operator",
            "manually checked against a trusted record",
        )
        .expect("review");
        let proposal = CurationProposal::create(&report, None, std::slice::from_ref(&review))
            .expect("proposal");
        let entry = proposal.entry(row_id).expect("entry");
        assert_eq!(entry.decision, CurationDecision::Include);
        assert_eq!(entry.basis, CurationDecisionBasis::HumanIncludeOverride);
        assert_eq!(entry.applied_row_review_id, Some(review.id));
    }

    #[test]
    fn rehashed_proposal_cannot_escalate_an_excluded_row() {
        let (report, _, _) = unaudited_report();
        let mut proposal = CurationProposal::create(&report, None, &[]).expect("proposal");
        let entry = proposal.entries.first_mut().expect("proposal entry");
        entry.decision = CurationDecision::Include;
        entry.basis = CurationDecisionBasis::QualifiedAssessment;
        entry.reasons = vec!["forged inclusion".into()];
        entry.fingerprint = entry.reproduce_fingerprint().expect("rehashed entry");
        proposal.counts = CurationCounts::from_entries(&proposal.entries);
        proposal.fingerprint = proposal.reproduce_fingerprint().expect("rehashed proposal");

        assert!(proposal.verify_against(&report, None, &[]).is_err());
    }

    #[test]
    fn stale_or_nonlatest_manifest_approval_is_rejected() {
        let (report, _, _) = unaudited_report();
        let proposal = CurationProposal::create(&report, None, &[]).expect("proposal");
        let approval = CurationManifestReview::create(
            &proposal,
            None,
            CurationManifestReviewDecision::Approve,
            "reviewer",
            "approve exclusions",
        )
        .expect("approval");
        let rejection = CurationManifestReview::create(
            &proposal,
            Some(&approval),
            CurationManifestReviewDecision::Reject,
            "reviewer",
            "approval withdrawn",
        )
        .expect("rejection");

        assert!(
            ApprovedCurationManifest::create(
                &report,
                &proposal,
                None,
                &[],
                &[approval, rejection],
            )
            .is_err()
        );
    }

    #[test]
    fn manifest_is_complete_and_pins_selected_members() {
        let (report, _, _) = unaudited_report();
        let row_id = report.rows[0].source_row_id;
        let review = RowQualityReview::create(
            &report,
            row_id,
            None,
            RowQualityReviewDecision::Include,
            "operator",
            "trusted manual verification",
        )
        .expect("review");
        let proposal = CurationProposal::create(&report, None, std::slice::from_ref(&review))
            .expect("proposal");
        let approval = CurationManifestReview::create(
            &proposal,
            None,
            CurationManifestReviewDecision::Approve,
            "approver",
            "complete dispositions reviewed",
        )
        .expect("approval");
        let manifest = ApprovedCurationManifest::create(
            &report,
            &proposal,
            None,
            std::slice::from_ref(&review),
            std::slice::from_ref(&approval),
        )
        .expect("manifest");

        assert_eq!(manifest.members.len(), report.rows.len());
        assert_eq!(manifest.selected_source_row_ids(), vec![row_id]);
        assert_eq!(
            manifest
                .reproduce_selected_member_fingerprint()
                .expect("fingerprint"),
            manifest.selected_member_fingerprint
        );

        let mut forged = manifest.clone();
        let excluded = forged
            .members
            .iter_mut()
            .find(|member| member.disposition == CurationDisposition::Exclude)
            .expect("excluded member");
        excluded.disposition = CurationDisposition::Include;
        forged.complete_member_fingerprint = forged
            .reproduce_complete_member_fingerprint()
            .expect("rehashed members");
        forged.selected_member_fingerprint = forged
            .reproduce_selected_member_fingerprint()
            .expect("rehashed selected");
        forged.fingerprint = forged.reproduce_fingerprint().expect("rehashed manifest");
        assert!(
            forged
                .verify_against(
                    &report,
                    &proposal,
                    None,
                    std::slice::from_ref(&review),
                    std::slice::from_ref(&approval),
                )
                .is_err()
        );

        let mut incomplete = manifest;
        incomplete.members.pop();
        incomplete.complete_member_fingerprint = incomplete
            .reproduce_complete_member_fingerprint()
            .expect("rehash members");
        incomplete.selected_member_fingerprint = incomplete
            .reproduce_selected_member_fingerprint()
            .expect("rehash selected");
        incomplete.fingerprint = incomplete.reproduce_fingerprint().expect("rehash manifest");
        assert!(
            incomplete
                .verify_against(
                    &report,
                    &proposal,
                    None,
                    std::slice::from_ref(&review),
                    std::slice::from_ref(&approval),
                )
                .is_err()
        );
    }

    #[test]
    fn curation_application_rejects_tampered_snapshot_row_contents() {
        let (report, source_rows, _) = unaudited_report();
        let row_id = report.rows[0].source_row_id;
        let row_review = RowQualityReview::create(
            &report,
            row_id,
            None,
            RowQualityReviewDecision::Include,
            "operator",
            "trusted manual verification",
        )
        .expect("row review");
        let proposal = CurationProposal::create(&report, None, std::slice::from_ref(&row_review))
            .expect("proposal");
        let approval = CurationManifestReview::create(
            &proposal,
            None,
            CurationManifestReviewDecision::Approve,
            "approver",
            "complete dispositions reviewed",
        )
        .expect("approval");
        let manifest = ApprovedCurationManifest::create(
            &report,
            &proposal,
            None,
            std::slice::from_ref(&row_review),
            std::slice::from_ref(&approval),
        )
        .expect("manifest");
        let selected = source_rows
            .into_iter()
            .filter(|row| row.id == row_id)
            .collect::<Vec<_>>();
        let (snapshot, members) = build_snapshot(
            report.dataset_definition_id,
            "qualified",
            None,
            SplitConfiguration::new(SplitRatios::new(1.0, 0.0, 0.0).expect("ratios"), 42),
            selected,
        )
        .expect("snapshot");
        let application = CurationApplication::create(
            &manifest,
            &report,
            &proposal,
            None,
            std::slice::from_ref(&row_review),
            std::slice::from_ref(&approval),
            &snapshot,
            &members,
        )
        .expect("application");
        application
            .verify_against(
                &manifest,
                &report,
                &proposal,
                None,
                std::slice::from_ref(&row_review),
                std::slice::from_ref(&approval),
                &snapshot,
                &members,
            )
            .expect("application verifies");
        let mut forged_application = application.clone();
        forged_application.snapshot_id = Uuid::new_v4();
        forged_application.fingerprint = forged_application
            .reproduce_fingerprint()
            .expect("rehashed application");
        assert!(
            forged_application
                .verify_against(
                    &manifest,
                    &report,
                    &proposal,
                    None,
                    std::slice::from_ref(&row_review),
                    std::slice::from_ref(&approval),
                    &snapshot,
                    &members,
                )
                .is_err()
        );

        let mut tampered = members;
        tampered[0].text.push_str(" changed after approval");
        assert!(
            CurationApplication::create(
                &manifest,
                &report,
                &proposal,
                None,
                std::slice::from_ref(&row_review),
                std::slice::from_ref(&approval),
                &snapshot,
                &tampered,
            )
            .is_err()
        );
    }
}
