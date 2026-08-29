use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use training_core::{
    domain::{BatchMetrics, LabelProbability, Prediction, TrainingExample, TrainingRequest},
    ports::{Predictor, PredictorLoader, TrainingBackend, TrainingBackendError, TrainingSession},
};

const MODEL_FORMAT: &str = "hashing-linear-v1";

#[derive(Debug, Clone, Default)]
pub struct HashingLinearBackend;

impl TrainingBackend for HashingLinearBackend {
    fn name(&self) -> &str {
        "hashing-linear"
    }

    fn model_format(&self) -> &str {
        MODEL_FORMAT
    }

    fn start(
        &self,
        request: TrainingRequest,
    ) -> Result<Box<dyn TrainingSession>, TrainingBackendError> {
        request
            .validate()
            .map_err(|error| TrainingBackendError::Configuration(error.to_string()))?;
        let training_indices =
            (0..request.examples.training_member_ids().len()).collect::<Vec<_>>();
        let training_examples = request
            .examples
            .training_batch(&training_indices)
            .map_err(TrainingBackendError::Configuration)?;
        let validation_examples = request
            .examples
            .validation_batch(0, request.examples.validation_len())
            .map_err(TrainingBackendError::Configuration)?;
        let label_count = request.labels.len();
        let model = HashingLinearModel {
            format_version: 1,
            feature_dimension: request.configuration.feature_dimension,
            labels: request.labels,
            weights: vec![vec![0.0; request.configuration.feature_dimension]; label_count],
            biases: vec![0.0; label_count],
        };
        Ok(Box::new(LinearTrainingSession {
            model,
            training_examples,
            validation_examples,
            learning_rate: f64::from(request.configuration.learning_rate),
            l2: f64::from(request.configuration.l2),
            seed: request.configuration.seed,
            completed_epochs: 0,
        }))
    }
}

struct LinearTrainingSession {
    model: HashingLinearModel,
    training_examples: Vec<TrainingExample>,
    validation_examples: Vec<TrainingExample>,
    learning_rate: f64,
    l2: f64,
    seed: u64,
    completed_epochs: u32,
}

impl TrainingSession for LinearTrainingSession {
    fn train_batch(&mut self) -> Result<BatchMetrics, TrainingBackendError> {
        self.completed_epochs += 1;
        let mut order = (0..self.training_examples.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| {
            stable_score(
                self.seed ^ u64::from(self.completed_epochs),
                self.training_examples[*index].snapshot_member_id.as_bytes(),
            )
        });
        let mut loss = 0.0;
        for index in order {
            let example = &self.training_examples[index];
            let expected = self
                .model
                .labels
                .iter()
                .position(|label| label == &example.label)
                .ok_or_else(|| {
                    TrainingBackendError::Training(format!(
                        "unknown training label: {}",
                        example.label
                    ))
                })?;
            let features = encode(&example.text, self.model.feature_dimension);
            let probabilities = self.model.probabilities(&features);
            loss -= probabilities[expected].max(1e-15).ln();
            for (class, probability) in probabilities.into_iter().enumerate() {
                let target = if class == expected { 1.0 } else { 0.0 };
                let error = probability - target;
                for (feature, value) in &features {
                    let weight = &mut self.model.weights[class][*feature];
                    *weight -= self.learning_rate * (error * value + self.l2 * *weight);
                }
                self.model.biases[class] -= self.learning_rate * error;
            }
        }
        let training_loss = loss / self.training_examples.len() as f64;
        let validation_loss = (!self.validation_examples.is_empty())
            .then(|| self.model.loss(&self.validation_examples))
            .transpose()?;
        Ok(BatchMetrics {
            epoch: self.completed_epochs,
            batch: 1,
            batches_in_epoch: 1,
            processed_examples: self.training_examples.len() as u64
                * u64::from(self.completed_epochs),
            training_loss,
            validation_loss,
            learning_rate: self.learning_rate,
            elapsed_milliseconds: 0,
            epoch_complete: true,
        })
    }

