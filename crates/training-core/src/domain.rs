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
    #[error("training input authority binding is invalid")]
    TrainingInputBinding,
    #[error("training input fingerprint failed: {0}")]
    TrainingInputFingerprint(String),
    #[error("training run evidence is invalid: {0}")]
    TrainingRunEvidence(String),
    #[error("training checkpoint evidence is invalid: {0}")]
    TrainingCheckpointEvidence(String),
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

/// Opaque, backend-neutral authority over the exact examples supplied to a
/// trainer. Training owns the immutability contract; the caller owns the
/// meaning of the authority artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingInputBinding {
    pub protocol: String,
    pub population_fingerprint: String,
    pub member_count: u64,
    pub input_fingerprint: String,
    pub authority_kind: String,
    pub authority_id: Uuid,
    pub authority_fingerprint: String,
}

impl TrainingInputBinding {
    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        if self.protocol.trim().is_empty()
            || self.population_fingerprint.trim().is_empty()
            || self.member_count == 0
            || self.input_fingerprint.trim().is_empty()
            || self.authority_kind.trim().is_empty()
            || self.authority_id.is_nil()
            || self.authority_fingerprint.trim().is_empty()
        {
            return Err(TrainingDomainError::TrainingInputBinding);
        }
        Ok(())
    }
}

/// Backend-neutral, bounded access to snapshot examples. The runner seals a
/// per-position fingerprint projection before handing this port to a backend;
/// implementations therefore do not have to be trusted to stay immutable.
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

struct FingerprintVerifiedTrainingExamples {
    source: Arc<dyn TrainingExampleSource>,
    training_member_ids: Vec<Uuid>,
    validation_len: usize,
    training_fingerprints: Vec<String>,
    validation_fingerprints: Vec<String>,
}

impl TrainingExampleSource for FingerprintVerifiedTrainingExamples {
    fn training_member_ids(&self) -> &[Uuid] {
        &self.training_member_ids
    }

    fn validation_len(&self) -> usize {
        self.validation_len
    }

    fn training_batch(&self, indices: &[usize]) -> Result<Vec<TrainingExample>, String> {
        if indices
            .iter()
            .any(|index| *index >= self.training_fingerprints.len())
        {
            return Err("training example index is out of sealed range".into());
        }
        let examples = self.source.training_batch(indices)?;
        if examples.len() != indices.len()
            || examples.iter().zip(indices).any(|(example, index)| {
                self.training_member_ids.get(*index) != Some(&example.snapshot_member_id)
                    || training_example_fingerprint(example).ok().as_ref()
                        != self.training_fingerprints.get(*index)
            })
        {
            return Err("training example source changed after it was sealed".into());
        }
        Ok(examples)
    }

