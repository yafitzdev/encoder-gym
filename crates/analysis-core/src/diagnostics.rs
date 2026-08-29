use std::collections::{BTreeMap, BTreeSet};

use evaluation_core::domain::EvaluationPrediction;
use thiserror::Error;

use crate::{
    domain::{
        AnalysisFinding, EvidenceCategory, FindingEvidenceReference, FindingIdentity, FindingKind,
        FindingRankingPolicy,
    },
    protocol::{AnalysisProtocol, AnalysisProtocolError},
};

const CONFIDENCE_BUCKETS: usize = 101;
const PROBABILITY_EPSILON: f64 = 1e-15;

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticAggregation {
    pub prediction_count: u64,
    pub error_count: u64,
    pub findings: Vec<AnalysisFinding>,
    pub evidence: BTreeMap<String, Vec<FindingEvidenceReference>>,
}

pub fn aggregate_predictions<'a>(
    predictions: impl IntoIterator<Item = &'a EvaluationPrediction>,
    protocol: &AnalysisProtocol,
) -> Result<DiagnosticAggregation, DiagnosticError> {
    let mut accumulator = DiagnosticAccumulator::new(protocol.clone())?;
    for prediction in predictions {
        accumulator.add(prediction)?;
    }
    accumulator.finish()
}

pub fn validate_prediction_evidence(
    prediction: &EvaluationPrediction,
) -> Result<(), DiagnosticError> {
    PredictionMeasures::new(prediction).map(|_| ())
}

pub struct DiagnosticAccumulator {
    protocol: AnalysisProtocol,
    groups: BTreeMap<FindingIdentity, GroupStatistics>,
    prediction_count: u64,
    error_count: u64,
}

/// Bounded second-pass state for exact overlap and cumulative error coverage.
///
/// Ranking must be finalized before this pass. Each error contributes to the
/// earliest retained finding it matches, so overlapping group counts are never
/// presented as additive.
pub struct OverlapAccumulator {
    protocol: AnalysisProtocol,
    rank_by_key: BTreeMap<String, usize>,
    marginal_errors: Vec<u64>,
    total_errors: u64,
}

impl OverlapAccumulator {
    pub fn new(
        findings: &[AnalysisFinding],
        total_errors: u64,
        protocol: AnalysisProtocol,
    ) -> Result<Self, DiagnosticError> {
        protocol.validate()?;
        let mut rank_by_key = BTreeMap::new();
        for (index, finding) in findings.iter().enumerate() {
            if finding.rank != index as u64 + 1
                || rank_by_key.insert(finding.key.clone(), index).is_some()
            {
                return Err(DiagnosticError::FindingRanking);
            }
        }
        Ok(Self {
            protocol,
            rank_by_key,
            marginal_errors: vec![0; findings.len()],
            total_errors,
        })
    }

    pub fn add(&mut self, prediction: &EvaluationPrediction) -> Result<(), DiagnosticError> {
        let measures = PredictionMeasures::new(prediction)?;
        if !measures.is_error {
            return Ok(());
        }
        let earliest = identities(prediction, &measures, &self.protocol)
            .into_iter()
            .filter_map(|identity| self.rank_by_key.get(&identity.key()).copied())
            .min();
        if let Some(index) = earliest {
            self.marginal_errors[index] += 1;
        }
        Ok(())
    }

    pub fn finish(self, findings: &mut [AnalysisFinding]) -> Result<(), DiagnosticError> {
        if findings.len() != self.marginal_errors.len() {
            return Err(DiagnosticError::FindingRanking);
        }
        let mut cumulative = 0_u64;
        for (finding, marginal) in findings.iter_mut().zip(self.marginal_errors) {
            cumulative = cumulative
                .checked_add(marginal)
                .ok_or(DiagnosticError::FindingRanking)?;
            if cumulative > self.total_errors {
                return Err(DiagnosticError::FindingRanking);
            }
            finding.marginal_error_count = marginal;
            finding.cumulative_error_count = cumulative;
            finding.cumulative_error_coverage = divide(cumulative, self.total_errors);
        }
        Ok(())
    }
}

