use std::{collections::BTreeMap, fs, sync::Mutex, time::Instant};

use candle_core::{DType, Device, Tensor, Var};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::bert::Config;
use serde::Serialize;
use training_core::{
    domain::{
        BatchMetrics, EncoderTrainingMode, LabelProbability, Prediction, RegisteredEncoder,
        TrainingExample, TrainingRequest, TransformerTrainingConfiguration,
    },
    ports::{Predictor, PredictorLoader, TrainingBackend, TrainingBackendError, TrainingSession},
};

use crate::{
    batching::BatchPlan,
    bundle::BertBundle,
    checkpoint::{BertCheckpoint, BertCheckpointManifest},
    model::BertClassifier,
    optimizer::SerializableAdamW,
    tokenization::BertTokenizer,
};

const MODEL_FORMAT: &str = "bert-classifier-v1";

pub struct BertTrainingBackend {
    encoder: RegisteredEncoder,
    configuration: TransformerTrainingConfiguration,
    continuation: Option<Vec<u8>>,
}

impl BertTrainingBackend {
    pub fn new(
        encoder: RegisteredEncoder,
        configuration: TransformerTrainingConfiguration,
    ) -> Result<Self, TrainingBackendError> {
        let bundle = BertBundle::verify(&encoder).map_err(configuration_error)?;
        validate_configuration(&configuration, bundle.configuration())
            .map_err(configuration_error)?;
        Ok(Self {
            encoder,
            configuration,
            continuation: None,
        })
    }

    pub fn continuing_from(
        encoder: RegisteredEncoder,
        configuration: TransformerTrainingConfiguration,
        checkpoint: Vec<u8>,
    ) -> Result<Self, TrainingBackendError> {
        let mut backend = Self::new(encoder, configuration)?;
        backend.continuation = Some(checkpoint);
        Ok(backend)
    }
}

impl TrainingBackend for BertTrainingBackend {
    fn name(&self) -> &str {
        "bert-cpu"
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
        let bundle = BertBundle::verify(&self.encoder).map_err(configuration_error)?;
        validate_configuration(&self.configuration, bundle.configuration())
            .map_err(configuration_error)?;
        let device = Device::Cpu;
        let variables = VarMap::new();
        let builder = VarBuilder::from_varmap(&variables, DType::F32, &device);
        let model = BertClassifier::load(builder, bundle.configuration(), request.labels.len())
            .map_err(training_error)?;
        load_base_weights(&variables, &self.encoder.weights.path).map_err(training_error)?;

        let tokenizer_json = fs::read(&self.encoder.tokenizer.path).map_err(training_error)?;
        let tokenizer = BertTokenizer::from_bytes(
            &tokenizer_json,
            bundle.configuration().pad_token_id,
            &self.configuration,
        )
        .map_err(TrainingBackendError::Configuration)?;
        let configuration_json =
            fs::read_to_string(&self.encoder.configuration.path).map_err(training_error)?;
        let trainable =
            trainable_variables(&variables, self.configuration.mode).map_err(training_error)?;
        let mut optimizer = SerializableAdamW::new(trainable, self.configuration.weight_decay)
            .map_err(training_error)?;

        if let Some(bytes) = &self.continuation {
            let checkpoint = BertCheckpoint::deserialize(bytes, &device)
                .map_err(TrainingBackendError::InvalidArtifact)?;
            validate_continuation(
                &checkpoint.manifest,
                &self.encoder,
                &self.configuration,
                &request,
            )?;
            restore_model_tensors(&variables, &checkpoint.tensors).map_err(training_error)?;
            optimizer
                .restore(checkpoint.manifest.optimizer_step, &checkpoint.tensors)
                .map_err(training_error)?;
        }

        Ok(Box::new(BertTrainingSession {
            model,
            variables,
            optimizer,
            tokenizer,
            device,
            request,
            transformer_configuration: self.configuration.clone(),
            encoder: self.encoder.clone(),
            bert_configuration_json: configuration_json,
            tokenizer_json: String::from_utf8(tokenizer_json).map_err(|error| {
                TrainingBackendError::Configuration(format!(
                    "tokenizer.json must be UTF-8: {error}"
                ))
            })?,
            completed_epochs: 0,
            batch_index: 0,
            processed_examples: 0,
            epoch_loss_sum: 0.0,
            epoch_example_count: 0,
            started_at: Instant::now(),
        }))
    }
}

