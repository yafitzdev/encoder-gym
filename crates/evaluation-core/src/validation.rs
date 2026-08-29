use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use training_core::domain::Prediction;

const PROBABILITY_SUM_TOLERANCE: f64 = 1e-6;
const CONFIDENCE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PredictionValidationError {
    #[error("predictor returned unknown selected label {0}")]
    UnknownSelectedLabel(String),
    #[error("predictor probabilities must contain every expected label exactly once")]
    ProbabilityLabels,
    #[error("predictor returned a non-finite or out-of-range probability")]
    ProbabilityRange,
    #[error("predictor probabilities sum to {actual}, expected one within tolerance {tolerance}")]
    ProbabilitySum { actual: f64, tolerance: f64 },
    #[error("predictor confidence does not equal the selected label probability")]
    ConfidenceMismatch,
    #[error("predictor selected label is not a maximum-probability label")]
    SelectedLabelNotMaximum,
}

pub fn validate_prediction(
    labels: &[String],
    prediction: &Prediction,
) -> Result<(), PredictionValidationError> {
    let expected = labels.iter().collect::<BTreeSet<_>>();
    if !expected.contains(&prediction.label) {
        return Err(PredictionValidationError::UnknownSelectedLabel(
            prediction.label.clone(),
        ));
    }
    if !prediction.confidence.is_finite() || !(0.0..=1.0).contains(&prediction.confidence) {
        return Err(PredictionValidationError::ProbabilityRange);
    }
    let mut probabilities = BTreeMap::new();
    for item in &prediction.probabilities {
        if !item.probability.is_finite() || !(0.0..=1.0).contains(&item.probability) {
            return Err(PredictionValidationError::ProbabilityRange);
        }
        if !expected.contains(&item.label)
            || probabilities
                .insert(item.label.as_str(), item.probability)
                .is_some()
        {
            return Err(PredictionValidationError::ProbabilityLabels);
        }
    }
    if probabilities.len() != labels.len() {
        return Err(PredictionValidationError::ProbabilityLabels);
    }
    let sum = probabilities.values().sum::<f64>();
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
        return Err(PredictionValidationError::ProbabilitySum {
            actual: sum,
            tolerance: PROBABILITY_SUM_TOLERANCE,
        });
    }
    let selected = probabilities[&prediction.label.as_str()];
    if (prediction.confidence - selected).abs() > CONFIDENCE_TOLERANCE {
        return Err(PredictionValidationError::ConfidenceMismatch);
    }
    let maximum = probabilities
        .values()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if selected + CONFIDENCE_TOLERANCE < maximum {
        return Err(PredictionValidationError::SelectedLabelNotMaximum);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use training_core::domain::{LabelProbability, Prediction};

    use super::{PredictionValidationError, validate_prediction};

    fn valid() -> Prediction {
        Prediction {
            label: "a".into(),
            confidence: 0.7,
            probabilities: vec![
                LabelProbability {
                    label: "a".into(),
                    probability: 0.7,
                },
                LabelProbability {
                    label: "b".into(),
                    probability: 0.3,
                },
            ],
        }
    }

    #[test]
    fn requires_a_complete_normalized_probability_vector() {
        let labels = vec!["a".into(), "b".into()];
        validate_prediction(&labels, &valid()).expect("valid prediction");

        let mut missing = valid();
        missing.probabilities.pop();
        assert_eq!(
            validate_prediction(&labels, &missing),
            Err(PredictionValidationError::ProbabilityLabels)
        );

        let mut duplicate = valid();
        duplicate.probabilities[1].label = "a".into();
        assert_eq!(
            validate_prediction(&labels, &duplicate),
            Err(PredictionValidationError::ProbabilityLabels)
        );

        let mut wrong_sum = valid();
        wrong_sum.probabilities[1].probability = 0.2;
        assert!(matches!(
            validate_prediction(&labels, &wrong_sum),
            Err(PredictionValidationError::ProbabilitySum { .. })
        ));

        let mut wrong_confidence = valid();
        wrong_confidence.confidence = 0.6;
        assert_eq!(
            validate_prediction(&labels, &wrong_confidence),
            Err(PredictionValidationError::ConfidenceMismatch)
        );
    }
}