impl DiagnosticAccumulator {
    pub fn new(protocol: AnalysisProtocol) -> Result<Self, DiagnosticError> {
        protocol.validate()?;
        Ok(Self {
            protocol,
            groups: BTreeMap::new(),
            prediction_count: 0,
            error_count: 0,
        })
    }

    pub fn add(&mut self, prediction: &EvaluationPrediction) -> Result<(), DiagnosticError> {
        let measures = PredictionMeasures::new(prediction)?;
        self.prediction_count += 1;
        self.error_count += u64::from(measures.is_error);
        for identity in identities(prediction, &measures, &self.protocol) {
            self.groups
                .entry(identity)
                .or_insert_with(|| {
                    GroupStatistics::new(self.protocol.maximum_representative_examples)
                })
                .record(prediction, &measures, &self.protocol);
        }
        Ok(())
    }

    pub fn finish(self) -> Result<DiagnosticAggregation, DiagnosticError> {
        if self.prediction_count == 0 {
            return Err(DiagnosticError::EmptyPredictions);
        }
        let baseline_error_rate = divide(self.error_count, self.prediction_count);
        let mut completed = self
            .groups
            .into_iter()
            .filter(|(_, group)| group.support >= self.protocol.minimum_support && group.errors > 0)
            .map(|(identity, group)| {
                group.finish(
                    identity,
                    baseline_error_rate,
                    self.error_count,
                    &self.protocol,
                )
            })
            .collect::<Vec<_>>();
        completed.sort_by(|left, right| {
            compare_findings(&left.0, &right.0, self.protocol.ranking_policy)
        });
        let mut findings = Vec::with_capacity(completed.len());
        let mut evidence = BTreeMap::new();
        for (index, (mut finding, references)) in completed.into_iter().enumerate() {
            finding.rank = index as u64 + 1;
            evidence.insert(finding.key.clone(), references);
            findings.push(finding);
        }
        Ok(DiagnosticAggregation {
            prediction_count: self.prediction_count,
            error_count: self.error_count,
            findings,
            evidence,
        })
    }
}

fn identities(
    prediction: &EvaluationPrediction,
    measures: &PredictionMeasures,
    protocol: &AnalysisProtocol,
) -> Vec<FindingIdentity> {
    let mut identities = Vec::new();
    let enabled = |kind| protocol.finding_kinds.binary_search(&kind).is_ok();
    if enabled(FindingKind::ExpectedLabel) {
        identities.push(identity(
            FindingKind::ExpectedLabel,
            [("label", prediction.expected_label.as_str())],
        ));
    }
    if enabled(FindingKind::PredictedLabel) {
        identities.push(identity(
            FindingKind::PredictedLabel,
            [("predicted_label", prediction.predicted_label.as_str())],
        ));
    }
    if enabled(FindingKind::ConfusionPair) {
        identities.push(identity(
            FindingKind::ConfusionPair,
            [
                ("expected_label", prediction.expected_label.as_str()),
                ("predicted_label", prediction.predicted_label.as_str()),
            ],
        ));
    }
    if enabled(FindingKind::DimensionValue) {
        for (name, value) in &prediction.dimensions {
            identities.push(identity(
                FindingKind::DimensionValue,
                [("dimension", name.as_str()), ("value", value.as_str())],
            ));
        }
    }
    if enabled(FindingKind::Cell) {
        let mut attributes = prediction.dimensions.clone();
        attributes.insert("label".into(), prediction.expected_label.clone());
        identities.push(FindingIdentity {
            kind: FindingKind::Cell,
            attributes,
        });
    }
    if enabled(FindingKind::DimensionIntersection) {
        for intersection in &protocol.dimension_intersections {
            if let Some(attributes) = intersection
                .iter()
                .map(|name| {
                    prediction
                        .dimensions
                        .get(name)
                        .map(|value| (name.clone(), value.clone()))
                })
                .collect::<Option<BTreeMap<_, _>>>()
            {
                identities.push(FindingIdentity {
                    kind: FindingKind::DimensionIntersection,
                    attributes,
                });
            }
        }
    }
    let band = confidence_band(prediction.confidence, &protocol.confidence_thresholds);
    if enabled(FindingKind::ConfidenceBand) {
        identities.push(identity(
            FindingKind::ConfidenceBand,
            [("band", band.as_str())],
        ));
    }
    if enabled(FindingKind::CorrectnessConfidence) {
        identities.push(identity(
            FindingKind::CorrectnessConfidence,
            [
                (
                    "correctness",
                    if measures.is_error {
                        "incorrect"
                    } else {
                        "correct"
                    },
                ),
                ("band", band.as_str()),
            ],
        ));
    }
    identities
}

