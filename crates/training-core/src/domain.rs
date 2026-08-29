use std::{collections::BTreeSet, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TrainingDomainError {
    #[error("feature_dimension must be at least 16")]
    FeatureDimension,
    #[error("epochs must be greater than zero")]
    Epochs,
    #[error("learning_rate must be finite and greater than zero")]
    LearningRate,
    #[error("l2 must be finite and non-negative")]
    L2,
    #[error("checkpoint_every must be greater than zero")]
    CheckpointEvery,
    #[error("training requires at least two unique, non-empty labels")]
    Labels,
    #[error("training requires at least one training example")]
    EmptyTrainingSet,
    #[error("example {member_id} has unknown label {label}")]
    UnknownLabel { member_id: Uuid, label: String },
    #[error("training example source failed: {0}")]
    ExampleSource(String),
    #[error("invalid training-run transition from {from:?} to {to:?}")]
    Transition {
        from: TrainingRunState,
        to: TrainingRunState,
    },
    #[error("encoder name must not be empty")]
    EncoderName,
    #[error("encoder source path must not be empty")]
    EncoderSourcePath,
    #[error("encoder fingerprint must be a SHA-256 fingerprint")]
    EncoderFingerprint,
    #[error("maximum_sequence_length must be at least two")]
    MaximumSequenceLength,
    #[error("batch_size must be greater than zero")]
    BatchSize,
    #[error("weight_decay must be finite and non-negative")]
    WeightDecay,
    #[error("warmup_ratio must be finite, non-negative, and less than one")]
    WarmupRatio,
    #[error("gradient_clip_norm must be finite and greater than zero")]
    GradientClip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncoderTrainingMode {
    FineTune,
    Frozen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformerTrainingConfiguration {
    pub maximum_sequence_length: usize,
    pub batch_size: usize,
    pub weight_decay: f64,
    pub warmup_ratio: f64,
    pub gradient_clip_norm: f64,
    pub mode: EncoderTrainingMode,
}

impl Default for TransformerTrainingConfiguration {
    fn default() -> Self {
        Self {
            maximum_sequence_length: 128,
            batch_size: 8,
            weight_decay: 0.01,
            warmup_ratio: 0.1,
            gradient_clip_norm: 1.0,
            mode: EncoderTrainingMode::FineTune,
        }
    }
}

impl TransformerTrainingConfiguration {
    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        if self.maximum_sequence_length < 2 {
            return Err(TrainingDomainError::MaximumSequenceLength);
        }
        if self.batch_size == 0 {
            return Err(TrainingDomainError::BatchSize);
        }
        if !self.weight_decay.is_finite() || self.weight_decay < 0.0 {
            return Err(TrainingDomainError::WeightDecay);
        }
        if !self.warmup_ratio.is_finite() || !(0.0..1.0).contains(&self.warmup_ratio) {
            return Err(TrainingDomainError::WarmupRatio);
        }
        if !self.gradient_clip_norm.is_finite() || self.gradient_clip_norm <= 0.0 {
            return Err(TrainingDomainError::GradientClip);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderArchitecture {
    Bert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderArtifact {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderMetadata {
    pub vocab_size: usize,
    pub hidden_size: usize,
    pub layer_count: usize,
    pub attention_head_count: usize,
    pub intermediate_size: usize,
    pub maximum_position_embeddings: usize,
    pub type_vocab_size: usize,
    pub padding_token_id: usize,
    pub tensor_dtype: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredEncoder {
    pub id: Uuid,
    pub name: String,
    pub architecture: EncoderArchitecture,
    pub source_path: String,
    pub fingerprint: String,
    pub configuration: EncoderArtifact,
    pub tokenizer: EncoderArtifact,
    pub weights: EncoderArtifact,
    pub metadata: EncoderMetadata,
    pub created_at: DateTime<Utc>,
}

impl RegisteredEncoder {
    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        if self.name.trim().is_empty() {
            return Err(TrainingDomainError::EncoderName);
        }
        if self.source_path.trim().is_empty() {
            return Err(TrainingDomainError::EncoderSourcePath);
        }
        if !is_sha256_fingerprint(&self.fingerprint) {
            return Err(TrainingDomainError::EncoderFingerprint);
        }
        Ok(())
    }
}

fn is_sha256_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingConfiguration {
    pub feature_dimension: usize,
    pub epochs: u32,
    pub learning_rate: f32,
    pub l2: f32,
    pub checkpoint_every: u32,
    pub seed: u64,
}

impl TrainingConfiguration {
    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        if self.feature_dimension < 16 {
            return Err(TrainingDomainError::FeatureDimension);
        }
        if self.epochs == 0 {
            return Err(TrainingDomainError::Epochs);
        }
        if !self.learning_rate.is_finite() || self.learning_rate <= 0.0 {
            return Err(TrainingDomainError::LearningRate);
        }
        if !self.l2.is_finite() || self.l2 < 0.0 {
            return Err(TrainingDomainError::L2);
        }
        if self.checkpoint_every == 0 {
            return Err(TrainingDomainError::CheckpointEvery);
        }
        Ok(())
    }
}

impl Default for TrainingConfiguration {
    fn default() -> Self {
        Self {
            feature_dimension: 1_024,
            epochs: 20,
            learning_rate: 0.1,
            l2: 0.0001,
            checkpoint_every: 5,
            seed: 42,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingExample {
    pub snapshot_member_id: Uuid,
    pub text: String,
    pub label: String,
}

/// Backend-neutral, bounded access to immutable snapshot examples.
pub trait TrainingExampleSource: Send + Sync {
    fn training_member_ids(&self) -> &[Uuid];
    fn validation_len(&self) -> usize;
    fn training_batch(&self, indices: &[usize]) -> Result<Vec<TrainingExample>, String>;
    fn validation_batch(&self, offset: usize, limit: usize)
    -> Result<Vec<TrainingExample>, String>;
}

#[derive(Debug, Clone)]
pub struct InMemoryTrainingExamples {
    training: Vec<TrainingExample>,
    validation: Vec<TrainingExample>,
    training_member_ids: Vec<Uuid>,
}

impl InMemoryTrainingExamples {
    pub fn new(training: Vec<TrainingExample>, validation: Vec<TrainingExample>) -> Self {
        let training_member_ids = training
            .iter()
            .map(|example| example.snapshot_member_id)
            .collect();
        Self {
            training,
            validation,
            training_member_ids,
        }
    }
}

impl TrainingExampleSource for InMemoryTrainingExamples {
    fn training_member_ids(&self) -> &[Uuid] {
        &self.training_member_ids
    }

    fn validation_len(&self) -> usize {
        self.validation.len()
    }

    fn training_batch(&self, indices: &[usize]) -> Result<Vec<TrainingExample>, String> {
        indices
            .iter()
            .map(|index| {
                self.training
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| format!("training example index is out of range: {index}"))
            })
            .collect()
    }

    fn validation_batch(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<TrainingExample>, String> {
        if offset > self.validation.len() {
            return Err(format!(
                "validation example offset is out of range: {offset}"
            ));
        }
        Ok(
            self.validation[offset..self.validation.len().min(offset.saturating_add(limit))]
                .to_vec(),
        )
    }
}

#[derive(Clone)]
pub struct TrainingRequest {
    pub run_id: Uuid,
    pub snapshot_id: Uuid,
    pub labels: Vec<String>,
    pub examples: Arc<dyn TrainingExampleSource>,
    pub configuration: TrainingConfiguration,
}

impl TrainingRequest {
    pub fn in_memory(
        run_id: Uuid,
        snapshot_id: Uuid,
        labels: Vec<String>,
        training_examples: Vec<TrainingExample>,
        validation_examples: Vec<TrainingExample>,
        configuration: TrainingConfiguration,
    ) -> Self {
        Self {
            run_id,
            snapshot_id,
            labels,
            examples: Arc::new(InMemoryTrainingExamples::new(
                training_examples,
                validation_examples,
            )),
            configuration,
        }
    }

    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        self.configuration.validate()?;
        let unique_labels = self.labels.iter().collect::<BTreeSet<_>>();
        if self.labels.len() < 2
            || unique_labels.len() != self.labels.len()
            || self.labels.iter().any(|label| label.trim().is_empty())
        {
            return Err(TrainingDomainError::Labels);
        }
        if self.examples.training_member_ids().is_empty() {
            return Err(TrainingDomainError::EmptyTrainingSet);
        }
        const VALIDATION_BATCH_SIZE: usize = 256;
        for offset in (0..self.examples.training_member_ids().len()).step_by(VALIDATION_BATCH_SIZE)
        {
            let indices = (offset
                ..self
                    .examples
                    .training_member_ids()
                    .len()
                    .min(offset.saturating_add(VALIDATION_BATCH_SIZE)))
                .collect::<Vec<_>>();
            self.validate_example_labels(
                &self
                    .examples
                    .training_batch(&indices)
                    .map_err(TrainingDomainError::ExampleSource)?,
            )?;
        }
        for offset in (0..self.examples.validation_len()).step_by(VALIDATION_BATCH_SIZE) {
            self.validate_example_labels(
                &self
                    .examples
                    .validation_batch(offset, VALIDATION_BATCH_SIZE)
                    .map_err(TrainingDomainError::ExampleSource)?,
            )?;
        }
        Ok(())
    }

    fn validate_example_labels(
        &self,
        examples: &[TrainingExample],
    ) -> Result<(), TrainingDomainError> {
        for example in examples {
            if !self.labels.contains(&example.label) {
                return Err(TrainingDomainError::UnknownLabel {
                    member_id: example.snapshot_member_id,
                    label: example.label.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EpochMetrics {
    pub epoch: u32,
    pub training_loss: f64,
    pub validation_loss: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BatchMetrics {
    pub epoch: u32,
    pub batch: u32,
    pub batches_in_epoch: u32,
    pub processed_examples: u64,
    pub training_loss: f64,
    pub validation_loss: Option<f64>,
    pub learning_rate: f64,
    pub elapsed_milliseconds: u64,
    pub epoch_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub label: String,
    pub confidence: f64,
    pub probabilities: Vec<LabelProbability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelProbability {
    pub label: String,
    pub probability: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingRunState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TrainingRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingRun {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub base_model_id: Option<Uuid>,
    pub parent_checkpoint_id: Option<Uuid>,
    pub transformer_configuration: Option<TransformerTrainingConfiguration>,
    pub backend_configuration_fingerprint: Option<String>,
    pub backend_name: String,
    pub model_format: String,
    pub state: TrainingRunState,
    pub configuration: TrainingConfiguration,
    pub completed_epochs: u32,
    pub current_epoch: u32,
    pub completed_batches: u32,
    pub batches_in_epoch: u32,
    pub processed_examples: u64,
    pub latest_training_loss: Option<f64>,
    pub latest_validation_loss: Option<f64>,
    pub latest_learning_rate: Option<f64>,
    pub elapsed_milliseconds: u64,
    pub cancel_requested: bool,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TrainingRun {
    pub fn queued(
        snapshot_id: Uuid,
        backend_name: impl Into<String>,
        model_format: impl Into<String>,
        configuration: TrainingConfiguration,
    ) -> Result<Self, TrainingDomainError> {
        Self::queued_with_context(
            snapshot_id,
            backend_name,
            model_format,
            configuration,
            None,
            None,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queued_with_context(
        snapshot_id: Uuid,
        backend_name: impl Into<String>,
        model_format: impl Into<String>,
        configuration: TrainingConfiguration,
        base_model_id: Option<Uuid>,
        parent_checkpoint_id: Option<Uuid>,
        transformer_configuration: Option<TransformerTrainingConfiguration>,
        backend_configuration_fingerprint: Option<String>,
    ) -> Result<Self, TrainingDomainError> {
        configuration.validate()?;
        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4(),
            snapshot_id,
            base_model_id,
            parent_checkpoint_id,
            transformer_configuration,
            backend_configuration_fingerprint,
            backend_name: backend_name.into(),
            model_format: model_format.into(),
            state: TrainingRunState::Queued,
            configuration,
            completed_epochs: 0,
            current_epoch: 0,
            completed_batches: 0,
            batches_in_epoch: 0,
            processed_examples: 0,
            latest_training_loss: None,
            latest_validation_loss: None,
            latest_learning_rate: None,
            elapsed_milliseconds: 0,
            cancel_requested: false,
            error_message: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn transition(&mut self, next: TrainingRunState) -> Result<(), TrainingDomainError> {
        let allowed = matches!(
            (self.state, next),
            (
                TrainingRunState::Queued,
                TrainingRunState::Running | TrainingRunState::Failed | TrainingRunState::Cancelled
            ) | (
                TrainingRunState::Running,
                TrainingRunState::Completed
                    | TrainingRunState::Failed
                    | TrainingRunState::Cancelled
            )
        );
        if !allowed {
            return Err(TrainingDomainError::Transition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        self.updated_at = Utc::now();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCheckpoint {
    pub id: Uuid,
    pub run_id: Uuid,
    pub epoch: u32,
    pub artifact_path: String,
    pub artifact_checksum: String,
    pub artifact_size_bytes: u64,
    pub model_format: String,
    pub training_loss: f64,
    pub validation_loss: Option<f64>,
    pub is_final: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArtifact {
    pub path: String,
    pub checksum: String,
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        EncoderArchitecture, EncoderArtifact, EncoderMetadata, RegisteredEncoder,
        TrainingConfiguration, TrainingRun, TrainingRunState,
    };
    use uuid::Uuid;

    #[test]
    fn validates_configuration_and_run_lifecycle() {
        let invalid = TrainingConfiguration {
            epochs: 0,
            ..TrainingConfiguration::default()
        };
        assert!(invalid.validate().is_err());

        let mut run = TrainingRun::queued(
            Uuid::new_v4(),
            "linear",
            "hashing-linear-v1",
            TrainingConfiguration::default(),
        )
        .expect("run");
        run.transition(TrainingRunState::Running).expect("running");
        run.transition(TrainingRunState::Completed)
            .expect("completed");
        assert!(run.transition(TrainingRunState::Running).is_err());
    }

    #[test]
    fn validates_registered_encoder_identity() {
        let artifact = EncoderArtifact {
            file_name: "model.safetensors".into(),
            path: "models/tiny/model.safetensors".into(),
            size_bytes: 42,
            checksum: "ab".repeat(32),
        };
        let encoder = RegisteredEncoder {
            id: Uuid::new_v4(),
            name: "tiny".into(),
            architecture: EncoderArchitecture::Bert,
            source_path: "models/tiny".into(),
            fingerprint: format!("sha256:{}", "cd".repeat(32)),
            configuration: artifact.clone(),
            tokenizer: artifact.clone(),
            weights: artifact,
            metadata: EncoderMetadata {
                vocab_size: 16,
                hidden_size: 8,
                layer_count: 1,
                attention_head_count: 2,
                intermediate_size: 16,
                maximum_position_embeddings: 16,
                type_vocab_size: 2,
                padding_token_id: 0,
                tensor_dtype: "F32".into(),
            },
            created_at: Utc::now(),
        };
        encoder.validate().expect("valid encoder");
    }
}