struct BertTrainingSession {
    model: BertClassifier,
    variables: VarMap,
    optimizer: SerializableAdamW,
    tokenizer: BertTokenizer,
    device: Device,
    request: TrainingRequest,
    transformer_configuration: TransformerTrainingConfiguration,
    encoder: RegisteredEncoder,
    bert_configuration_json: String,
    tokenizer_json: String,
    completed_epochs: u32,
    batch_index: usize,
    processed_examples: u64,
    epoch_loss_sum: f64,
    epoch_example_count: usize,
    started_at: Instant,
}

impl TrainingSession for BertTrainingSession {
    fn train_batch(&mut self) -> Result<BatchMetrics, TrainingBackendError> {
        let epoch = self.completed_epochs + 1;
        let plan = BatchPlan::deterministic(
            self.request.examples.training_member_ids(),
            self.transformer_configuration.batch_size,
            self.request.configuration.seed,
            epoch,
        );
        let indices = plan
            .batch(self.batch_index)
            .ok_or_else(|| TrainingBackendError::Training("batch plan is exhausted".into()))?;
        let examples = self
            .request
            .examples
            .training_batch(indices)
            .map_err(TrainingBackendError::Training)?;
        let example_refs = examples.iter().collect::<Vec<_>>();
        let batch = self
            .tokenizer
            .encode(&example_refs, &self.request.labels, &self.device)
            .map_err(TrainingBackendError::Training)?;
        let logits = self
            .model
            .forward(
                &batch.input_ids,
                &batch.token_type_ids,
                &batch.attention_mask,
            )
            .map_err(training_error)?;
        let loss =
            candle_nn::loss::cross_entropy(&logits, &batch.targets).map_err(training_error)?;
        let loss_value = f64::from(loss.to_scalar::<f32>().map_err(training_error)?);
        let gradients = loss.backward().map_err(training_error)?;
        let local_step = usize::try_from(self.completed_epochs)
            .ok()
            .and_then(|epochs| epochs.checked_mul(plan.len()))
            .and_then(|steps| steps.checked_add(self.batch_index + 1))
            .ok_or_else(|| TrainingBackendError::Training("training step overflow".into()))?;
        let total_steps = usize::try_from(self.request.configuration.epochs)
            .ok()
            .and_then(|epochs| epochs.checked_mul(plan.len()))
            .ok_or_else(|| {
                TrainingBackendError::Training("total training steps overflow".into())
            })?;
        let learning_rate = scheduled_learning_rate(
            f64::from(self.request.configuration.learning_rate),
            local_step,
            total_steps,
            self.transformer_configuration.warmup_ratio,
        );
        self.optimizer
            .step(
                &gradients,
                learning_rate,
                self.transformer_configuration.gradient_clip_norm,
            )
            .map_err(training_error)?;

        self.batch_index += 1;
        self.processed_examples = self
            .processed_examples
            .saturating_add(u64::try_from(batch.size).unwrap_or(u64::MAX));
        self.epoch_loss_sum += loss_value * batch.size as f64;
        self.epoch_example_count += batch.size;
        let epoch_complete = self.batch_index == plan.len();
        let training_loss = self.epoch_loss_sum / self.epoch_example_count as f64;
        let validation_loss = if epoch_complete {
            self.validation_loss()?
        } else {
            None
        };
        let metrics = BatchMetrics {
            epoch,
            batch: u32::try_from(self.batch_index).map_err(|_| {
                TrainingBackendError::Training("batch index does not fit u32".into())
            })?,
            batches_in_epoch: u32::try_from(plan.len()).map_err(|_| {
                TrainingBackendError::Training("batch count does not fit u32".into())
            })?,
            processed_examples: self.processed_examples,
            training_loss,
            validation_loss,
            learning_rate,
            elapsed_milliseconds: u64::try_from(self.started_at.elapsed().as_millis())
                .unwrap_or(u64::MAX),
            epoch_complete,
        };
        if epoch_complete {
            self.completed_epochs = epoch;
            self.batch_index = 0;
            self.epoch_loss_sum = 0.0;
            self.epoch_example_count = 0;
        }
        Ok(metrics)
    }