fn identity<const N: usize>(kind: FindingKind, values: [(&str, &str); N]) -> FindingIdentity {
    FindingIdentity {
        kind,
        attributes: values
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
    }
}

fn confidence_band(confidence: f64, thresholds: &[f64]) -> String {
    let mut lower = 0.0;
    for threshold in thresholds {
        if confidence < *threshold {
            return format!("[{lower:.6},{threshold:.6})");
        }
        lower = *threshold;
    }
    format!("[{lower:.6},1.000000]")
}

struct PredictionMeasures {
    is_error: bool,
    expected_probability: f64,
    margin: f64,
    entropy: f64,
}

impl PredictionMeasures {
    fn new(prediction: &EvaluationPrediction) -> Result<Self, DiagnosticError> {
        if prediction.probabilities.is_empty() {
            return Err(DiagnosticError::ProbabilityVector(prediction.id));
        }
        let mut labels = BTreeSet::new();
        let mut expected_probability = None;
        let mut selected_probability = None;
        let mut sum = 0.0;
        let mut values = Vec::with_capacity(prediction.probabilities.len());
        let mut entropy = 0.0;
        for item in &prediction.probabilities {
            if !labels.insert(&item.label)
                || !item.probability.is_finite()
                || !(0.0..=1.0).contains(&item.probability)
            {
                return Err(DiagnosticError::ProbabilityVector(prediction.id));
            }
            if item.label == prediction.expected_label {
                expected_probability = Some(item.probability);
            }
            if item.label == prediction.predicted_label {
                selected_probability = Some(item.probability);
            }
            sum += item.probability;
            values.push(item.probability);
            if item.probability > 0.0 {
                entropy -= item.probability * item.probability.clamp(PROBABILITY_EPSILON, 1.0).ln();
            }
        }
        if (sum - 1.0).abs() > 1e-6 {
            return Err(DiagnosticError::ProbabilityVector(prediction.id));
        }
        values.sort_by(|left, right| right.total_cmp(left));
        let selected_probability =
            selected_probability.ok_or(DiagnosticError::ProbabilityVector(prediction.id))?;
        if !prediction.confidence.is_finite()
            || (prediction.confidence - selected_probability).abs() > 1e-9
            || selected_probability + 1e-9 < values[0]
        {
            return Err(DiagnosticError::ProbabilityVector(prediction.id));
        }
        let margin = values[0] - values.get(1).copied().unwrap_or(0.0);
        Ok(Self {
            is_error: prediction.expected_label != prediction.predicted_label,
            expected_probability: expected_probability
                .ok_or(DiagnosticError::ProbabilityVector(prediction.id))?,
            margin,
            entropy,
        })
    }
}

#[derive(Clone)]
struct Candidate {
    prediction_id: uuid::Uuid,
    snapshot_member_id: uuid::Uuid,
    source_row_id: uuid::Uuid,
    confidence: f64,
    margin: f64,
}

impl Candidate {
    fn from_prediction(prediction: &EvaluationPrediction, margin: f64) -> Self {
        Self {
            prediction_id: prediction.id,
            snapshot_member_id: prediction.snapshot_member_id,
            source_row_id: prediction.source_row_id,
            confidence: prediction.confidence,
            margin,
        }
    }

