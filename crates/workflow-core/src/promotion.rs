//! Immutable model-promotion decisions after explicit final assessment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::benchmark::{AcceptanceAssessment, AcceptanceState};
use crate::training_benchmark::TrainingBenchmarkCheck;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_benchmark_check_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_benchmark_check_fingerprint: Option<String>,
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
        training_benchmark_check: &TrainingBenchmarkCheck,
        development_assessment: &AcceptanceAssessment,
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
            &development_suite_fingerprint,
            &sealed_suite_fingerprint,
            &policy_fingerprint,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || workflow_run_id.is_nil()
            || checkpoint_id.is_nil()
            || training_snapshot_id.is_nil()
            || development_suite_id.is_nil()
            || sealed_suite_id.is_nil()
            || !assessment_matches(
                development_assessment,
                checkpoint_id,
                development_suite_id,
                &development_suite_fingerprint,
            )
            || !assessment_matches(
                sealed_assessment,
                checkpoint_id,
                sealed_suite_id,
                &sealed_suite_fingerprint,
            )
            || training_benchmark_check.validate_integrity().is_err()
            || !training_benchmark_check.training_allowed()
            || training_benchmark_check.training_snapshot_id != training_snapshot_id
            || training_benchmark_check.training_snapshot_fingerprint
                != training_snapshot_fingerprint
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
            training_benchmark_check_id: Some(training_benchmark_check.id),
            training_benchmark_check_fingerprint: Some(
                training_benchmark_check.fingerprint.clone(),
            ),
            development_assessment_id: development_assessment.id,
            development_assessment_fingerprint: development_assessment.fingerprint.clone(),
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
        if self.training_benchmark_check_id.is_some()
            != self.training_benchmark_check_fingerprint.is_some()
        {
            return Err(PromotionError::InvalidInput);
        }
        let mut document = serde_json::json!({
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
        });
        if let (Some(id), Some(fingerprint)) = (
            self.training_benchmark_check_id,
            self.training_benchmark_check_fingerprint.as_deref(),
        ) {
            let object = document
                .as_object_mut()
                .expect("promotion fingerprint document is an object");
            object.insert("training_benchmark_check_id".into(), serde_json::json!(id));
            object.insert(
                "training_benchmark_check_fingerprint".into(),
                serde_json::json!(fingerprint),
            );
        }
        artifact_core::fingerprint(&document)
            .map_err(|error| PromotionError::Fingerprint(error.to_string()))
    }
}

