//! Provider-neutral contracts for dataset qualification and curation.

pub mod assessment;
pub mod curation;
pub mod lifecycle;
pub mod policy;
pub mod population;
pub mod ports;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum QualityError {
    #[error("dataset quality validation failed: {0}")]
    Validation(String),
    #[error("dataset quality artifact integrity failed: {0}")]
    Integrity(String),
    #[error("dataset quality state transition failed: {0}")]
    InvalidTransition(String),
    #[error("dataset quality budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("dataset quality fingerprint failed: {0}")]
    Fingerprint(String),
}

pub(crate) fn required(value: impl Into<String>, field: &str) -> Result<String, QualityError> {
    let value = value.into().trim().to_owned();
    if value.is_empty() {
        Err(QualityError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

pub(crate) fn bounded_required(
    value: impl Into<String>,
    field: &str,
    maximum_characters: usize,
) -> Result<String, QualityError> {
    let value = required(value, field)?;
    if value.chars().count() <= maximum_characters {
        return Ok(value);
    }
    Ok(value.chars().take(maximum_characters).collect())
}

pub(crate) fn fingerprint<T: serde::Serialize>(value: &T) -> Result<String, QualityError> {
    artifact_core::fingerprint(value).map_err(|error| QualityError::Fingerprint(error.to_string()))
}