    fn reference(&self, category: EvidenceCategory) -> FindingEvidenceReference {
        FindingEvidenceReference {
            category,
            prediction_id: self.prediction_id,
            snapshot_member_id: self.snapshot_member_id,
            source_row_id: self.source_row_id,
            confidence: self.confidence,
            prediction_margin: self.margin,
        }
    }
}

struct GroupStatistics {
    support: u64,
    errors: u64,
    error_confidence_sum: f64,
    expected_probability_sum: f64,
    margin_sum: f64,
    entropy_sum: f64,
    severity_sum: f64,
    confidence_histogram: [u64; CONFIDENCE_BUCKETS],
    highest_confidence: Option<Candidate>,
    lowest_margin: Option<Candidate>,
    error_pool: Vec<Candidate>,
    contrasts: Vec<Candidate>,
    pool_limit: usize,
}

impl GroupStatistics {
    fn new(maximum_examples: usize) -> Self {
        Self {
            support: 0,
            errors: 0,
            error_confidence_sum: 0.0,
            expected_probability_sum: 0.0,
            margin_sum: 0.0,
            entropy_sum: 0.0,
            severity_sum: 0.0,
            confidence_histogram: [0; CONFIDENCE_BUCKETS],
            highest_confidence: None,
            lowest_margin: None,
            error_pool: Vec::new(),
            contrasts: Vec::new(),
            pool_limit: maximum_examples.saturating_mul(4),
        }
    }

    fn record(
        &mut self,
        prediction: &EvaluationPrediction,
        measures: &PredictionMeasures,
        protocol: &AnalysisProtocol,
    ) {
        self.support += 1;
        if measures.is_error {
            self.errors += 1;
            self.error_confidence_sum += prediction.confidence;
            self.expected_probability_sum += measures.expected_probability;
            self.margin_sum += measures.margin;
            self.entropy_sum += measures.entropy;
            self.severity_sum += prediction.confidence * (1.0 - measures.expected_probability);
            let bucket = ((prediction.confidence * 100.0).round() as usize).min(100);
            self.confidence_histogram[bucket] += 1;
            let candidate = Candidate::from_prediction(prediction, measures.margin);
            replace_if(&mut self.highest_confidence, &candidate, |left, right| {
                left.confidence
                    .total_cmp(&right.confidence)
                    .then_with(|| right.prediction_id.cmp(&left.prediction_id))
            });
            replace_if(&mut self.lowest_margin, &candidate, |left, right| {
                right
                    .margin
                    .total_cmp(&left.margin)
                    .then_with(|| right.prediction_id.cmp(&left.prediction_id))
            });
            retain_bounded(
                &mut self.error_pool,
                candidate,
                self.pool_limit,
                protocol.deterministic_seed,
            );
        } else if protocol.include_correct_contrasts {
            retain_bounded(
                &mut self.contrasts,
                Candidate::from_prediction(prediction, measures.margin),
                protocol.maximum_representative_examples,
                protocol.deterministic_seed,
            );
        }
    }

    fn finish(
        self,
        identity: FindingIdentity,
        baseline: f64,
        total_errors: u64,
        protocol: &AnalysisProtocol,
    ) -> (AnalysisFinding, Vec<FindingEvidenceReference>) {
        let median = histogram_median(&self.confidence_histogram, self.errors);
        let mut references = Vec::new();
        let mut seen = BTreeSet::new();
        let mut push = |candidate: Option<&Candidate>, category| {
            if references.len() < protocol.maximum_representative_examples {
                if let Some(candidate) = candidate.filter(|value| seen.insert(value.prediction_id))
                {
                    references.push(candidate.reference(category));
                }
            }
        };
        push(
            self.highest_confidence.as_ref(),
            EvidenceCategory::HighestConfidenceError,
        );
        push(
            self.lowest_margin.as_ref(),
            EvidenceCategory::LowestMarginError,
        );
        let median_candidate = self.error_pool.iter().min_by(|left, right| {
            (left.confidence - median)
                .abs()
                .total_cmp(&(right.confidence - median).abs())
                .then_with(|| left.prediction_id.cmp(&right.prediction_id))
        });
        push(median_candidate, EvidenceCategory::MedianConfidenceError);
        for candidate in &self.error_pool {
            push(Some(candidate), EvidenceCategory::StableError);
        }
        for candidate in &self.contrasts {
            push(Some(candidate), EvidenceCategory::CorrectContrast);
        }
        let error_rate = divide(self.errors, self.support);
        let key = identity.key();
        (
            AnalysisFinding {
                rank: 0,
                kind: identity.kind,
                key,
                attributes: identity.attributes,
                support: self.support,
                error_count: self.errors,
                error_rate,
                mean_error_confidence: mean(self.error_confidence_sum, self.errors),
                median_error_confidence: median,
                mean_expected_probability: mean(self.expected_probability_sum, self.errors),
                mean_prediction_margin: mean(self.margin_sum, self.errors),
                mean_entropy: mean(self.entropy_sum, self.errors),
                error_rate_lift: error_rate - baseline,
                error_share: divide(self.errors, total_errors),
                high_confidence_error_severity: mean(self.severity_sum, self.errors),
                marginal_error_count: 0,
                cumulative_error_count: 0,
                cumulative_error_coverage: 0.0,
                fingerprint: String::new(),
            },
            references,
        )
    }
}

