use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotSplit;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use training_core::domain::LabelProbability;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EvaluationDomainError {
    #[error("evaluation requires at least one example")]
    EmptyEvaluationSet,
    #[error("evaluation requires at least two unique labels")]
    Labels,
    #[error("evaluation batch_size must be between 1 and 10,000")]
    BatchSize,
    #[error("top_k must contain unique positive values in ascending order")]
    TopK,
    #[error("calibration_bins must be between 2 and 1,000")]
    CalibrationBins,
    #[error("minimum_slice_support must be greater than zero")]
    MinimumSliceSupport,
    #[error("bootstrap_samples must be between 1 and 100,000")]
    BootstrapSamples,
    #[error("confidence_level must be finite and strictly between zero and one")]
    ConfidenceLevel,
    #[error("dimension intersections must contain sorted, unique, non-empty names")]
    DimensionIntersections,
    #[error("invalid slice identity: {reason}")]
    InvalidSliceIdentity { reason: String },
    #[error("invalid evaluation-run transition from {from:?} to {to:?}")]
    Transition {
        from: EvaluationRunState,
        to: EvaluationRunState,
    },
    #[error("a reserved evaluation-run identity requires a non-nil pristine queued run")]
    RunIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationProtocol {
    pub split: SnapshotSplit,
    pub batch_size: usize,
    pub top_k: Vec<usize>,
    pub calibration_bins: usize,
    pub minimum_slice_support: u64,
    pub dimension_intersections: Vec<Vec<String>>,
    pub bootstrap_samples: u32,
    pub statistical_seed: u64,
    pub confidence_level: f64,
}

impl Default for EvaluationProtocol {
    fn default() -> Self {
        Self {
            split: SnapshotSplit::Test,
            batch_size: 32,
            top_k: vec![1],
            calibration_bins: 10,
            minimum_slice_support: 1,
            dimension_intersections: Vec::new(),
            bootstrap_samples: 1_000,
            statistical_seed: 42,
            confidence_level: 0.95,
        }
    }
}

impl EvaluationProtocol {
    pub fn validate(&self) -> Result<(), EvaluationDomainError> {
        if !(1..=10_000).contains(&self.batch_size) {
            return Err(EvaluationDomainError::BatchSize);
        }
        if self.top_k.is_empty()
            || self.top_k.contains(&0)
            || self.top_k.windows(2).any(|values| values[0] >= values[1])
        {
            return Err(EvaluationDomainError::TopK);
        }
        if !(2..=1_000).contains(&self.calibration_bins) {
            return Err(EvaluationDomainError::CalibrationBins);
        }
        if self.minimum_slice_support == 0 {
            return Err(EvaluationDomainError::MinimumSliceSupport);
        }
        if !(1..=100_000).contains(&self.bootstrap_samples) {
            return Err(EvaluationDomainError::BootstrapSamples);
        }
        if !self.confidence_level.is_finite() || !(0.0..1.0).contains(&self.confidence_level) {
            return Err(EvaluationDomainError::ConfidenceLevel);
        }
        if self.dimension_intersections.iter().any(|intersection| {
            intersection.is_empty()
                || intersection.iter().any(|name| name.trim().is_empty())
                || intersection.windows(2).any(|names| names[0] >= names[1])
        }) || self
            .dimension_intersections
            .windows(2)
            .any(|intersections| intersections[0] >= intersections[1])
        {
            return Err(EvaluationDomainError::DimensionIntersections);
        }
        Ok(())
    }

    pub fn validate_for_labels(&self, labels: &[String]) -> Result<(), EvaluationDomainError> {
        self.validate()?;
        if self.top_k.iter().any(|value| *value > labels.len()) {
            return Err(EvaluationDomainError::TopK);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        artifact_core::fingerprint(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationRunState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl EvaluationRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSourceIdentity {
    pub checkpoint_checksum: String,
    pub checkpoint_model_format: String,
    pub base_model_fingerprint: Option<String>,
    pub tokenizer_fingerprint: Option<String>,
    pub snapshot_fingerprint: String,
    pub cohort_fingerprint: String,
    pub labels: Vec<String>,
}

impl EvaluationSourceIdentity {
    pub fn legacy() -> Self {
        Self {
            checkpoint_checksum: "legacy:unavailable".into(),
            checkpoint_model_format: "legacy:unavailable".into(),
            base_model_fingerprint: None,
            tokenizer_fingerprint: None,
            snapshot_fingerprint: "legacy:unavailable".into(),
            cohort_fingerprint: "legacy:unavailable".into(),
            labels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationRun {
    pub id: Uuid,
    pub checkpoint_id: Uuid,
    pub snapshot_id: Uuid,
    pub split: SnapshotSplit,
    pub input_fingerprint: String,
    #[serde(default)]
    pub protocol: EvaluationProtocol,
    #[serde(default = "legacy_fingerprint")]
    pub protocol_fingerprint: String,
    #[serde(default = "EvaluationSourceIdentity::legacy")]
    pub source_identity: EvaluationSourceIdentity,
    pub state: EvaluationRunState,
    #[serde(default)]
    pub total_examples: u64,
    #[serde(default)]
    pub processed_examples: u64,
    #[serde(default)]
    pub completed_batches: u64,
    #[serde(default)]
    pub current_batch: u64,
    #[serde(default)]
    pub correct_predictions: u64,
    #[serde(default)]
    pub elapsed_milliseconds: u64,
    #[serde(default)]
    pub cancel_requested: bool,
    pub example_count: u64,
    pub metrics: Option<EvaluationMetrics>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EvaluationRun {
    pub fn queued(
        checkpoint_id: Uuid,
        snapshot_id: Uuid,
        split: SnapshotSplit,
        input_fingerprint: impl Into<String>,
    ) -> Self {
        let protocol = EvaluationProtocol {
            split,
            ..EvaluationProtocol::default()
        };
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            checkpoint_id,
            snapshot_id,
            split,
            input_fingerprint: input_fingerprint.into(),
            protocol_fingerprint: protocol
                .fingerprint()
                .expect("default evaluation protocol is serializable"),
            protocol,
            source_identity: EvaluationSourceIdentity::legacy(),
            state: EvaluationRunState::Queued,
            total_examples: 0,
            processed_examples: 0,
            completed_batches: 0,
            current_batch: 0,
            correct_predictions: 0,
            elapsed_milliseconds: 0,
            cancel_requested: false,
            example_count: 0,
            metrics: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn queued_with_protocol(
        checkpoint_id: Uuid,
        snapshot_id: Uuid,
        protocol: EvaluationProtocol,
        source_identity: EvaluationSourceIdentity,
        total_examples: u64,
        input_fingerprint: impl Into<String>,
    ) -> Result<Self, EvaluationDomainError> {
        protocol.validate_for_labels(&source_identity.labels)?;
        if total_examples == 0 {
            return Err(EvaluationDomainError::EmptyEvaluationSet);
        }
        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4(),
            checkpoint_id,
            snapshot_id,
            split: protocol.split,
            input_fingerprint: input_fingerprint.into(),
            protocol_fingerprint: protocol
                .fingerprint()
                .expect("evaluation protocol is serializable"),
            protocol,
            source_identity,
            state: EvaluationRunState::Queued,
            total_examples,
            processed_examples: 0,
            completed_batches: 0,
            current_batch: 0,
            correct_predictions: 0,
            elapsed_milliseconds: 0,
            cancel_requested: false,
            example_count: 0,
            metrics: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn transition(&mut self, next: EvaluationRunState) -> Result<(), EvaluationDomainError> {
        let allowed = matches!(
            (self.state, next),
            (
                EvaluationRunState::Queued,
                EvaluationRunState::Running
                    | EvaluationRunState::Failed
                    | EvaluationRunState::Cancelled
            ) | (
                EvaluationRunState::Running,
                EvaluationRunState::Completed
                    | EvaluationRunState::Failed
                    | EvaluationRunState::Cancelled
            )
        );
        if !allowed {
            return Err(EvaluationDomainError::Transition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Replaces the generated identity before a pristine queued run enters
    /// persistence so orchestration can reserve and link it durably.
    pub fn with_reserved_id(mut self, id: Uuid) -> Result<Self, EvaluationDomainError> {
        if id.is_nil()
            || self.state != EvaluationRunState::Queued
            || self.processed_examples != 0
            || self.completed_batches != 0
            || self.current_batch != 0
            || self.correct_predictions != 0
            || self.elapsed_milliseconds != 0
            || self.cancel_requested
            || self.example_count != 0
            || self.metrics.is_some()
            || self.error_message.is_some()
        {
            return Err(EvaluationDomainError::RunIdentity);
        }
        self.id = id;
        Ok(self)
    }
}

fn legacy_fingerprint() -> String {
    "legacy:unavailable".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationExample {
    pub snapshot_member_id: Uuid,
    pub source_row_id: Uuid,
    pub text: String,
    pub expected_label: String,
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationPrediction {
    pub id: Uuid,
    pub evaluation_run_id: Uuid,
    pub snapshot_member_id: Uuid,
    pub source_row_id: Uuid,
    pub text: String,
    pub expected_label: String,
    pub predicted_label: String,
    pub confidence: f64,
    pub probabilities: Vec<LabelProbability>,
    pub dimensions: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub overall: ClassificationMetrics,
    pub by_dimension: BTreeMap<String, ClassificationMetrics>,
    #[serde(default)]
    pub slices: BTreeMap<String, SliceMetrics>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassificationMetrics {
    pub total: u64,
    pub correct: u64,
    pub accuracy: f64,
    pub macro_precision: f64,
    pub macro_recall: f64,
    pub macro_f1: f64,
    #[serde(default)]
    pub weighted_precision: f64,
    #[serde(default)]
    pub weighted_recall: f64,
    #[serde(default)]
    pub weighted_f1: f64,
    #[serde(default)]
    pub top_k_accuracy: BTreeMap<usize, f64>,
    #[serde(default)]
    pub log_loss: f64,
    #[serde(default)]
    pub brier_score: f64,
    #[serde(default)]
    pub expected_calibration_error: f64,
    #[serde(default)]
    pub mean_confidence: f64,
    #[serde(default)]
    pub mean_correct_confidence: f64,
    #[serde(default)]
    pub mean_incorrect_confidence: f64,
    pub per_label: BTreeMap<String, LabelMetrics>,
    pub confusion_matrix: BTreeMap<String, BTreeMap<String, u64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceKind {
    ExpectedLabel,
    DimensionValue,
    Cell,
    DimensionIntersection,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceIdentity {
    pub kind: SliceKind,
    pub attributes: BTreeMap<String, String>,
}

impl SliceIdentity {
    pub fn key(&self) -> String {
        serde_json::to_string(self).expect("slice identity serialization cannot fail")
    }

    pub fn parse_key_for_labels(
        key: &str,
        labels: &[String],
    ) -> Result<Self, EvaluationDomainError> {
        let identity = serde_json::from_str::<Self>(key).map_err(|error| {
            invalid_slice_identity(format!("key is not a valid slice identity: {error}"))
        })?;
        identity.validate_for_labels(labels)?;
        if identity.key() != key {
            return Err(invalid_slice_identity(
                "key is not in canonical serialized form",
            ));
        }
        Ok(identity)
    }

    pub fn validate_for_labels(&self, labels: &[String]) -> Result<(), EvaluationDomainError> {
        if self
            .attributes
            .iter()
            .any(|(name, value)| !is_canonical_field(name) || !is_canonical_field(value))
        {
            return Err(invalid_slice_identity(
                "attribute names and values must be non-empty and have no surrounding whitespace",
            ));
        }

        match self.kind {
            SliceKind::ExpectedLabel => {
                let label = self.attributes.get("label").ok_or_else(|| {
                    invalid_slice_identity(
                        "expected-label slices require exactly one `label` attribute",
                    )
                })?;
                if self.attributes.len() != 1 {
                    return Err(invalid_slice_identity(
                        "expected-label slices require exactly one `label` attribute",
                    ));
                }
                validate_known_label(label, labels)
            }
            SliceKind::DimensionValue => {
                let dimension = self.attributes.get("dimension").ok_or_else(|| {
                    invalid_slice_identity(
                        "dimension-value slices require exactly `dimension` and `value` attributes",
                    )
                })?;
                if self.attributes.len() != 2
                    || !self.attributes.contains_key("value")
                    || self.attributes.contains_key("label")
                    || dimension == "label"
                {
                    return Err(invalid_slice_identity(
                        "dimension-value slices require exactly `dimension` and `value` attributes, and `dimension` must not be `label`",
                    ));
                }
                Ok(())
            }
            SliceKind::Cell => {
                let label = self.attributes.get("label").ok_or_else(|| {
                    invalid_slice_identity(
                        "cell slices require a `label` and at least one dimension attribute",
                    )
                })?;
                if self.attributes.len() < 2 {
                    return Err(invalid_slice_identity(
                        "cell slices require a `label` and at least one dimension attribute",
                    ));
                }
                validate_known_label(label, labels)
            }
            SliceKind::DimensionIntersection => {
                if self.attributes.is_empty() || self.attributes.contains_key("label") {
                    return Err(invalid_slice_identity(
                        "dimension-intersection slices require at least one dimension attribute and must not contain `label`",
                    ));
                }
                Ok(())
            }
        }
    }
}

fn is_canonical_field(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

fn validate_known_label(label: &str, labels: &[String]) -> Result<(), EvaluationDomainError> {
    if labels.iter().any(|known| known == label) {
        Ok(())
    } else {
        Err(invalid_slice_identity(format!(
            "`{label}` is not a known label"
        )))
    }
}

fn invalid_slice_identity(reason: impl Into<String>) -> EvaluationDomainError {
    EvaluationDomainError::InvalidSliceIdentity {
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceMetrics {
    pub identity: SliceIdentity,
    pub support: u64,
    pub metrics: ClassificationMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelMetrics {
    pub support: u64,
    pub predicted: u64,
    pub true_positive: u64,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationComparison {
    pub left_run_id: Uuid,
    pub right_run_id: Uuid,
    pub accuracy_delta: f64,
    pub macro_f1_delta: f64,
    pub per_label_f1_delta: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    pub level: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McNemarResult {
    pub left_only_correct: u64,
    pub right_only_correct: u64,
    pub two_sided_p_value: f64,
    pub significant: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedSliceDelta {
    pub identity: SliceIdentity,
    pub support: u64,
    pub accuracy_delta: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationComparisonReport {
    pub id: Uuid,
    pub left_run_id: Uuid,
    pub right_run_id: Uuid,
    pub cohort_fingerprint: String,
    pub protocol_fingerprint: String,
    pub left_metrics: EvaluationMetrics,
    pub right_metrics: EvaluationMetrics,
    pub accuracy_delta: f64,
    pub macro_f1_delta: f64,
    pub per_label_f1_delta: BTreeMap<String, f64>,
    pub both_correct: u64,
    pub both_wrong: u64,
    pub left_only_correct: u64,
    pub right_only_correct: u64,
    pub fixed_snapshot_member_ids: Vec<Uuid>,
    pub regressed_snapshot_member_ids: Vec<Uuid>,
    pub accuracy_delta_interval: ConfidenceInterval,
    pub macro_f1_delta_interval: ConfidenceInterval,
    pub mcnemar: McNemarResult,
    pub slice_deltas: BTreeMap<String, PairedSliceDelta>,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardMetric {
    Accuracy,
    MacroF1,
    WeightedF1,
    LogLoss,
    BrierScore,
    ExpectedCalibrationError,
}

impl LeaderboardMetric {
    pub const fn lower_is_better(self) -> bool {
        matches!(
            self,
            Self::LogLoss | Self::BrierScore | Self::ExpectedCalibrationError
        )
    }

    pub fn value(self, metrics: &ClassificationMetrics) -> f64 {
        match self {
            Self::Accuracy => metrics.accuracy,
            Self::MacroF1 => metrics.macro_f1,
            Self::WeightedF1 => metrics.weighted_f1,
            Self::LogLoss => metrics.log_loss,
            Self::BrierScore => metrics.brier_score,
            Self::ExpectedCalibrationError => metrics.expected_calibration_error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub evaluation_run_id: Uuid,
    pub checkpoint_id: Uuid,
    pub metric_value: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSelectionReport {
    pub id: Uuid,
    pub candidate_run_ids: Vec<Uuid>,
    pub metric: LeaderboardMetric,
    pub minimum_improvement: Option<f64>,
    pub required_confidence: Option<f64>,
    pub selected_run_id: Option<Uuid>,
    pub selected_checkpoint_id: Option<Uuid>,
    pub reasons: Vec<String>,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{EvaluationDomainError, EvaluationProtocol, SliceIdentity, SliceKind};

    #[test]
    fn validates_and_fingerprints_normalized_protocols() {
        let protocol = EvaluationProtocol {
            top_k: vec![1, 2],
            dimension_intersections: vec![vec!["difficulty".into(), "style".into()]],
            ..EvaluationProtocol::default()
        };
        protocol
            .validate_for_labels(&["a".into(), "b".into()])
            .expect("valid protocol");
        assert_eq!(
            protocol.fingerprint().expect("fingerprint"),
            protocol.fingerprint().expect("repeat fingerprint")
        );

        let unsorted = EvaluationProtocol {
            top_k: vec![2, 1],
            ..EvaluationProtocol::default()
        };
        assert_eq!(unsorted.validate(), Err(EvaluationDomainError::TopK));

        let oversized = EvaluationProtocol {
            top_k: vec![1, 3],
            ..EvaluationProtocol::default()
        };
        assert_eq!(
            oversized.validate_for_labels(&["a".into(), "b".into()]),
            Err(EvaluationDomainError::TopK)
        );
    }

    #[test]
    fn parses_canonical_slice_keys_for_each_supported_shape() {
        let labels = vec!["billing".into(), "fraud".into()];
        let identities = [
            SliceIdentity {
                kind: SliceKind::ExpectedLabel,
                attributes: BTreeMap::from([("label".into(), "billing".into())]),
            },
            SliceIdentity {
                kind: SliceKind::DimensionValue,
                attributes: BTreeMap::from([
                    ("dimension".into(), "difficulty".into()),
                    ("value".into(), "hard".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::Cell,
                attributes: BTreeMap::from([
                    ("difficulty".into(), "hard".into()),
                    ("label".into(), "fraud".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::DimensionIntersection,
                attributes: BTreeMap::from([
                    ("difficulty".into(), "hard".into()),
                    ("style".into(), "messy".into()),
                ]),
            },
        ];

        for identity in identities {
            identity
                .validate_for_labels(&labels)
                .expect("valid slice identity");
            assert_eq!(
                SliceIdentity::parse_key_for_labels(&identity.key(), &labels)
                    .expect("canonical slice key"),
                identity
            );
        }
    }

    #[test]
    fn rejects_invalid_slice_shapes_and_values() {
        let labels = vec!["billing".into(), "fraud".into()];
        let invalid = [
            SliceIdentity {
                kind: SliceKind::ExpectedLabel,
                attributes: BTreeMap::new(),
            },
            SliceIdentity {
                kind: SliceKind::ExpectedLabel,
                attributes: BTreeMap::from([("label".into(), "unknown".into())]),
            },
            SliceIdentity {
                kind: SliceKind::ExpectedLabel,
                attributes: BTreeMap::from([
                    ("label".into(), "billing".into()),
                    ("style".into(), "clean".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::DimensionValue,
                attributes: BTreeMap::from([("dimension".into(), "difficulty".into())]),
            },
            SliceIdentity {
                kind: SliceKind::DimensionValue,
                attributes: BTreeMap::from([
                    ("dimension".into(), "label".into()),
                    ("value".into(), "billing".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::Cell,
                attributes: BTreeMap::from([("label".into(), "billing".into())]),
            },
            SliceIdentity {
                kind: SliceKind::Cell,
                attributes: BTreeMap::from([
                    ("difficulty".into(), "hard".into()),
                    ("label".into(), "unknown".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::Cell,
                attributes: BTreeMap::from([
                    ("difficulty".into(), " ".into()),
                    ("label".into(), "billing".into()),
                ]),
            },
            SliceIdentity {
                kind: SliceKind::DimensionIntersection,
                attributes: BTreeMap::new(),
            },
            SliceIdentity {
                kind: SliceKind::DimensionIntersection,
                attributes: BTreeMap::from([("label".into(), "billing".into())]),
            },
        ];

        for identity in invalid {
            assert!(matches!(
                identity.validate_for_labels(&labels),
                Err(EvaluationDomainError::InvalidSliceIdentity { .. })
            ));
        }
    }

    #[test]
    fn rejects_noncanonical_missing_and_unknown_slice_key_fields() {
        let labels = vec!["billing".into(), "fraud".into()];
        let noncanonical = [
            r#"{ "kind":"expected_label","attributes":{"label":"billing"}}"#,
            r#"{"attributes":{"label":"billing"},"kind":"expected_label"}"#,
            r#"{"kind":"expected_label","attributes":{"label":"billing"},"extra":true}"#,
            r#"{"kind":"expected_label"}"#,
            r#"{"kind":"expected_label","attributes":{"label":"billing","unknown":"value"}}"#,
        ];

        for key in noncanonical {
            assert!(matches!(
                SliceIdentity::parse_key_for_labels(key, &labels),
                Err(EvaluationDomainError::InvalidSliceIdentity { .. })
            ));
        }
    }
}
