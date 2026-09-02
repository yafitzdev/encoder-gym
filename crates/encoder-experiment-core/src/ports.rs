use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::{
    BackendIdentity, ExternalProjectSnapshot, ModelArtifactIdentity, TrainingCandidate,
};
use crate::metrics::{EvaluationReport, MetricContract};
use crate::{journal::ExperimentEvent, protocol::ExperimentProtocol};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("encoder task adapter failed: {0}")]
pub struct EncoderTaskAdapterError(pub String);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("encoder experiment store failed: {0}")]
pub struct ExperimentStoreError(pub String);

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

/// Append-only persistence for immutable experiment artifacts and event journals.
pub trait ExperimentStore: Send + Sync {
    fn create_project(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>>;

    fn get_project(
        &self,
        id: uuid::Uuid,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>>;

    fn find_project_by_fingerprint(
        &self,
        fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>>;

    fn find_project_by_source_fingerprint(
        &self,
        source_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ExternalProjectSnapshot>, ExperimentStoreError>>;

    fn create_protocol(
        &self,
        protocol: ExperimentProtocol,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>>;

    fn get_protocol(
        &self,
        id: uuid::Uuid,
    ) -> BoxFuture<'_, Result<Option<ExperimentProtocol>, ExperimentStoreError>>;

    fn create_run(
        &self,
        first_event: ExperimentEvent,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>>;

    fn append_event(
        &self,
        event: ExperimentEvent,
    ) -> BoxFuture<'_, Result<(), ExperimentStoreError>>;

    fn load_events(
        &self,
        run_id: uuid::Uuid,
    ) -> BoxFuture<'_, Result<Vec<ExperimentEvent>, ExperimentStoreError>>;
}
