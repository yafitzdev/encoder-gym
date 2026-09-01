use std::collections::{BTreeMap, BTreeSet};

use benchmark_architect_core::blueprint::BenchmarkBlueprintDraft;
use research_core::evidence::{EvidenceConfidence, ResearchEvidenceDraft};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SearchInput {
    pub query: String,
    #[serde(default)]
    pub source_classes: Vec<String>,
    pub maximum_results: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FetchInput {
    pub url: String,
    pub maximum_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EvidenceInput {
    pub key: String,
    url: String,
    title: String,
    query: String,
    source_class: String,
    content_hash: String,
    excerpt: String,
    #[serde(default)]
    location: Option<String>,
    observation: String,
    applicability: String,
    confidence: EvidenceConfidence,
}

impl EvidenceInput {
    pub fn into_draft(self) -> ResearchEvidenceDraft {
        ResearchEvidenceDraft {
            url: self.url,
            title: self.title,
            query: self.query,
            source_class: self.source_class,
            content_hash: self.content_hash,
            excerpt: self.excerpt,
            location: self.location,
            observation: self.observation,
            applicability: self.applicability,
            confidence: self.confidence,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EmptyInput {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BlueprintInput {
    pub blueprint: BenchmarkBlueprintDraft,
    #[serde(default)]
    pub evidence_bindings: Vec<RiskEvidenceBinding>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RiskEvidenceBinding {
    pub requirement_key: String,
    #[serde(default)]
    pub supporting_evidence_keys: Vec<String>,
    #[serde(default)]
    pub conflicting_evidence_keys: Vec<String>,
}

impl BlueprintInput {
    pub fn resolve(
        mut self,
        evidence_keys: &BTreeMap<String, uuid::Uuid>,
    ) -> Result<BenchmarkBlueprintDraft, crate::RunnerError> {
        if self.blueprint.risk_coverage.iter().any(|requirement| {
            !requirement.supporting_evidence_ids.is_empty()
                || !requirement.conflicting_evidence_ids.is_empty()
        }) {
            return Err(crate::RunnerError::Validation(
                "tool input must bind run evidence by logical key, not inject evidence IDs".into(),
            ));
        }
        let mut bindings = BTreeMap::new();
        for binding in self.evidence_bindings {
            if binding.requirement_key.trim().is_empty()
                || bindings
                    .insert(binding.requirement_key.clone(), binding)
                    .is_some()
            {
                return Err(crate::RunnerError::Validation(
                    "evidence binding requirement keys must be nonempty and unique".into(),
                ));
            }
        }
        let requirement_keys = self
            .blueprint
            .risk_coverage
            .iter()
            .map(|requirement| requirement.key.as_str())
            .collect::<BTreeSet<_>>();
        if bindings
            .keys()
            .any(|key| !requirement_keys.contains(key.as_str()))
        {
            return Err(crate::RunnerError::Validation(
                "evidence binding names an unknown risk requirement".into(),
            ));
        }
        for requirement in &mut self.blueprint.risk_coverage {
            let Some(binding) = bindings.remove(&requirement.key) else {
                continue;
            };
            let resolve = |keys: Vec<String>| {
                keys.into_iter()
                    .map(|key| {
                        evidence_keys.get(&key).copied().ok_or_else(|| {
                            crate::RunnerError::Validation(format!(
                                "risk requirement references missing evidence key {key:?}"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            };
            requirement.supporting_evidence_ids = resolve(binding.supporting_evidence_keys)?;
            requirement.conflicting_evidence_ids = resolve(binding.conflicting_evidence_keys)?;
        }
        Ok(self.blueprint)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FinishReason {
    ProposalSubmitted,
    BudgetExhausted,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FinishInput {
    pub reason: FinishReason,
    pub summary: String,
}
