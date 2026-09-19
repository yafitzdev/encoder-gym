//! Nomos projection into the provider-neutral native semantic-assessment
//! contract. Expected labels remain in a separate host-private authority and
//! never enter either reviewer request.

use std::collections::BTreeMap;

use dataset_quality_core::native_assessment::{
    NativeBlindContext, NativeBlindRow, NativeCandidateSemantics, NativeLabelAuthority,
    NativeMetricDirection, NativeRepairStrategy, NativeTargetBrief,
};
use encoder_optimization_core::{
    OptimizationError, fingerprint,
    repair_strategy::{MetricDirection, RepairOperation, RepairTarget},
};
use serde_json::Value;

use crate::managed_training::validate_native_training_row;

pub const NOMOS_LABEL_POLICY_VERSION: &str = "nomos-compatible-answer-set-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NomosNativeAssessmentBinding {
    pub blind_row: NativeBlindRow,
    pub label_authority: NativeLabelAuthority,
}

impl NomosNativeAssessmentBinding {
    pub fn from_generated(
        row_id: impl Into<String>,
        row: &Value,
    ) -> Result<Self, OptimizationError> {
        validate_native_training_row(row).map_err(native_error)?;
        let row_id = row_id.into();
        let legal = strings(row.get("legal_candidate_ids"), "legal_candidate_ids")?;
        let tools = row["tool_registry"]["tools"]
            .as_array()
            .ok_or_else(|| invalid("Nomos tool registry is missing tools"))?;
        let mut candidates = Vec::with_capacity(legal.len());
        for candidate_id in &legal {
            let tool = tools
                .iter()
                .find(|tool| tool["tool_id"].as_str() == Some(candidate_id))
                .ok_or_else(|| invalid("Nomos legal candidate is absent from the tool registry"))?;
            let description = tool["description"]
                .as_str()
                .ok_or_else(|| invalid("Nomos candidate description is missing"))?;
            let capabilities = strings(tool.get("capabilities"), "tool capabilities")?;
            candidates.push(
                NativeCandidateSemantics::new(candidate_id, description, capabilities)
                    .map_err(native_error)?,
            );
        }
        candidates.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
        let context = NativeBlindContext::new(BTreeMap::from([
            (
                "taskKind".into(),
                row.get("task_kind")
                    .cloned()
                    .ok_or_else(|| invalid("Nomos task kind is missing"))?,
            ),
            (
                "previousCandidateIds".into(),
                row.get("previous_candidate_ids")
                    .cloned()
                    .ok_or_else(|| invalid("Nomos previous candidate history is missing"))?,
            ),
        ]))
        .map_err(native_error)?;
        let blind_row = NativeBlindRow {
            row_id,
            row_fingerprint: fingerprint(row)?,
            question: row["question"]
                .as_str()
                .ok_or_else(|| invalid("Nomos generated question is missing"))?
                .into(),
            context,
            candidates,
        };
        blind_row.validate().map_err(native_error)?;
        let acceptable = strings(
            row["label"].get("acceptable_tools"),
            "label.acceptable_tools",
        )?;
        let label_authority =
            NativeLabelAuthority::new(&blind_row, acceptable, NOMOS_LABEL_POLICY_VERSION)
                .map_err(native_error)?;
        Ok(Self {
            blind_row,
            label_authority,
        })
    }
}

pub fn target_brief(target: &RepairTarget) -> Result<NativeTargetBrief, OptimizationError> {
    let strategy = match &target.operation {
        RepairOperation::LabelPreservingVariants { .. } => {
            NativeRepairStrategy::LabelPreservingVariants
        }
        RepairOperation::ExistingAnchorContrast { .. } => {
            NativeRepairStrategy::ExistingAnchorContrast
        }
        RepairOperation::ProvenRedundantRowRemoval { .. } => {
            return Err(invalid(
                "A redundant-row removal has no generated row to assess",
            ));
        }
    };
    NativeTargetBrief::new(
        strategy,
        target.cluster_keys.clone(),
        target.target_metric.name.clone(),
        match target.target_metric.direction {
            MetricDirection::Increase => NativeMetricDirection::Increase,
            MetricDirection::Decrease => NativeMetricDirection::Decrease,
        },
    )
    .map_err(native_error)
}