    fn validation_batch(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<TrainingExample>, String> {
        if offset > self.validation_len {
            return Err(format!(
                "validation example offset is out of sealed range: {offset}"
            ));
        }
        let end = self.validation_len.min(offset.saturating_add(limit));
        let examples = self.source.validation_batch(offset, limit)?;
        if examples.len() != end - offset
            || examples.iter().enumerate().any(|(relative, example)| {
                training_example_fingerprint(example).ok().as_ref()
                    != self.validation_fingerprints.get(offset + relative)
            })
        {
            return Err("validation example source changed after it was sealed".into());
        }
        Ok(examples)
    }
}

#[derive(Clone)]
pub struct TrainingRequest {
    pub run_id: Uuid,
    pub snapshot_id: Uuid,
    pub labels: Vec<String>,
    pub examples: Arc<dyn TrainingExampleSource>,
    pub configuration: TrainingConfiguration,
    pub input_binding: Option<TrainingInputBinding>,
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
            input_binding: None,
        }
    }

    pub fn validate(&self) -> Result<(), TrainingDomainError> {
        self.configuration.validate()?;
        if let Some(binding) = &self.input_binding {
            binding.validate()?;
            let observed_count = self
                .examples
                .training_member_ids()
                .len()
                .checked_add(self.examples.validation_len())
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| {
                    TrainingDomainError::TrainingInputFingerprint("example count overflow".into())
                })?;
            if binding.member_count != observed_count
                || binding.input_fingerprint != self.reproduce_input_fingerprint()?
            {
                return Err(TrainingDomainError::TrainingInputBinding);
            }
        }
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

    /// Captures one complete, bounded projection of the source and returns a
    /// request whose backend port verifies every later read against that exact
    /// projection. This closes the gap between host validation and a backend's
    /// later shuffled or repeated reads without materializing all text in RAM.
    pub fn seal_for_backend(mut self) -> Result<Self, TrainingDomainError> {
        self.configuration.validate()?;
        let unique_labels = self.labels.iter().collect::<BTreeSet<_>>();
        if self.labels.len() < 2
            || unique_labels.len() != self.labels.len()
            || self.labels.iter().any(|label| label.trim().is_empty())
        {
            return Err(TrainingDomainError::Labels);
        }

        const BATCH_SIZE: usize = 256;
        let training_member_ids = self.examples.training_member_ids().to_vec();
        if training_member_ids.is_empty() {
            return Err(TrainingDomainError::EmptyTrainingSet);
        }
        let validation_len = self.examples.validation_len();
        let mut training_fingerprints = Vec::with_capacity(training_member_ids.len());
        for offset in (0..training_member_ids.len()).step_by(BATCH_SIZE) {
            let indices = (offset
                ..training_member_ids
                    .len()
                    .min(offset.saturating_add(BATCH_SIZE)))
                .collect::<Vec<_>>();
            let examples = self
                .examples
                .training_batch(&indices)
                .map_err(TrainingDomainError::ExampleSource)?;
            if examples.len() != indices.len()
                || examples.iter().zip(&indices).any(|(example, index)| {
                    training_member_ids.get(*index) != Some(&example.snapshot_member_id)
                })
            {
                return Err(TrainingDomainError::TrainingInputFingerprint(
                    "training source identity projection differs".into(),
                ));
            }
            self.validate_example_labels(&examples)?;
            training_fingerprints.extend(
                examples
                    .iter()
                    .map(training_example_fingerprint)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }

        let mut validation_fingerprints = Vec::with_capacity(validation_len);
        for offset in (0..validation_len).step_by(BATCH_SIZE) {
            let expected = validation_len.min(offset.saturating_add(BATCH_SIZE)) - offset;
            let examples = self
                .examples
                .validation_batch(offset, BATCH_SIZE)
                .map_err(TrainingDomainError::ExampleSource)?;
            if examples.len() != expected {
                return Err(TrainingDomainError::TrainingInputFingerprint(
                    "validation source length projection differs".into(),
                ));
            }
            self.validate_example_labels(&examples)?;
            validation_fingerprints.extend(
                examples
                    .iter()
                    .map(training_example_fingerprint)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }

        if let Some(binding) = &self.input_binding {
            binding.validate()?;
            let observed_count = training_member_ids
                .len()
                .checked_add(validation_len)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| {
                    TrainingDomainError::TrainingInputFingerprint("example count overflow".into())
                })?;
            let observed_fingerprint = request_input_fingerprint(
                self.snapshot_id,
                &self.labels,
                &training_fingerprints,
                &validation_fingerprints,
            )?;
            if binding.member_count != observed_count
                || binding.input_fingerprint != observed_fingerprint
            {
                return Err(TrainingDomainError::TrainingInputBinding);
            }
        }

        self.examples = Arc::new(FingerprintVerifiedTrainingExamples {
            source: self.examples,
            training_member_ids,
            validation_len,
            training_fingerprints,
            validation_fingerprints,
        });
        Ok(self)
    }

    /// Reproduces a versioned digest over exactly the ordered labels and
    /// Train/Validation examples visible through the backend port.
    pub fn reproduce_input_fingerprint(&self) -> Result<String, TrainingDomainError> {
        const BATCH_SIZE: usize = 256;
        let mut training = Vec::with_capacity(self.examples.training_member_ids().len());
        for offset in (0..self.examples.training_member_ids().len()).step_by(BATCH_SIZE) {
            let indices = (offset
                ..self
                    .examples
                    .training_member_ids()
                    .len()
                    .min(offset.saturating_add(BATCH_SIZE)))
                .collect::<Vec<_>>();
            let examples = self
                .examples
                .training_batch(&indices)
                .map_err(TrainingDomainError::ExampleSource)?;
            if examples.len() != indices.len()
                || examples.iter().zip(&indices).any(|(example, index)| {
                    self.examples.training_member_ids().get(*index)
                        != Some(&example.snapshot_member_id)
                })
            {
                return Err(TrainingDomainError::TrainingInputFingerprint(
                    "training source identity projection differs".into(),
                ));
            }
            training.extend(
                examples
                    .iter()
                    .map(artifact_core::fingerprint)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        TrainingDomainError::TrainingInputFingerprint(error.to_string())
                    })?,
            );
        }
        let mut validation = Vec::with_capacity(self.examples.validation_len());
        for offset in (0..self.examples.validation_len()).step_by(BATCH_SIZE) {
            let expected = self
                .examples
                .validation_len()
                .min(offset.saturating_add(BATCH_SIZE))
                - offset;
            let examples = self
                .examples
                .validation_batch(offset, BATCH_SIZE)
                .map_err(TrainingDomainError::ExampleSource)?;
            if examples.len() != expected {
                return Err(TrainingDomainError::TrainingInputFingerprint(
                    "validation source length projection differs".into(),
                ));
            }
            validation.extend(
                examples
                    .iter()
                    .map(artifact_core::fingerprint)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        TrainingDomainError::TrainingInputFingerprint(error.to_string())
                    })?,
            );
        }
        request_input_fingerprint(self.snapshot_id, &self.labels, &training, &validation)
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

