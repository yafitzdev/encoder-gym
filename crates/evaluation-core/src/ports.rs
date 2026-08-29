use std::{future::Future, pin::Pin};

use std::collections::BTreeMap;

use dataset_core::domain::SnapshotSplit;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    EvaluationComparisonReport, EvaluationExample, EvaluationPrediction, EvaluationRun,
    EvaluationRunState, ModelSelectionReport,
};

#[derive(Debug, Clone, Copy)]
pub struct EvaluationRunQuery {
    pub checkpoint_id: Option<Uuid>,
    pub snapshot_id: Option<Uuid>,
    pub state: Option<EvaluationRunState>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PredictionQuery {
    pub run_id: Uuid,
    pub correct: Option<bool>,
    pub expected_label: Option<String>,
    pub predicted_label: Option<String>,
    pub minimum_confidence: Option<f64>,
    pub maximum_confidence: Option<f64>,
    pub dimensions: BTreeMap<String, String>,
    pub snapshot_member_id: Option<Uuid>,
    pub source_row_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

impl PredictionQuery {
    pub fn page(run_id: Uuid, limit: u32, offset: u32) -> Self {
        Self {
            run_id,
            correct: None,
            expected_label: None,
            predicted_label: None,
            minimum_confidence: None,
            maximum_confidence: None,
            dimensions: BTreeMap::new(),
            snapshot_member_id: None,
            source_row_id: None,
            limit,
            offset,
        }
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("evaluation persistence operation failed: {0}")]
pub struct EvaluationStoreError(pub String);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("evaluation example source failed: {0}")]
pub struct EvaluationExampleSourceError(pub String);

pub trait EvaluationExampleSource: Send + Sync {
    fn count_examples(
        &self,
        snapshot_id: Uuid,
        split: SnapshotSplit,
    ) -> BoxFuture<'_, Result<u64, EvaluationExampleSourceError>>;

    fn query_examples(
        &self,
        snapshot_id: Uuid,
        split: SnapshotSplit,
        limit: u32,
        offset: u64,
    ) -> BoxFuture<'_, Result<Vec<EvaluationExample>, EvaluationExampleSourceError>>;
}

pub trait EvaluationStore: Send + Sync {
    fn create_evaluation_run(
        &self,
        run: &EvaluationRun,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;

    fn save_evaluation_run(
        &self,
        run: &EvaluationRun,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;

    fn get_evaluation_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationRun>, EvaluationStoreError>>;

    fn list_evaluation_runs(
        &self,
    ) -> BoxFuture<'_, Result<Vec<EvaluationRun>, EvaluationStoreError>>;

    fn query_evaluation_runs(
        &self,
        query: EvaluationRunQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationRun>, EvaluationStoreError>>;

    fn insert_predictions(
        &self,
        predictions: &[EvaluationPrediction],
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;

    /// Atomically persists one prediction batch together with its durable run progress.
    fn commit_evaluation_batch(
        &self,
        run: &EvaluationRun,
        predictions: &[EvaluationPrediction],
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;

    fn request_evaluation_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, EvaluationStoreError>>;

    fn list_predictions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, EvaluationStoreError>>;

    fn query_predictions(
        &self,
        query: PredictionQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, EvaluationStoreError>>;

    fn count_predictions(&self, run_id: Uuid) -> BoxFuture<'_, Result<u64, EvaluationStoreError>>;

    fn create_comparison(
        &self,
        report: &EvaluationComparisonReport,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;
    fn get_comparison(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationComparisonReport>, EvaluationStoreError>>;
    fn list_comparisons(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<EvaluationComparisonReport>, EvaluationStoreError>>;
    fn create_selection(
        &self,
        report: &ModelSelectionReport,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>>;
    fn get_selection(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelSelectionReport>, EvaluationStoreError>>;
    fn list_selections(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<ModelSelectionReport>, EvaluationStoreError>>;
}