fn strings(value: Option<&Value>, field: &str) -> Result<Vec<String>, OptimizationError> {
    let mut result = value
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("Nomos {field} must be an array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("Nomos {field} contains invalid text")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    result.sort();
    result.dedup();
    if result.is_empty() {
        return Err(invalid(format!("Nomos {field} is empty")));
    }
    Ok(result)
}

fn invalid(message: impl Into<String>) -> OptimizationError {
    OptimizationError::Validation(message.into())
}

fn native_error(error: impl std::fmt::Display) -> OptimizationError {
    invalid(error.to_string())
}

#[cfg(test)]
mod tests {
    use dataset_quality_core::{
        native_assessment::{
            NativeAdmissionDecision, NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest,
            NativeTargetFitEvidence, NativeTargetFitRequest, decide_native_admission,
        },
        ports::NativeSemanticReviewer,
    };
    use dataset_quality_fake::FakeQualityEvaluator;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn tool(id: &str, description: &str, capability: &str) -> Value {
        json!({
            "tool_id": id,
            "tool_family": capability,
            "description": description,
            "capabilities": [capability],
            "input_modalities": ["text"],
            "output_modalities": ["text"],
            "evidence_roles": ["primary"],
            "side_effect_class": "none",
            "argument_schema": {},
        })
    }

    fn generated(question: &str) -> Value {
        json!({
            "schema_version":"decision-state.v2",
            "decision_state_id":"generated",
            "question": question,
            "evaluation_partition":"train",
            "accepted":true,
            "task_kind":"route",
            "previous_candidate_ids":[],
            "legal_candidate_ids":["read","write"],
            "label":{"acceptable_tools":["read"],"hard_negative_tools":["write"]},
            "tool_registry":{
                "registry_id":"r",
                "registry_fingerprint":"sha256:registry",
                "tools":[
                    tool("read", "Find saved invoices", "find"),
                    tool("write", "Modify saved invoices", "modify")
                ]
            }
        })
    }

    #[tokio::test]
    async fn schema_valid_wrong_label_question_is_excluded_by_blind_native_review() {
        let binding = NomosNativeAssessmentBinding::from_generated(
            "generation-task:0",
            &generated("Modify saved invoices"),
        )
        .unwrap();
        let reviewer = FakeQualityEvaluator::default();
        let reviewer_id = reviewer.native_identity();
        let blind_request = NativeBlindAssessmentRequest::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            reviewer_id.fingerprint.clone(),
            vec![binding.blind_row.clone()],
        )
        .unwrap();
        let blind_output = reviewer.assess_blind(blind_request.clone()).await.unwrap();
        let blind = NativeBlindAssessmentEvidence::record(
            &blind_request,
            blind_output.assessments[0].clone(),
        )
        .unwrap();
        assert_eq!(blind.draft.supported_candidate_ids, vec!["write"]);

        let target = NativeTargetBrief::new(
            NativeRepairStrategy::LabelPreservingVariants,
            vec!["nomos-cluster-read".into()],
            "recall_at_1",
            NativeMetricDirection::Increase,
        )
        .unwrap();
        let fit_request = NativeTargetFitRequest::new(
            Uuid::new_v4(),
            &blind_request,
            reviewer_id.fingerprint,
            vec![(blind.clone(), target)],
        )
        .unwrap();
        let fit_output = reviewer
            .assess_target_fit(fit_request.clone())
            .await
            .unwrap();
        let fit = NativeTargetFitEvidence::record(&fit_request, fit_output.assessments[0].clone())
            .unwrap();
        let decision =
            decide_native_admission(&binding.label_authority, Some(&blind), Some(&fit)).unwrap();
        assert_eq!(decision.decision, NativeAdmissionDecision::LabelMismatch);
        assert!(!decision.admitted());

        let blind_json = serde_json::to_string(&blind_request).unwrap();
        assert!(!blind_json.contains("acceptable_tools"));
        assert!(!blind_json.contains("hard_negative_tools"));
        assert!(!blind_json.contains("expectedCandidate"));
    }
}
