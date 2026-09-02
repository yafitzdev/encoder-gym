//! Provider-neutral evidence and decision contracts for bounded production-encoder repair.
//!
//! Native adapters normalize development-only row observations. This crate owns immutable
//! observation sets and deterministic comparative diagnosis. It cannot represent sealed rows,
//! invoke a model, inspect native files, persist SQL, or start a repair action.

pub mod collection;
pub mod diagnosis;
pub mod observation;
pub mod ports;
pub mod proposal;
pub mod quality;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum EncoderRepairError {
    #[error("encoder repair validation failed: {0}")]
    Validation(String),
    #[error("encoder repair artifact integrity failed: {0}")]
    Integrity(String),
    #[error("encoder repair fingerprint failed: {0}")]
    Fingerprint(String),
    #[error("encoder repair experiment evidence failed: {0}")]
    Experiment(String),
}

pub(crate) fn fingerprint<T: serde::Serialize>(value: &T) -> Result<String, EncoderRepairError> {
    artifact_core::fingerprint(value)
        .map_err(|error| EncoderRepairError::Fingerprint(error.to_string()))
}

pub(crate) fn required(
    value: impl Into<String>,
    field: &str,
) -> Result<String, EncoderRepairError> {
    let value = value.into();
    if value.is_empty() || value.trim() != value {
        Err(EncoderRepairError::Validation(format!(
            "{field} must be non-empty and canonical"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