fn replace_if(
    candidate: &mut Option<Candidate>,
    next: &Candidate,
    compare: impl Fn(&Candidate, &Candidate) -> std::cmp::Ordering,
) {
    if candidate
        .as_ref()
        .is_none_or(|current| compare(next, current).is_gt())
    {
        *candidate = Some(next.clone());
    }
}

fn retain_bounded(values: &mut Vec<Candidate>, next: Candidate, limit: usize, seed: u64) {
    values.push(next);
    values.sort_by_key(|value| stable_hash(value.prediction_id, seed));
    values.truncate(limit);
}

fn stable_hash(id: uuid::Uuid, seed: u64) -> u64 {
    id.as_bytes()
        .iter()
        .fold(seed ^ 0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn histogram_median(histogram: &[u64; CONFIDENCE_BUCKETS], count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    let target = (count - 1) / 2;
    let mut seen = 0;
    for (index, value) in histogram.iter().enumerate() {
        seen += value;
        if seen > target {
            return index as f64 / 100.0;
        }
    }
    1.0
}

fn compare_findings(
    left: &AnalysisFinding,
    right: &AnalysisFinding,
    policy: FindingRankingPolicy,
) -> std::cmp::Ordering {
    let order = match policy {
        FindingRankingPolicy::ErrorRate => right.error_rate.total_cmp(&left.error_rate),
        FindingRankingPolicy::ErrorCount => right.error_count.cmp(&left.error_count),
        FindingRankingPolicy::ErrorRateLift => {
            right.error_rate_lift.total_cmp(&left.error_rate_lift)
        }
        FindingRankingPolicy::ErrorShare => right.error_share.total_cmp(&left.error_share),
        FindingRankingPolicy::HighConfidenceErrorSeverity => right
            .high_confidence_error_severity
            .total_cmp(&left.high_confidence_error_severity),
    };
    order
        .then_with(|| right.error_count.cmp(&left.error_count))
        .then_with(|| left.key.cmp(&right.key))
}

fn divide(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        stable(numerator as f64 / denominator as f64)
    }
}
fn mean(sum: f64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        stable(sum / count as f64)
    }
}
fn stable(value: f64) -> f64 {
    (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum DiagnosticError {
    #[error(transparent)]
    Protocol(#[from] AnalysisProtocolError),
    #[error("analysis requires at least one persisted prediction")]
    EmptyPredictions,
    #[error("prediction {0} contains inconsistent probability evidence")]
    ProbabilityVector(uuid::Uuid),
    #[error("ranked findings are not uniquely and consecutively ordered")]
    FindingRanking,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use evaluation_core::domain::EvaluationPrediction;
    use training_core::domain::LabelProbability;
    use uuid::Uuid;

    use super::{DiagnosticError, OverlapAccumulator, aggregate_predictions};
    use crate::{
        domain::{EvidenceCategory, FindingKind, FindingRankingPolicy},
        protocol::AnalysisProtocol,
    };

    #[test]
    fn aggregates_confidence_cells_intersections_and_bounded_evidence() {
        let predictions = vec![
            prediction("a", "b", 0.95, 0.05, "hard", "clean"),
            prediction("a", "a", 0.8, 0.8, "hard", "clean"),
            prediction("b", "a", 0.7, 0.3, "easy", "messy/value=x"),
            prediction("b", "b", 0.7, 0.7, "easy", "messy/value=x"),
        ];
        let protocol = AnalysisProtocol {
            finding_kinds: vec![
                FindingKind::ExpectedLabel,
                FindingKind::ConfusionPair,
                FindingKind::DimensionValue,
                FindingKind::Cell,
                FindingKind::DimensionIntersection,
                FindingKind::ConfidenceBand,
                FindingKind::CorrectnessConfidence,
            ],
            dimension_intersections: vec![vec!["difficulty".into(), "style".into()]],
            maximum_representative_examples: 3,
            ranking_policy: FindingRankingPolicy::HighConfidenceErrorSeverity,
            ..AnalysisProtocol::default()
        };
        let aggregation = aggregate_predictions(&predictions, &protocol).expect("diagnostics");
        assert_eq!(aggregation.prediction_count, 4);
        assert_eq!(aggregation.error_count, 2);
        assert!(aggregation.findings.iter().all(|finding| {
            finding.error_count <= finding.support
                && finding.error_share > 0.0
                && finding.mean_entropy.is_finite()
        }));
        assert!(aggregation.findings.iter().any(|finding| {
            finding.kind == FindingKind::DimensionIntersection
                && finding.attributes["difficulty"] == "hard"
        }));
        assert!(aggregation.findings.iter().any(|finding| {
            finding.kind == FindingKind::DimensionValue
                && finding.attributes["value"] == "messy/value=x"
                && finding.key.starts_with('{')
        }));
        assert!(aggregation.evidence.values().all(|items| items.len() <= 3));
        assert!(
            aggregation
                .evidence
                .values()
                .flatten()
                .any(|item| { item.category == EvidenceCategory::CorrectContrast })
        );

        let mut findings = aggregation.findings;
        let mut overlap =
            OverlapAccumulator::new(&findings, aggregation.error_count, protocol.clone())
                .expect("overlap");
        for prediction in &predictions {
            overlap.add(prediction).expect("valid prediction");
        }
        overlap.finish(&mut findings).expect("coverage");
        assert_eq!(findings.last().expect("finding").cumulative_error_count, 2);
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.marginal_error_count)
                .sum::<u64>(),
            2
        );
    }

    #[test]
    fn rejects_inconsistent_probability_evidence() {
        let mut prediction = prediction("a", "a", 0.8, 0.8, "hard", "clean");
        prediction.probabilities[1].probability = 0.8;
        assert!(matches!(
            aggregate_predictions([&prediction], &AnalysisProtocol::default()),
            Err(DiagnosticError::ProbabilityVector(_))
        ));
    }

    fn prediction(
        expected: &str,
        predicted: &str,
        confidence: f64,
        expected_probability: f64,
        difficulty: &str,
        style: &str,
    ) -> EvaluationPrediction {
        let a_probability = if expected == "a" {
            expected_probability
        } else {
            1.0 - expected_probability
        };
        let b_probability = 1.0 - a_probability;
        EvaluationPrediction {
            id: Uuid::new_v4(),
            evaluation_run_id: Uuid::new_v4(),
            snapshot_member_id: Uuid::new_v4(),
            source_row_id: Uuid::new_v4(),
            text: "example".into(),
            expected_label: expected.into(),
            predicted_label: predicted.into(),
            confidence,
            probabilities: vec![
                LabelProbability {
                    label: "a".into(),
                    probability: a_probability,
                },
                LabelProbability {
                    label: "b".into(),
                    probability: b_probability,
                },
            ],
            dimensions: BTreeMap::from([
                ("difficulty".into(), difficulty.into()),
                ("style".into(), style.into()),
            ]),
            created_at: Utc::now(),
        }
    }
}
