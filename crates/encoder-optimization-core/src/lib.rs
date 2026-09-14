//! Bounded, evidence-linked decisions for the input-first encoder optimizer.
//! Native row admission, benchmark scoring and provider transport remain ports.

pub mod agent;
pub mod generation;
pub mod ports;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OptimizationError {
    #[error("encoder optimization rejected the operation: {0}")]
    Validation(String),
    #[error("encoder optimization adapter failed: {0}")]
    Adapter(String),
    #[error("encoder optimization budget exhausted: {0}")]
    Budget(String),
    #[error("encoder optimization stopped")]
    Stopped,
    #[error("encoder optimization artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn fingerprint(value: &impl serde::Serialize) -> Result<String, OptimizationError> {
    artifact_core::fingerprint(value)
        .map_err(|error| OptimizationError::Validation(error.to_string()))
}

pub(crate) fn require(condition: bool, message: &str) -> Result<(), OptimizationError> {
    if condition {
        Ok(())
    } else {
        Err(OptimizationError::Validation(message.into()))
    }
}

/// Content-addressed UUIDv8 for replayable optimization child artifacts.
pub fn child_id(value: &impl serde::Serialize) -> Result<uuid::Uuid, OptimizationError> {
    let hash = fingerprint(value)?;
    let parsed = uuid::Uuid::parse_str(&hash[7..39])
        .map_err(|_| OptimizationError::Validation("Invalid child fingerprint".into()))?;
    let mut bytes = *parsed.as_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid::Uuid::from_bytes(bytes))
}
