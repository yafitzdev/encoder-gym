use std::collections::BTreeMap;

use research_core::{
    evidence::{ResearchClaim, ResearchClaimDraft, ResearchEvidence, ResearchEvidenceDraft},
    profile::{AuthenticityProfileDraft, AuthenticitySection},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::RunnerError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EvidenceInput {
    pub(crate) key: String,
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
    confidence: research_core::evidence::EvidenceConfidence,
}

impl EvidenceInput {
    pub(crate) fn into_draft(self) -> ResearchEvidenceDraft {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchInput {
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) source_classes: Vec<String>,
    pub(crate) maximum_results: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FetchInput {
    pub(crate) url: String,
    pub(crate) maximum_bytes: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InspectEvidenceInput {
    #[serde(default, rename = "sourceClass")]
    _source_class: Option<String>,
    #[serde(default, rename = "confidence")]
    _confidence: Option<research_core::evidence::EvidenceConfidence>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProfileSubmission {
    pub(crate) claims: Vec<ClaimSubmission>,
    pub(crate) profile: ProfileDraftSubmission,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaimSubmission {
    key: String,
    statement: String,
    confidence: research_core::evidence::EvidenceConfidence,
    #[serde(default)]
    supporting_evidence_keys: Vec<String>,
    #[serde(default)]
    conflicting_evidence_keys: Vec<String>,
    #[serde(default)]
    inference: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProfileDraftSubmission {
    schema_version: u32,
    #[serde(default)]
    predecessor_id: Option<Uuid>,
    summary: String,
    sections: BTreeMap<String, SectionSubmission>,
    #[serde(default)]
    generation_instructions: Vec<String>,
    #[serde(default)]
    caveats: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SectionSubmission {
    #[serde(default)]
    observations: Vec<String>,
    #[serde(default)]
    generation_instructions: Vec<String>,
    claim_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinishReason {
    SufficientEvidence,
    BudgetExhausted,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FinishInput {
    pub(crate) reason: FinishReason,
    #[serde(default, rename = "summary")]
    _summary: Option<String>,
}

pub(crate) fn validate_profile_keys(
    submission: &ProfileSubmission,
    evidence_keys: &BTreeMap<String, Uuid>,
) -> Result<(), RunnerError> {
    if submission.claims.is_empty() {
        return Err(RunnerError::Validation(
            "profile draft requires claims".into(),
        ));
    }
    let mut claims = BTreeMap::new();
    for claim in &submission.claims {
        if claim.key.trim().is_empty() || claims.insert(claim.key.as_str(), ()).is_some() {
            return Err(RunnerError::Validation(
                "claim keys must be nonempty and unique".into(),
            ));
        }
        for key in claim
            .supporting_evidence_keys
            .iter()
            .chain(&claim.conflicting_evidence_keys)
        {
            if !evidence_keys.contains_key(key) {
                return Err(RunnerError::Validation(format!(
                    "claim references missing evidence key {key:?}"
                )));
            }
        }
    }
    for section in submission.profile.sections.values() {
        for key in &section.claim_keys {
            if !claims.contains_key(key.as_str()) {
                return Err(RunnerError::Validation(format!(
                    "profile section references missing claim key {key:?}"
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn build_claims(
    run_id: Uuid,
    evidence: &[ResearchEvidence],
    evidence_keys: &BTreeMap<String, Uuid>,
    submissions: &[ClaimSubmission],
) -> Result<Vec<(String, ResearchClaim)>, RunnerError> {
    submissions
        .iter()
        .map(|submission| {
            let ids = |keys: &[String]| {
                keys.iter()
                    .map(|key| {
                        evidence_keys.get(key).copied().ok_or_else(|| {
                            RunnerError::Validation(format!("missing evidence key {key:?}"))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            };
            let claim = ResearchClaim::create(
                run_id,
                evidence,
                ResearchClaimDraft {
                    statement: submission.statement.clone(),
                    confidence: submission.confidence,
                    supporting_evidence_ids: ids(&submission.supporting_evidence_keys)?,
                    conflicting_evidence_ids: ids(&submission.conflicting_evidence_keys)?,
                    inference: submission.inference,
                },
            )?;
            Ok((submission.key.clone(), claim))
        })
        .collect()
}

pub(crate) fn build_profile_draft(
    claims: &[(String, ResearchClaim)],
    submission: ProfileDraftSubmission,
) -> Result<AuthenticityProfileDraft, RunnerError> {
    let claim_ids = claims
        .iter()
        .map(|(key, claim)| (key.as_str(), claim.id))
        .collect::<BTreeMap<_, _>>();
    let sections = submission
        .sections
        .into_iter()
        .map(|(name, section)| {
            let ids = section
                .claim_keys
                .iter()
                .map(|key| {
                    claim_ids.get(key.as_str()).copied().ok_or_else(|| {
                        RunnerError::Validation(format!("missing claim key {key:?}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                name,
                AuthenticitySection {
                    observations: section.observations,
                    generation_instructions: section.generation_instructions,
                    claim_ids: ids,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RunnerError>>()?;
    Ok(AuthenticityProfileDraft {
        schema_version: submission.schema_version,
        predecessor_id: submission.predecessor_id,
        summary: submission.summary,
        sections,
        generation_instructions: submission.generation_instructions,
        caveats: submission.caveats,
    })
}
