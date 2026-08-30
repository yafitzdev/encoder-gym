use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    brief::ResolvedResearchBrief,
    evidence::{
        FetchRequest, ResearchClaim, ResearchEvidence, SearchRequest, SearchResult, UntrustedPage,
    },
    lifecycle::{ResearchRun, ResearchToolCall},
    profile::{
        AuthenticityProfile, GenerationAuthenticityAssignment, ProfileBinding, ProfileReview,
        ResolvedAuthenticityContext,
    },
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("research adapter failed: {0}")]
pub struct ResearchAdapterError(pub String);

pub trait SearchProvider: Send + Sync {
    fn search(
        &self,
        request: SearchRequest,
    ) -> BoxFuture<'_, Result<Vec<SearchResult>, ResearchAdapterError>>;
}

pub trait PageFetcher: Send + Sync {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> BoxFuture<'_, Result<UntrustedPage, ResearchAdapterError>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchAgentRequest {
    pub protocol_version: u32,
    pub run_id: Uuid,
    pub run_specification_fingerprint: String,
    pub brief: ResolvedResearchBrief,
    pub enabled_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResearchAgentEvent {
    Plan {
        steps: Vec<String>,
    },
    ModelTurnStarted {
        sequence: u32,
    },
    ModelTurnCompleted {
        sequence: u32,
        input_tokens: u64,
        output_tokens: u64,
        cost_microusd: u64,
    },
    ToolCallRequested {
        call: ResearchToolCall,
    },
    Status {
        message: String,
    },
    ProfileReady,
    Finished {
        reason: String,
    },
}

pub trait ResearchAgentRuntime: Send + Sync {
    fn run(
        &self,
        request: ResearchAgentRequest,
    ) -> BoxFuture<'_, Result<Vec<ResearchAgentEvent>, ResearchAdapterError>>;

    fn cancel(&self, run_id: Uuid) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;
}

pub trait ResearchStore: Send + Sync {
    fn create_run(
        &self,
        brief: &ResolvedResearchBrief,
        run: &ResearchRun,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedResearchBrief>, ResearchAdapterError>>;

    fn get_run(&self, id: Uuid)
    -> BoxFuture<'_, Result<Option<ResearchRun>, ResearchAdapterError>>;

    fn save_run(&self, run: &ResearchRun) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn record_tool_call(
        &self,
        call: &ResearchToolCall,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchToolCall>, ResearchAdapterError>>;

    fn record_evidence(
        &self,
        evidence: &ResearchEvidence,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn list_evidence(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchEvidence>, ResearchAdapterError>>;

    fn record_claim(
        &self,
        claim: &ResearchClaim,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn list_claims(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchClaim>, ResearchAdapterError>>;

    fn save_profile(
        &self,
        profile: &AuthenticityProfile,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn get_profile(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuthenticityProfile>, ResearchAdapterError>>;

    fn append_review(
        &self,
        review: &ProfileReview,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn latest_review(
        &self,
        profile_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProfileReview>, ResearchAdapterError>>;

    fn append_binding(
        &self,
        binding: &ProfileBinding,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn resolve_context(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedAuthenticityContext>, ResearchAdapterError>>;

    fn save_generation_authenticity(
        &self,
        assignment: &GenerationAuthenticityAssignment,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn get_generation_authenticity(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationAuthenticityAssignment>, ResearchAdapterError>>;
}
