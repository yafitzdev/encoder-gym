use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::protocol::AnalysisProtocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    ExpectedLabel,
    PredictedLabel,
    ConfusionPair,
    DimensionValue,
    Cell,
    DimensionIntersection,
    ConfidenceBand,
    CorrectnessConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FindingIdentity {
    pub kind: FindingKind,
    pub attributes: BTreeMap<String, String>,
}

impl FindingIdentity {
    pub fn key(&self) -> String {
        serde_json::to_string(self).expect("finding identity serialization cannot fail")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingRankingPolicy {
    #[default]
    ErrorRate,
    ErrorCount,
    ErrorRateLift,
    ErrorShare,
    HighConfidenceErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSourceIdentity {
    pub evaluation_run_id: Uuid,
    pub evaluation_input_fingerprint: String,
    pub evaluation_protocol_fingerprint: String,
    pub cohort_fingerprint: String,
    pub prediction_count: u64,
    pub comparison_id: Option<Uuid>,
    pub comparison_fingerprint: Option<String>,
}

impl AnalysisSourceIdentity {
    pub fn legacy(evaluation_run_id: Uuid, prediction_count: u64) -> Self {
        Self {
            evaluation_run_id,
            evaluation_input_fingerprint: "legacy:unavailable".into(),
            evaluation_protocol_fingerprint: "legacy:unavailable".into(),
            cohort_fingerprint: "legacy:unavailable".into(),
            prediction_count,
            comparison_id: None,
            comparison_fingerprint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisFinding {
    pub rank: u64,
    pub kind: FindingKind,
    pub key: String,
    pub attributes: BTreeMap<String, String>,
    pub support: u64,
    pub error_count: u64,
    pub error_rate: f64,
    #[serde(default)]
    pub mean_error_confidence: f64,
    #[serde(default)]
    pub median_error_confidence: f64,
    #[serde(default)]
    pub mean_expected_probability: f64,
    #[serde(default)]
    pub mean_prediction_margin: f64,
    #[serde(default)]
    pub mean_entropy: f64,
    #[serde(default)]
    pub error_rate_lift: f64,
    #[serde(default)]
    pub error_share: f64,
    #[serde(default)]
    pub high_confidence_error_severity: f64,
    #[serde(default)]
    pub marginal_error_count: u64,
    #[serde(default)]
    pub cumulative_error_count: u64,
    #[serde(default)]
    pub cumulative_error_coverage: f64,
    #[serde(default)]
    pub fingerprint: String,
}

impl AnalysisFinding {
    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.fingerprint.clear();
        artifact_core::fingerprint(&input)
    }

    pub fn refresh_fingerprint(&mut self) -> Result<(), artifact_core::FingerprintError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceCategory {
    HighestConfidenceError,
    LowestMarginError,
    MedianConfidenceError,
    StableError,
    CorrectContrast,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingEvidenceReference {
    pub category: EvidenceCategory,
    pub prediction_id: Uuid,
    pub snapshot_member_id: Uuid,
    pub source_row_id: Uuid,
    pub confidence: f64,
    pub prediction_margin: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonEvidenceCategory {
    FixedByRight,
    RegressedByRight,
    PersistentError,
    HighConfidenceRegression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonEvidenceReference {
    pub category: ComparisonEvidenceCategory,
    pub snapshot_member_id: Uuid,
    pub source_row_id: Uuid,
    pub left_prediction_id: Uuid,
    pub right_prediction_id: Uuid,
    pub left_confidence: f64,
    pub right_confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticSliceDelta {
    pub identity_key: String,
    pub support: u64,
    pub accuracy_delta: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonCellEvidence {
    pub paired_support: u64,
    pub fixed_by_right_count: u64,
    pub regressed_by_right_count: u64,
    pub persistent_error_count: u64,
    pub high_confidence_regression_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonDiagnosis {
    pub comparison_id: Uuid,
    pub left_evaluation_run_id: Uuid,
    pub right_evaluation_run_id: Uuid,
    pub paired_prediction_count: u64,
    pub fixed_by_right_count: u64,
    pub regressed_by_right_count: u64,
    pub persistent_error_count: u64,
    pub high_confidence_regression_count: u64,
    pub slice_deltas: Vec<DiagnosticSliceDelta>,
    #[serde(default)]
    pub cell_evidence: BTreeMap<String, ComparisonCellEvidence>,
    pub evidence: BTreeMap<ComparisonEvidenceCategory, Vec<ComparisonEvidenceReference>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MisclassifiedExample {
    pub prediction_id: Uuid,
    pub snapshot_member_id: Uuid,
    pub source_row_id: Uuid,
    pub text: String,
    pub expected_label: String,
    pub predicted_label: String,
    pub confidence: f64,
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub id: Uuid,
    pub evaluation_run_id: Uuid,
    pub minimum_support: u64,
    pub prediction_count: u64,
    pub error_count: u64,
    pub findings: Vec<AnalysisFinding>,
    pub errors: Vec<MisclassifiedExample>,
    #[serde(default)]
    pub protocol: AnalysisProtocol,
    #[serde(default)]
    pub protocol_fingerprint: String,
    #[serde(default)]
    pub source_identity: Option<AnalysisSourceIdentity>,
    #[serde(default)]
    pub finding_evidence: BTreeMap<String, Vec<FindingEvidenceReference>>,
    #[serde(default)]
    pub comparison_diagnosis: Option<ComparisonDiagnosis>,
    #[serde(default)]
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingReviewState {
    Open,
    Acknowledged,
    AcceptedLimitation,
    CandidateForMoreData,
    CandidateForLabelSchemaReview,
    ResolvedByLaterEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingReviewRecord {
    pub id: Uuid,
    pub analysis_report_id: Uuid,
    pub finding_key: String,
    pub state: FindingReviewState,
    pub note: Option<String>,
    pub resolution_evaluation_run_id: Option<Uuid>,
    pub resolution_comparison_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
