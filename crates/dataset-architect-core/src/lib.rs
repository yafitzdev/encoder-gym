//! Provider-neutral contracts for bounded, agent-assisted dataset architecture.

pub mod brief;
pub mod lifecycle;
pub mod proposal;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArchitectError {
    #[error("dataset architect validation failed: {0}")]
    Validation(String),
    #[error("dataset architect artifact integrity failed: {0}")]
    Integrity(String),
    #[error("dataset architect budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("dataset architect fingerprint failed: {0}")]
    Fingerprint(String),
    #[error("deterministic allocation rejected the proposal: {0}")]
    Allocation(String),
}

pub(crate) fn required(value: impl Into<String>, field: &str) -> Result<String, ArchitectError> {
    let value = value.into().trim().to_owned();
    if value.is_empty() {
        Err(ArchitectError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn fingerprint<T: serde::Serialize>(value: &T) -> Result<String, ArchitectError> {
    artifact_core::fingerprint(value)
        .map_err(|error| ArchitectError::Fingerprint(error.to_string()))
}
