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
    pub provider: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub system_prompt: String,
    pub initial_prompt: String,
    pub max_model_turns: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResearchAgentEvent {
    AgentStarted,
    ModelTurnStarted {
        sequence: u32,
    },
    ModelTurnCompleted {
        sequence: u32,
        input_tokens: u64,
        output_tokens: u64,
        cost_microusd: u64,
    },
    AgentText {
        text: String,
    },
    ToolStarted {
        external_call_id: String,
        name: String,
    },
    ToolCompleted {
        external_call_id: String,
        name: String,
        failed: bool,
    },
    AgentFinished {
        turns: u32,
        aborted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolRequest {
    pub external_call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResearchAgentMessage {
    Event { event: ResearchAgentEvent },
    ToolRequest { request: AgentToolRequest },
    Completed,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolResult {
    pub content: serde_json::Value,
    #[serde(default)]
    pub details: serde_json::Value,
    #[serde(default)]
    pub terminate: bool,
}

pub trait ResearchAgentSession: Send {
    fn next_message(&mut self)
    -> BoxFuture<'_, Result<ResearchAgentMessage, ResearchAdapterError>>;

    fn send_tool_result(
        &mut self,
        external_call_id: &str,
        result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn send_tool_error(
        &mut self,
        external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn cancel(&mut self, run_id: Uuid) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;
}

pub trait ResearchAgentRuntime: Send + Sync {
    fn start(
        &self,
        request: ResearchAgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn ResearchAgentSession>, ResearchAdapterError>>;
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

    fn save_profile_bundle(
        &self,
        claims: &[ResearchClaim],
        profile: &AuthenticityProfile,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>>;

    fn latest_profile_for_run(
        &self,
        run_id: Uuid,
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