    fn serialize_checkpoint(&self) -> Result<Vec<u8>, TrainingBackendError> {
        serde_json::to_vec_pretty(&self.model)
            .map_err(|error| TrainingBackendError::Training(error.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HashingLinearModel {
    format_version: u32,
    feature_dimension: usize,
    labels: Vec<String>,
    weights: Vec<Vec<f64>>,
    biases: Vec<f64>,
}

impl HashingLinearModel {
    fn validate(&self) -> Result<(), TrainingBackendError> {
        if self.format_version != 1
            || self.feature_dimension < 16
            || self.labels.len() < 2
            || self.weights.len() != self.labels.len()
            || self.biases.len() != self.labels.len()
            || self
                .weights
                .iter()
                .any(|weights| weights.len() != self.feature_dimension)
        {
            return Err(TrainingBackendError::InvalidArtifact(
                "model dimensions or format version are invalid".into(),
            ));
        }
        Ok(())
    }

    fn probabilities(&self, features: &[(usize, f64)]) -> Vec<f64> {
        let logits = self
            .weights
            .iter()
            .zip(&self.biases)
            .map(|(weights, bias)| {
                features.iter().fold(*bias, |value, (index, feature)| {
                    value + weights[*index] * feature
                })
            })
            .collect::<Vec<_>>();
        softmax(&logits)
    }

    fn loss(&self, examples: &[TrainingExample]) -> Result<f64, TrainingBackendError> {
        let mut loss = 0.0;
        for example in examples {
            let expected = self
                .labels
                .iter()
                .position(|label| label == &example.label)
                .ok_or_else(|| {
                    TrainingBackendError::Training(format!(
                        "unknown validation label: {}",
                        example.label
                    ))
                })?;
            let probabilities = self.probabilities(&encode(&example.text, self.feature_dimension));
            loss -= probabilities[expected].max(1e-15).ln();
        }
        Ok(loss / examples.len() as f64)
    }
}

struct HashingLinearPredictor {
    model: HashingLinearModel,
}

impl Predictor for HashingLinearPredictor {
    fn labels(&self) -> &[String] {
        &self.model.labels
    }

    fn predict(&self, text: &str) -> Result<Prediction, TrainingBackendError> {
        let probabilities = self
            .model
            .probabilities(&encode(text, self.model.feature_dimension));
        let (best, confidence) = probabilities
            .iter()
            .copied()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .ok_or_else(|| TrainingBackendError::InvalidArtifact("model has no labels".into()))?;
        Ok(Prediction {
            label: self.model.labels[best].clone(),
            confidence,
            probabilities: self
                .model
                .labels
                .iter()
                .cloned()
                .zip(probabilities)
                .map(|(label, probability)| LabelProbability { label, probability })
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct HashingLinearPredictorLoader;

impl PredictorLoader for HashingLinearPredictorLoader {
    fn model_format(&self) -> &str {
        MODEL_FORMAT
    }

    fn load(&self, artifact: &[u8]) -> Result<Box<dyn Predictor>, TrainingBackendError> {
        let model: HashingLinearModel = serde_json::from_slice(artifact)
            .map_err(|error| TrainingBackendError::InvalidArtifact(error.to_string()))?;
        model.validate()?;
        Ok(Box::new(HashingLinearPredictor { model }))
    }
}

fn encode(text: &str, dimension: usize) -> Vec<(usize, f64)> {
    let mut counts = BTreeMap::<usize, f64>::new();
    for token in text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        let lowered = token.to_lowercase();
        let index = stable_score(0, lowered.as_bytes()) as usize % dimension;
        *counts.entry(index).or_default() += 1.0;
    }
    let norm = counts
        .values()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        counts
            .into_iter()
            .map(|(index, value)| (index, value / norm))
            .collect()
    } else {
        Vec::new()
    }
}

fn softmax(logits: &[f64]) -> Vec<f64> {
    let maximum = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exponentials = logits
        .iter()
        .map(|value| (value - maximum).exp())
        .collect::<Vec<_>>();
    let sum = exponentials.iter().sum::<f64>();
    exponentials.into_iter().map(|value| value / sum).collect()
}

fn stable_score(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use training_core::{
        domain::{TrainingConfiguration, TrainingExample, TrainingRequest},
        ports::{PredictorLoader, TrainingBackend},
    };
    use uuid::Uuid;

    use super::{HashingLinearBackend, HashingLinearPredictorLoader};

    #[test]
    fn training_reduces_loss_and_checkpoint_predicts() {
        let backend = HashingLinearBackend;
        let configuration = TrainingConfiguration {
            feature_dimension: 128,
            epochs: 30,
            learning_rate: 0.2,
            l2: 0.0,
            checkpoint_every: 10,
            seed: 7,
        };
        let examples = (0..20)
            .map(|index| TrainingExample {
                snapshot_member_id: Uuid::from_u128(index + 1),
                text: if index % 2 == 0 {
                    "invoice charged payment billing"
                } else {
                    "login password account access"
                }
                .into(),
                label: if index % 2 == 0 { "billing" } else { "account" }.into(),
            })
            .collect::<Vec<_>>();
        let mut session = backend
            .start(TrainingRequest::in_memory(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec!["billing".into(), "account".into()],
                examples.clone(),
                examples,
                configuration,
            ))
            .expect("session");
        let first = session.train_batch().expect("first epoch");
        let mut last = first;
        for _ in 1..30 {
            last = session.train_batch().expect("epoch");
        }
        assert!(last.training_loss < first.training_loss);
        let artifact = session.serialize_checkpoint().expect("checkpoint");
        let predictor = HashingLinearPredictorLoader
            .load(&artifact)
            .expect("predictor");
        assert_eq!(
            predictor
                .predict("unexpected invoice payment")
                .expect("prediction")
                .label,
            "billing"
        );
        assert_eq!(predictor.labels(), &["billing", "account"]);
    }
}
