//! Immutable model-promotion decisions after explicit final assessment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::benchmark::{AcceptanceAssessment, AcceptanceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionState {
    Promoted,
    Rejected,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPromotion {
    pub id: Uuid,
    pub workflow_run_id: Uuid,
    pub checkpoint_id: Uuid,
    pub checkpoint_fingerprint: String,
    pub training_snapshot_id: Uuid,
    pub training_snapshot_fingerprint: String,
    pub development_assessment_id: Uuid,
    pub development_assessment_fingerprint: String,
    pub sealed_assessment_id: Uuid,
    pub sealed_assessment_fingerprint: String,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub policy_fingerprint: String,
    pub state: PromotionState,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[allow(clippy::too_many_arguments)]
impl ModelPromotion {
    pub fn new(
        workflow_run_id: Uuid,
        checkpoint_id: Uuid,
        checkpoint_fingerprint: String,
        training_snapshot_id: Uuid,
        training_snapshot_fingerprint: String,
        development_assessment_id: Uuid,
        development_assessment_fingerprint: String,
        sealed_assessment: &AcceptanceAssessment,
        development_suite_id: Uuid,
        development_suite_fingerprint: String,
        sealed_suite_id: Uuid,
        sealed_suite_fingerprint: String,
        policy_fingerprint: String,
    ) -> Result<Self, PromotionError> {
        if [
            &checkpoint_fingerprint,
            &training_snapshot_fingerprint,
            &development_assessment_fingerprint,
            &development_suite_fingerprint,
            &sealed_suite_fingerprint,
            &policy_fingerprint,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || sealed_assessment.suite_id != sealed_suite_id
        {
            return Err(PromotionError::InvalidInput);
        }
        let (state, reason) = match sealed_assessment.state {
            AcceptanceState::Pass => (
                PromotionState::Promoted,
                "sealed acceptance contract passed".to_owned(),
            ),
            AcceptanceState::Fail => (
                PromotionState::Rejected,
                "sealed acceptance contract failed".to_owned(),
            ),
            AcceptanceState::Inconclusive | AcceptanceState::Invalid => (
                PromotionState::Inconclusive,
                format!(
                    "sealed acceptance contract returned {:?}",
                    sealed_assessment.state
                )
                .to_ascii_lowercase(),
            ),
        };
        let mut value = Self {
            id: Uuid::new_v4(),
            workflow_run_id,
            checkpoint_id,
            checkpoint_fingerprint,
            training_snapshot_id,
            training_snapshot_fingerprint,
            development_assessment_id,
            development_assessment_fingerprint,
            sealed_assessment_id: sealed_assessment.id,
            sealed_assessment_fingerprint: sealed_assessment.fingerprint.clone(),
            development_suite_id,
            development_suite_fingerprint,
            sealed_suite_id,
            sealed_suite_fingerprint,
            policy_fingerprint,
            state,
            reason,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, PromotionError> {
        artifact_core::fingerprint(&serde_json::json!({
            "workflow_run_id": self.workflow_run_id,
            "checkpoint_id": self.checkpoint_id,
            "checkpoint_fingerprint": self.checkpoint_fingerprint,
            "training_snapshot_id": self.training_snapshot_id,
            "training_snapshot_fingerprint": self.training_snapshot_fingerprint,
            "development_assessment_id": self.development_assessment_id,
            "development_assessment_fingerprint": self.development_assessment_fingerprint,
            "sealed_assessment_id": self.sealed_assessment_id,
            "sealed_assessment_fingerprint": self.sealed_assessment_fingerprint,
            "development_suite_id": self.development_suite_id,
            "development_suite_fingerprint": self.development_suite_fingerprint,
            "sealed_suite_id": self.sealed_suite_id,
            "sealed_suite_fingerprint": self.sealed_suite_fingerprint,
            "policy_fingerprint": self.policy_fingerprint,
            "state": self.state,
            "reason": self.reason,
        }))
        .map_err(|error| PromotionError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PromotionError {
    #[error("model promotion input is invalid or incompatible")]
    InvalidInput,
    #[error("model promotion fingerprint failed: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::benchmark::AcceptanceReason;

    fn assessment(state: AcceptanceState) -> AcceptanceAssessment {
        let mut value = AcceptanceAssessment {
            id: Uuid::new_v4(),
            suite_id: Uuid::new_v4(),
            suite_fingerprint: "sha256:suite".into(),
            checkpoint_id: Some(Uuid::new_v4()),
            evaluation_run_ids: BTreeMap::new(),
            comparison_ids: BTreeMap::new(),
            state,
            reasons: Vec::<AcceptanceReason>::new(),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint().expect("assessment");
        value
    }

    #[test]
    fn sealed_outcome_maps_to_three_immutable_promotion_states() {
        for (acceptance_state, expected) in [
            (AcceptanceState::Pass, PromotionState::Promoted),
            (AcceptanceState::Fail, PromotionState::Rejected),
            (AcceptanceState::Inconclusive, PromotionState::Inconclusive),
            (AcceptanceState::Invalid, PromotionState::Inconclusive),
        ] {
            let sealed = assessment(acceptance_state);
            let promotion = ModelPromotion::new(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "sha256:checkpoint".into(),
                Uuid::new_v4(),
                "sha256:snapshot".into(),
                Uuid::new_v4(),
                "sha256:development".into(),
                &sealed,
                Uuid::new_v4(),
                "sha256:development-suite".into(),
                sealed.suite_id,
                "sha256:sealed-suite".into(),
                "sha256:policy".into(),
            )
            .expect("promotion decision");
            assert_eq!(promotion.state, expected);
            assert_eq!(
                promotion.reproduce_fingerprint().expect("fingerprint"),
                promotion.fingerprint
            );
        }
    }
}
