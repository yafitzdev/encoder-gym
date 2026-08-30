use chrono::{DateTime, Utc};
use optimization_core::{
    campaigns::{CampaignArtifactLink, CampaignOutcomeAssessment, OptimizationCampaign},
    domain::{OptimizationProposal, ProposalApplication},
    ports::OptimizationStoreError,
    reviews::ProposalReviewRecord,
    scenarios::OptimizationScenarioGroup,
};
use sqlx::FromRow;
use uuid::Uuid;

use super::{campaign_artifact_kind_text, review_state_text, store_error, to_u64};

#[derive(Debug, FromRow)]
pub(super) struct CampaignRecord {
    id: Uuid,
    proposal_id: Uuid,
    approval_review_id: Uuid,
    fingerprint: String,
    campaign_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignRecord {
    pub(super) fn into_domain(self) -> Result<OptimizationCampaign, OptimizationStoreError> {
        let campaign: OptimizationCampaign =
            serde_json::from_str(&self.campaign_json).map_err(store_error)?;
        if campaign.id != self.id
            || campaign.proposal_id != self.proposal_id
            || campaign.approval_review_id != self.approval_review_id
            || campaign.fingerprint != self.fingerprint
            || campaign.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign columns do not match its immutable payload".into(),
            ));
        }
        Ok(campaign)
    }
}

#[derive(Debug, FromRow)]
pub(super) struct CampaignLinkRecord {
    id: Uuid,
    campaign_id: Uuid,
    artifact_kind: String,
    artifact_id: Uuid,
    fingerprint: String,
    link_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignLinkRecord {
    pub(super) fn into_domain(self) -> Result<CampaignArtifactLink, OptimizationStoreError> {
        let link: CampaignArtifactLink =
            serde_json::from_str(&self.link_json).map_err(store_error)?;
        if link.id != self.id
            || link.campaign_id != self.campaign_id
            || campaign_artifact_kind_text(link.artifact_kind) != self.artifact_kind
            || link.artifact_id != self.artifact_id
            || link.fingerprint != self.fingerprint
            || link.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign link columns do not match its immutable payload".into(),
            ));
        }
        Ok(link)
    }
}

#[derive(Debug, FromRow)]
pub(super) struct CampaignOutcomeRecord {
    id: Uuid,
    campaign_id: Uuid,
    comparison_id: Uuid,
    fingerprint: String,
    outcome_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignOutcomeRecord {
    pub(super) fn into_domain(self) -> Result<CampaignOutcomeAssessment, OptimizationStoreError> {
        let outcome: CampaignOutcomeAssessment =
            serde_json::from_str(&self.outcome_json).map_err(store_error)?;
        if outcome.id != self.id
            || outcome.campaign_id != self.campaign_id
            || outcome.comparison_id != self.comparison_id
            || outcome.fingerprint != self.fingerprint
            || outcome.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign outcome columns do not match its immutable payload".into(),
            ));
        }
        Ok(outcome)
    }
}

#[derive(Debug, FromRow)]
pub(super) struct ProposalRecord {
    id: Uuid,
    analysis_report_id: Uuid,
    dataset_id: Uuid,
    additional_example_budget: i64,
    minimum_support: i64,
    proposal_json: String,
    fingerprint: Option<String>,
    created_at: DateTime<Utc>,
}

impl ProposalRecord {
    pub(super) fn into_domain(self) -> Result<OptimizationProposal, OptimizationStoreError> {
        let proposal: OptimizationProposal =
            serde_json::from_str(&self.proposal_json).map_err(store_error)?;
        if proposal.id != self.id
            || proposal.analysis_report_id != self.analysis_report_id
            || proposal.dataset_id != self.dataset_id
            || u32::try_from(self.additional_example_budget).map_err(store_error)?
                != proposal.additional_example_budget
            || to_u64(self.minimum_support)? != proposal.minimum_support
            || self
                .fingerprint
                .as_deref()
                .is_some_and(|fingerprint| proposal.fingerprint != fingerprint)
            || proposal.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization proposal columns do not match its immutable payload".into(),
            ));
        }
        Ok(proposal)
    }
}

#[derive(Debug, FromRow)]
pub(super) struct ApplicationRecord {
    proposal_id: Uuid,
    generation_plan_id: Uuid,
    approval_review_id: Option<Uuid>,
    approval_fingerprint: Option<String>,
    selected_recommendation_ids_json: String,
    verified_coverage_fingerprint: Option<String>,
    applied_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
pub(super) struct ScenarioGroupRecord {
    id: Uuid,
    source_evidence_fingerprint: String,
    fingerprint: String,
    group_json: String,
    created_at: DateTime<Utc>,
}

impl ScenarioGroupRecord {
    pub(super) fn into_domain(self) -> Result<OptimizationScenarioGroup, OptimizationStoreError> {
        let group: OptimizationScenarioGroup =
            serde_json::from_str(&self.group_json).map_err(store_error)?;
        if group.id != self.id
            || group.source_evidence_fingerprint != self.source_evidence_fingerprint
            || group.fingerprint != self.fingerprint
            || group.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization scenario-group columns do not match its immutable payload".into(),
            ));
        }
        Ok(group)
    }
}

#[derive(Debug, FromRow)]
pub(super) struct ProposalReviewRecordRow {
    id: Uuid,
    proposal_id: Uuid,
    proposal_fingerprint: String,
    state: String,
    note: Option<String>,
    superseding_proposal_id: Option<Uuid>,
    campaign_id: Option<Uuid>,
    fingerprint: String,
    review_json: String,
    created_at: DateTime<Utc>,
}

impl ProposalReviewRecordRow {
    pub(super) fn into_domain(self) -> Result<ProposalReviewRecord, OptimizationStoreError> {
        let review: ProposalReviewRecord =
            serde_json::from_str(&self.review_json).map_err(store_error)?;
        if review.id != self.id
            || review.proposal_id != self.proposal_id
            || review.proposal_fingerprint != self.proposal_fingerprint
            || review_state_text(review.state) != self.state
            || review.note != self.note
            || review.superseding_proposal_id != self.superseding_proposal_id
            || review.campaign_id != self.campaign_id
            || review.fingerprint != self.fingerprint
            || review.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization review columns do not match its immutable payload".into(),
            ));
        }
        Ok(review)
    }
}

impl ApplicationRecord {
    pub(super) fn into_domain(self) -> Result<ProposalApplication, OptimizationStoreError> {
        Ok(ProposalApplication {
            proposal_id: self.proposal_id,
            generation_plan_id: self.generation_plan_id,
            approval_review_id: self.approval_review_id,
            approval_fingerprint: self.approval_fingerprint,
            selected_recommendation_ids: serde_json::from_str(
                &self.selected_recommendation_ids_json,
            )
            .map_err(store_error)?,
            verified_coverage_fingerprint: self.verified_coverage_fingerprint,
            applied_at: self.applied_at,
        })
    }
}
