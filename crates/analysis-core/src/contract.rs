use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    AnalysisReport, AnalysisSourceIdentity, FindingIdentity, FindingKind, FindingReviewRecord,
    FindingReviewState,
};

const METRIC_EPSILON: f64 = 1.1e-12;

/// Stable, persistence- and presentation-independent input for Slice 6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticContract {
    pub analysis_report_id: Uuid,
    pub analysis_fingerprint: String,
    pub analysis_protocol_fingerprint: String,
    pub source_identity: DiagnosticSourceIdentity,
    pub minimum_support: u64,
    pub prediction_count: u64,
    pub error_count: u64,
    pub baseline_error_rate: f64,
    pub cells: Vec<CellDiagnosticEvidence>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSourceIdentity {
    pub evaluation_run_id: Uuid,
    pub evaluation_input_fingerprint: String,
    pub evaluation_protocol_fingerprint: String,
    pub cohort_fingerprint: String,
    pub prediction_count: u64,
    pub comparison_id: Option<Uuid>,
    pub comparison_fingerprint: Option<String>,
    pub legacy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellDiagnosticEvidence {
    pub finding_key: String,
    pub finding_fingerprint: String,
    pub identity: BTreeMap<String, String>,
    pub support: u64,
    pub error_count: u64,
    pub error_rate: f64,
    pub error_rate_lift: f64,
    pub error_share: f64,
    pub high_confidence_error_severity: f64,
    pub marginal_error_count: u64,
    pub cumulative_error_coverage: f64,
    pub comparison: Option<DiagnosticComparisonEvidence>,
    pub latest_review: Option<DiagnosticReviewDisposition>,
    pub evidence_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticComparisonEvidence {
    pub comparison_id: Uuid,
    pub analyzed_run_is_right: bool,
    pub paired_support: u64,
    pub fixed_by_right_count: u64,
    pub regressed_by_right_count: u64,
    pub persistent_error_count: u64,
    pub high_confidence_regression_count: u64,
    pub analyzed_accuracy_delta: Option<f64>,
}

impl DiagnosticComparisonEvidence {
    pub fn analyzed_only_error_count(&self) -> u64 {
        if self.analyzed_run_is_right {
            self.regressed_by_right_count
        } else {
            self.fixed_by_right_count
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReviewDisposition {
    pub review_id: Uuid,
    pub state: FindingReviewState,
    pub resolution_evaluation_run_id: Option<Uuid>,
    pub resolution_comparison_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl DiagnosticContract {
    pub fn from_report_and_reviews(
        report: &AnalysisReport,
        reviews: &[FindingReviewRecord],
    ) -> Result<Self, DiagnosticContractError> {
        validate_report_counts(report)?;
        let source = report.source_identity.clone().unwrap_or_else(|| {
            AnalysisSourceIdentity::legacy(report.evaluation_run_id, report.prediction_count)
        });
        if source.evaluation_run_id != report.evaluation_run_id
            || source.prediction_count != report.prediction_count
            || source.comparison_id.is_some() != source.comparison_fingerprint.is_some()
        {
            return Err(DiagnosticContractError::SourceIdentity);
        }
        let legacy = is_legacy_source(&source);
        let source_identity = DiagnosticSourceIdentity {
            evaluation_run_id: source.evaluation_run_id,
            evaluation_input_fingerprint: source.evaluation_input_fingerprint,
            evaluation_protocol_fingerprint: source.evaluation_protocol_fingerprint,
            cohort_fingerprint: source.cohort_fingerprint,
            prediction_count: source.prediction_count,
            comparison_id: source.comparison_id,
            comparison_fingerprint: source.comparison_fingerprint,
            legacy,
        };
        let baseline_error_rate = divide(report.error_count, report.prediction_count);
        let latest_reviews = latest_reviews(report, reviews)?;
        let comparison = report.comparison_diagnosis.as_ref();
        if source_identity.comparison_id != comparison.map(|diagnosis| diagnosis.comparison_id) {
            return Err(DiagnosticContractError::ComparisonEvidence);
        }
        let mut keys = BTreeSet::new();
        let mut cells = Vec::new();
        for finding in report
            .findings
            .iter()
            .filter(|finding| finding.kind == FindingKind::Cell)
        {
            let identity = FindingIdentity {
                kind: finding.kind,
                attributes: finding.attributes.clone(),
            };
            if identity.key() != finding.key || !keys.insert(finding.key.clone()) {
                return Err(DiagnosticContractError::InvalidCell(finding.key.clone()));
            }
            validate_finding_metrics(
                finding.support,
                finding.error_count,
                finding.error_rate,
                finding.error_rate_lift,
                finding.error_share,
                finding.high_confidence_error_severity,
                finding.marginal_error_count,
                finding.cumulative_error_coverage,
                baseline_error_rate,
                report.error_count,
                &finding.key,
            )?;
            let mut cell = CellDiagnosticEvidence {
                finding_key: finding.key.clone(),
                finding_fingerprint: finding.fingerprint.clone(),
                identity: finding.attributes.clone(),
                support: finding.support,
                error_count: finding.error_count,
                error_rate: rounded(finding.error_rate),
                error_rate_lift: rounded(finding.error_rate_lift),
                error_share: rounded(finding.error_share),
                high_confidence_error_severity: rounded(finding.high_confidence_error_severity),
                marginal_error_count: finding.marginal_error_count,
                cumulative_error_coverage: rounded(finding.cumulative_error_coverage),
                comparison: comparison.and_then(|diagnosis| {
                    diagnosis.cell_evidence.get(&finding.key).map(|evidence| {
                        let analyzed_run_is_right =
                            report.evaluation_run_id == diagnosis.right_evaluation_run_id;
                        let analyzed_accuracy_delta = diagnosis
                            .slice_deltas
                            .iter()
                            .find(|delta| delta.identity_key == finding.key)
                            .map(|delta| {
                                if analyzed_run_is_right {
                                    rounded(delta.accuracy_delta)
                                } else {
                                    rounded(-delta.accuracy_delta)
                                }
                            });
                        DiagnosticComparisonEvidence {
                            comparison_id: diagnosis.comparison_id,
                            analyzed_run_is_right,
                            paired_support: evidence.paired_support,
                            fixed_by_right_count: evidence.fixed_by_right_count,
                            regressed_by_right_count: evidence.regressed_by_right_count,
                            persistent_error_count: evidence.persistent_error_count,
                            high_confidence_regression_count: evidence
                                .high_confidence_regression_count,
                            analyzed_accuracy_delta,
                        }
                    })
                }),
                latest_review: latest_reviews.get(&finding.key).cloned(),
                evidence_fingerprint: String::new(),
            };
            cell.evidence_fingerprint = cell
                .reproduce_fingerprint()
                .map_err(|error| DiagnosticContractError::Fingerprint(error.to_string()))?;
            cells.push(cell);
        }
        cells.sort_by(|left, right| left.finding_key.cmp(&right.finding_key));
        let mut contract = Self {
            analysis_report_id: report.id,
            analysis_fingerprint: report.fingerprint.clone(),
            analysis_protocol_fingerprint: report.protocol_fingerprint.clone(),
            source_identity,
            minimum_support: report.minimum_support,
            prediction_count: report.prediction_count,
            error_count: report.error_count,
            baseline_error_rate,
            cells,
            fingerprint: String::new(),
        };
        contract.fingerprint = contract
            .reproduce_fingerprint()
            .map_err(|error| DiagnosticContractError::Fingerprint(error.to_string()))?;
        Ok(contract)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.fingerprint.clear();
        artifact_core::fingerprint(&input)
    }

    pub fn validate(&self, require_complete_source: bool) -> Result<(), DiagnosticContractError> {
        if self.prediction_count == 0
            || self.error_count > self.prediction_count
            || !close_enough(
                self.baseline_error_rate,
                divide(self.error_count, self.prediction_count),
            )
        {
            return Err(DiagnosticContractError::ReportCounts);
        }
        if self.source_identity.evaluation_run_id == Uuid::nil()
            || self.source_identity.prediction_count != self.prediction_count
            || self.source_identity.comparison_id.is_some()
                != self.source_identity.comparison_fingerprint.is_some()
        {
            return Err(DiagnosticContractError::SourceIdentity);
        }
        if require_complete_source && self.source_identity.legacy {
            return Err(DiagnosticContractError::LegacySourceIdentity);
        }
        let mut previous = None;
        for cell in &self.cells {
            if previous
                .as_ref()
                .is_some_and(|key| key >= &cell.finding_key)
            {
                return Err(DiagnosticContractError::CellOrder);
            }
            let identity = FindingIdentity {
                kind: FindingKind::Cell,
                attributes: cell.identity.clone(),
            };
            if identity.key() != cell.finding_key {
                return Err(DiagnosticContractError::InvalidCell(
                    cell.finding_key.clone(),
                ));
            }
            validate_finding_metrics(
                cell.support,
                cell.error_count,
                cell.error_rate,
                cell.error_rate_lift,
                cell.error_share,
                cell.high_confidence_error_severity,
                cell.marginal_error_count,
                cell.cumulative_error_coverage,
                self.baseline_error_rate,
                self.error_count,
                &cell.finding_key,
            )?;
            validate_comparison_evidence(cell.comparison.as_ref(), &self.source_identity)?;
            if cell
                .reproduce_fingerprint()
                .map_err(|error| DiagnosticContractError::Fingerprint(error.to_string()))?
                != cell.evidence_fingerprint
            {
                return Err(DiagnosticContractError::CellFingerprint(
                    cell.finding_key.clone(),
                ));
            }
            previous = Some(cell.finding_key.clone());
        }
        if self
            .reproduce_fingerprint()
            .map_err(|error| DiagnosticContractError::Fingerprint(error.to_string()))?
            != self.fingerprint
        {
            return Err(DiagnosticContractError::ContractFingerprint);
        }
        Ok(())
    }
}

fn validate_comparison_evidence(
    evidence: Option<&DiagnosticComparisonEvidence>,
    source: &DiagnosticSourceIdentity,
) -> Result<(), DiagnosticContractError> {
    let Some(evidence) = evidence else {
        return Ok(());
    };
    if source.comparison_id != Some(evidence.comparison_id)
        || evidence.fixed_by_right_count > evidence.paired_support
        || evidence.regressed_by_right_count > evidence.paired_support
        || evidence.persistent_error_count > evidence.paired_support
        || evidence.high_confidence_regression_count > evidence.regressed_by_right_count
        || evidence
            .analyzed_accuracy_delta
            .is_some_and(|delta| !delta.is_finite() || !(-1.0..=1.0).contains(&delta))
    {
        return Err(DiagnosticContractError::ComparisonEvidence);
    }
    let classified = evidence
        .fixed_by_right_count
        .checked_add(evidence.regressed_by_right_count)
        .and_then(|value| value.checked_add(evidence.persistent_error_count))
        .ok_or(DiagnosticContractError::ComparisonEvidence)?;
    if classified > evidence.paired_support {
        return Err(DiagnosticContractError::ComparisonEvidence);
    }
    Ok(())
}

impl CellDiagnosticEvidence {
    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.evidence_fingerprint.clear();
        artifact_core::fingerprint(&input)
    }
}

impl TryFrom<&AnalysisReport> for DiagnosticContract {
    type Error = DiagnosticContractError;

    fn try_from(report: &AnalysisReport) -> Result<Self, Self::Error> {
        Self::from_report_and_reviews(report, &[])
    }
}

fn latest_reviews(
    report: &AnalysisReport,
    reviews: &[FindingReviewRecord],
) -> Result<BTreeMap<String, DiagnosticReviewDisposition>, DiagnosticContractError> {
    let finding_keys = report
        .findings
        .iter()
        .map(|finding| finding.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut latest = BTreeMap::<String, &FindingReviewRecord>::new();
    for review in reviews {
        if review.analysis_report_id != report.id
            || !finding_keys.contains(review.finding_key.as_str())
        {
            return Err(DiagnosticContractError::ReviewReference(review.id));
        }
        let replace = latest.get(&review.finding_key).is_none_or(|current| {
            (review.created_at, review.id) > (current.created_at, current.id)
        });
        if replace {
            latest.insert(review.finding_key.clone(), review);
        }
    }
    Ok(latest
        .into_iter()
        .map(|(key, review)| {
            (
                key,
                DiagnosticReviewDisposition {
                    review_id: review.id,
                    state: review.state,
                    resolution_evaluation_run_id: review.resolution_evaluation_run_id,
                    resolution_comparison_id: review.resolution_comparison_id,
                    created_at: review.created_at,
                },
            )
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn validate_finding_metrics(
    support: u64,
    error_count: u64,
    error_rate: f64,
    error_rate_lift: f64,
    error_share: f64,
    severity: f64,
    marginal_error_count: u64,
    cumulative_error_coverage: f64,
    baseline_error_rate: f64,
    total_errors: u64,
    finding_key: &str,
) -> Result<(), DiagnosticContractError> {
    let valid = support > 0
        && error_count <= support
        && marginal_error_count <= error_count
        && finite_unit(error_rate)
        && error_rate_lift.is_finite()
        && (-1.0..=1.0).contains(&error_rate_lift)
        && finite_unit(error_share)
        && finite_unit(severity)
        && finite_unit(cumulative_error_coverage)
        && close_enough(error_rate, divide(error_count, support))
        && close_enough(error_rate_lift, rounded(error_rate - baseline_error_rate))
        && close_enough(error_share, divide(error_count, total_errors));
    if !valid {
        return Err(DiagnosticContractError::InvalidCell(finding_key.into()));
    }
    Ok(())
}

fn validate_report_counts(report: &AnalysisReport) -> Result<(), DiagnosticContractError> {
    if report.prediction_count == 0
        || report.error_count > report.prediction_count
        || report.minimum_support == 0
        || report.evaluation_run_id == Uuid::nil()
    {
        return Err(DiagnosticContractError::ReportCounts);
    }
    Ok(())
}

fn is_legacy_source(source: &AnalysisSourceIdentity) -> bool {
    [
        source.evaluation_input_fingerprint.as_str(),
        source.evaluation_protocol_fingerprint.as_str(),
        source.cohort_fingerprint.as_str(),
    ]
    .iter()
    .any(|value| value.starts_with("legacy:"))
}

fn divide(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        rounded(numerator as f64 / denominator as f64)
    }
}

fn rounded(value: f64) -> f64 {
    (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
}

fn close_enough(left: f64, right: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= METRIC_EPSILON
}

fn finite_unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DiagnosticContractError {
    #[error("analysis report counts or minimum support are invalid")]
    ReportCounts,
    #[error("analysis source identity is missing or inconsistent")]
    SourceIdentity,
    #[error("decision-grade optimization requires non-legacy evaluation evidence identity")]
    LegacySourceIdentity,
    #[error("analysis report contains invalid cell finding {0}")]
    InvalidCell(String),
    #[error("cell findings are not in canonical order")]
    CellOrder,
    #[error("analysis review {0} does not reference this report and one of its findings")]
    ReviewReference(Uuid),
    #[error("comparison cell evidence is missing or internally inconsistent")]
    ComparisonEvidence,
    #[error("cell evidence fingerprint does not reproduce for {0}")]
    CellFingerprint(String),
    #[error("diagnostic contract fingerprint does not reproduce")]
    ContractFingerprint,
    #[error("could not fingerprint diagnostic evidence: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use uuid::Uuid;

    use crate::{
        domain::{
            AnalysisFinding, AnalysisReport, AnalysisSourceIdentity, FindingIdentity, FindingKind,
            FindingReviewRecord, FindingReviewState,
        },
        protocol::AnalysisProtocol,
    };

    use super::{DiagnosticContract, DiagnosticContractError};

    #[test]
    fn exposes_fingerprinted_metrics_source_and_latest_review() {
        let report = report();
        let old_review = review(&report, FindingReviewState::Open, 0);
        let latest_review = review(&report, FindingReviewState::AcceptedLimitation, 1);
        let contract = DiagnosticContract::from_report_and_reviews(
            &report,
            &[latest_review.clone(), old_review],
        )
        .expect("diagnostic contract");

        contract.validate(true).expect("valid contract");
        assert_eq!(contract.baseline_error_rate, 0.2);
        assert_eq!(contract.cells[0].error_share, 1.0);
        assert_eq!(
            contract.cells[0]
                .latest_review
                .as_ref()
                .map(|review| review.review_id),
            Some(latest_review.id)
        );
        assert_eq!(
            contract.reproduce_fingerprint().expect("fingerprint"),
            contract.fingerprint
        );
    }

    #[test]
    fn rejects_tampered_metrics_and_contract_fingerprints() {
        let mut contract = DiagnosticContract::try_from(&report()).expect("contract");
        contract.cells[0].error_rate = 0.9;
        assert!(matches!(
            contract.validate(true),
            Err(DiagnosticContractError::InvalidCell(_))
        ));

        let mut contract = DiagnosticContract::try_from(&report()).expect("contract");
        contract.fingerprint = "sha256:tampered".into();
        assert_eq!(
            contract.validate(true),
            Err(DiagnosticContractError::ContractFingerprint)
        );
    }

    fn report() -> AnalysisReport {
        let identity = FindingIdentity {
            kind: FindingKind::Cell,
            attributes: BTreeMap::from([
                ("label".into(), "billing".into()),
                ("style".into(), "clean".into()),
            ]),
        };
        let mut finding = AnalysisFinding {
            rank: 1,
            kind: FindingKind::Cell,
            key: identity.key(),
            attributes: identity.attributes,
            support: 10,
            error_count: 4,
            error_rate: 0.4,
            mean_error_confidence: 0.75,
            median_error_confidence: 0.75,
            mean_expected_probability: 0.2,
            mean_prediction_margin: 0.5,
            mean_entropy: 0.4,
            error_rate_lift: 0.2,
            error_share: 1.0,
            high_confidence_error_severity: 0.6,
            marginal_error_count: 4,
            cumulative_error_count: 4,
            cumulative_error_coverage: 1.0,
            fingerprint: String::new(),
        };
        finding.refresh_fingerprint().expect("finding fingerprint");
        let protocol = AnalysisProtocol::default();
        let evaluation_run_id = Uuid::new_v4();
        AnalysisReport {
            id: Uuid::new_v4(),
            evaluation_run_id,
            minimum_support: 1,
            prediction_count: 20,
            error_count: 4,
            findings: vec![finding],
            errors: Vec::new(),
            protocol_fingerprint: protocol.fingerprint().expect("protocol fingerprint"),
            protocol,
            source_identity: Some(AnalysisSourceIdentity {
                evaluation_run_id,
                evaluation_input_fingerprint: "sha256:evaluation".into(),
                evaluation_protocol_fingerprint: "sha256:evaluation-protocol".into(),
                cohort_fingerprint: "sha256:cohort".into(),
                prediction_count: 20,
                comparison_id: None,
                comparison_fingerprint: None,
            }),
            finding_evidence: BTreeMap::new(),
            comparison_diagnosis: None,
            fingerprint: "sha256:analysis".into(),
            created_at: Utc::now(),
        }
    }

    fn review(
        report: &AnalysisReport,
        state: FindingReviewState,
        seconds: i64,
    ) -> FindingReviewRecord {
        FindingReviewRecord {
            id: Uuid::new_v4(),
            analysis_report_id: report.id,
            finding_key: report.findings[0].key.clone(),
            state,
            note: None,
            resolution_evaluation_run_id: None,
            resolution_comparison_id: None,
            created_at: report.created_at + chrono::Duration::seconds(seconds),
        }
    }
}
