//! Provider-neutral contracts for bounded, evidence-backed benchmark design.

pub mod blueprint;
pub mod brief;
pub mod lifecycle;
pub mod ports;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchmarkArchitectError {
    #[error("benchmark architect validation failed: {0}")]
    Validation(String),
    #[error("benchmark architect artifact integrity failed: {0}")]
    Integrity(String),
    #[error("benchmark architect budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("benchmark architect fingerprint failed: {0}")]
    Fingerprint(String),
}

pub(crate) fn required(
    value: impl Into<String>,
    field: &str,
) -> Result<String, BenchmarkArchitectError> {
    let value = value.into().trim().to_owned();
    if value.is_empty() {
        Err(BenchmarkArchitectError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn fingerprint<T: serde::Serialize>(
    value: &T,
) -> Result<String, BenchmarkArchitectError> {
    artifact_core::fingerprint(value)
        .map_err(|error| BenchmarkArchitectError::Fingerprint(error.to_string()))
}

pub(crate) fn canonical_fingerprint(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
}