    fn serialize_checkpoint(&self) -> Result<Vec<u8>, TrainingBackendError> {
        let mut tensors = model_tensors(&self.variables).map_err(training_error)?;
        tensors.extend(self.optimizer.state_tensors());
        BertCheckpoint {
            manifest: BertCheckpointManifest {
                format_version: 1,
                architecture: "bert".into(),
                base_model_id: self.encoder.id,
                base_model_fingerprint: self.encoder.fingerprint.clone(),
                tokenizer_fingerprint: format!("sha256:{}", self.encoder.tokenizer.checksum),
                snapshot_id: self.request.snapshot_id,
                labels: self.request.labels.clone(),
                bert_configuration_json: self.bert_configuration_json.clone(),
                tokenizer_json: self.tokenizer_json.clone(),
                training_configuration: self.request.configuration.clone(),
                transformer_configuration: self.transformer_configuration.clone(),
                completed_epoch: self.completed_epochs,
                completed_batch: u32::try_from(self.batch_index).unwrap_or(u32::MAX),
                optimizer_step: self.optimizer.step_count(),
            },
            tensors,
        }
        .serialize()
        .map_err(TrainingBackendError::Training)
    }
}

impl BertTrainingSession {
    fn validation_loss(&self) -> Result<Option<f64>, TrainingBackendError> {
        if self.request.examples.validation_len() == 0 {
            return Ok(None);
        }
        let mut loss_sum = 0.0;
        let mut count = 0_usize;
        for offset in (0..self.request.examples.validation_len())
            .step_by(self.transformer_configuration.batch_size)
        {
            let examples = self
                .request
                .examples
                .validation_batch(offset, self.transformer_configuration.batch_size)
                .map_err(TrainingBackendError::Training)?;
            let examples = examples.iter().collect::<Vec<_>>();
            let batch = self
                .tokenizer
                .encode(&examples, &self.request.labels, &self.device)
                .map_err(TrainingBackendError::Training)?;
            let logits = self
                .model
                .forward(
                    &batch.input_ids,
                    &batch.token_type_ids,
                    &batch.attention_mask,
                )
                .map_err(training_error)?;
            let loss = candle_nn::loss::cross_entropy(&logits, &batch.targets)
                .map_err(training_error)?
                .to_scalar::<f32>()
                .map_err(training_error)?;
            loss_sum += f64::from(loss) * batch.size as f64;
            count += batch.size;
        }
        Ok(Some(loss_sum / count as f64))
    }
}

pub struct BertPredictorLoader;

#[derive(Debug, Clone, Serialize)]
pub struct BertCheckpointMetadata {
    pub base_model_id: uuid::Uuid,
    pub base_model_fingerprint: String,
    pub tokenizer_fingerprint: String,
    pub snapshot_id: uuid::Uuid,
    pub labels: Vec<String>,
    pub training_configuration: training_core::domain::TrainingConfiguration,
    pub transformer_configuration: TransformerTrainingConfiguration,
    pub completed_epoch: u32,
    pub optimizer_step: u64,
}