fn assessment_matches(
    assessment: &AcceptanceAssessment,
    checkpoint_id: Uuid,
    suite_id: Uuid,
    suite_fingerprint: &str,
) -> bool {
    !assessment.id.is_nil()
        && assessment.checkpoint_id == Some(checkpoint_id)
        && assessment.suite_id == suite_id
        && assessment.suite_fingerprint == suite_fingerprint
        && assessment
            .reproduce_fingerprint()
            .is_ok_and(|fingerprint| fingerprint == assessment.fingerprint)
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

    fn assessment(
        state: AcceptanceState,
        checkpoint_id: Uuid,
        suite_id: Uuid,
        suite_fingerprint: &str,
    ) -> AcceptanceAssessment {
        let mut value = AcceptanceAssessment {
            id: Uuid::new_v4(),
            suite_id,
            suite_fingerprint: suite_fingerprint.into(),
            checkpoint_id: Some(checkpoint_id),
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
            let checkpoint_id = Uuid::new_v4();
            let development_suite_id = Uuid::new_v4();
            let sealed_suite_id = Uuid::new_v4();
            let development = assessment(
                AcceptanceState::Pass,
                checkpoint_id,
                development_suite_id,
                "sha256:development-suite",
            );
            let sealed = assessment(
                acceptance_state,
                checkpoint_id,
                sealed_suite_id,
                "sha256:sealed-suite",
            );
            let check = clean_check();
            let promotion = ModelPromotion::new(
                Uuid::new_v4(),
                checkpoint_id,
                "sha256:checkpoint".into(),
                check.training_snapshot_id,
                check.training_snapshot_fingerprint.clone(),
                &check,
                &development,
                &sealed,
                development_suite_id,
                "sha256:development-suite".into(),
                sealed_suite_id,
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

    #[test]
    fn assessment_for_another_checkpoint_cannot_promote_the_selected_model() {
        let checkpoint_id = Uuid::new_v4();
        let development_suite_id = Uuid::new_v4();
        let sealed_suite_id = Uuid::new_v4();
        let development = assessment(
            AcceptanceState::Pass,
            checkpoint_id,
            development_suite_id,
            "sha256:development-suite",
        );
        let sealed = assessment(
            AcceptanceState::Pass,
            Uuid::new_v4(),
            sealed_suite_id,
            "sha256:sealed-suite",
        );
        let check = clean_check();

        assert_eq!(
            ModelPromotion::new(
                Uuid::new_v4(),
                checkpoint_id,
                "sha256:checkpoint".into(),
                check.training_snapshot_id,
                check.training_snapshot_fingerprint.clone(),
                &check,
                &development,
                &sealed,
                development_suite_id,
                "sha256:development-suite".into(),
                sealed_suite_id,
                "sha256:sealed-suite".into(),
                "sha256:policy".into(),
            ),
            Err(PromotionError::InvalidInput)
        );
    }

    fn clean_check() -> TrainingBenchmarkCheck {
        use crate::{
            contamination::ContaminationStatus,
            training_benchmark::{
                TRAINING_BENCHMARK_CHECK_PROTOCOL, TrainingCohortBinding, TrainingInputProtocol,
            },
        };
        let mut value = TrainingBenchmarkCheck {
            id: Uuid::new_v4(),
            training_snapshot_id: Uuid::new_v4(),
            training_snapshot_fingerprint: "sha256:snapshot".into(),
            check_protocol_version: TRAINING_BENCHMARK_CHECK_PROTOCOL.into(),
            training_input_protocol: TrainingInputProtocol::TrainAndValidationV1,
            training_population_fingerprint: "sha256:population".into(),
            training_member_count: 1,
            training_cohorts: vec![TrainingCohortBinding {
                split: dataset_core::domain::SnapshotSplit::Train,
                member_count: 1,
                cohort_id: Uuid::new_v4(),
                cohort_fingerprint: "sha256:cohort".into(),
                role_decision_id: Uuid::new_v4(),
                role_decision_fingerprint: "sha256:role".into(),
            }],
            benchmark_bundle_id: Uuid::new_v4(),
            benchmark_bundle_fingerprint: "sha256:bundle".into(),
            benchmark_cohort_ids: vec![Uuid::new_v4()],
            contamination_report_id: Uuid::new_v4(),
            contamination_report_fingerprint: "sha256:report".into(),
            status: ContaminationStatus::Clean,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint().expect("check fingerprint");
        value
    }

    #[test]
    fn legacy_promotion_without_training_check_keeps_its_original_document() {
        let mut value = ModelPromotion {
            id: Uuid::new_v4(),
            workflow_run_id: Uuid::new_v4(),
            checkpoint_id: Uuid::new_v4(),
            checkpoint_fingerprint: "sha256:checkpoint".into(),
            training_snapshot_id: Uuid::new_v4(),
            training_snapshot_fingerprint: "sha256:snapshot".into(),
            training_benchmark_check_id: None,
            training_benchmark_check_fingerprint: None,
            development_assessment_id: Uuid::new_v4(),
            development_assessment_fingerprint: "sha256:development".into(),
            sealed_assessment_id: Uuid::new_v4(),
            sealed_assessment_fingerprint: "sha256:sealed".into(),
            development_suite_id: Uuid::new_v4(),
            development_suite_fingerprint: "sha256:development-suite".into(),
            sealed_suite_id: Uuid::new_v4(),
            sealed_suite_fingerprint: "sha256:sealed-suite".into(),
            policy_fingerprint: "sha256:policy".into(),
            state: PromotionState::Promoted,
            reason: "sealed acceptance contract passed".into(),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        let legacy_document = serde_json::json!({
            "workflow_run_id": value.workflow_run_id,
            "checkpoint_id": value.checkpoint_id,
            "checkpoint_fingerprint": value.checkpoint_fingerprint,
            "training_snapshot_id": value.training_snapshot_id,
            "training_snapshot_fingerprint": value.training_snapshot_fingerprint,
            "development_assessment_id": value.development_assessment_id,
            "development_assessment_fingerprint": value.development_assessment_fingerprint,
            "sealed_assessment_id": value.sealed_assessment_id,
            "sealed_assessment_fingerprint": value.sealed_assessment_fingerprint,
            "development_suite_id": value.development_suite_id,
            "development_suite_fingerprint": value.development_suite_fingerprint,
            "sealed_suite_id": value.sealed_suite_id,
            "sealed_suite_fingerprint": value.sealed_suite_fingerprint,
            "policy_fingerprint": value.policy_fingerprint,
            "state": value.state,
            "reason": value.reason,
        });
        value.fingerprint = artifact_core::fingerprint(&legacy_document).expect("legacy digest");
        assert_eq!(
            value.reproduce_fingerprint().expect("reproduced digest"),
            value.fingerprint
        );
        let serialized = serde_json::to_value(&value).expect("legacy promotion JSON");
        assert!(serialized.get("training_benchmark_check_id").is_none());
        let decoded: ModelPromotion = serde_json::from_value(serialized).expect("legacy decode");
        assert_eq!(decoded, value);
    }
}
