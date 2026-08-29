use std::collections::BTreeMap;

use evaluation_core::domain::EvaluationComparisonReport;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    diagnostics::{DiagnosticError, validate_prediction_evidence},
    domain::{
        ComparisonCellEvidence, ComparisonDiagnosis, ComparisonEvidenceCategory,
        ComparisonEvidenceReference, DiagnosticSliceDelta, FindingIdentity, FindingKind,
    },
    ports::PairedEvaluationPrediction,
    protocol::AnalysisProtocol,
};

pub struct ComparisonAccumulator<'a> {
    report: &'a EvaluationComparisonReport,
    maximum_examples: usize,
    seed: u64,
    high_confidence_threshold: f64,
    minimum_support: u64,
    paired: u64,
    fixed: u64,
    regressed: u64,
    persistent: u64,
    high_confidence_regressions: u64,
    evidence: BTreeMap<ComparisonEvidenceCategory, Vec<ComparisonEvidenceReference>>,
    cell_evidence: BTreeMap<String, ComparisonCellEvidence>,
}

impl<'a> ComparisonAccumulator<'a> {
    pub fn new(report: &'a EvaluationComparisonReport, protocol: &AnalysisProtocol) -> Self {
        Self {
            report,
            maximum_examples: protocol.maximum_representative_examples,
            seed: protocol.deterministic_seed,
            high_confidence_threshold: protocol
                .confidence_thresholds
                .last()
                .copied()
                .unwrap_or(0.8),
            minimum_support: protocol.minimum_support,
            paired: 0,
            fixed: 0,
            regressed: 0,
            persistent: 0,
            high_confidence_regressions: 0,
            evidence: BTreeMap::new(),
            cell_evidence: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, pair: &PairedEvaluationPrediction) -> Result<(), ComparisonError> {
        validate_prediction_evidence(&pair.left)?;
        validate_prediction_evidence(&pair.right)?;
        if pair.left.evaluation_run_id != self.report.left_run_id
            || pair.right.evaluation_run_id != self.report.right_run_id
            || pair.left.snapshot_member_id != pair.right.snapshot_member_id
            || pair.left.source_row_id != pair.right.source_row_id
            || pair.left.expected_label != pair.right.expected_label
            || pair.left.dimensions != pair.right.dimensions
        {
            return Err(ComparisonError::MismatchedPair {
                left_prediction_id: pair.left.id,
                right_prediction_id: pair.right.id,
            });
        }
        self.paired += 1;
        let left_correct = pair.left.expected_label == pair.left.predicted_label;
        let right_correct = pair.right.expected_label == pair.right.predicted_label;
        let category = match (left_correct, right_correct) {
            (false, true) => {
                self.fixed += 1;
                Some(ComparisonEvidenceCategory::FixedByRight)
            }
            (true, false) => {
                self.regressed += 1;
                Some(ComparisonEvidenceCategory::RegressedByRight)
            }
            (false, false) => {
                self.persistent += 1;
                Some(ComparisonEvidenceCategory::PersistentError)
            }
            (true, true) => None,
        };
        let high_confidence_regression = left_correct
            && !right_correct
            && pair.right.confidence >= self.high_confidence_threshold;
        {
            let cell = self
                .cell_evidence
                .entry(comparison_cell_key(pair))
                .or_default();
            cell.paired_support += 1;
            match category {
                Some(ComparisonEvidenceCategory::FixedByRight) => {
                    cell.fixed_by_right_count += 1;
                }
                Some(ComparisonEvidenceCategory::RegressedByRight) => {
                    cell.regressed_by_right_count += 1;
                }
                Some(ComparisonEvidenceCategory::PersistentError) => {
                    cell.persistent_error_count += 1;
                }
                Some(ComparisonEvidenceCategory::HighConfidenceRegression) | None => {}
            }
            if high_confidence_regression {
                cell.high_confidence_regression_count += 1;
            }
        }
        if let Some(category) = category {
            self.retain(category, pair);
        }
        if high_confidence_regression {
            self.high_confidence_regressions += 1;
            self.retain(ComparisonEvidenceCategory::HighConfidenceRegression, pair);
        }
        Ok(())
    }

    pub fn finish(self) -> Result<ComparisonDiagnosis, ComparisonError> {
        if self.paired
            != self
                .report
                .both_correct
                .checked_add(self.report.both_wrong)
                .and_then(|value| value.checked_add(self.report.left_only_correct))
                .and_then(|value| value.checked_add(self.report.right_only_correct))
                .ok_or(ComparisonError::InconsistentReport)?
            || self.fixed != self.report.right_only_correct
            || self.regressed != self.report.left_only_correct
            || self.persistent != self.report.both_wrong
            || self.report.fixed_snapshot_member_ids.len() as u64 != self.fixed
            || self.report.regressed_snapshot_member_ids.len() as u64 != self.regressed
        {
            return Err(ComparisonError::InconsistentReport);
        }
        let mut slice_deltas = self
            .report
            .slice_deltas
            .iter()
            .filter(|(_, delta)| {
                delta.support >= self.minimum_support && delta.accuracy_delta.abs() > f64::EPSILON
            })
            .map(|(key, delta)| DiagnosticSliceDelta {
                identity_key: key.clone(),
                support: delta.support,
                accuracy_delta: delta.accuracy_delta,
            })
            .collect::<Vec<_>>();
        slice_deltas.sort_by(|left, right| {
            right
                .accuracy_delta
                .abs()
                .total_cmp(&left.accuracy_delta.abs())
                .then_with(|| left.identity_key.cmp(&right.identity_key))
        });
        Ok(ComparisonDiagnosis {
            comparison_id: self.report.id,
            left_evaluation_run_id: self.report.left_run_id,
            right_evaluation_run_id: self.report.right_run_id,
            paired_prediction_count: self.paired,
            fixed_by_right_count: self.fixed,
            regressed_by_right_count: self.regressed,
            persistent_error_count: self.persistent,
            high_confidence_regression_count: self.high_confidence_regressions,
            slice_deltas,
            cell_evidence: self.cell_evidence,
            evidence: self.evidence,
        })
    }

    fn retain(&mut self, category: ComparisonEvidenceCategory, pair: &PairedEvaluationPrediction) {
        let values = self.evidence.entry(category).or_default();
        values.push(ComparisonEvidenceReference {
            category,
            snapshot_member_id: pair.left.snapshot_member_id,
            source_row_id: pair.left.source_row_id,
            left_prediction_id: pair.left.id,
            right_prediction_id: pair.right.id,
            left_confidence: pair.left.confidence,
            right_confidence: pair.right.confidence,
        });
        let seed = self.seed;
        values.sort_by_key(|value| stable_hash(value.snapshot_member_id, seed));
        values.truncate(self.maximum_examples);
    }
}

fn comparison_cell_key(pair: &PairedEvaluationPrediction) -> String {
    let mut attributes = pair.left.dimensions.clone();
    attributes.insert("label".into(), pair.left.expected_label.clone());
    FindingIdentity {
        kind: FindingKind::Cell,
        attributes,
    }
    .key()
}

fn stable_hash(id: Uuid, seed: u64) -> u64 {
    id.as_bytes()
        .iter()
        .fold(seed ^ 0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[derive(Debug, Error)]
pub enum ComparisonError {
    #[error(transparent)]
    Prediction(#[from] DiagnosticError),
    #[error(
        "paired predictions {left_prediction_id} and {right_prediction_id} do not describe the same immutable example"
    )]
    MismatchedPair {
        left_prediction_id: Uuid,
        right_prediction_id: Uuid,
    },
    #[error("paired prediction evidence does not reproduce the persisted comparison report")]
    InconsistentReport,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use evaluation_core::{
        comparison::paired_comparison,
        domain::{
            EvaluationPrediction, EvaluationProtocol, EvaluationRun, EvaluationRunState,
            EvaluationSourceIdentity,
        },
        metrics::calculate_metrics_with_protocol,
    };
    use training_core::domain::LabelProbability;
    use uuid::Uuid;

    use super::ComparisonAccumulator;
    use crate::{ports::PairedEvaluationPrediction, protocol::AnalysisProtocol};

    #[test]
    fn classifies_fixed_regressed_persistent_and_high_confidence_pairs() {
        let labels = vec!["a".into(), "b".into()];
        let members = [
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        ];
        let protocol = EvaluationProtocol {
            bootstrap_samples: 10,
            ..EvaluationProtocol::default()
        };
        let mut left_predictions = vec![
            prediction(Uuid::nil(), members[0], "a", "a", 0.8),
            prediction(Uuid::nil(), members[1], "a", "b", 0.7),
            prediction(Uuid::nil(), members[2], "b", "b", 0.7),
            prediction(Uuid::nil(), members[3], "b", "a", 0.6),
        ];
        let mut right_predictions = vec![
            prediction(Uuid::nil(), members[0], "a", "a", 0.8),
            prediction(Uuid::nil(), members[1], "a", "a", 0.9),
            prediction(Uuid::nil(), members[2], "b", "a", 0.9),
            prediction(Uuid::nil(), members[3], "b", "a", 0.65),
        ];
        for (left, right) in left_predictions.iter().zip(&mut right_predictions) {
            right.source_row_id = left.source_row_id;
        }
        let mut left = completed_run(&labels, protocol.clone(), &left_predictions);
        let mut right = completed_run(&labels, protocol, &right_predictions);
        for prediction in &mut left_predictions {
            prediction.evaluation_run_id = left.id;
        }
        for prediction in &mut right_predictions {
            prediction.evaluation_run_id = right.id;
        }
        left.metrics = Some(calculate_metrics_with_protocol(
            &labels,
            &left_predictions,
            &left.protocol,
        ));
        right.metrics = Some(calculate_metrics_with_protocol(
            &labels,
            &right_predictions,
            &right.protocol,
        ));
        let report = paired_comparison(&left, &right, &left_predictions, &right_predictions)
            .expect("comparison");
        let analysis_protocol = AnalysisProtocol {
            maximum_representative_examples: 2,
            ..AnalysisProtocol::default()
        };
        let mut accumulator = ComparisonAccumulator::new(&report, &analysis_protocol);
        for (left, right) in left_predictions.iter().zip(&right_predictions) {
            accumulator
                .add(&PairedEvaluationPrediction {
                    left: left.clone(),
                    right: right.clone(),
                })
                .expect("pair");
        }
        let diagnosis = accumulator.finish().expect("diagnosis");
        assert_eq!(diagnosis.fixed_by_right_count, 1);
        assert_eq!(diagnosis.regressed_by_right_count, 1);
        assert_eq!(diagnosis.persistent_error_count, 1);
        assert_eq!(diagnosis.high_confidence_regression_count, 1);
        assert!(diagnosis.evidence.values().all(|values| values.len() <= 2));
    }

    fn completed_run(
        labels: &[String],
        protocol: EvaluationProtocol,
        predictions: &[EvaluationPrediction],
    ) -> EvaluationRun {
        let source = EvaluationSourceIdentity {
            checkpoint_checksum: "sha256:checkpoint".into(),
            checkpoint_model_format: "fixture".into(),
            base_model_fingerprint: None,
            tokenizer_fingerprint: None,
            snapshot_fingerprint: "sha256:snapshot".into(),
            cohort_fingerprint: "sha256:cohort".into(),
            labels: labels.to_vec(),
        };
        let mut run = EvaluationRun::queued_with_protocol(
            Uuid::new_v4(),
            Uuid::from_u128(1),
            protocol,
            source,
            predictions.len() as u64,
            "sha256:input",
        )
        .expect("run");
        run.state = EvaluationRunState::Completed;
        run.processed_examples = predictions.len() as u64;
        run.example_count = predictions.len() as u64;
        run
    }

    fn prediction(
        run_id: Uuid,
        member_id: Uuid,
        expected: &str,
        predicted: &str,
        confidence: f64,
    ) -> EvaluationPrediction {
        let a = if predicted == "a" {
            confidence
        } else {
            1.0 - confidence
        };
        EvaluationPrediction {
            id: Uuid::new_v4(),
            evaluation_run_id: run_id,
            snapshot_member_id: member_id,
            source_row_id: Uuid::new_v4(),
            text: "example".into(),
            expected_label: expected.into(),
            predicted_label: predicted.into(),
            confidence,
            probabilities: vec![
                LabelProbability {
                    label: "a".into(),
                    probability: a,
                },
                LabelProbability {
                    label: "b".into(),
                    probability: 1.0 - a,
                },
            ],
            dimensions: BTreeMap::from([("difficulty".into(), "hard".into())]),
            created_at: Utc::now(),
        }
    }
}