impl BertPredictorLoader {
    pub fn inspect(artifact: &[u8]) -> Result<BertCheckpointMetadata, TrainingBackendError> {
        let checkpoint = BertCheckpoint::deserialize(artifact, &Device::Cpu)
            .map_err(TrainingBackendError::InvalidArtifact)?;
        let manifest = checkpoint.manifest;
        Ok(BertCheckpointMetadata {
            base_model_id: manifest.base_model_id,
            base_model_fingerprint: manifest.base_model_fingerprint,
            tokenizer_fingerprint: manifest.tokenizer_fingerprint,
            snapshot_id: manifest.snapshot_id,
            labels: manifest.labels,
            training_configuration: manifest.training_configuration,
            transformer_configuration: manifest.transformer_configuration,
            completed_epoch: manifest.completed_epoch,
            optimizer_step: manifest.optimizer_step,
        })
    }
}

impl PredictorLoader for BertPredictorLoader {
    fn model_format(&self) -> &str {
        MODEL_FORMAT
    }

    fn load(&self, artifact: &[u8]) -> Result<Box<dyn Predictor>, TrainingBackendError> {
        let device = Device::Cpu;
        let checkpoint = BertCheckpoint::deserialize(artifact, &device)
            .map_err(TrainingBackendError::InvalidArtifact)?;
        let configuration: Config =
            serde_json::from_str(&checkpoint.manifest.bert_configuration_json)
                .map_err(|error| TrainingBackendError::InvalidArtifact(error.to_string()))?;
        validate_configuration(
            &checkpoint.manifest.transformer_configuration,
            &configuration,
        )
        .map_err(|error| TrainingBackendError::InvalidArtifact(error.to_string()))?;
        let tokenizer = BertTokenizer::from_bytes(
            checkpoint.manifest.tokenizer_json.as_bytes(),
            configuration.pad_token_id,
            &checkpoint.manifest.transformer_configuration,
        )
        .map_err(TrainingBackendError::InvalidArtifact)?;
        let variables = VarMap::new();
        let builder = VarBuilder::from_varmap(&variables, DType::F32, &device);
        let model = BertClassifier::load(builder, &configuration, checkpoint.manifest.labels.len())
            .map_err(invalid_artifact)?;
        restore_model_tensors(&variables, &checkpoint.tensors).map_err(invalid_artifact)?;
        Ok(Box::new(BertPredictor {
            model: Mutex::new(model),
            tokenizer,
            device,
            labels: checkpoint.manifest.labels,
        }))
    }
}

struct BertPredictor {
    model: Mutex<BertClassifier>,
    tokenizer: BertTokenizer,
    device: Device,
    labels: Vec<String>,
}

impl Predictor for BertPredictor {
    fn labels(&self) -> &[String] {
        &self.labels
    }

    fn predict(&self, text: &str) -> Result<Prediction, TrainingBackendError> {
        self.predict_batch(&[text.to_owned()])?
            .into_iter()
            .next()
            .ok_or_else(|| TrainingBackendError::Training("predictor returned no rows".into()))
    }

