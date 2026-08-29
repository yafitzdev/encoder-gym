use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    ClassificationMetrics, EvaluationComparison, EvaluationMetrics, EvaluationPrediction,
    EvaluationProtocol, EvaluationRun, LabelMetrics, SliceIdentity, SliceKind, SliceMetrics,
};

const LOG_LOSS_EPSILON: f64 = 1e-15;

pub fn calculate_metrics(
    labels: &[String],
    predictions: &[EvaluationPrediction],
) -> EvaluationMetrics {
    calculate_metrics_with_protocol(labels, predictions, &EvaluationProtocol::default())
}

pub fn calculate_metrics_with_protocol(
    labels: &[String],
    predictions: &[EvaluationPrediction],
    protocol: &EvaluationProtocol,
) -> EvaluationMetrics {
    let mut all = predictions.iter().collect::<Vec<_>>();
    all.sort_by_key(|prediction| prediction.snapshot_member_id);
    let mut accumulator = EvaluationMetricsAccumulator::new(labels, protocol);
    for prediction in all {
        accumulator.add(prediction);
    }
    accumulator.finish()
}

/// Streaming metric state. Its memory use is proportional to labels and
/// observed slice identities, never to the number of predictions.
pub struct EvaluationMetricsAccumulator {
    labels: Vec<String>,
    protocol: EvaluationProtocol,
    overall: ClassificationAccumulator,
    by_dimension: BTreeMap<String, ClassificationAccumulator>,
    slices: BTreeMap<SliceIdentity, ClassificationAccumulator>,
}

