//! Deterministic offline implementation of the workflow advisor transport.

use workflow_core::advisor::{
    AdvisorError, AdvisorPrompt, AdvisorTransportResult, AdvisorUsage, AdvisoryActionKind,
    AdvisoryCandidateCell, AdvisoryDraft, AdvisoryHypothesis, AnalysisAdvisor, BoxFuture,
};

#[derive(Debug, Clone, Default)]
pub struct FakeAnalysisAdvisor;

impl AnalysisAdvisor for FakeAnalysisAdvisor {
    fn backend_name(&self) -> &str {
        "fake"
    }

    fn generate(
        &self,
        prompt: AdvisorPrompt,
    ) -> BoxFuture<'_, Result<AdvisorTransportResult, AdvisorError>> {
        Box::pin(async move {
            let evidence: serde_json::Value = serde_json::from_str(&prompt.user_prompt)
                .map_err(|error| AdvisorError::InvalidRequestWithReason(error.to_string()))?;
            let finding = evidence["findings"]
                .as_array()
                .and_then(|values| values.first())
                .and_then(|value| value["key"].as_str());
            let cell = serde_json::from_value(
                evidence["allowed_cells"]
                    .as_array()
                    .and_then(|values| values.first())
                    .cloned()
                    .ok_or_else(|| AdvisorError::InvalidRequestWithReason("no cell".into()))?,
            )
            .map_err(|error| AdvisorError::InvalidRequestWithReason(error.to_string()))?;
            let draft = AdvisoryDraft {
                interpretation: "Persisted development evidence suggests one bounded follow-up experiment may be useful.".into(),
                hypotheses: finding.map_or_else(Vec::new, |finding| vec![AdvisoryHypothesis {
                    statement: "The highest-ranked development finding may identify an underrepresented decision boundary.".into(),
                    finding_keys: vec![finding.into()],
                    confidence: 0.5,
                }]),
                candidate_cells: finding.map_or_else(Vec::new, |finding| vec![AdvisoryCandidateCell {
                    cell,
                    finding_keys: vec![finding.into()],
                    rationale: "Consider this cell only through the deterministic optimization policy.".into(),
                }]),
                cautions: vec!["This is an advisory hypothesis, not causal evidence or an acceptance decision.".into()],
                recommended_action: if finding.is_some() {
                    AdvisoryActionKind::ConsiderExperiment
                } else {
                    AdvisoryActionKind::Stop
                },
            };
            Ok(AdvisorTransportResult {
                content: serde_json::to_string(&draft)
                    .map_err(|error| AdvisorError::InvalidResponse(error.to_string()))?,
                usage: AdvisorUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(50),
                    total_tokens: Some(150),
                },
                metadata: serde_json::json!({"deterministic": true}),
            })
        })
    }
}