    fn predict_batch(&self, texts: &[String]) -> Result<Vec<Prediction>, TrainingBackendError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let placeholder = self.labels.first().cloned().ok_or_else(|| {
            TrainingBackendError::InvalidArtifact("checkpoint has no labels".into())
        })?;
        let examples = texts
            .iter()
            .map(|text| TrainingExample {
                snapshot_member_id: uuid::Uuid::nil(),
                text: text.clone(),
                label: placeholder.clone(),
            })
            .collect::<Vec<_>>();
        let example_refs = examples.iter().collect::<Vec<_>>();
        let batch = self
            .tokenizer
            .encode(&example_refs, &self.labels, &self.device)
            .map_err(TrainingBackendError::Training)?;
        let logits = self
            .model
            .lock()
            .map_err(|error| TrainingBackendError::Training(error.to_string()))?
            .forward(
                &batch.input_ids,
                &batch.token_type_ids,
                &batch.attention_mask,
            )
            .map_err(training_error)?;
        let probabilities = candle_nn::ops::softmax(&logits, 1)
            .map_err(training_error)?
            .to_vec2::<f32>()
            .map_err(training_error)?;
        probabilities
            .into_iter()
            .map(|probabilities| {
                let (best, confidence) = probabilities
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|left, right| left.1.total_cmp(&right.1))
                    .ok_or_else(|| {
                        TrainingBackendError::Training("predictor returned no labels".into())
                    })?;
                Ok(Prediction {
                    label: self.labels[best].clone(),
                    confidence: f64::from(confidence),
                    probabilities: self
                        .labels
                        .iter()
                        .cloned()
                        .zip(probabilities)
                        .map(|(label, probability)| LabelProbability {
                            label,
                            probability: f64::from(probability),
                        })
                        .collect(),
                })
            })
            .collect()
    }
}

fn load_base_weights(variables: &VarMap, path: &str) -> candle_core::Result<()> {
    let tensors = candle_core::safetensors::load(path, &Device::Cpu)?;
    let variables = named_variables(variables)?;
    for (name, variable) in variables {
        if name.starts_with("classifier.") {
            variable.set(&Tensor::zeros(
                variable.shape(),
                variable.dtype(),
                variable.device(),
            )?)?;
            continue;
        }
        let tensor = tensors.get(&name).ok_or_else(|| {
            candle_core::Error::Msg(format!("base model is missing tensor {name}"))
        })?;
        variable.set(tensor)?;
    }
    Ok(())
}

fn restore_model_tensors(
    variables: &VarMap,
    tensors: &BTreeMap<String, Tensor>,
) -> candle_core::Result<()> {
    for (name, variable) in named_variables(variables)? {
        let tensor = tensors.get(&format!("model.{name}")).ok_or_else(|| {
            candle_core::Error::Msg(format!("checkpoint is missing tensor {name}"))
        })?;
        variable.set(tensor)?;
    }
    Ok(())
}

fn model_tensors(variables: &VarMap) -> candle_core::Result<BTreeMap<String, Tensor>> {
    Ok(named_variables(variables)?
        .into_iter()
        .map(|(name, variable)| (format!("model.{name}"), variable.as_tensor().clone()))
        .collect())
}

fn named_variables(variables: &VarMap) -> candle_core::Result<BTreeMap<String, Var>> {
    variables
        .data()
        .lock()
        .map(|variables| {
            variables
                .iter()
                .map(|(name, variable)| (name.clone(), variable.clone()))
                .collect()
        })
        .map_err(|error| candle_core::Error::Msg(error.to_string()))
}

fn trainable_variables(
    variables: &VarMap,
    mode: EncoderTrainingMode,
) -> candle_core::Result<BTreeMap<String, Var>> {
    Ok(named_variables(variables)?
        .into_iter()
        .filter(|(name, _)| {
            mode == EncoderTrainingMode::FineTune || name.starts_with("classifier.")
        })
        .collect())
}

fn validate_continuation(
    manifest: &BertCheckpointManifest,
    encoder: &RegisteredEncoder,
    configuration: &TransformerTrainingConfiguration,
    request: &TrainingRequest,
) -> Result<(), TrainingBackendError> {
    if manifest.base_model_id != encoder.id
        || manifest.base_model_fingerprint != encoder.fingerprint
        || manifest.tokenizer_fingerprint != format!("sha256:{}", encoder.tokenizer.checksum)
        || manifest.labels != request.labels
        || &manifest.transformer_configuration != configuration
    {
        return Err(TrainingBackendError::Configuration(
            "continuation checkpoint is incompatible with the base model, tokenizer, labels, or transformer configuration".into(),
        ));
    }
    Ok(())
}

