//! Task-neutral contracts for bounded experiments against production encoder projects.
//!
//! A compiled adapter owns task-specific parsing, training, and evaluation. This crate owns
//! immutable identities, finite candidate authority, normalized metric contracts, deterministic
//! comparison, and the development/sealed evidence boundary.

pub mod domain;
pub mod metrics;
pub mod ports;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum EncoderExperimentError {
    #[error("encoder experiment validation failed: {0}")]
    Validation(String),
    #[error("encoder experiment artifact integrity failed: {0}")]
    Integrity(String),
    #[error("encoder experiment budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("encoder experiment fingerprint failed: {0}")]
    Fingerprint(String),
}

pub(crate) fn fingerprint<T: serde::Serialize>(
    value: &T,
) -> Result<String, EncoderExperimentError> {
    artifact_core::fingerprint(value)
        .map_err(|error| EncoderExperimentError::Fingerprint(error.to_string()))
}

pub(crate) fn required(
    value: impl Into<String>,
    field: &str,
) -> Result<String, EncoderExperimentError> {
    let value = value.into().trim().to_owned();
    if value.is_empty() {
        Err(EncoderExperimentError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn canonical_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
}
