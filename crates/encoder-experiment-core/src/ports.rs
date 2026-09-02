use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::{
    BackendIdentity, ExternalProjectSnapshot, ModelArtifactIdentity, TrainingCandidate,
};
use crate::metrics::{EvaluationReport, MetricContract};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("encoder task adapter failed: {0}")]
pub struct EncoderTaskAdapterError(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterInspection {
    pub source_fingerprint: String,
    pub verified_artifact_keys: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainOutput {
    pub model: ModelArtifactIdentity,
    pub duration_seconds: u64,
    pub metadata: Value,
}

/// Replaceable compiled adapter for one production encoder task family.
///
/// The adapter may use Python, CUDA, or a project-native runtime internally. Paths, subprocess
/// types, and task-specific row shapes do not cross this boundary. Encoder Gym remains the owner
/// of finite authority, artifact identities, evaluation roles, and deterministic acceptance.
pub trait EncoderTaskBackend: Send + Sync {
    fn identity(&self) -> BackendIdentity;

    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>>;

    fn train(
        &self,
        project: ExternalProjectSnapshot,
        candidate: TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainOutput, EncoderTaskAdapterError>>;

    fn evaluate(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
        maximum_seconds: u64,
    ) -> BoxFuture<'_, Result<EvaluationReport, EncoderTaskAdapterError>>;
}