fn validate_configuration(
    configuration: &TransformerTrainingConfiguration,
    model: &Config,
) -> Result<(), String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    if configuration.maximum_sequence_length > model.max_position_embeddings {
        return Err(format!(
            "maximum_sequence_length {} exceeds model maximum {}",
            configuration.maximum_sequence_length, model.max_position_embeddings
        ));
    }
    Ok(())
}

fn scheduled_learning_rate(base: f64, step: usize, total: usize, warmup_ratio: f64) -> f64 {
    let warmup = (total as f64 * warmup_ratio).ceil() as usize;
    if warmup > 0 && step <= warmup {
        return base * step as f64 / warmup as f64;
    }
    let decay_steps = total.saturating_sub(warmup).max(1);
    let decay_step = step.saturating_sub(warmup).min(decay_steps);
    base * (decay_steps - decay_step + 1) as f64 / decay_steps as f64
}

fn configuration_error(error: impl std::fmt::Display) -> TrainingBackendError {
    TrainingBackendError::Configuration(error.to_string())
}

fn training_error(error: impl std::fmt::Display) -> TrainingBackendError {
    TrainingBackendError::Training(error.to_string())
}

fn invalid_artifact(error: impl std::fmt::Display) -> TrainingBackendError {
    TrainingBackendError::InvalidArtifact(error.to_string())
}

#[cfg(test)]
mod tests {
    use training_core::{
        domain::{TrainingConfiguration, TrainingExample, TrainingRequest},
        ports::{PredictorLoader, TrainingBackend},
    };
    use uuid::Uuid;

    use crate::{
        BertBundle, BertPredictorLoader, BertTrainingBackend, EncoderTrainingMode,
        TransformerTrainingConfiguration, bundle::tests::write_tiny_bundle,
    };

    fn request(epochs: u32) -> TrainingRequest {
        let examples = (0..16)
            .map(|index| TrainingExample {
                snapshot_member_id: Uuid::from_u128(index + 1),
                text: if index % 2 == 0 {
                    "billing invoice payment"
                } else {
                    "account login password"
                }
                .into(),
                label: if index % 2 == 0 { "billing" } else { "account" }.into(),
            })
            .collect::<Vec<_>>();
        TrainingRequest::in_memory(
            Uuid::new_v4(),
            Uuid::new_v4(),
            vec!["billing".into(), "account".into()],
            examples.clone(),
            examples,
            TrainingConfiguration {
                feature_dimension: 16,
                epochs,
                learning_rate: 0.01,
                l2: 0.0,
                checkpoint_every: 1,
                seed: 7,
            },
        )
    }

    #[test]
    fn tiny_transformer_trains_checkpoints_and_predicts_offline() {
        let directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(directory.path());
        let encoder = BertBundle::inspect("tiny", directory.path())
            .expect("bundle")
            .into_registration();
        let configuration = TransformerTrainingConfiguration {
            maximum_sequence_length: 8,
            batch_size: 4,
            weight_decay: 0.0,
            warmup_ratio: 0.0,
            gradient_clip_norm: 1.0,
            mode: EncoderTrainingMode::FineTune,
        };
        let backend = BertTrainingBackend::new(encoder, configuration).expect("backend");
        let request = request(6);
        let total_batches = request.configuration.epochs as usize
            * request.examples.training_member_ids().len().div_ceil(4);
        let mut session = backend.start(request).expect("session");
        let first = session.train_batch().expect("first batch").training_loss;
        let mut last = first;
        for _ in 1..total_batches {
            last = session.train_batch().expect("batch").training_loss;
        }

        assert!(last < first, "loss did not decrease: {first} -> {last}");
        let bytes = session.serialize_checkpoint().expect("checkpoint");
        let predictor = BertPredictorLoader.load(&bytes).expect("predictor");
        let prediction = predictor.predict("billing invoice").expect("prediction");
        assert!(predictor.labels().contains(&prediction.label));
        assert_eq!(prediction.probabilities.len(), 2);
    }

