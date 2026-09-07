//! Produces a checksum-valid checkpoint whose finite coefficients overflow at inference.
//! The normal training runner and artifact store publish it; no historical facts are edited.
use std::{path::Path, sync::Arc};

use dataset_core::{domain::SnapshotSplit, ports::SnapshotStore};
use generation_core::ports::DatasetStore;
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{BatchMetrics, TrainingConfiguration, TrainingExample, TrainingRequest, TrainingRun},
    ports::{TrainingBackend, TrainingBackendError, TrainingSession, TrainingStore},
    runner::TrainingRunner,
};
use training_linear::{HashingLinearBackend, LocalCheckpointStore};
use uuid::Uuid;

pub async fn checkpoint(database_url: &str, snapshot_id: Uuid, directory: &Path) -> Uuid {
    let store = SqliteStore::connect(database_url).await.unwrap();
    let snapshot = store.get_snapshot(snapshot_id).await.unwrap().unwrap();
    let dataset = store
        .get_dataset(snapshot.source_dataset_id)
        .await
        .unwrap()
        .unwrap();
    let members = store.list_snapshot_members(snapshot_id).await.unwrap();
    let config = TrainingConfiguration {
        epochs: 1,
        feature_dimension: 16,
        ..Default::default()
    };
    let backend = OverflowingPredictorArtifact;
    let run = TrainingRun::queued(
        snapshot_id,
        backend.name(),
        backend.model_format(),
        config.clone(),
    )
    .unwrap();
    store.create_training_run(&run).await.unwrap();
    let request = TrainingRequest::in_memory(
        run.id,
        snapshot_id,
        dataset.labels,
        members
            .into_iter()
            .filter(|row| row.split == SnapshotSplit::Train)
            .map(|row| TrainingExample {
                snapshot_member_id: row.id,
                text: row.text,
                label: row.label,
            })
            .collect(),
        vec![],
        config,
    );
    TrainingRunner::new(
        Arc::new(store.clone()),
        Arc::new(backend),
        Arc::new(LocalCheckpointStore::new(directory.join("artifacts"))),
    )
    .run(run.id, request)
    .await
    .unwrap();
    store.list_checkpoints(run.id).await.unwrap()[0].id
}

struct OverflowingPredictorArtifact;

impl TrainingBackend for OverflowingPredictorArtifact {
    fn name(&self) -> &str {
        "overflow-fixture"
    }
    fn model_format(&self) -> &str {
        "hashing-linear-v1"
    }
    fn start(
        &self,
        request: TrainingRequest,
    ) -> Result<Box<dyn TrainingSession>, TrainingBackendError> {
        Ok(Box::new(OverflowingSession(
            HashingLinearBackend.start(request)?,
        )))
    }
}

struct OverflowingSession(Box<dyn TrainingSession>);

impl TrainingSession for OverflowingSession {
    fn train_batch(&mut self) -> Result<BatchMetrics, TrainingBackendError> {
        self.0.train_batch()
    }
    fn serialize_checkpoint(&self) -> Result<Vec<u8>, TrainingBackendError> {
        let mut model: serde_json::Value =
            serde_json::from_slice(&self.0.serialize_checkpoint()?).unwrap();
        model["weights"] = serde_json::json!([vec![f64::MAX; 16], vec![f64::MAX; 16]]);
        model["biases"] = serde_json::json!([f64::MAX, f64::MAX]);
        Ok(serde_json::to_vec(&model).unwrap())
    }
}