impl EvaluationMetricsAccumulator {
    pub fn new(labels: &[String], protocol: &EvaluationProtocol) -> Self {
        Self {
            labels: labels.to_vec(),
            protocol: protocol.clone(),
            overall: ClassificationAccumulator::new(labels, protocol),
            by_dimension: BTreeMap::new(),
            slices: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, prediction: &EvaluationPrediction) {
        self.overall.add(prediction);
        self.add_slice(
            SliceIdentity {
                kind: SliceKind::ExpectedLabel,
                attributes: BTreeMap::from([("label".into(), prediction.expected_label.clone())]),
            },
            prediction,
        );
        for (name, value) in &prediction.dimensions {
            self.by_dimension
                .entry(format!("{name}={value}"))
                .or_insert_with(|| ClassificationAccumulator::new(&self.labels, &self.protocol))
                .add(prediction);
            self.add_slice(
                SliceIdentity {
                    kind: SliceKind::DimensionValue,
                    attributes: BTreeMap::from([
                        ("dimension".into(), name.clone()),
                        ("value".into(), value.clone()),
                    ]),
                },
                prediction,
            );
        }
        let mut cell = prediction.dimensions.clone();
        cell.insert("label".into(), prediction.expected_label.clone());
        self.add_slice(
            SliceIdentity {
                kind: SliceKind::Cell,
                attributes: cell,
            },
            prediction,
        );
        for intersection in self.protocol.dimension_intersections.clone() {
            let attributes = intersection
                .iter()
                .map(|name| {
                    prediction
                        .dimensions
                        .get(name)
                        .map(|value| (name.clone(), value.clone()))
                })
                .collect::<Option<BTreeMap<_, _>>>();
            if let Some(attributes) = attributes {
                self.add_slice(
                    SliceIdentity {
                        kind: SliceKind::DimensionIntersection,
                        attributes,
                    },
                    prediction,
                );
            }
        }
    }

    pub fn finish(self) -> EvaluationMetrics {
        let minimum_support = self.protocol.minimum_slice_support;
        EvaluationMetrics {
            overall: self.overall.finish(),
            by_dimension: self
                .by_dimension
                .into_iter()
                .map(|(key, value)| (key, value.finish()))
                .collect(),
            slices: self
                .slices
                .into_iter()
                .filter(|(_, value)| value.total >= minimum_support)
                .map(|(identity, value)| {
                    let support = value.total;
                    (
                        identity.key(),
                        SliceMetrics {
                            identity,
                            support,
                            metrics: value.finish(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn add_slice(&mut self, identity: SliceIdentity, prediction: &EvaluationPrediction) {
        self.slices
            .entry(identity)
            .or_insert_with(|| ClassificationAccumulator::new(&self.labels, &self.protocol))
            .add(prediction);
    }
}

/// Legacy aggregate comparison retained for source compatibility. New CLI
/// workflows use immutable paired comparison reports.
pub fn compare_runs(
    left: &EvaluationRun,
    right: &EvaluationRun,
) -> Result<EvaluationComparison, ComparisonError> {
    let left_metrics = left
        .metrics
        .as_ref()
        .ok_or(ComparisonError::IncompleteRun(left.id))?;
    let right_metrics = right
        .metrics
        .as_ref()
        .ok_or(ComparisonError::IncompleteRun(right.id))?;
    let left_labels = left_metrics
        .overall
        .per_label
        .keys()
        .collect::<BTreeSet<_>>();
    let right_labels = right_metrics
        .overall
        .per_label
        .keys()
        .collect::<BTreeSet<_>>();
    if left_labels != right_labels {
        return Err(ComparisonError::IncompatibleLabels);
    }
    Ok(EvaluationComparison {
        left_run_id: left.id,
        right_run_id: right.id,
        accuracy_delta: right_metrics.overall.accuracy - left_metrics.overall.accuracy,
        macro_f1_delta: right_metrics.overall.macro_f1 - left_metrics.overall.macro_f1,
        per_label_f1_delta: left_metrics
            .overall
            .per_label
            .iter()
            .map(|(label, metrics)| {
                (
                    label.clone(),
                    right_metrics.overall.per_label[label].f1 - metrics.f1,
                )
            })
            .collect(),
    })
}

struct ClassificationAccumulator {
    labels: Vec<String>,
    protocol: EvaluationProtocol,
    confusion_matrix: BTreeMap<String, BTreeMap<String, u64>>,
    total: u64,
    correct: u64,
    top_k_correct: BTreeMap<usize, u64>,
    log_loss_sum: f64,
    brier_sum: f64,
    confidence_sum: f64,
    correct_confidence_sum: f64,
    incorrect_confidence_sum: f64,
    correct_confidence_count: u64,
    incorrect_confidence_count: u64,
    calibration: Vec<CalibrationBin>,
}

impl ClassificationAccumulator {
    fn new(labels: &[String], protocol: &EvaluationProtocol) -> Self {
        Self {
            labels: labels.to_vec(),
            protocol: protocol.clone(),
            confusion_matrix: labels
                .iter()
                .map(|expected| {
                    (
                        expected.clone(),
                        labels
                            .iter()
                            .map(|predicted| (predicted.clone(), 0))
                            .collect(),
                    )
                })
                .collect(),
            total: 0,
            correct: 0,
            top_k_correct: protocol.top_k.iter().map(|value| (*value, 0)).collect(),
            log_loss_sum: 0.0,
            brier_sum: 0.0,
            confidence_sum: 0.0,
            correct_confidence_sum: 0.0,
            incorrect_confidence_sum: 0.0,
            correct_confidence_count: 0,
            incorrect_confidence_count: 0,
            calibration: vec![CalibrationBin::default(); protocol.calibration_bins],
        }
    }

    fn add(&mut self, prediction: &EvaluationPrediction) {
        self.total += 1;
        *self
            .confusion_matrix
            .entry(prediction.expected_label.clone())
            .or_default()
            .entry(prediction.predicted_label.clone())
            .or_default() += 1;
        let is_correct = prediction.expected_label == prediction.predicted_label;
        if is_correct {
            self.correct += 1;
            self.correct_confidence_sum += prediction.confidence;
            self.correct_confidence_count += 1;
        } else {
            self.incorrect_confidence_sum += prediction.confidence;
            self.incorrect_confidence_count += 1;
        }
        self.confidence_sum += prediction.confidence;
        let probabilities = prediction
            .probabilities
            .iter()
            .map(|item| (item.label.as_str(), item.probability))
            .collect::<BTreeMap<_, _>>();
        let expected_probability = probabilities
            .get(prediction.expected_label.as_str())
            .copied()
            .unwrap_or(0.0)
            .clamp(LOG_LOSS_EPSILON, 1.0);
        self.log_loss_sum -= expected_probability.ln();
        self.brier_sum += self
            .labels
            .iter()
            .map(|label| {
                let probability = probabilities.get(label.as_str()).copied().unwrap_or(0.0);
                let target = f64::from(label == &prediction.expected_label);
                (probability - target).powi(2)
            })
            .sum::<f64>();
        let mut ranking = prediction.probabilities.iter().collect::<Vec<_>>();
        ranking.sort_by(|left, right| {
            right
                .probability
                .total_cmp(&left.probability)
                .then_with(|| left.label.cmp(&right.label))
        });
        for (top_k, count) in &mut self.top_k_correct {
            *count += u64::from(
                ranking
                    .iter()
                    .take(*top_k)
                    .any(|item| item.label == prediction.expected_label),
            );
        }
        let bin_index = ((prediction.confidence * self.protocol.calibration_bins as f64) as usize)
            .min(self.protocol.calibration_bins - 1);
        self.calibration[bin_index].count += 1;
        self.calibration[bin_index].confidence_sum += prediction.confidence;
        self.calibration[bin_index].correct += u64::from(is_correct);
    }

    fn finish(self) -> ClassificationMetrics {
        let per_label = label_metrics(&self.labels, &self.confusion_matrix);
        let label_count = per_label.len() as f64;
        let total = self.total;
        let macro_average = |sum: f64| {
            if label_count == 0.0 {
                0.0
            } else {
                stable(sum / label_count)
            }
        };
        let weighted = |metric: fn(&LabelMetrics) -> f64| {
            if total == 0 {
                0.0
            } else {
                stable(
                    per_label
                        .values()
                        .map(|value| metric(value) * value.support as f64)
                        .sum::<f64>()
                        / total as f64,
                )
            }
        };
        let expected_calibration_error = if total == 0 {
            0.0
        } else {
            self.calibration
                .iter()
                .filter(|bin| bin.count > 0)
                .map(|bin| {
                    let accuracy = bin.correct as f64 / bin.count as f64;
                    let confidence = bin.confidence_sum / bin.count as f64;
                    (accuracy - confidence).abs() * bin.count as f64 / total as f64
                })
                .sum()
        };
        ClassificationMetrics {
            total,
            correct: self.correct,
            accuracy: divide(self.correct, total),
            macro_precision: macro_average(per_label.values().map(|value| value.precision).sum()),
            macro_recall: macro_average(per_label.values().map(|value| value.recall).sum()),
            macro_f1: macro_average(per_label.values().map(|value| value.f1).sum()),
            weighted_precision: weighted(|value| value.precision),
            weighted_recall: weighted(|value| value.recall),
            weighted_f1: weighted(|value| value.f1),
            top_k_accuracy: self
                .top_k_correct
                .into_iter()
                .map(|(k, count)| (k, divide(count, total)))
                .collect(),
            log_loss: mean(self.log_loss_sum, total),
            brier_score: mean(self.brier_sum, total),
            expected_calibration_error: stable(expected_calibration_error),
            mean_confidence: mean(self.confidence_sum, total),
            mean_correct_confidence: mean(
                self.correct_confidence_sum,
                self.correct_confidence_count,
            ),
            mean_incorrect_confidence: mean(
                self.incorrect_confidence_sum,
                self.incorrect_confidence_count,
            ),
            per_label,
            confusion_matrix: self.confusion_matrix,
        }
    }
}

#[allow(dead_code)]
fn classification_metrics(
    labels: &[String],
    predictions: &[&EvaluationPrediction],
    protocol: &EvaluationProtocol,
) -> ClassificationMetrics {
    let mut predictions = predictions.to_vec();
    predictions.sort_by_key(|prediction| prediction.snapshot_member_id);
    let mut confusion_matrix = labels
        .iter()
        .map(|expected| {
            (
                expected.clone(),
                labels
                    .iter()
                    .map(|predicted| (predicted.clone(), 0))
                    .collect(),
            )
        })
        .collect::<BTreeMap<String, BTreeMap<String, u64>>>();
    let mut correct = 0_u64;
    let mut top_k_correct = protocol
        .top_k
        .iter()
        .map(|value| (*value, 0_u64))
        .collect::<BTreeMap<_, _>>();
    let mut log_loss_sum = 0.0;
    let mut brier_sum = 0.0;
    let mut confidence_sum = 0.0;
    let mut correct_confidence_sum = 0.0;
    let mut incorrect_confidence_sum = 0.0;
    let mut correct_confidence_count = 0_u64;
    let mut incorrect_confidence_count = 0_u64;
    let mut calibration = vec![CalibrationBin::default(); protocol.calibration_bins];

    for prediction in &predictions {
        *confusion_matrix
            .entry(prediction.expected_label.clone())
            .or_default()
            .entry(prediction.predicted_label.clone())
            .or_default() += 1;
        let is_correct = prediction.expected_label == prediction.predicted_label;
        if is_correct {
            correct += 1;
            correct_confidence_sum += prediction.confidence;
            correct_confidence_count += 1;
        } else {
            incorrect_confidence_sum += prediction.confidence;
            incorrect_confidence_count += 1;
        }
        confidence_sum += prediction.confidence;
        let probabilities = prediction
            .probabilities
            .iter()
            .map(|item| (item.label.as_str(), item.probability))
            .collect::<BTreeMap<_, _>>();
        let expected_probability = probabilities
            .get(prediction.expected_label.as_str())
            .copied()
            .unwrap_or(0.0)
            .clamp(LOG_LOSS_EPSILON, 1.0);
        log_loss_sum -= expected_probability.ln();
        brier_sum += labels
            .iter()
            .map(|label| {
                let probability = probabilities.get(label.as_str()).copied().unwrap_or(0.0);
                let target = f64::from(label == &prediction.expected_label);
                (probability - target).powi(2)
            })
            .sum::<f64>();
        let mut ranking = prediction.probabilities.iter().collect::<Vec<_>>();
        ranking.sort_by(|left, right| {
            right
                .probability
                .total_cmp(&left.probability)
                .then_with(|| left.label.cmp(&right.label))
        });
        for (top_k, count) in &mut top_k_correct {
            *count += u64::from(
                ranking
                    .iter()
                    .take(*top_k)
                    .any(|item| item.label == prediction.expected_label),
            );
        }
        let bin_index = ((prediction.confidence * protocol.calibration_bins as f64) as usize)
            .min(protocol.calibration_bins - 1);
        calibration[bin_index].count += 1;
        calibration[bin_index].confidence_sum += prediction.confidence;
        calibration[bin_index].correct += u64::from(is_correct);
    }

    let per_label = label_metrics(labels, &confusion_matrix);
    let label_count = per_label.len() as f64;
    let total = predictions.len() as u64;
    let macro_average = |sum: f64| {
        if label_count == 0.0 {
            0.0
        } else {
            sum / label_count
        }
    };
    let weighted = |metric: fn(&LabelMetrics) -> f64| {
        if total == 0 {
            0.0
        } else {
            per_label
                .values()
                .map(|value| metric(value) * value.support as f64)
                .sum::<f64>()
                / total as f64
        }
    };
    let expected_calibration_error = if total == 0 {
        0.0
    } else {
        calibration
            .iter()
            .filter(|bin| bin.count > 0)
            .map(|bin| {
                let accuracy = bin.correct as f64 / bin.count as f64;
                let confidence = bin.confidence_sum / bin.count as f64;
                (accuracy - confidence).abs() * bin.count as f64 / total as f64
            })
            .sum()
    };
    ClassificationMetrics {
        total,
        correct,
        accuracy: divide(correct, total),
        macro_precision: macro_average(per_label.values().map(|value| value.precision).sum()),
        macro_recall: macro_average(per_label.values().map(|value| value.recall).sum()),
        macro_f1: macro_average(per_label.values().map(|value| value.f1).sum()),
        weighted_precision: weighted(|value| value.precision),
        weighted_recall: weighted(|value| value.recall),
        weighted_f1: weighted(|value| value.f1),
        top_k_accuracy: top_k_correct
            .into_iter()
            .map(|(top_k, count)| (top_k, divide(count, total)))
            .collect(),
        log_loss: mean(log_loss_sum, total),
        brier_score: mean(brier_sum, total),
        expected_calibration_error,
        mean_confidence: mean(confidence_sum, total),
        mean_correct_confidence: mean(correct_confidence_sum, correct_confidence_count),
        mean_incorrect_confidence: mean(incorrect_confidence_sum, incorrect_confidence_count),
        per_label,
        confusion_matrix,
    }
}

fn label_metrics(
    labels: &[String],
    confusion_matrix: &BTreeMap<String, BTreeMap<String, u64>>,
) -> BTreeMap<String, LabelMetrics> {
    labels
        .iter()
        .map(|label| {
            let support = confusion_matrix
                .get(label)
                .map(|row| row.values().sum())
                .unwrap_or(0);
            let predicted = confusion_matrix
                .values()
                .map(|row| row.get(label).copied().unwrap_or(0))
                .sum();
            let true_positive = confusion_matrix
                .get(label)
                .and_then(|row| row.get(label))
                .copied()
                .unwrap_or(0);
            let precision = divide(true_positive, predicted);
            let recall = divide(true_positive, support);
            let f1 = if precision + recall > 0.0 {
                2.0 * precision * recall / (precision + recall)
            } else {
                0.0
            };
            (
                label.clone(),
                LabelMetrics {
                    support,
                    predicted,
                    true_positive,
                    precision,
                    recall,
                    f1: stable(f1),
                },
            )
        })
        .collect()
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

// Fifteen published decimal places make metrics stable across persistence
// round trips without affecting practical classification interpretation.
fn stable(value: f64) -> f64 {
    (value * 1_000_000_000_000_000.0).round() / 1_000_000_000_000_000.0
}

#[derive(Debug, Clone, Default)]
struct CalibrationBin {
    count: u64,
    correct: u64,
    confidence_sum: f64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ComparisonError {
    #[error("evaluation run is incomplete: {0}")]
    IncompleteRun(Uuid),
    #[error("evaluation runs use incompatible label sets")]
    IncompatibleLabels,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use training_core::domain::LabelProbability;
    use uuid::Uuid;

    use super::calculate_metrics_with_protocol;
    use crate::domain::{EvaluationPrediction, EvaluationProtocol, SliceIdentity, SliceKind};

    #[test]
    fn calculates_classification_confidence_and_slice_metrics() {
        let labels = vec!["a".into(), "b".into(), "unused".into()];
        let predictions = vec![
            prediction("a", "a", "easy", [0.8, 0.1, 0.1]),
            prediction("a", "b", "hard", [0.3, 0.6, 0.1]),
            prediction("b", "b", "easy", [0.1, 0.8, 0.1]),
            prediction("b", "a", "hard", [0.6, 0.3, 0.1]),
        ];
        let protocol = EvaluationProtocol {
            top_k: vec![1, 2],
            minimum_slice_support: 2,
            dimension_intersections: vec![vec!["difficulty".into(), "style".into()]],
            ..EvaluationProtocol::default()
        };
        let metrics = calculate_metrics_with_protocol(&labels, &predictions, &protocol);
        assert_eq!(metrics.overall.total, 4);
        assert_eq!(metrics.overall.accuracy, 0.5);
        assert_eq!(metrics.overall.top_k_accuracy[&1], 0.5);
        assert_eq!(metrics.overall.top_k_accuracy[&2], 1.0);
        assert_eq!(metrics.overall.per_label["a"].precision, 0.5);
        assert_eq!(metrics.overall.per_label["unused"].f1, 0.0);
        assert_eq!(metrics.overall.confusion_matrix["a"]["b"], 1);
        assert_eq!(metrics.by_dimension["difficulty=easy"].accuracy, 1.0);
        assert_eq!(metrics.by_dimension["difficulty=hard"].accuracy, 0.0);
        assert!(metrics.overall.log_loss.is_finite());
        assert!(metrics.overall.brier_score > 0.0);
        assert!(metrics.overall.expected_calibration_error > 0.0);
        assert_eq!(metrics.overall.mean_correct_confidence, 0.8);
        assert_eq!(metrics.overall.mean_incorrect_confidence, 0.6);
        assert!(metrics.slices.values().all(|slice| slice.support >= 2));
        assert!(metrics.slices.values().any(|slice| {
            slice.identity.kind == SliceKind::DimensionIntersection
                && slice.identity.attributes["difficulty"] == "easy"
        }));
    }

    #[test]
    fn slice_keys_do_not_collide_on_delimiters() {
        let left = SliceIdentity {
            kind: SliceKind::Cell,
            attributes: BTreeMap::from([("a".into(), "b/c=d".into())]),
        };
        let right = SliceIdentity {
            kind: SliceKind::Cell,
            attributes: BTreeMap::from([("a".into(), "b".into()), ("c".into(), "d".into())]),
        };
        assert_ne!(left.key(), right.key());
    }

    fn prediction(
        expected: &str,
        predicted: &str,
        difficulty: &str,
        probabilities: [f64; 3],
    ) -> EvaluationPrediction {
        let labels = ["a", "b", "unused"];
        let predicted_index = labels
            .iter()
            .position(|label| *label == predicted)
            .expect("predicted label");
        EvaluationPrediction {
            id: Uuid::new_v4(),
            evaluation_run_id: Uuid::new_v4(),
            snapshot_member_id: Uuid::new_v4(),
            source_row_id: Uuid::new_v4(),
            text: "example".into(),
            expected_label: expected.into(),
            predicted_label: predicted.into(),
            confidence: probabilities[predicted_index],
            probabilities: labels
                .into_iter()
                .zip(probabilities)
                .map(|(label, probability)| LabelProbability {
                    label: label.into(),
                    probability,
                })
                .collect(),
            dimensions: BTreeMap::from([
                ("difficulty".into(), difficulty.into()),
                ("style".into(), "clean".into()),
            ]),
            created_at: Utc::now(),
        }
    }
}
