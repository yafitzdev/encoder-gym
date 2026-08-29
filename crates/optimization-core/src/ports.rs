use std::{future::Future, pin::Pin};

use generation_core::domain::GenerationPlan;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    campaigns::{CampaignArtifactLink, CampaignOutcomeAssessment, OptimizationCampaign},
    domain::{OptimizationProposal, ProposalApplication},
    reviews::{ProposalReviewRecord, ProposalReviewState},
    scenarios::OptimizationScenarioGroup,
};

#[derive(Debug, Clone, Copy)]
pub struct OptimizationProposalQuery {
    pub analysis_report_id: Option<Uuid>,
    pub dataset_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ProposalReviewQuery {
    pub proposal_id: Option<Uuid>,
    pub state: Option<ProposalReviewState>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct CampaignQuery {
    pub proposal_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("optimization persistence operation failed: {0}")]
pub struct OptimizationStoreError(pub String);

pub trait OptimizationStore: Send + Sync {
    fn create_optimization_proposal(
        &self,
        proposal: &OptimizationProposal,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn get_optimization_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationProposal>, OptimizationStoreError>>;

    fn list_optimization_proposals(
        &self,
    ) -> BoxFuture<'_, Result<Vec<OptimizationProposal>, OptimizationStoreError>>;
    fn query_optimization_proposals(
        &self,
        query: OptimizationProposalQuery,
    ) -> BoxFuture<'_, Result<Vec<OptimizationProposal>, OptimizationStoreError>>;

    fn record_proposal_application(
        &self,
        application: &ProposalApplication,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    /// Atomically persists a derived generation plan and its application marker.
    /// Repeating the call returns the original application without creating another plan.
    fn apply_proposal_plan(
        &self,
        plan: &GenerationPlan,
        application: &ProposalApplication,
    ) -> BoxFuture<'_, Result<ProposalApplication, OptimizationStoreError>>;

    fn get_proposal_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProposalApplication>, OptimizationStoreError>>;

    fn create_scenario_group(
        &self,
        group: &OptimizationScenarioGroup,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn get_scenario_group(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationScenarioGroup>, OptimizationStoreError>>;

    fn list_scenario_groups(
        &self,
    ) -> BoxFuture<'_, Result<Vec<OptimizationScenarioGroup>, OptimizationStoreError>>;

    fn append_proposal_review(
        &self,
        review: &ProposalReviewRecord,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn query_proposal_reviews(
        &self,
        query: ProposalReviewQuery,
    ) -> BoxFuture<'_, Result<Vec<ProposalReviewRecord>, OptimizationStoreError>>;

    fn create_campaign(
        &self,
        campaign: &OptimizationCampaign,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn get_campaign(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationCampaign>, OptimizationStoreError>>;

    fn query_campaigns(
        &self,
        query: CampaignQuery,
    ) -> BoxFuture<'_, Result<Vec<OptimizationCampaign>, OptimizationStoreError>>;

    fn append_campaign_link(
        &self,
        link: &CampaignArtifactLink,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn list_campaign_links(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CampaignArtifactLink>, OptimizationStoreError>>;

    fn create_campaign_outcome(
        &self,
        outcome: &CampaignOutcomeAssessment,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn get_campaign_outcome(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CampaignOutcomeAssessment>, OptimizationStoreError>>;
}
