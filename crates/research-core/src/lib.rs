//! Evidence-backed authenticity research contracts.
//!
//! This crate owns research policy and immutable artifacts. It deliberately
//! knows nothing about Pi, SQLite, HTTP clients, CLI parsing, or prompts.

pub mod brief;
pub mod evidence;
pub mod lifecycle;
pub mod ports;
pub mod profile;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResearchError {
    #[error("research validation failed: {0}")]
    Validation(String),
    #[error("research integrity check failed: {0}")]
    Integrity(String),
    #[error("research transition is not allowed: {0}")]
    InvalidTransition(String),
    #[error("research budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("research fingerprinting failed: {0}")]
    Fingerprint(String),
}

pub(crate) fn nonempty(value: String, path: &str) -> Result<String, ResearchError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ResearchError::Validation(format!(
            "{path} must not be empty"
        )));
    }
    Ok(value)
}

pub(crate) fn optional(value: Option<String>, path: &str) -> Result<Option<String>, ResearchError> {
    value.map(|value| nonempty(value, path)).transpose()
}

pub(crate) fn normalized_list(
    values: Vec<String>,
    path: &str,
) -> Result<Vec<String>, ResearchError> {
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = nonempty(value, path)?;
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}
