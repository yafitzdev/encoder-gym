//! Persisted classification evaluation and exact metric calculation.

pub mod comparison;
pub mod domain;
pub mod metrics;
pub mod ports;
pub mod runner;
pub mod validation;

use serde::Serialize;

use crate::domain::{EvaluationExample, EvaluationProtocol, EvaluationSourceIdentity};

#[derive(Serialize)]
struct EvaluationInput<'a> {
    checkpoint_checksum: &'a str,
    labels: &'a [String],
    examples: Vec<&'a EvaluationExample>,
}

pub fn input_fingerprint(
    checkpoint_checksum: &str,
    labels: &[String],
    examples: &[EvaluationExample],
) -> Result<String, artifact_core::FingerprintError> {
    let mut examples = examples.iter().collect::<Vec<_>>();
    examples.sort_by_key(|example| example.snapshot_member_id);
    artifact_core::fingerprint(&EvaluationInput {
        checkpoint_checksum,
        labels,
        examples,
    })
}

#[derive(Serialize)]
struct EvaluationRunInput<'a> {
    protocol: &'a EvaluationProtocol,
    source: &'a EvaluationSourceIdentity,
}

pub fn run_input_fingerprint(
    protocol: &EvaluationProtocol,
    source: &EvaluationSourceIdentity,
) -> Result<String, artifact_core::FingerprintError> {
    artifact_core::fingerprint(&EvaluationRunInput { protocol, source })
}
