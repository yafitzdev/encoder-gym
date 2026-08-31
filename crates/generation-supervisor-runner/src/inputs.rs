use generation_supervisor_core::{
    observation::QualityScope,
    revision::{DiagnosisCause, ExpectedImprovement},
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RevisionInput {
    pub cause: DiagnosisCause,
    pub summary: String,
    pub replacement_guidance: Vec<String>,
    pub expected_improvements: Vec<ExpectedImprovement>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreviewInput {
    pub replacement_guidance: Vec<String>,
    pub expected_improvements: Vec<ExpectedImprovement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FinishOutcome {
    RevisionSubmitted,
    Escalate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinishInput {
    pub outcome: FinishOutcome,
    pub summary: String,
    #[serde(default)]
    pub cause: Option<DiagnosisCause>,
}

pub(super) fn only_scope(scope: &QualityScope) -> std::collections::BTreeSet<QualityScope> {
    std::collections::BTreeSet::from([scope.clone()])
}
