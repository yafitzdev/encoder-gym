use std::{future::Future, pin::Pin};

use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    BatchMetrics, Prediction, RegisteredEncoder, StoredArtifact, TrainingCheckpoint,
    TrainingRequest, TrainingRun, TrainingRunState,
};

#[derive(Debug, Clone, Copy)]
pub struct TrainingRunQuery {
    pub snapshot_id: Option<Uuid>,
    pub input_authority_id: Option<Uuid>,
    pub state: Option<TrainingRunState>,
    pub limit: u32,
    pub offset: u32,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrainingBackendError {
    #[error("training backend configuration is invalid: {0}")]
    Configuration(String),
    #[error("training backend failed: {0}")]
    Training(String),
    #[error("model artifact is invalid: {0}")]
    InvalidArtifact(String),
}

pub trait TrainingSession: Send {
    /// Trains one bounded batch and reports durable cumulative progress.
    fn train_batch(&mut self) -> Result<BatchMetrics, TrainingBackendError>;
    fn serialize_checkpoint(&self) -> Result<Vec<u8>, TrainingBackendError>;
}

pub trait TrainingBackend: Send + Sync {
    fn name(&self) -> &str;
    fn model_format(&self) -> &str;
    fn start(
        &self,
        request: TrainingRequest,
    ) -> Result<Box<dyn TrainingSession>, TrainingBackendError>;
}

pub trait Predictor: Send + Sync {
    fn labels(&self) -> &[String];
    fn predict(&self, text: &str) -> Result<Prediction, TrainingBackendError>;
    fn predict_batch(&self, texts: &[String]) -> Result<Vec<Prediction>, TrainingBackendError> {
        texts.iter().map(|text| self.predict(text)).collect()
    }
}

pub trait PredictorLoader: Send + Sync {
    fn model_format(&self) -> &str;
    fn load(&self, artifact: &[u8]) -> Result<Box<dyn Predictor>, TrainingBackendError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("training persistence operation failed: {0}")]
pub struct TrainingStoreError(pub String);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("encoder registry persistence failed: {0}")]
pub struct EncoderRegistryError(pub String);

pub trait EncoderRegistry: Send + Sync {
    fn register_encoder(
        &self,
        encoder: &RegisteredEncoder,
    ) -> BoxFuture<'_, Result<(), EncoderRegistryError>>;
    fn get_encoder(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RegisteredEncoder>, EncoderRegistryError>>;
    fn list_encoders(&self) -> BoxFuture<'_, Result<Vec<RegisteredEncoder>, EncoderRegistryError>>;
}

pub trait TrainingStore: Send + Sync {
    fn create_training_run(
        &self,
        run: &TrainingRun,
    ) -> BoxFuture<'_, Result<(), TrainingStoreError>>;
    fn save_training_run(&self, run: &TrainingRun)
    -> BoxFuture<'_, Result<(), TrainingStoreError>>;
    fn get_training_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingRun>, TrainingStoreError>>;
    fn list_training_runs(&self) -> BoxFuture<'_, Result<Vec<TrainingRun>, TrainingStoreError>>;
    fn query_training_runs(
        &self,
        query: TrainingRunQuery,
    ) -> BoxFuture<'_, Result<Vec<TrainingRun>, TrainingStoreError>>;
    fn request_training_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, TrainingStoreError>>;
    fn create_checkpoint(
        &self,
        checkpoint: &TrainingCheckpoint,
    ) -> BoxFuture<'_, Result<(), TrainingStoreError>>;
    fn get_checkpoint(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingCheckpoint>, TrainingStoreError>>;
    fn list_checkpoints(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<TrainingCheckpoint>, TrainingStoreError>>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("checkpoint artifact operation failed: {0}")]
pub struct ArtifactError(pub String);

pub trait CheckpointSink: Send + Sync {
    fn write(
        &self,
        run_id: Uuid,
        epoch: u32,
        model_format: &str,
        artifact: &[u8],
    ) -> Result<StoredArtifact, ArtifactError>;

    fn read(&self, path: &str) -> Result<Vec<u8>, ArtifactError>;
}
