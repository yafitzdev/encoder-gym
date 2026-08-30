use std::collections::{BTreeMap, BTreeSet};

use artifact_core::fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ResearchError,
    brief::ResolvedResearchBrief,
    evidence::{ResearchClaim, ResearchEvidence},
    lifecycle::{ResearchRun, ResearchRunState},
    nonempty, normalized_list,
};

pub const AUTHENTICITY_PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticitySection {
    #[serde(default)]
    pub observations: Vec<String>,
    #[serde(default)]
    pub generation_instructions: Vec<String>,
    #[serde(default)]
    pub claim_ids: Vec<Uuid>,
}

impl AuthenticitySection {
    fn normalize(mut self, path: &str) -> Result<Self, ResearchError> {
        self.observations = normalized_list(self.observations, &format!("{path}.observations"))?;
        self.generation_instructions = normalized_list(
            self.generation_instructions,
            &format!("{path}.generation_instructions"),
        )?;
        self.claim_ids.sort_unstable();
        self.claim_ids.dedup();
        if self.observations.is_empty()
            && self.generation_instructions.is_empty()
            && self.claim_ids.is_empty()
        {
            return Err(ResearchError::Validation(format!(
                "{path} must contain profile guidance"
            )));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticityProfileDraft {
    pub schema_version: u32,
    #[serde(default)]
    pub predecessor_id: Option<Uuid>,
    pub summary: String,
    pub sections: BTreeMap<String, AuthenticitySection>,
    #[serde(default)]
    pub generation_instructions: Vec<String>,
    #[serde(default)]
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticityProfile {
    pub id: Uuid,
    pub schema_version: u32,
    pub version: u32,
    pub predecessor_id: Option<Uuid>,
    pub run_id: Uuid,
    pub run_specification_fingerprint: String,
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub summary: String,
    pub sections: BTreeMap<String, AuthenticitySection>,
    pub generation_instructions: Vec<String>,
    pub caveats: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub evidence_fingerprints: Vec<String>,
    pub claims: Vec<ResearchClaim>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AuthenticityProfile {
    pub fn create(
        brief: &ResolvedResearchBrief,
        run: &ResearchRun,
        predecessor: Option<&Self>,
        evidence: &[ResearchEvidence],
        claims: Vec<ResearchClaim>,
        draft: AuthenticityProfileDraft,
    ) -> Result<Self, ResearchError> {
        if run.state != ResearchRunState::AwaitingReview {
            return Err(ResearchError::Validation(
                "a profile requires a research run awaiting review".into(),
            ));
        }
        if run.brief_id != brief.id
            || run.brief_fingerprint != brief.fingerprint
            || run.reproduce_specification_fingerprint()? != run.specification_fingerprint
        {
            return Err(ResearchError::Integrity(
                "research run does not match the resolved brief".into(),
            ));
        }
        if draft.schema_version != AUTHENTICITY_PROFILE_SCHEMA_VERSION {
            return Err(ResearchError::Validation(format!(
                "unsupported authenticity profile schema_version {}; expected {AUTHENTICITY_PROFILE_SCHEMA_VERSION}",
                draft.schema_version
            )));
        }
        let version = match predecessor {
            Some(previous) => {
                if draft.predecessor_id != Some(previous.id)
                    || previous.dataset_id != brief.dataset.id
                {
                    return Err(ResearchError::Validation(
                        "profile revision must identify a predecessor for the same dataset".into(),
                    ));
                }
                previous
                    .version
                    .checked_add(1)
                    .ok_or_else(|| ResearchError::Validation("profile version overflowed".into()))?
            }
            None => {
                if draft.predecessor_id.is_some() {
                    return Err(ResearchError::Validation(
                        "initial profile cannot identify a predecessor".into(),
                    ));
                }
                1
            }
        };
        let mut sections = BTreeMap::new();
        for (name, section) in draft.sections {
            let name = nonempty(name, "profile section name")?;
            sections.insert(
                name.clone(),
                section.normalize(&format!("sections.{name}"))?,
            );
        }
        for required in &brief.required_profile_sections {
            if !sections.contains_key(required) {
                return Err(ResearchError::Validation(format!(
                    "profile is missing required section {required:?}"
                )));
            }
        }
        if evidence.is_empty() || claims.is_empty() {
            return Err(ResearchError::Validation(
                "an authenticity profile requires persisted evidence and claims".into(),
            ));
        }
        let claim_ids = claims.iter().map(|claim| claim.id).collect::<BTreeSet<_>>();
        for (name, section) in &sections {
            if section.claim_ids.is_empty() {
                return Err(ResearchError::Validation(format!(
                    "profile section {name:?} must cite at least one research claim"
                )));
            }
            if let Some(missing) = section.claim_ids.iter().find(|id| !claim_ids.contains(id)) {
                return Err(ResearchError::Validation(format!(
                    "profile section references missing claim {missing}"
                )));
            }
        }
        for claim in &claims {
            if claim.run_id != run.id || claim.reproduce_fingerprint()? != claim.fingerprint {
                return Err(ResearchError::Integrity(format!(
                    "claim {} does not belong to this run or failed integrity",
                    claim.id
                )));
            }
        }
        let evidence_ids = evidence.iter().map(|item| item.id).collect::<Vec<_>>();
        let evidence_id_set = evidence_ids.iter().copied().collect::<BTreeSet<_>>();
        for claim in &claims {
            if let Some(missing) = claim
                .supporting_evidence_ids
                .iter()
                .chain(claim.conflicting_evidence_ids.iter())
                .find(|id| !evidence_id_set.contains(id))
            {
                return Err(ResearchError::Integrity(format!(
                    "claim {} references evidence {missing} outside the profile corpus",
                    claim.id
                )));
            }
        }
        let mut evidence_fingerprints = Vec::with_capacity(evidence.len());
        for item in evidence {
            if item.run_id != run.id || item.reproduce_fingerprint()? != item.fingerprint {
                return Err(ResearchError::Integrity(format!(
                    "evidence {} does not belong to this run or failed integrity",
                    item.id
                )));
            }
            evidence_fingerprints.push(item.fingerprint.clone());
        }
        let mut profile = Self {
            id: Uuid::new_v4(),
            schema_version: draft.schema_version,
            version,
            predecessor_id: draft.predecessor_id,
            run_id: run.id,
            run_specification_fingerprint: run.specification_fingerprint.clone(),
            dataset_id: brief.dataset.id,
            dataset_fingerprint: brief.dataset.fingerprint.clone(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            summary: nonempty(draft.summary, "profile.summary")?,
            sections,
            generation_instructions: normalized_list(
                draft.generation_instructions,
                "profile.generation_instructions",
            )?,
            caveats: normalized_list(draft.caveats, "profile.caveats")?,
            evidence_ids,
            evidence_fingerprints,
            claims,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        profile.fingerprint = profile.reproduce_fingerprint()?;
        Ok(profile)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            (
                self.id,
                self.schema_version,
                self.version,
                self.predecessor_id,
                self.run_id,
                &self.run_specification_fingerprint,
                self.dataset_id,
                &self.dataset_fingerprint,
                self.brief_id,
                &self.brief_fingerprint,
            ),
            (
                &self.summary,
                &self.sections,
                &self.generation_instructions,
                &self.caveats,
                &self.evidence_ids,
                &self.evidence_fingerprints,
                &self.claims,
                self.created_at,
            ),
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileReview {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub profile_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub decision: ProfileReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProfileReview {
    pub fn create(
        profile: &AuthenticityProfile,
        predecessor: Option<&Self>,
        decision: ProfileReviewDecision,
        reviewer: String,
        reason: String,
    ) -> Result<Self, ResearchError> {
        if profile.reproduce_fingerprint()? != profile.fingerprint {
            return Err(ResearchError::Integrity(
                "profile fingerprint mismatch".into(),
            ));
        }
        if predecessor.is_some_and(|previous| previous.profile_id != profile.id) {
            return Err(ResearchError::Validation(
                "review predecessor belongs to another profile".into(),
            ));
        }
        let mut review = Self {
            id: Uuid::new_v4(),
            profile_id: profile.id,
            profile_fingerprint: profile.fingerprint.clone(),
            predecessor_id: predecessor.map(|previous| previous.id),
            decision,
            reviewer: nonempty(reviewer, "review.reviewer")?,
            reason: nonempty(reason, "review.reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        review.fingerprint = review.reproduce_fingerprint()?;
        Ok(review)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            self.id,
            self.profile_id,
            &self.profile_fingerprint,
            self.predecessor_id,
            self.decision,
            &self.reviewer,
            &self.reason,
            self.created_at,
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBinding {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub profile_id: Uuid,
    pub profile_version: u32,
    pub profile_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProfileBinding {
    pub fn bind(
        profile: &AuthenticityProfile,
        latest_review: &ProfileReview,
        predecessor_id: Option<Uuid>,
    ) -> Result<Self, ResearchError> {
        if profile.reproduce_fingerprint()? != profile.fingerprint
            || latest_review.reproduce_fingerprint()? != latest_review.fingerprint
            || latest_review.profile_id != profile.id
            || latest_review.profile_fingerprint != profile.fingerprint
        {
            return Err(ResearchError::Integrity(
                "profile approval does not match the profile".into(),
            ));
        }
        if latest_review.decision != ProfileReviewDecision::Approve {
            return Err(ResearchError::Validation(
                "only an explicitly approved profile can be bound".into(),
            ));
        }
        let mut binding = Self {
            id: Uuid::new_v4(),
            dataset_id: profile.dataset_id,
            dataset_fingerprint: profile.dataset_fingerprint.clone(),
            profile_id: profile.id,
            profile_version: profile.version,
            profile_fingerprint: profile.fingerprint.clone(),
            approval_id: latest_review.id,
            approval_fingerprint: latest_review.fingerprint.clone(),
            predecessor_id,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        binding.fingerprint = binding.reproduce_fingerprint()?;
        Ok(binding)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            self.id,
            self.dataset_id,
            &self.dataset_fingerprint,
            self.profile_id,
            self.profile_version,
            &self.profile_fingerprint,
            self.approval_id,
            &self.approval_fingerprint,
            self.predecessor_id,
            self.created_at,
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAuthenticityContext {
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub binding_id: Uuid,
    pub binding_fingerprint: String,
    pub profile_id: Uuid,
    pub profile_version: u32,
    pub profile_fingerprint: String,
    pub summary: String,
    pub sections: BTreeMap<String, AuthenticitySection>,
    pub generation_instructions: Vec<String>,
    pub caveats: Vec<String>,
    pub resolved_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Immutable handoff from an approved dataset binding to one generation job.
///
/// The context intentionally contains abstract profile guidance only. Raw page
/// content, evidence excerpts, and research claims remain on the research side
/// of the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationAuthenticityAssignment {
    pub job_id: Uuid,
    pub context: ResolvedAuthenticityContext,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationAuthenticityAssignment {
    pub fn new(job_id: Uuid, context: ResolvedAuthenticityContext) -> Result<Self, ResearchError> {
        if context.reproduce_fingerprint()? != context.fingerprint {
            return Err(ResearchError::Integrity(
                "authenticity context fingerprint mismatch".into(),
            ));
        }
        let mut assignment = Self {
            job_id,
            context,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        assignment.fingerprint = assignment.reproduce_fingerprint()?;
        Ok(assignment)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(self.job_id, &self.context, self.created_at))
            .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

impl ResolvedAuthenticityContext {
    pub fn resolve(
        binding: &ProfileBinding,
        profile: &AuthenticityProfile,
        review: &ProfileReview,
    ) -> Result<Self, ResearchError> {
        if binding.reproduce_fingerprint()? != binding.fingerprint
            || profile.reproduce_fingerprint()? != profile.fingerprint
            || review.reproduce_fingerprint()? != review.fingerprint
            || binding.profile_id != profile.id
            || binding.profile_fingerprint != profile.fingerprint
            || binding.approval_id != review.id
            || review.decision != ProfileReviewDecision::Approve
        {
            return Err(ResearchError::Integrity(
                "cannot resolve a mismatched or unapproved authenticity binding".into(),
            ));
        }
        let mut context = Self {
            dataset_id: binding.dataset_id,
            dataset_fingerprint: binding.dataset_fingerprint.clone(),
            binding_id: binding.id,
            binding_fingerprint: binding.fingerprint.clone(),
            profile_id: profile.id,
            profile_version: profile.version,
            profile_fingerprint: profile.fingerprint.clone(),
            summary: profile.summary.clone(),
            sections: profile.sections.clone(),
            generation_instructions: profile.generation_instructions.clone(),
            caveats: profile.caveats.clone(),
            resolved_at: Utc::now(),
            fingerprint: String::new(),
        };
        context.fingerprint = context.reproduce_fingerprint()?;
        Ok(context)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            self.dataset_id,
            &self.dataset_fingerprint,
            self.binding_id,
            &self.binding_fingerprint,
            self.profile_id,
            self.profile_version,
            &self.profile_fingerprint,
            &self.summary,
            &self.sections,
            &self.generation_instructions,
            &self.caveats,
            self.resolved_at,
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        brief::*,
        evidence::{EvidenceConfidence, ResearchClaimDraft, ResearchEvidenceDraft},
        lifecycle::ResearchStopReason,
    };

    use super::*;

    fn brief_and_run() -> (ResolvedResearchBrief, ResearchRun) {
        let brief = ResolvedResearchBrief::create(ResearchBriefDraft {
            schema_version: 1,
            dataset: ArtifactReference {
                id: Uuid::new_v4(),
                fingerprint: "sha256:dataset".into(),
            },
            task: "task".into(),
            labels: vec!["a".into()],
            dimensions: BTreeMap::new(),
            semantic_context: None,
            target: ResearchTarget {
                language: "English".into(),
                ..ResearchTarget::default()
            },
            questions: vec!["q".into()],
            desired_source_diversity: 1,
            source_policy: SourcePolicy::default(),
            budgets: ResearchBudgets {
                max_model_turns: 1,
                max_searches: 1,
                max_fetched_pages: 1,
                max_fetched_bytes: 100,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_wall_clock_seconds: 60,
                max_cost_microusd: 100,
                max_retries_per_call: 0,
            },
            provider: ResearchProviderConfiguration {
                runtime: "pi".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
            },
            required_profile_sections: vec!["language".into()],
        })
        .unwrap();
        let mut run = ResearchRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        run.await_review(ResearchStopReason::SufficientEvidence)
            .unwrap();
        (brief, run)
    }

    fn profile() -> AuthenticityProfile {
        let (brief, run) = brief_and_run();
        let evidence = ResearchEvidence::create(
            run.id,
            Uuid::new_v4(),
            &brief.source_policy,
            ResearchEvidenceDraft {
                url: "https://example.com/messages".into(),
                title: "Messages".into(),
                query: "authentic messages".into(),
                source_class: "documentation".into(),
                content_hash: "sha256:page".into(),
                excerpt: "short fragment".into(),
                location: None,
                observation: "Fragments are common.".into(),
                applicability: "English support chat".into(),
                confidence: EvidenceConfidence::Medium,
            },
        )
        .unwrap();
        let claim = ResearchClaim::create(
            run.id,
            std::slice::from_ref(&evidence),
            ResearchClaimDraft {
                statement: "Fragments are common.".into(),
                confidence: EvidenceConfidence::Medium,
                supporting_evidence_ids: vec![evidence.id],
                conflicting_evidence_ids: vec![],
                inference: false,
            },
        )
        .unwrap();
        AuthenticityProfile::create(
            &brief,
            &run,
            None,
            std::slice::from_ref(&evidence),
            vec![claim.clone()],
            AuthenticityProfileDraft {
                schema_version: 1,
                predecessor_id: None,
                summary: "Authentic messages are short.".into(),
                sections: BTreeMap::from([(
                    "language".into(),
                    AuthenticitySection {
                        observations: vec!["Fragments are common.".into()],
                        generation_instructions: vec!["Use occasional fragments.".into()],
                        claim_ids: vec![claim.id],
                    },
                )]),
                generation_instructions: vec!["Vary message length.".into()],
                caveats: vec!["Small source corpus.".into()],
            },
        )
        .unwrap()
    }

    #[test]
    fn rejected_profile_cannot_be_bound() {
        let profile = profile();
        let review = ProfileReview::create(
            &profile,
            None,
            ProfileReviewDecision::Reject,
            "operator".into(),
            "insufficient diversity".into(),
        )
        .unwrap();
        assert!(ProfileBinding::bind(&profile, &review, None).is_err());
    }

    #[test]
    fn approved_profile_resolves_to_pinned_generation_context() {
        let profile = profile();
        let review = ProfileReview::create(
            &profile,
            None,
            ProfileReviewDecision::Approve,
            "operator".into(),
            "evidence is sufficient".into(),
        )
        .unwrap();
        let binding = ProfileBinding::bind(&profile, &review, None).unwrap();
        let context = ResolvedAuthenticityContext::resolve(&binding, &profile, &review).unwrap();
        assert_eq!(context.profile_fingerprint, profile.fingerprint);
        assert_eq!(
            context.reproduce_fingerprint().unwrap(),
            context.fingerprint
        );
    }

    #[test]
    fn missing_required_section_blocks_profile() {
        let (brief, run) = brief_and_run();
        let result = AuthenticityProfile::create(
            &brief,
            &run,
            None,
            &[],
            vec![],
            AuthenticityProfileDraft {
                schema_version: 1,
                predecessor_id: None,
                summary: "summary".into(),
                sections: BTreeMap::new(),
                generation_instructions: vec![],
                caveats: vec![],
            },
        );
        assert!(
            matches!(result, Err(ResearchError::Validation(message)) if message.contains("language"))
        );
    }
}
