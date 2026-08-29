use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{FindingKind, FindingRankingPolicy};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisProtocol {
    pub minimum_support: u64,
    pub finding_kinds: Vec<FindingKind>,
    pub dimension_intersections: Vec<Vec<String>>,
    pub maximum_representative_examples: usize,
    pub confidence_thresholds: Vec<f64>,
    pub ranking_policy: FindingRankingPolicy,
    pub include_correct_contrasts: bool,
    pub deterministic_seed: u64,
    pub comparison_id: Option<uuid::Uuid>,
}

impl Default for AnalysisProtocol {
    fn default() -> Self {
        Self {
            minimum_support: 1,
            finding_kinds: vec![
                FindingKind::ExpectedLabel,
                FindingKind::PredictedLabel,
                FindingKind::ConfusionPair,
                FindingKind::DimensionValue,
                FindingKind::Cell,
                FindingKind::ConfidenceBand,
                FindingKind::CorrectnessConfidence,
            ],
            dimension_intersections: Vec::new(),
            maximum_representative_examples: 5,
            confidence_thresholds: vec![0.5, 0.8],
            ranking_policy: FindingRankingPolicy::ErrorRate,
            include_correct_contrasts: true,
            deterministic_seed: 42,
            comparison_id: None,
        }
    }
}

impl AnalysisProtocol {
    pub fn validate(&self) -> Result<(), AnalysisProtocolError> {
        if self.minimum_support == 0 {
            return Err(AnalysisProtocolError::MinimumSupport);
        }
        if self.finding_kinds.is_empty()
            || self
                .finding_kinds
                .windows(2)
                .any(|values| values[0] >= values[1])
        {
            return Err(AnalysisProtocolError::FindingKinds);
        }
        if !(1..=1_000).contains(&self.maximum_representative_examples) {
            return Err(AnalysisProtocolError::RepresentativeLimit);
        }
        if self
            .confidence_thresholds
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value >= 1.0)
            || self
                .confidence_thresholds
                .windows(2)
                .any(|values| values[0] >= values[1])
        {
            return Err(AnalysisProtocolError::ConfidenceThresholds);
        }
        if self.dimension_intersections.iter().any(|intersection| {
            intersection.is_empty()
                || intersection.iter().any(|name| name.trim().is_empty())
                || intersection.windows(2).any(|names| names[0] >= names[1])
        }) || self
            .dimension_intersections
            .windows(2)
            .any(|intersections| intersections[0] >= intersections[1])
        {
            return Err(AnalysisProtocolError::DimensionIntersections);
        }
        if self.comparison_id.is_some() && self.finding_kinds.is_empty() {
            return Err(AnalysisProtocolError::FindingKinds);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        artifact_core::fingerprint(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AnalysisProtocolError {
    #[error("minimum support must be greater than zero")]
    MinimumSupport,
    #[error("finding kinds must be unique and in canonical order")]
    FindingKinds,
    #[error("maximum representative examples must be between 1 and 1,000")]
    RepresentativeLimit,
    #[error("confidence thresholds must be finite, unique, strictly increasing, and inside (0, 1)")]
    ConfidenceThresholds,
    #[error("dimension intersections must contain canonical non-empty dimension names")]
    DimensionIntersections,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::domain::{FindingIdentity, FindingKind};

    use super::{AnalysisProtocol, AnalysisProtocolError};

    #[test]
    fn validates_and_fingerprints_normalized_protocols() {
        let protocol = AnalysisProtocol::default();
        protocol.validate().expect("default protocol");
        assert_eq!(
            protocol.fingerprint().expect("fingerprint"),
            protocol.fingerprint().expect("repeat fingerprint")
        );

        let invalid = AnalysisProtocol {
            confidence_thresholds: vec![0.8, 0.5],
            ..protocol
        };
        assert_eq!(
            invalid.validate(),
            Err(AnalysisProtocolError::ConfidenceThresholds)
        );
    }

    #[test]
    fn canonical_finding_keys_cannot_collide_on_delimiters() {
        let left = FindingIdentity {
            kind: FindingKind::Cell,
            attributes: BTreeMap::from([("a".into(), "b/c=d".into())]),
        };
        let right = FindingIdentity {
            kind: FindingKind::Cell,
            attributes: BTreeMap::from([("a".into(), "b".into()), ("c".into(), "d".into())]),
        };
        assert_ne!(left.key(), right.key());
    }
}
