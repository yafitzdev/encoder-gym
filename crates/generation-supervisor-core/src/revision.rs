//! Bounded diagnosis, guidance-only revisions, review, canary, and activation.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    SupervisorError,
    contract::{GenerationQualityContract, PromptRevisionKind, RevisionApprovalPolicy},
    decision::{DeterministicQualityDecision, SupervisorDecisionState},
    fingerprint,
    observation::QualityScope,
    required,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisCause {
    PromptGuidance,
    SemanticConflict,
    DimensionStrategyConflict,
    GeneratorInadequacy,
    EvaluatorDisagreement,
    InsufficientEvidence,
    RepetitionModeCollapse,
    AuthenticityFailure,
    StrategyFailure,
    NotSafelyRepairable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorRuntimeIdentity {
    pub runtime: String,
    pub model: String,
    pub protocol_version: String,
    pub capability_set_version: u32,
    pub configuration_fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvisorUsage {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_microunits: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorDiagnosis {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub triggering_decision_id: Uuid,
    pub triggering_decision_fingerprint: String,
    pub cause: DiagnosisCause,
    pub summary: String,
    pub repairable_by_guidance: bool,
    pub runtime: AdvisorRuntimeIdentity,
    pub usage: AdvisorUsage,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SupervisorDiagnosis {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        run_id: Uuid,
        decision: &DeterministicQualityDecision,
        cause: DiagnosisCause,
        summary: impl Into<String>,
        repairable_by_guidance: bool,
        runtime: AdvisorRuntimeIdentity,
        usage: AdvisorUsage,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil()
            || run_id.is_nil()
            || decision.supervisor_run_id != run_id
            || decision.state != SupervisorDecisionState::PauseForDiagnosis
            || decision.fingerprint.is_empty()
            || decision.reproduce_fingerprint()? != decision.fingerprint
        {
            return Err(SupervisorError::InvalidTransition(
                "diagnosis requires an exact deterministic pause in the same run".into(),
            ));
        }
        if cause == DiagnosisCause::NotSafelyRepairable && repairable_by_guidance {
            return Err(SupervisorError::Validation(
                "not-safely-repairable diagnosis cannot authorize guidance repair".into(),
            ));
        }
        let runtime = normalize_runtime(runtime)?;
        let mut value = Self {
            id,
            supervisor_run_id: run_id,
            triggering_decision_id: decision.id,
            triggering_decision_fingerprint: decision.fingerprint.clone(),
            cause,
            summary: required(summary, "diagnosis summary")?,
            repairable_by_guidance,
            runtime,
            usage,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptGuidanceVersion {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_version_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_proposal_id: Option<Uuid>,
    pub base_prompt_fingerprint: String,
    pub protected_fields_fingerprint: String,
    pub guidance: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PromptGuidanceVersion {
    pub fn initial(
        id: Uuid,
        run_id: Uuid,
        base_prompt_fingerprint: impl Into<String>,
        protected_fields_fingerprint: impl Into<String>,
        guidance: Vec<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        let mut value = Self {
            id,
            supervisor_run_id: run_id,
            sequence: 0,
            parent_version_id: None,
            source_proposal_id: None,
            base_prompt_fingerprint: required(base_prompt_fingerprint, "base prompt fingerprint")?,
            protected_fields_fingerprint: required(
                protected_fields_fingerprint,
                "protected prompt fields fingerprint",
            )?,
            guidance: normalize_instructions(guidance)?,
            created_at,
            fingerprint: String::new(),
        };
        if id.is_nil() || run_id.is_nil() {
            return Err(SupervisorError::Validation(
                "prompt version identities must not be nil".into(),
            ));
        }
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    fn revised(
        id: Uuid,
        parent: &Self,
        proposal: &PromptRevisionProposal,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        let mut value = Self {
            id,
            supervisor_run_id: parent.supervisor_run_id,
            sequence: parent.sequence.checked_add(1).ok_or_else(|| {
                SupervisorError::Validation("prompt version sequence overflow".into())
            })?,
            parent_version_id: Some(parent.id),
            source_proposal_id: Some(proposal.id),
            base_prompt_fingerprint: parent.base_prompt_fingerprint.clone(),
            protected_fields_fingerprint: parent.protected_fields_fingerprint.clone(),
            guidance: proposal.replacement_guidance.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.id.is_nil()
            || self.supervisor_run_id.is_nil()
            || self.base_prompt_fingerprint.is_empty()
            || self.protected_fields_fingerprint.is_empty()
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
            || (self.sequence == 0
                && (self.parent_version_id.is_some() || self.source_proposal_id.is_some()))
            || (self.sequence > 0
                && (self.parent_version_id.is_none() || self.source_proposal_id.is_none()))
        {
            return Err(SupervisorError::Integrity(
                "prompt guidance version does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedFieldProof {
    pub fields: BTreeSet<crate::contract::ProtectedPromptField>,
    pub before_fingerprint: String,
    pub after_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedImprovement {
    pub metric: String,
    pub minimum_delta_basis_points: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptRevisionProposal {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub revision_sequence: u32,
    pub parent_prompt_version_id: Uuid,
    pub parent_prompt_version_fingerprint: String,
    pub triggering_decision_id: Uuid,
    pub triggering_decision_fingerprint: String,
    pub triggering_window_id: Uuid,
    pub diagnosis_id: Uuid,
    pub diagnosis_fingerprint: String,
    pub kind: PromptRevisionKind,
    pub affected_scopes: BTreeSet<QualityScope>,
    pub replacement_guidance: Vec<String>,
    pub expected_improvements: Vec<ExpectedImprovement>,
    pub protected_field_proof: ProtectedFieldProof,
    pub runtime: AdvisorRuntimeIdentity,
    pub usage: AdvisorUsage,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PromptRevisionProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        contract: &GenerationQualityContract,
        decision: &DeterministicQualityDecision,
        diagnosis: &SupervisorDiagnosis,
        parent: &PromptGuidanceVersion,
        revision_sequence: u32,
        affected_scopes: BTreeSet<QualityScope>,
        replacement_guidance: Vec<String>,
        expected_improvements: Vec<ExpectedImprovement>,
        runtime: AdvisorRuntimeIdentity,
        usage: AdvisorUsage,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        contract.validate()?;
        decision.validate(contract)?;
        parent.validate()?;
        if id.is_nil()
            || diagnosis.supervisor_run_id != decision.supervisor_run_id
            || diagnosis.triggering_decision_id != decision.id
            || diagnosis.triggering_decision_fingerprint != decision.fingerprint
            || diagnosis.reproduce_fingerprint()? != diagnosis.fingerprint
            || !diagnosis.repairable_by_guidance
            || parent.supervisor_run_id != decision.supervisor_run_id
            || revision_sequence == 0
            || affected_scopes.is_empty()
            || !affected_scopes.contains(&decision.scope)
            || expected_improvements.is_empty()
        {
            return Err(SupervisorError::InvalidTransition(
                "revision proposal is not bound to a repairable paused scope".into(),
            ));
        }
        let replacement_guidance = normalize_instructions(replacement_guidance)?;
        validate_patch(contract, &replacement_guidance, affected_scopes.len())?;
        for expected in &expected_improvements {
            required(expected.metric.clone(), "expected improvement metric")?;
            if expected.minimum_delta_basis_points > 10_000 {
                return Err(SupervisorError::Validation(
                    "expected improvement delta exceeds 10000 basis points".into(),
                ));
            }
        }
        let protected_field_proof = ProtectedFieldProof {
            fields: contract.revision_policy.protected_fields.clone(),
            before_fingerprint: parent.protected_fields_fingerprint.clone(),
            after_fingerprint: parent.protected_fields_fingerprint.clone(),
        };
        let mut value = Self {
            id,
            supervisor_run_id: decision.supervisor_run_id,
            revision_sequence,
            parent_prompt_version_id: parent.id,
            parent_prompt_version_fingerprint: parent.fingerprint.clone(),
            triggering_decision_id: decision.id,
            triggering_decision_fingerprint: decision.fingerprint.clone(),
            triggering_window_id: decision.window_id,
            diagnosis_id: diagnosis.id,
            diagnosis_fingerprint: diagnosis.fingerprint.clone(),
            kind: contract.revision_policy.allowed_kind,
            affected_scopes,
            replacement_guidance,
            expected_improvements,
            protected_field_proof,
            runtime: normalize_runtime(runtime)?,
            usage,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(
        &self,
        contract: &GenerationQualityContract,
        parent: &PromptGuidanceVersion,
    ) -> Result<(), SupervisorError> {
        if self.parent_prompt_version_id != parent.id
            || self.parent_prompt_version_fingerprint != parent.fingerprint
            || self.protected_field_proof.fields != contract.revision_policy.protected_fields
            || self.protected_field_proof.before_fingerprint != parent.protected_fields_fingerprint
            || self.protected_field_proof.after_fingerprint != parent.protected_fields_fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "revision proposal changes protected fields or does not reproduce".into(),
            ));
        }
        validate_patch(
            contract,
            &self.replacement_guidance,
            self.affected_scopes.len(),
        )
    }

    pub fn fits_preauthorization(
        &self,
        contract: &GenerationQualityContract,
        now: DateTime<Utc>,
    ) -> bool {
        let RevisionApprovalPolicy::FinitePreauthorization { envelope } = &contract.approval_policy
        else {
            return false;
        };
        let total_characters = self
            .replacement_guidance
            .iter()
            .map(|value| value.chars().count())
            .sum::<usize>();
        envelope.expires_at > now
            && envelope.revision_kind == self.kind
            && self.affected_scopes.len()
                <= usize::try_from(envelope.maximum_affected_scopes).unwrap_or(usize::MAX)
            && self.replacement_guidance.len()
                <= usize::try_from(envelope.maximum_instructions).unwrap_or(usize::MAX)
            && total_characters
                <= usize::try_from(envelope.maximum_total_characters).unwrap_or(usize::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptRevisionReview {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_fingerprint: Option<String>,
    pub decision: RevisionReviewDecision,
    pub reviewer: String,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PromptRevisionReview {
    pub fn create(
        id: Uuid,
        proposal: &PromptRevisionProposal,
        predecessor: Option<&Self>,
        decision: RevisionReviewDecision,
        reviewer: impl Into<String>,
        rationale: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil() || proposal.fingerprint.is_empty() {
            return Err(SupervisorError::Validation(
                "review identity and exact proposal fingerprint are required".into(),
            ));
        }
        if predecessor.is_some_and(|value| {
            value.proposal_id != proposal.id
                || value.proposal_fingerprint != proposal.fingerprint
                || value.reproduce_fingerprint().ok().as_deref() != Some(value.fingerprint.as_str())
        }) {
            return Err(SupervisorError::Integrity(
                "revision review predecessor is invalid or belongs to another proposal".into(),
            ));
        }
        let mut value = Self {
            id,
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            predecessor_fingerprint: predecessor.map(|value| value.fingerprint.clone()),
            decision,
            reviewer: required(reviewer, "revision reviewer")?,
            rationale: required(rationale, "revision review rationale")?,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptRevisionAuthorization {
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_fingerprint: Option<String>,
    pub preauthorized: bool,
    pub authorized_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PromptRevisionAuthorization {
    pub fn authorize(
        contract: &GenerationQualityContract,
        proposal: &PromptRevisionProposal,
        parent: &PromptGuidanceVersion,
        review: Option<&PromptRevisionReview>,
        now: DateTime<Utc>,
    ) -> Result<(Self, PromptGuidanceVersion), SupervisorError> {
        proposal.validate(contract, parent)?;
        let preauthorized = proposal.fits_preauthorization(contract, now);
        if let Some(review) = review {
            if review.proposal_id != proposal.id
                || review.proposal_fingerprint != proposal.fingerprint
                || review.decision != RevisionReviewDecision::Approve
                || review.reproduce_fingerprint()? != review.fingerprint
            {
                return Err(SupervisorError::InvalidTransition(
                    "only exact append-only approval authorizes a revision".into(),
                ));
            }
        } else if !preauthorized {
            return Err(SupervisorError::InvalidTransition(
                "revision requires explicit review or matching finite preauthorization".into(),
            ));
        }
        let mut authorization = Self {
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            review_id: review.map(|value| value.id),
            review_fingerprint: review.map(|value| value.fingerprint.clone()),
            preauthorized: review.is_none(),
            authorized_at: now,
            fingerprint: String::new(),
        };
        authorization.fingerprint = authorization.reproduce_fingerprint()?;
        let version = PromptGuidanceVersion::revised(Uuid::new_v4(), parent, proposal, now)?;
        Ok((authorization, version))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptRevisionActivation {
    pub id: Uuid,
    pub supervisor_run_id: Uuid,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    pub authorization_fingerprint: String,
    pub canary_decision_id: Uuid,
    pub canary_decision_fingerprint: String,
    pub activated_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PromptRevisionActivation {
    pub fn create(
        id: Uuid,
        version: &PromptGuidanceVersion,
        authorization: &PromptRevisionAuthorization,
        canary_decision: &DeterministicQualityDecision,
        activated_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil()
            || version.supervisor_run_id != canary_decision.supervisor_run_id
            || version.source_proposal_id != Some(authorization.proposal_id)
            || canary_decision.prompt_version_id != version.id
            || canary_decision.prompt_version_fingerprint != version.fingerprint
            || authorization.proposal_fingerprint.is_empty()
            || authorization.reproduce_fingerprint()? != authorization.fingerprint
            || canary_decision.state != SupervisorDecisionState::RevisionPassed
            || canary_decision.fingerprint.is_empty()
            || canary_decision.reproduce_fingerprint()? != canary_decision.fingerprint
        {
            return Err(SupervisorError::InvalidTransition(
                "prompt revision activation requires its successful deterministic canary".into(),
            ));
        }
        let mut value = Self {
            id,
            supervisor_run_id: version.supervisor_run_id,
            prompt_version_id: version.id,
            prompt_version_fingerprint: version.fingerprint.clone(),
            authorization_fingerprint: authorization.fingerprint.clone(),
            canary_decision_id: canary_decision.id,
            canary_decision_fingerprint: canary_decision.fingerprint.clone(),
            activated_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn validate_patch(
    contract: &GenerationQualityContract,
    instructions: &[String],
    affected_scopes: usize,
) -> Result<(), SupervisorError> {
    let policy = &contract.revision_policy;
    if instructions.is_empty()
        || instructions.len() > usize::try_from(policy.maximum_instructions).unwrap_or(usize::MAX)
        || instructions.iter().any(|value| {
            value.chars().count()
                > usize::try_from(policy.maximum_characters_per_instruction).unwrap_or(usize::MAX)
        })
        || instructions
            .iter()
            .map(|value| value.chars().count())
            .sum::<usize>()
            > usize::try_from(policy.maximum_total_characters).unwrap_or(usize::MAX)
        || affected_scopes == 0
    {
        return Err(SupervisorError::Validation(
            "guidance patch exceeds its resolved finite policy".into(),
        ));
    }
    Ok(())
}

fn normalize_instructions(values: Vec<String>) -> Result<Vec<String>, SupervisorError> {
    let values = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .collect::<Vec<_>>();
    if values.iter().any(String::is_empty) {
        return Err(SupervisorError::Validation(
            "prompt guidance cannot contain empty instructions".into(),
        ));
    }
    Ok(values)
}

fn normalize_runtime(
    mut runtime: AdvisorRuntimeIdentity,
) -> Result<AdvisorRuntimeIdentity, SupervisorError> {
    runtime.runtime = required(runtime.runtime, "advisor runtime")?;
    runtime.model = required(runtime.model, "advisor model")?;
    runtime.protocol_version = required(runtime.protocol_version, "advisor protocol version")?;
    runtime.configuration_fingerprint = required(
        runtime.configuration_fingerprint,
        "advisor configuration fingerprint",
    )?;
    if runtime.capability_set_version == 0 {
        return Err(SupervisorError::Validation(
            "advisor capability set version must be positive".into(),
        ));
    }
    Ok(runtime)
}
