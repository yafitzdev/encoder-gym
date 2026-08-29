use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{domain::OptimizationProposal, protocol::RecommendationKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReviewState {
    Open,
    ApprovedForPlanCreation,
    Rejected,
    Superseded,
    PartiallyAccepted,
    AcceptedTrainingExperimentCandidate,
    AcceptedReviewOnlyCandidates,
    CompletedAwaitingOutcomeAssessment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalReviewRecord {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub state: ProposalReviewState,
    pub selected_recommendation_ids: Vec<String>,
    pub note: Option<String>,
    pub superseding_proposal_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl ProposalReviewRecord {
    pub fn new(
        proposal: &OptimizationProposal,
        state: ProposalReviewState,
        mut selected_recommendation_ids: Vec<String>,
        note: Option<String>,
        superseding_proposal_id: Option<Uuid>,
        campaign_id: Option<Uuid>,
    ) -> Result<Self, ProposalReviewError> {
        if proposal.normalized_recommendations.is_empty()
            && proposal
                .training_candidate_set
                .as_ref()
                .is_none_or(|set| set.candidates.is_empty())
            && proposal.review_only_recommendations.is_empty()
            && !matches!(
                state,
                ProposalReviewState::Open | ProposalReviewState::Rejected
            )
        {
            return Err(ProposalReviewError::LegacyProposal);
        }
        selected_recommendation_ids.sort();
        if selected_recommendation_ids
            .windows(2)
            .any(|values| values[0] == values[1])
        {
            return Err(ProposalReviewError::DuplicateSelection);
        }
        let note = note.and_then(|value| {
            let trimmed = value.trim().to_owned();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let mut review = Self {
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            state,
            selected_recommendation_ids,
            note,
            superseding_proposal_id,
            campaign_id,
            fingerprint: String::new(),
            created_at: Utc::now(),
        };
        review.validate(proposal)?;
        review.fingerprint = review
            .reproduce_fingerprint()
            .map_err(|error| ProposalReviewError::Fingerprint(error.to_string()))?;
        Ok(review)
    }

    pub fn validate(&self, proposal: &OptimizationProposal) -> Result<(), ProposalReviewError> {
        if self.proposal_id != proposal.id
            || self.proposal_fingerprint != proposal.fingerprint
            || self.proposal_fingerprint.trim().is_empty()
        {
            return Err(ProposalReviewError::ProposalIdentity);
        }
        if self
            .selected_recommendation_ids
            .windows(2)
            .any(|values| values[0] >= values[1])
        {
            return Err(ProposalReviewError::SelectionOrder);
        }
        let selected =
            self.selected_recommendation_ids
                .iter()
                .map(|id| {
                    if let Some(recommendation) = proposal
                        .normalized_recommendations
                        .iter()
                        .find(|recommendation| &recommendation.id == id)
                    {
                        return Ok(recommendation.kind);
                    }
                    if proposal.training_candidate_set.as_ref().is_some_and(|set| {
                        set.candidates.iter().any(|candidate| &candidate.id == id)
                    }) {
                        return Ok(RecommendationKind::TrainingConfiguration);
                    }
                    if proposal
                        .review_only_recommendations
                        .iter()
                        .any(|recommendation| &recommendation.id == id)
                    {
                        return Ok(RecommendationKind::ReviewOnly);
                    }
                    Err(ProposalReviewError::UnknownRecommendation(id.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
        match self.state {
            ProposalReviewState::Open | ProposalReviewState::Rejected => {
                if !selected.is_empty() {
                    return Err(ProposalReviewError::UnexpectedSelection);
                }
            }
            ProposalReviewState::ApprovedForPlanCreation => {
                if !selected.is_empty()
                    || !proposal
                        .normalized_recommendations
                        .iter()
                        .any(|recommendation| {
                            recommendation.kind == RecommendationKind::DataGeneration
                        })
                {
                    return Err(ProposalReviewError::FullApproval);
                }
            }
            ProposalReviewState::PartiallyAccepted => {
                if selected.is_empty()
                    || selected
                        .iter()
                        .any(|kind| *kind != RecommendationKind::DataGeneration)
                {
                    return Err(ProposalReviewError::PartialSelection);
                }
            }
            ProposalReviewState::AcceptedTrainingExperimentCandidate => {
                if selected.is_empty()
                    || selected
                        .iter()
                        .any(|kind| *kind != RecommendationKind::TrainingConfiguration)
                {
                    return Err(ProposalReviewError::TrainingSelection);
                }
            }
            ProposalReviewState::AcceptedReviewOnlyCandidates => {
                if selected.is_empty()
                    || selected
                        .iter()
                        .any(|kind| *kind != RecommendationKind::ReviewOnly)
                {
                    return Err(ProposalReviewError::ReviewOnlySelection);
                }
            }
            ProposalReviewState::Superseded => {
                if !selected.is_empty()
                    || self.superseding_proposal_id.is_none() && self.campaign_id.is_none()
                {
                    return Err(ProposalReviewError::SupersedingReference);
                }
            }
            ProposalReviewState::CompletedAwaitingOutcomeAssessment => {
                if !selected.is_empty() || self.campaign_id.is_none() {
                    return Err(ProposalReviewError::CampaignReference);
                }
            }
        }
        if self.state != ProposalReviewState::Superseded && self.superseding_proposal_id.is_some() {
            return Err(ProposalReviewError::SupersedingReference);
        }
        if !self.fingerprint.is_empty()
            && self
                .reproduce_fingerprint()
                .map_err(|error| ProposalReviewError::Fingerprint(error.to_string()))?
                != self.fingerprint
        {
            return Err(ProposalReviewError::ReviewFingerprint);
        }
        Ok(())
    }

    pub fn selected_data_recommendation_ids(
        &self,
        proposal: &OptimizationProposal,
    ) -> Result<Vec<String>, ProposalReviewError> {
        self.validate(proposal)?;
        match self.state {
            ProposalReviewState::ApprovedForPlanCreation => Ok(proposal
                .normalized_recommendations
                .iter()
                .filter(|recommendation| recommendation.kind == RecommendationKind::DataGeneration)
                .map(|recommendation| recommendation.id.clone())
                .collect()),
            ProposalReviewState::PartiallyAccepted => Ok(self.selected_recommendation_ids.clone()),
            _ => Err(ProposalReviewError::NotApprovedForPlan),
        }
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.fingerprint.clear();
        artifact_core::fingerprint(&input)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProposalReviewError {
    #[error("legacy proposals can only receive open or rejected review records")]
    LegacyProposal,
    #[error("proposal review does not match the immutable proposal identity")]
    ProposalIdentity,
    #[error("selected recommendation IDs contain a duplicate")]
    DuplicateSelection,
    #[error("selected recommendation IDs must be in canonical order")]
    SelectionOrder,
    #[error("selected recommendation does not exist in the proposal: {0}")]
    UnknownRecommendation(String),
    #[error("this review state does not permit selected recommendations")]
    UnexpectedSelection,
    #[error("full plan approval requires data recommendations and no explicit subset")]
    FullApproval,
    #[error("partial acceptance requires a non-empty subset of data recommendations")]
    PartialSelection,
    #[error("training acceptance requires a non-empty subset of training recommendations")]
    TrainingSelection,
    #[error("review-only acceptance requires a non-empty subset of advisory recommendations")]
    ReviewOnlySelection,
    #[error("superseded reviews require a superseding proposal or campaign reference")]
    SupersedingReference,
    #[error("completion awaiting assessment requires a campaign reference")]
    CampaignReference,
    #[error("review state is not compatible with generation-plan creation")]
    NotApprovedForPlan,
    #[error("proposal review fingerprint does not reproduce")]
    ReviewFingerprint,
    #[error("could not fingerprint proposal review: {0}")]
    Fingerprint(String),
}