fn training_example_fingerprint(example: &TrainingExample) -> Result<String, TrainingDomainError> {
    artifact_core::fingerprint(example)
        .map_err(|error| TrainingDomainError::TrainingInputFingerprint(error.to_string()))
}

fn request_input_fingerprint(
    snapshot_id: Uuid,
    labels: &[String],
    training: &[String],
    validation: &[String],
) -> Result<String, TrainingDomainError> {
    artifact_core::fingerprint(&serde_json::json!({
        "protocol": "training-request-input-v1",
        "snapshot_id": snapshot_id,
        "labels": labels,
        "training_example_fingerprints": training,
        "validation_example_fingerprints": validation,
    }))
    .map_err(|error| TrainingDomainError::TrainingInputFingerprint(error.to_string()))
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_binding: Option<TrainingInputBinding>,
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
            input_binding: None,
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

    pub fn with_input_binding(
        mut self,
        binding: TrainingInputBinding,
    ) -> Result<Self, TrainingDomainError> {
        binding.validate()?;
        self.input_binding = Some(binding);
        Ok(self)
    }

    /// Replaces the generated identity before a pristine queued run enters
    /// persistence so orchestration can reserve and link it durably.
    pub fn with_reserved_id(mut self, id: Uuid) -> Result<Self, TrainingDomainError> {
        self.validate_new()?;
        if id.is_nil() {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "reserved run identity must not be nil".into(),
            ));
        }
        self.id = id;
        Ok(self)
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

    /// Validates the only state that may enter persistence as a new run.
    pub fn validate_new(&self) -> Result<(), TrainingDomainError> {
        self.validate_evidence_shape()?;
        if self.state != TrainingRunState::Queued
            || self.completed_epochs != 0
            || self.current_epoch != 0
            || self.completed_batches != 0
            || self.batches_in_epoch != 0
            || self.processed_examples != 0
            || self.latest_training_loss.is_some()
            || self.latest_validation_loss.is_some()
            || self.latest_learning_rate.is_some()
            || self.elapsed_milliseconds != 0
            || self.cancel_requested
            || self.error_message.is_some()
            || self.created_at != self.updated_at
        {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "new runs must be canonical queued runs with zero progress".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_persisted(&self) -> Result<(), TrainingDomainError> {
        self.validate_evidence_shape()
    }

    /// Validates an append-forward persisted lifecycle transition. Mutable
    /// progress remains public for backend-neutral runners, while persistence
    /// refuses callers that skip or reverse lifecycle facts.
    pub fn validate_update_from(&self, previous: &Self) -> Result<(), TrainingDomainError> {
        self.validate_evidence_shape()?;
        if self.id != previous.id
            || self.snapshot_id != previous.snapshot_id
            || self.base_model_id != previous.base_model_id
            || self.parent_checkpoint_id != previous.parent_checkpoint_id
            || self.transformer_configuration != previous.transformer_configuration
            || self.backend_configuration_fingerprint != previous.backend_configuration_fingerprint
            || self.input_binding != previous.input_binding
            || self.backend_name != previous.backend_name
            || self.model_format != previous.model_format
            || self.configuration != previous.configuration
            || self.created_at != previous.created_at
        {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "immutable run identity or configuration changed".into(),
            ));
        }
        let legal_state = matches!(
            (previous.state, self.state),
            (
                TrainingRunState::Queued,
                TrainingRunState::Running | TrainingRunState::Failed | TrainingRunState::Cancelled
            ) | (
                TrainingRunState::Running,
                TrainingRunState::Running
                    | TrainingRunState::Completed
                    | TrainingRunState::Failed
                    | TrainingRunState::Cancelled
            )
        );
        if !legal_state
            || self.completed_epochs < previous.completed_epochs
            || self.current_epoch < previous.current_epoch
            || self.processed_examples < previous.processed_examples
            || self.elapsed_milliseconds < previous.elapsed_milliseconds
            || (self.current_epoch == previous.current_epoch
                && self.completed_batches < previous.completed_batches)
        {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "run lifecycle or cumulative progress moved backward or skipped state".into(),
            ));
        }
        if previous.state == TrainingRunState::Queued && !self.has_zero_progress() {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "queued runs must enter their first state before recording progress".into(),
            ));
        }
        if previous.state == TrainingRunState::Running {
            match self.state {
                TrainingRunState::Running => {
                    let same_epoch = self.current_epoch == previous.current_epoch;
                    let next_epoch = self.current_epoch == previous.current_epoch.saturating_add(1)
                        && self.completed_batches == 1
                        && previous.current_epoch == previous.completed_epochs
                        && (previous.current_epoch == 0
                            || previous.completed_batches == previous.batches_in_epoch);
                    let one_batch = self.processed_examples > previous.processed_examples
                        && ((same_epoch
                            && self.completed_batches
                                == previous.completed_batches.saturating_add(1))
                            || next_epoch);
                    let epoch_progress = if self.completed_epochs == previous.completed_epochs {
                        true
                    } else {
                        self.completed_epochs == previous.completed_epochs.saturating_add(1)
                            && self.completed_epochs == self.current_epoch
                            && self.completed_batches == self.batches_in_epoch
                    };
                    if !one_batch || !epoch_progress {
                        return Err(TrainingDomainError::TrainingRunEvidence(
                            "a running update must commit exactly one next backend batch".into(),
                        ));
                    }
                }
                TrainingRunState::Completed
                | TrainingRunState::Failed
                | TrainingRunState::Cancelled => {
                    if !self.has_same_progress(previous) {
                        return Err(TrainingDomainError::TrainingRunEvidence(
                            "terminal transition must use the last durably saved progress".into(),
                        ));
                    }
                }
                TrainingRunState::Queued => unreachable!("legal-state check rejects regression"),
            }
        }
        Ok(())
    }

    fn has_zero_progress(&self) -> bool {
        self.completed_epochs == 0
            && self.current_epoch == 0
            && self.completed_batches == 0
            && self.batches_in_epoch == 0
            && self.processed_examples == 0
            && self.latest_training_loss.is_none()
            && self.latest_validation_loss.is_none()
            && self.latest_learning_rate.is_none()
            && self.elapsed_milliseconds == 0
    }

    fn has_same_progress(&self, previous: &Self) -> bool {
        self.completed_epochs == previous.completed_epochs
            && self.current_epoch == previous.current_epoch
            && self.completed_batches == previous.completed_batches
            && self.batches_in_epoch == previous.batches_in_epoch
            && self.processed_examples == previous.processed_examples
            && self.latest_training_loss == previous.latest_training_loss
            && self.latest_validation_loss == previous.latest_validation_loss
            && self.latest_learning_rate == previous.latest_learning_rate
            && self.elapsed_milliseconds == previous.elapsed_milliseconds
    }

    fn validate_evidence_shape(&self) -> Result<(), TrainingDomainError> {
        self.configuration.validate()?;
        if let Some(configuration) = &self.transformer_configuration {
            configuration.validate()?;
        }
        if let Some(binding) = &self.input_binding {
            binding.validate()?;
        }
        let invalid_identity = self.id.is_nil()
            || self.snapshot_id.is_nil()
            || self.base_model_id.is_some_and(|id| id.is_nil())
            || self.parent_checkpoint_id.is_some_and(|id| id.is_nil())
            || self.backend_name.trim().is_empty()
            || self.backend_name.trim() != self.backend_name
            || self.model_format.trim().is_empty()
            || self.model_format.trim() != self.model_format
            || self
                .backend_configuration_fingerprint
                .as_deref()
                .is_some_and(|value| value.trim().is_empty());
        let invalid_progress = self.completed_epochs > self.configuration.epochs
            || self.current_epoch > self.configuration.epochs
            || self.current_epoch < self.completed_epochs
            || self.current_epoch > self.completed_epochs.saturating_add(1)
            || self.completed_batches > self.batches_in_epoch
            || ((self.completed_batches == 0) != (self.batches_in_epoch == 0))
            || (self.completed_batches > 0 && self.current_epoch == 0)
            || self
                .latest_training_loss
                .is_some_and(|value| !value.is_finite())
            || self
                .latest_validation_loss
                .is_some_and(|value| !value.is_finite())
            || self
                .latest_learning_rate
                .is_some_and(|value| !value.is_finite() || value <= 0.0)
            || (self.completed_batches > 0
                && (self.latest_training_loss.is_none() || self.latest_learning_rate.is_none()));
        let invalid_terminal = match self.state {
            TrainingRunState::Completed => {
                self.completed_epochs != self.configuration.epochs
                    || self.current_epoch != self.configuration.epochs
                    || self.completed_batches == 0
                    || self.completed_batches != self.batches_in_epoch
                    || self.processed_examples == 0
                    || self.error_message.is_some()
            }
            TrainingRunState::Failed => self
                .error_message
                .as_deref()
                .is_none_or(|message| message.trim().is_empty()),
            TrainingRunState::Queued | TrainingRunState::Running | TrainingRunState::Cancelled => {
                self.error_message.is_some()
            }
        };
        if invalid_identity || invalid_progress || invalid_terminal {
            return Err(TrainingDomainError::TrainingRunEvidence(
                "identity, progress, metrics, error, or terminal state is inconsistent".into(),
            ));
        }
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

impl TrainingCheckpoint {
    pub fn validate_for_run(&self, run: &TrainingRun) -> Result<(), TrainingDomainError> {
        let invalid = self.id.is_nil()
            || self.run_id != run.id
            || run.state != TrainingRunState::Running
            || self.epoch == 0
            || self.epoch != run.completed_epochs
            || self.epoch > run.configuration.epochs
            || self.is_final != (self.epoch == run.configuration.epochs)
            || self.artifact_path.trim().is_empty()
            || self.artifact_checksum.trim().is_empty()
            || self.artifact_size_bytes == 0
            || self.model_format != run.model_format
            || !self.training_loss.is_finite()
            || self.validation_loss.is_some_and(|value| !value.is_finite());
        if invalid {
            return Err(TrainingDomainError::TrainingCheckpointEvidence(
                "checkpoint does not match current completed-epoch run evidence".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArtifact {
    pub path: String,
    pub checksum: String,
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use chrono::Utc;

    use super::{
        EncoderArchitecture, EncoderArtifact, EncoderMetadata, RegisteredEncoder,
        TrainingConfiguration, TrainingExample, TrainingExampleSource, TrainingInputBinding,
        TrainingRequest, TrainingRun, TrainingRunState,
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

    #[test]
    fn bound_request_rejects_backend_facing_example_changes() {
        let snapshot_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let labels = vec!["billing".into(), "fraud".into()];
        let configuration = TrainingConfiguration::default();
        let mut original = TrainingRequest::in_memory(
            Uuid::new_v4(),
            snapshot_id,
            labels.clone(),
            vec![TrainingExample {
                snapshot_member_id: member_id,
                text: "charged twice".into(),
                label: "billing".into(),
            }],
            Vec::new(),
            configuration.clone(),
        );
        let input_fingerprint = original
            .reproduce_input_fingerprint()
            .expect("input fingerprint");
        let binding = TrainingInputBinding {
            protocol: "train_and_validation_v1".into(),
            population_fingerprint: "sha256:population".into(),
            member_count: 1,
            input_fingerprint,
            authority_kind: "training_benchmark_check".into(),
            authority_id: Uuid::new_v4(),
            authority_fingerprint: "sha256:authority".into(),
        };
        original.input_binding = Some(binding.clone());
        original.validate().expect("exact request is authorized");

        let mut changed = TrainingRequest::in_memory(
            original.run_id,
            snapshot_id,
            labels,
            vec![TrainingExample {
                snapshot_member_id: member_id,
                text: "different text".into(),
                label: "billing".into(),
            }],
            Vec::new(),
            configuration,
        );
        changed.input_binding = Some(binding);
        assert!(matches!(
            changed.validate(),
            Err(super::TrainingDomainError::TrainingInputBinding)
        ));
    }

    struct ChangingSource {
        member_ids: Vec<Uuid>,
        reads: AtomicUsize,
    }

    impl TrainingExampleSource for ChangingSource {
        fn training_member_ids(&self) -> &[Uuid] {
            &self.member_ids
        }

        fn validation_len(&self) -> usize {
            0
        }

        fn training_batch(&self, indices: &[usize]) -> Result<Vec<TrainingExample>, String> {
            let text = if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                "cleared text"
            } else {
                "changed after clearance"
            };
            indices
                .iter()
                .map(|index| {
                    Ok(TrainingExample {
                        snapshot_member_id: *self
                            .member_ids
                            .get(*index)
                            .ok_or_else(|| "index out of range".to_owned())?,
                        text: text.into(),
                        label: "billing".into(),
                    })
                })
                .collect()
        }

        fn validation_batch(
            &self,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<TrainingExample>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn sealed_request_rejects_a_source_that_changes_before_backend_read() {
        let request = TrainingRequest {
            run_id: Uuid::new_v4(),
            snapshot_id: Uuid::new_v4(),
            labels: vec!["billing".into(), "fraud".into()],
            examples: Arc::new(ChangingSource {
                member_ids: vec![Uuid::new_v4()],
                reads: AtomicUsize::new(0),
            }),
            configuration: TrainingConfiguration::default(),
            input_binding: None,
        };

        let sealed = request.seal_for_backend().expect("source seals once");
        assert!(sealed.examples.training_batch(&[0]).is_err());
    }
}
