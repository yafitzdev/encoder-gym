//! Deterministic local hashing encoder, linear classifier, and file artifacts.

mod artifacts;
mod model;

pub use artifacts::{LocalCheckpointStore, verify_checksum, verify_file};
pub use model::{HashingLinearBackend, HashingLinearPredictorLoader};
