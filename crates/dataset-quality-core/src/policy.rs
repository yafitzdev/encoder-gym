//! Explicit integer quality thresholds and finite audit budgets.

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{QualityError, fingerprint};

pub const QUALITY_POLICY_SCHEMA_VERSION: u32 = 1;
pub const MAX_BASIS_POINTS: u16 = 10_000;

/// An exact score in hundredths of one percent.
///
/// The inner value is private so invalid scores cannot be constructed by
/// ordinary Rust callers. The custom deserializer applies the same invariant
/// to persisted and provider-produced JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct BasisPoints(u16);

impl BasisPoints {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(MAX_BASIS_POINTS);

    pub fn new(value: u16) -> Result<Self, QualityError> {
        if value > MAX_BASIS_POINTS {
            return Err(QualityError::Validation(format!(
                "basis-point score {value} exceeds {MAX_BASIS_POINTS}"
            )));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for BasisPoints {
    type Error = QualityError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<BasisPoints> for u16 {
    fn from(value: BasisPoints) -> Self {
        value.get()
    }
}

impl<'de> Deserialize<'de> for BasisPoints {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    Fast,
    Balanced,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditMode {
    /// Every pinned row must receive direct assessment evidence.
    FullPopulation,
    /// Selects a stable bounded subset for a report only. Unselected rows are
    /// never inferred to be qualified and remain explicit in the audit plan.
    DeterministicSampleReportOnly {
        sample_size: u64,
        seed: u64,
        minimum_rows_per_cell: u32,
    },
}

impl AuditMode {
    pub fn validate(&self) -> Result<(), QualityError> {
        if let Self::DeterministicSampleReportOnly {
            sample_size,
            minimum_rows_per_cell,
            ..
        } = self
        {
            if *sample_size == 0 || *minimum_rows_per_cell == 0 {
                return Err(QualityError::Validation(
                    "deterministic report-only sample size and per-cell minimum must be positive"
                        .into(),
                ));
            }
        }
        Ok(())
    }
}

/// Whether candidate text may leave the local process boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorEgressPolicy {
    LocalOnly,
    ExternalCandidateText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidEvaluatorOutputPolicy {
    Quarantine,
    FailAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BorderlineReviewPolicy {
    None,
    Independent { maximum_additional_assessments: u8 },
}

impl BorderlineReviewPolicy {
    pub fn validate(self) -> Result<(), QualityError> {
        if matches!(
            self,
            Self::Independent {
                maximum_additional_assessments: 0
            }
        ) {
            return Err(QualityError::Validation(
                "independent borderline review requires at least one additional assessment".into(),
            ));
        }
        Ok(())
    }

    /// Number of independently configured reviewers that must assess a row
    /// whose primary assessment is borderline before a run may complete.
    pub const fn required_additional_assessments(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Independent {
                maximum_additional_assessments,
            } => maximum_additional_assessments,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityThresholds {
    pub minimum_assigned_label_score: BasisPoints,
    pub minimum_label_margin: BasisPoints,
    pub minimum_dimension_adherence_score: BasisPoints,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_authenticity_score: Option<BasisPoints>,
    pub maximum_label_leakage_risk: BasisPoints,
    pub maximum_shortcut_risk: BasisPoints,
    pub minimum_evaluator_confidence: BasisPoints,
    pub borderline_margin: BasisPoints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditBudgets {
    pub maximum_rows_per_batch: u32,
    pub maximum_evaluator_requests: u32,
    pub maximum_attempts_per_request: u32,
    pub maximum_input_tokens: u64,
    pub maximum_output_tokens: u64,
    pub maximum_total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microusd: Option<u64>,
}

impl AuditBudgets {
    pub fn validate(&self) -> Result<(), QualityError> {
        if self.maximum_rows_per_batch == 0
            || self.maximum_evaluator_requests == 0
            || self.maximum_attempts_per_request == 0
            || self.maximum_input_tokens == 0
            || self.maximum_output_tokens == 0
            || self.maximum_total_tokens == 0
        {
            return Err(QualityError::Validation(
                "audit row, request, attempt, and token budgets must be finite and positive".into(),
            ));
        }
        if self.maximum_rows_per_batch > 10_000
            || self.maximum_evaluator_requests > 1_000_000
            || self.maximum_attempts_per_request > 100
            || self.maximum_input_tokens > 1_000_000_000_000
            || self.maximum_output_tokens > 1_000_000_000_000
            || self.maximum_total_tokens > 2_000_000_000_000
            || self.maximum_cost_microusd.unwrap_or(0) > 1_000_000_000_000
        {
            return Err(QualityError::Validation(
                "audit budgets exceed local safety ceilings".into(),
            ));
        }
        Ok(())
    }

    pub fn maximum_row_capacity(&self) -> u64 {
        u64::from(self.maximum_rows_per_batch) * u64::from(self.maximum_evaluator_requests)
    }

    pub fn maximum_request_attempts(&self) -> u64 {
        u64::from(self.maximum_evaluator_requests) * u64::from(self.maximum_attempts_per_request)
    }

    pub fn required_requests_for_rows(
        &self,
        rows: u64,
        assessment_rounds: u64,
    ) -> Result<u64, QualityError> {
        if assessment_rounds == 0 {
            return Err(QualityError::Validation(
                "audit assessment rounds must be positive".into(),
            ));
        }
        let batch_size = u64::from(self.maximum_rows_per_batch);
        let primary_requests = rows
            .checked_add(batch_size - 1)
            .ok_or_else(|| QualityError::Validation("audit request capacity overflowed".into()))?
            / batch_size;
        primary_requests
            .checked_mul(assessment_rounds)
            .ok_or_else(|| QualityError::Validation("audit request capacity overflowed".into()))
    }
}

/// Controls that cannot safely be hidden inside an operator-facing preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityPolicyPresetControls {
    pub audit_mode: AuditMode,
    pub egress_policy: EvaluatorEgressPolicy,
    pub evaluate_authenticity: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microusd: Option<u64>,
}

/// A fully resolved immutable policy. Runners use only these explicit values;
/// they never reinterpret a preset name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityPolicy {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<QualityPreset>,
    pub thresholds: QualityThresholds,
    pub invalid_output_policy: InvalidEvaluatorOutputPolicy,
    pub borderline_review_policy: BorderlineReviewPolicy,
    pub budgets: AuditBudgets,
    pub egress_policy: EvaluatorEgressPolicy,
    pub audit_mode: AuditMode,
    pub fingerprint: String,
}

impl QualityPreset {
    pub fn compile(
        self,
        controls: QualityPolicyPresetControls,
    ) -> Result<QualityPolicy, QualityError> {
        controls.audit_mode.validate()?;
        let (thresholds, invalid_output_policy, borderline_review_policy, budgets) = match self {
            Self::Fast => (
                thresholds(
                    7_000, 750, 6_500, 6_000, 2_500, 3_000, 6_000, 500, &controls,
                )?,
                InvalidEvaluatorOutputPolicy::Quarantine,
                BorderlineReviewPolicy::None,
                preset_budgets(
                    64,
                    100_000,
                    2,
                    250_000_000,
                    100_000_000,
                    300_000_000,
                    &controls,
                ),
            ),
            Self::Balanced => (
                thresholds(
                    8_000, 1_200, 7_500, 7_000, 1_500, 2_000, 7_000, 600, &controls,
                )?,
                InvalidEvaluatorOutputPolicy::Quarantine,
                BorderlineReviewPolicy::Independent {
                    maximum_additional_assessments: 1,
                },
                preset_budgets(
                    32,
                    200_000,
                    3,
                    500_000_000,
                    200_000_000,
                    600_000_000,
                    &controls,
                ),
            ),
            Self::Strict => (
                thresholds(
                    9_000, 1_800, 8_500, 8_000, 750, 1_000, 8_000, 750, &controls,
                )?,
                InvalidEvaluatorOutputPolicy::FailAudit,
                BorderlineReviewPolicy::Independent {
                    maximum_additional_assessments: 2,
                },
                preset_budgets(
                    16,
                    400_000,
                    3,
                    1_000_000_000,
                    400_000_000,
                    1_200_000_000,
                    &controls,
                ),
            ),
        };
        QualityPolicy::new(
            Some(self),
            thresholds,
            invalid_output_policy,
            borderline_review_policy,
            budgets,
            controls.egress_policy,
            controls.audit_mode,
        )
    }
}

impl QualityPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        preset: Option<QualityPreset>,
        thresholds: QualityThresholds,
        invalid_output_policy: InvalidEvaluatorOutputPolicy,
        borderline_review_policy: BorderlineReviewPolicy,
        budgets: AuditBudgets,
        egress_policy: EvaluatorEgressPolicy,
        audit_mode: AuditMode,
    ) -> Result<Self, QualityError> {
        let mut value = Self {
            schema_version: QUALITY_POLICY_SCHEMA_VERSION,
            preset,
            thresholds,
            invalid_output_policy,
            borderline_review_policy,
            budgets,
            egress_policy,
            audit_mode,
            fingerprint: String::new(),
        };
        value.validate()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.schema_version != QUALITY_POLICY_SCHEMA_VERSION {
            return Err(QualityError::Validation(format!(
                "unsupported quality policy schema version {}",
                self.schema_version
            )));
        }
        self.audit_mode.validate()?;
        self.borderline_review_policy.validate()?;
        self.budgets.validate()?;
        Ok(())
    }

    pub fn verify_integrity(&self) -> Result<(), QualityError> {
        self.validate()?;
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(QualityError::Integrity(
                "quality policy fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[allow(clippy::too_many_arguments)]
fn thresholds(
    assigned_label: u16,
    label_margin: u16,
    dimension: u16,
    authenticity: u16,
    leakage: u16,
    shortcut: u16,
    confidence: u16,
    borderline: u16,
    controls: &QualityPolicyPresetControls,
) -> Result<QualityThresholds, QualityError> {
    Ok(QualityThresholds {
        minimum_assigned_label_score: BasisPoints::new(assigned_label)?,
        minimum_label_margin: BasisPoints::new(label_margin)?,
        minimum_dimension_adherence_score: BasisPoints::new(dimension)?,
        minimum_authenticity_score: controls
            .evaluate_authenticity
            .then(|| BasisPoints::new(authenticity))
            .transpose()?,
        maximum_label_leakage_risk: BasisPoints::new(leakage)?,
        maximum_shortcut_risk: BasisPoints::new(shortcut)?,
        minimum_evaluator_confidence: BasisPoints::new(confidence)?,
        borderline_margin: BasisPoints::new(borderline)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn preset_budgets(
    maximum_rows_per_batch: u32,
    maximum_evaluator_requests: u32,
    maximum_attempts_per_request: u32,
    maximum_input_tokens: u64,
    maximum_output_tokens: u64,
    maximum_total_tokens: u64,
    controls: &QualityPolicyPresetControls,
) -> AuditBudgets {
    AuditBudgets {
        maximum_rows_per_batch,
        maximum_evaluator_requests,
        maximum_attempts_per_request,
        maximum_input_tokens,
        maximum_output_tokens,
        maximum_total_tokens,
        maximum_cost_microusd: controls.maximum_cost_microusd,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuditBudgets, AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
        QualityPreset,
    };

    fn controls() -> QualityPolicyPresetControls {
        QualityPolicyPresetControls {
            audit_mode: AuditMode::FullPopulation,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            evaluate_authenticity: false,
            maximum_cost_microusd: Some(1_000_000),
        }
    }

    #[test]
    fn basis_points_reject_out_of_range_construction_and_json() {
        assert!(BasisPoints::new(10_001).is_err());
        assert!(serde_json::from_str::<BasisPoints>("10001").is_err());
        assert!(serde_json::from_str::<BasisPoints>("70000").is_err());
        assert_eq!(
            serde_json::from_str::<BasisPoints>("10000")
                .expect("maximum score")
                .get(),
            10_000
        );
    }

    #[test]
    fn preset_compiles_to_explicit_reproducible_policy() {
        let policy = QualityPreset::Balanced
            .compile(controls())
            .expect("resolved policy");

        assert_eq!(policy.preset, Some(QualityPreset::Balanced));
        assert_eq!(
            policy.reproduce_fingerprint().expect("fingerprint"),
            policy.fingerprint
        );
        assert!(policy.verify_integrity().is_ok());
    }

    #[test]
    fn tampered_policy_and_zero_budgets_are_rejected() {
        let mut policy = QualityPreset::Balanced
            .compile(controls())
            .expect("resolved policy");
        policy.budgets.maximum_rows_per_batch = 0;
        assert!(policy.verify_integrity().is_err());

        let invalid = AuditBudgets {
            maximum_rows_per_batch: 1,
            maximum_evaluator_requests: 1,
            maximum_attempts_per_request: 1,
            maximum_input_tokens: 0,
            maximum_output_tokens: 1,
            maximum_total_tokens: 1,
            maximum_cost_microusd: None,
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn report_only_sampling_must_select_at_least_one_row() {
        let mut value = controls();
        value.audit_mode = AuditMode::DeterministicSampleReportOnly {
            sample_size: 0,
            seed: 42,
            minimum_rows_per_cell: 1,
        };
        assert!(QualityPreset::Fast.compile(value).is_err());
    }
}
