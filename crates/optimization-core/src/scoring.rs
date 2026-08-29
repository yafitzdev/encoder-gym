use std::collections::{BTreeMap, BTreeSet};

use analysis_core::{
    contract::{CellDiagnosticEvidence, DiagnosticComparisonEvidence},
    domain::FindingReviewState,
};
use generation_core::domain::GenerationCell;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    evidence::OptimizationEvidence,
    protocol::{ExcludedReviewDisposition, OptimizationProtocol, RiskPolicy, ScoringPolicy},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IneligibilityReason {
    BelowMinimumSupport,
    NoObservedErrors,
    UnknownDatasetCell,
    LabelNotAllowed,
    LabelExcluded,
    CellNotAllowed,
    CellExcluded,
    DimensionValueNotAllowed,
    DimensionValueExcluded,
    ReviewDispositionExcluded,
    MissingComparisonEvidence,
    NonPositiveScore,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EligibilityDecision {
    pub eligible: bool,
    pub reasons: Vec<IneligibilityReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub policy: ScoringPolicy,
    pub risk_policy: RiskPolicy,
    pub evidence_support: u64,
    pub evidence_errors: u64,
    pub error_rate: f64,
    pub baseline_error_rate: f64,
    pub error_rate_lift: f64,
    pub error_share: f64,
    pub high_confidence_error_severity: f64,
    pub marginal_error_count: u64,
    pub comparison_analyzed_only_errors: Option<u64>,
    pub comparison_persistent_errors: Option<u64>,
    pub adjusted_error_rate: f64,
    pub uncertainty_factor: f64,
    pub support_factor: f64,
    pub raw_policy_score: f64,
    pub comparison_boost: f64,
    pub review_penalty: f64,
    pub final_score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredCell {
    pub cell: GenerationCell,
    pub finding_key: String,
    pub finding_fingerprint: String,
    pub evidence_fingerprint: String,
    pub eligibility: EligibilityDecision,
    pub score: ScoreBreakdown,
}

pub fn score_evidence(
    evidence: &OptimizationEvidence,
    protocol: &OptimizationProtocol,
) -> Result<Vec<ScoredCell>, ScoringError> {
    evidence.validate(protocol, true)?;
    let known_cells = evidence
        .current_coverage
        .iter()
        .map(|coverage| coverage.cell.key())
        .collect::<BTreeSet<_>>();
    let mut scored = evidence
        .diagnostics
        .cells
        .iter()
        .map(|cell| {
            score_one(
                cell,
                evidence.diagnostics.baseline_error_rate,
                evidence.diagnostics.error_count,
                protocol,
                &known_cells,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    scored.sort_by(|left, right| {
        right
            .score
            .final_score
            .total_cmp(&left.score.final_score)
            .then_with(|| left.cell.key().cmp(&right.cell.key()))
    });
    Ok(scored)
}

fn score_one(
    evidence: &CellDiagnosticEvidence,
    baseline_error_rate: f64,
    total_errors: u64,
    protocol: &OptimizationProtocol,
    known_cells: &BTreeSet<String>,
) -> Result<ScoredCell, ScoringError> {
    let cell = diagnostic_cell(evidence)?;
    let mut reasons = eligibility_reasons(evidence, &cell, protocol, known_cells);
    let adjusted_error_rate =
        adjusted_error_rate(evidence, baseline_error_rate, protocol.risk_policy);
    let uncertainty_factor = round(if evidence.error_rate > 0.0 {
        clamp(adjusted_error_rate / evidence.error_rate, 0.0, 1.0)
    } else {
        0.0
    });
    let support_scale = protocol.minimum_evidence_support.saturating_mul(2).max(1) as f64;
    let support_factor = round(evidence.support as f64 / (evidence.support as f64 + support_scale));
    let raw_policy_score = raw_score(
        evidence,
        adjusted_error_rate,
        baseline_error_rate,
        total_errors,
        support_factor,
        protocol.scoring_policy,
    )?;
    let comparison_boost = comparison_boost(evidence.comparison.as_ref(), protocol);
    let review_penalty = if review_is_excluded(evidence, protocol) {
        0.0
    } else {
        1.0
    };
    let final_score = round(raw_policy_score * comparison_boost * review_penalty);
    if final_score <= 0.0 && reasons.is_empty() {
        reasons.push(IneligibilityReason::NonPositiveScore);
    }
    reasons.sort_unstable();
    reasons.dedup();
    Ok(ScoredCell {
        cell,
        finding_key: evidence.finding_key.clone(),
        finding_fingerprint: evidence.finding_fingerprint.clone(),
        evidence_fingerprint: evidence.evidence_fingerprint.clone(),
        eligibility: EligibilityDecision {
            eligible: reasons.is_empty(),
            reasons,
        },
        score: ScoreBreakdown {
            policy: protocol.scoring_policy,
            risk_policy: protocol.risk_policy,
            evidence_support: evidence.support,
            evidence_errors: evidence.error_count,
            error_rate: evidence.error_rate,
            baseline_error_rate,
            error_rate_lift: evidence.error_rate_lift,
            error_share: evidence.error_share,
            high_confidence_error_severity: evidence.high_confidence_error_severity,
            marginal_error_count: evidence.marginal_error_count,
            comparison_analyzed_only_errors: evidence
                .comparison
                .as_ref()
                .map(DiagnosticComparisonEvidence::analyzed_only_error_count),
            comparison_persistent_errors: evidence
                .comparison
                .as_ref()
                .map(|comparison| comparison.persistent_error_count),
            adjusted_error_rate,
            uncertainty_factor,
            support_factor,
            raw_policy_score,
            comparison_boost,
            review_penalty,
            final_score,
        },
    })
}

fn eligibility_reasons(
    evidence: &CellDiagnosticEvidence,
    cell: &GenerationCell,
    protocol: &OptimizationProtocol,
    known_cells: &BTreeSet<String>,
) -> Vec<IneligibilityReason> {
    let mut reasons = Vec::new();
    if evidence.support < protocol.minimum_evidence_support {
        reasons.push(IneligibilityReason::BelowMinimumSupport);
    }
    if evidence.error_count == 0 {
        reasons.push(IneligibilityReason::NoObservedErrors);
    }
    if !known_cells.contains(&cell.key()) {
        reasons.push(IneligibilityReason::UnknownDatasetCell);
    }
    if !protocol.allowed_labels.is_empty()
        && protocol.allowed_labels.binary_search(&cell.label).is_err()
    {
        reasons.push(IneligibilityReason::LabelNotAllowed);
    }
    if protocol.excluded_labels.binary_search(&cell.label).is_ok() {
        reasons.push(IneligibilityReason::LabelExcluded);
    }
    if !protocol.allowed_cells.is_empty()
        && protocol
            .allowed_cells
            .binary_search_by_key(&cell.key(), GenerationCell::key)
            .is_err()
    {
        reasons.push(IneligibilityReason::CellNotAllowed);
    }
    if protocol
        .excluded_cells
        .binary_search_by_key(&cell.key(), GenerationCell::key)
        .is_ok()
    {
        reasons.push(IneligibilityReason::CellExcluded);
    }
    let allowed = protocol.allowed_dimension_values.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut values, item| {
            values
                .entry(item.dimension.as_str())
                .or_default()
                .insert(item.value.as_str());
            values
        },
    );
    if allowed.iter().any(|(name, values)| {
        cell.dimensions
            .get(*name)
            .is_none_or(|value| !values.contains(value.as_str()))
    }) {
        reasons.push(IneligibilityReason::DimensionValueNotAllowed);
    }
    if protocol.excluded_dimension_values.iter().any(|item| {
        cell.dimensions
            .get(&item.dimension)
            .is_some_and(|value| value == &item.value)
    }) {
        reasons.push(IneligibilityReason::DimensionValueExcluded);
    }
    if review_is_excluded(evidence, protocol) {
        reasons.push(IneligibilityReason::ReviewDispositionExcluded);
    }
    if protocol.scoring_policy == ScoringPolicy::ComparisonRegression
        && evidence.comparison.is_none()
    {
        reasons.push(IneligibilityReason::MissingComparisonEvidence);
    }
    reasons
}

fn raw_score(
    evidence: &CellDiagnosticEvidence,
    adjusted_error_rate: f64,
    baseline_error_rate: f64,
    total_errors: u64,
    support_factor: f64,
    policy: ScoringPolicy,
) -> Result<f64, ScoringError> {
    let uncertainty_factor = if evidence.error_rate > 0.0 {
        clamp(adjusted_error_rate / evidence.error_rate, 0.0, 1.0)
    } else {
        0.0
    };
    let score = match policy {
        ScoringPolicy::ErrorCount => evidence.error_count as f64 * uncertainty_factor,
        ScoringPolicy::ErrorRate => adjusted_error_rate,
        ScoringPolicy::ErrorRateLift => (adjusted_error_rate - baseline_error_rate).max(0.0),
        ScoringPolicy::HighConfidenceErrorSeverity => {
            evidence.high_confidence_error_severity * uncertainty_factor
        }
        ScoringPolicy::MarginalErrorCoverage => {
            evidence.marginal_error_count as f64 * uncertainty_factor
        }
        ScoringPolicy::ComparisonRegression => {
            evidence.comparison.as_ref().map_or(0.0, |comparison| {
                let high_confidence = if comparison.analyzed_run_is_right {
                    comparison.high_confidence_regression_count
                } else {
                    0
                };
                (comparison.analyzed_only_error_count() + high_confidence) as f64
                    * uncertainty_factor
            })
        }
        ScoringPolicy::ConservativeComposite => {
            let adjusted_lift = (adjusted_error_rate - baseline_error_rate).max(0.0);
            let marginal_share = if total_errors == 0 {
                0.0
            } else {
                evidence.marginal_error_count as f64 / total_errors as f64
            };
            (0.35 * adjusted_error_rate
                + 0.20 * adjusted_lift
                + 0.20 * evidence.high_confidence_error_severity * uncertainty_factor
                + 0.15 * marginal_share
                + 0.10 * evidence.error_share)
                * support_factor
        }
    };
    if !score.is_finite() || score < 0.0 {
        return Err(ScoringError::NonFiniteScore(evidence.finding_key.clone()));
    }
    Ok(round(score))
}

fn adjusted_error_rate(
    evidence: &CellDiagnosticEvidence,
    baseline_error_rate: f64,
    policy: RiskPolicy,
) -> f64 {
    let adjusted = match policy {
        RiskPolicy::None => evidence.error_rate,
        RiskPolicy::WilsonLowerBound { z_score } => {
            wilson_lower_bound(evidence.error_count, evidence.support, z_score)
        }
        RiskPolicy::BaselineShrinkage { prior_strength } => {
            (evidence.error_count as f64 + baseline_error_rate * prior_strength)
                / (evidence.support as f64 + prior_strength)
        }
    };
    round(clamp(adjusted, 0.0, 1.0))
}

fn wilson_lower_bound(errors: u64, support: u64, z_score: f64) -> f64 {
    if support == 0 {
        return 0.0;
    }
    let n = support as f64;
    let probability = errors as f64 / n;
    let z_squared = z_score * z_score;
    let center = probability + z_squared / (2.0 * n);
    let radius = z_score * ((probability * (1.0 - probability) + z_squared / (4.0 * n)) / n).sqrt();
    (center - radius) / (1.0 + z_squared / n)
}

fn comparison_boost(
    comparison: Option<&DiagnosticComparisonEvidence>,
    protocol: &OptimizationProtocol,
) -> f64 {
    if !protocol.use_comparison_regression_evidence {
        return 1.0;
    }
    comparison.map_or(1.0, |comparison| {
        let regression_rate = if comparison.paired_support == 0 {
            0.0
        } else {
            comparison.analyzed_only_error_count() as f64 / comparison.paired_support as f64
        };
        round(1.0 + 0.5 * clamp(regression_rate, 0.0, 1.0))
    })
}

fn review_is_excluded(evidence: &CellDiagnosticEvidence, protocol: &OptimizationProtocol) -> bool {
    let Some(review) = &evidence.latest_review else {
        return false;
    };
    protocol
        .excluded_review_dispositions
        .iter()
        .any(|disposition| {
            matches!(
                (disposition, review.state),
                (
                    ExcludedReviewDisposition::AcceptedLimitation,
                    FindingReviewState::AcceptedLimitation
                ) | (
                    ExcludedReviewDisposition::ResolvedByLaterEvidence,
                    FindingReviewState::ResolvedByLaterEvidence
                )
            )
        })
}

fn diagnostic_cell(evidence: &CellDiagnosticEvidence) -> Result<GenerationCell, ScoringError> {
    let label = evidence
        .identity
        .get("label")
        .filter(|label| !label.trim().is_empty())
        .cloned()
        .ok_or_else(|| ScoringError::MissingLabel(evidence.finding_key.clone()))?;
    let dimensions = evidence
        .identity
        .iter()
        .filter(|(name, _)| name.as_str() != "label")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Ok(GenerationCell { label, dimensions })
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
}

fn round(value: f64) -> f64 {
    (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
}

#[derive(Debug, Error)]
pub enum ScoringError {
    #[error(transparent)]
    Evidence(#[from] crate::evidence::OptimizationEvidenceError),
    #[error("cell finding is missing its label attribute: {0}")]
    MissingLabel(String),
    #[error("score is non-finite or negative for cell finding {0}")]
    NonFiniteScore(String),
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use analysis_core::contract::{
        CellDiagnosticEvidence, DiagnosticComparisonEvidence, DiagnosticReviewDisposition,
    };
    use chrono::Utc;
    use generation_core::domain::GenerationCell;
    use uuid::Uuid;

    use crate::protocol::{
        ExcludedReviewDisposition, OptimizationProtocol, RiskPolicy, ScoringPolicy,
    };

    use super::{IneligibilityReason, score_one};

    #[test]
    fn supports_every_policy_with_auditable_finite_scores() {
        let cell = evidence();
        let known = BTreeSet::from([generation_cell().key()]);
        for policy in [
            ScoringPolicy::ErrorCount,
            ScoringPolicy::ErrorRate,
            ScoringPolicy::ErrorRateLift,
            ScoringPolicy::HighConfidenceErrorSeverity,
            ScoringPolicy::MarginalErrorCoverage,
            ScoringPolicy::ComparisonRegression,
            ScoringPolicy::ConservativeComposite,
        ] {
            let protocol = OptimizationProtocol {
                scoring_policy: policy,
                risk_policy: RiskPolicy::WilsonLowerBound { z_score: 1.96 },
                use_comparison_regression_evidence: true,
                ..OptimizationProtocol::legacy(100, 1)
            };
            let scored = score_one(&cell, 0.2, 10, &protocol, &known).expect("score");
            assert!(scored.eligibility.eligible);
            assert!(scored.score.final_score.is_finite());
            assert!(scored.score.final_score > 0.0);
            assert!(scored.score.adjusted_error_rate < cell.error_rate);
        }
    }

    #[test]
    fn excludes_reviewed_unknown_and_filtered_cells_explicitly() {
        let mut cell = evidence();
        cell.latest_review = Some(DiagnosticReviewDisposition {
            review_id: Uuid::new_v4(),
            state: analysis_core::domain::FindingReviewState::AcceptedLimitation,
            resolution_evaluation_run_id: None,
            resolution_comparison_id: None,
            created_at: Utc::now(),
        });
        let protocol = OptimizationProtocol {
            allowed_labels: vec!["fraud".into()],
            excluded_review_dispositions: vec![ExcludedReviewDisposition::AcceptedLimitation],
            ..OptimizationProtocol::legacy(100, 1)
        };
        let scored = score_one(&cell, 0.2, 10, &protocol, &BTreeSet::new()).expect("score");
        assert!(!scored.eligibility.eligible);
        assert!(
            scored
                .eligibility
                .reasons
                .contains(&IneligibilityReason::UnknownDatasetCell)
        );
        assert!(
            scored
                .eligibility
                .reasons
                .contains(&IneligibilityReason::LabelNotAllowed)
        );
        assert!(
            scored
                .eligibility
                .reasons
                .contains(&IneligibilityReason::ReviewDispositionExcluded)
        );
        assert_eq!(scored.score.review_penalty, 0.0);
    }

    fn evidence() -> CellDiagnosticEvidence {
        CellDiagnosticEvidence {
            finding_key: "cell".into(),
            finding_fingerprint: "sha256:finding".into(),
            identity: BTreeMap::from([
                ("label".into(), "billing".into()),
                ("style".into(), "clean".into()),
            ]),
            support: 20,
            error_count: 8,
            error_rate: 0.4,
            error_rate_lift: 0.2,
            error_share: 0.8,
            high_confidence_error_severity: 0.6,
            marginal_error_count: 5,
            cumulative_error_coverage: 0.5,
            comparison: Some(DiagnosticComparisonEvidence {
                comparison_id: Uuid::new_v4(),
                analyzed_run_is_right: true,
                paired_support: 20,
                fixed_by_right_count: 2,
                regressed_by_right_count: 3,
                persistent_error_count: 4,
                high_confidence_regression_count: 2,
                analyzed_accuracy_delta: Some(-0.05),
            }),
            latest_review: None,
            evidence_fingerprint: "sha256:evidence".into(),
        }
    }

    fn generation_cell() -> GenerationCell {
        GenerationCell {
            label: "billing".into(),
            dimensions: BTreeMap::from([("style".into(), "clean".into())]),
        }
    }
}
