//! Provider-neutral generation quality supervision contracts and pure policy.
//!
//! This crate deliberately owns no provider, SQLite, CLI, or process types.
//! It turns immutable generation and qualification facts into reproducible,
//! scoped decisions. External work belongs to an application runner.

pub mod advisor;
pub mod contract;
pub mod decision;
pub mod lifecycle;
pub mod observation;
pub mod ports;
pub mod qualification;
pub mod revision;
pub mod strategy;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SupervisorError {
    #[error("generation supervisor validation failed: {0}")]
    Validation(String),
    #[error("generation supervisor artifact integrity failed: {0}")]
    Integrity(String),
    #[error("generation supervisor state transition failed: {0}")]
    InvalidTransition(String),
    #[error("generation supervisor budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("generation supervisor fingerprint failed: {0}")]
    Fingerprint(String),
}

pub(crate) fn required(value: impl Into<String>, field: &str) -> Result<String, SupervisorError> {
    let value = value.into().trim().to_owned();
    if value.is_empty() {
        Err(SupervisorError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn fingerprint<T: serde::Serialize>(value: &T) -> Result<String, SupervisorError> {
    artifact_core::fingerprint(value)
        .map_err(|error| SupervisorError::Fingerprint(error.to_string()))
}
