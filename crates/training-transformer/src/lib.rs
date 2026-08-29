//! CPU BERT sequence-classification adapter for the project-owned training ports.

mod backend;
mod batching;
mod bundle;
mod checkpoint;
mod model;
mod optimizer;
mod tokenization;

#[cfg(feature = "test-fixtures")]
pub mod fixture;

pub use backend::{BertCheckpointMetadata, BertPredictorLoader, BertTrainingBackend};
pub use batching::BatchPlan;
pub use bundle::{BertBundle, BundleError};
pub use model::BertClassifier;
pub use training_core::domain::{EncoderTrainingMode, TransformerTrainingConfiguration};