    #[test]
    fn continuation_rejects_a_different_label_mapping() {
        let directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(directory.path());
        let encoder = BertBundle::inspect("tiny", directory.path())
            .expect("bundle")
            .into_registration();
        let configuration = TransformerTrainingConfiguration {
            maximum_sequence_length: 8,
            batch_size: 4,
            weight_decay: 0.0,
            warmup_ratio: 0.0,
            gradient_clip_norm: 1.0,
            mode: EncoderTrainingMode::Frozen,
        };
        let backend =
            BertTrainingBackend::new(encoder.clone(), configuration.clone()).expect("backend");
        let mut session = backend.start(request(1)).expect("session");
        for _ in 0..4 {
            session.train_batch().expect("batch");
        }
        let checkpoint = session.serialize_checkpoint().expect("checkpoint");
        let continued = BertTrainingBackend::continuing_from(
            encoder.clone(),
            configuration.clone(),
            checkpoint.clone(),
        )
        .expect("continuation backend");
        let mut incompatible = request(1);
        incompatible.labels.reverse();

        assert!(continued.start(incompatible).is_err());

        let changed_configuration = TransformerTrainingConfiguration {
            maximum_sequence_length: 7,
            ..configuration
        };
        let continued =
            BertTrainingBackend::continuing_from(encoder, changed_configuration, checkpoint)
                .expect("changed backend is constructed before manifest comparison");
        assert!(continued.start(request(1)).is_err());
    }

    #[test]
    fn identical_inputs_produce_identical_checkpoint_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(directory.path());
        let encoder = BertBundle::inspect("tiny", directory.path())
            .expect("bundle")
            .into_registration();
        let configuration = TransformerTrainingConfiguration {
            maximum_sequence_length: 8,
            batch_size: 4,
            weight_decay: 0.01,
            warmup_ratio: 0.25,
            gradient_clip_norm: 1.0,
            mode: EncoderTrainingMode::FineTune,
        };
        let training_request = request(1);
        let train = |backend: BertTrainingBackend| {
            let mut session = backend
                .start(training_request.clone())
                .expect("training session");
            for _ in 0..4 {
                session.train_batch().expect("batch");
            }
            session.serialize_checkpoint().expect("checkpoint")
        };

        let first = train(
            BertTrainingBackend::new(encoder.clone(), configuration.clone()).expect("backend"),
        );
        let second =
            train(BertTrainingBackend::new(encoder, configuration).expect("second backend"));

        assert_eq!(first, second);
    }

    #[test]
    #[ignore = "requires SYNTH_PUBLIC_BERT_BUNDLE pointing to a separately downloaded bundle"]
    fn public_bert_bundle_smoke_is_explicit_and_offline_at_runtime() {
        let path = std::env::var_os("SYNTH_PUBLIC_BERT_BUNDLE")
            .expect("set SYNTH_PUBLIC_BERT_BUNDLE to a supported local BERT bundle");
        let encoder = BertBundle::inspect("public-smoke", path)
            .expect("supported public bundle")
            .into_registration();
        let backend = BertTrainingBackend::new(
            encoder,
            TransformerTrainingConfiguration {
                maximum_sequence_length: 16,
                batch_size: 2,
                weight_decay: 0.0,
                warmup_ratio: 0.0,
                gradient_clip_norm: 1.0,
                mode: EncoderTrainingMode::Frozen,
            },
        )
        .expect("backend");
        let mut training_request = request(1);
        training_request.configuration.learning_rate = 0.00002;
        let mut session = backend.start(training_request).expect("session");
        session.train_batch().expect("one CPU batch");
        let checkpoint = session.serialize_checkpoint().expect("checkpoint");
        let predictor = BertPredictorLoader.load(&checkpoint).expect("predictor");
        predictor.predict("a local smoke test").expect("prediction");
    }
}
