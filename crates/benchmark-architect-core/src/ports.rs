pub use agent_runtime_core::{
    AgentAdapterError as BenchmarkArchitectAdapterError,
    AgentEvent as BenchmarkArchitectAgentEvent, AgentMessage as BenchmarkArchitectAgentMessage,
    AgentRequest as BenchmarkArchitectAgentRequest, AgentRuntime as BenchmarkArchitectAgentRuntime,
    AgentSession as BenchmarkArchitectAgentSession, AgentToolRequest, AgentToolResult, BoxFuture,
};
pub use research_core::ports::{PageFetcher, SearchProvider};

use research_core::evidence::ResearchEvidence;
use uuid::Uuid;

use crate::{
    blueprint::{
        BenchmarkAcquisitionHandoff, BenchmarkArchitectureProposal, BenchmarkArchitectureReview,
    },
    brief::ResolvedBenchmarkArchitectBrief,
    lifecycle::{BenchmarkArchitectRun, BenchmarkArchitectToolCall},
};

pub trait BenchmarkArchitectStore: Send + Sync {
    fn create_run(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<
        '_,
        Result<Option<ResolvedBenchmarkArchitectBrief>, BenchmarkArchitectAdapterError>,
    >;

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectRun>, BenchmarkArchitectAdapterError>>;

    fn save_run(
        &self,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn record_tool_call(
        &self,
        call: &BenchmarkArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkArchitectToolCall>, BenchmarkArchitectAdapterError>>;

    fn record_evidence(
        &self,
        evidence: &ResearchEvidence,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn list_evidence(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchEvidence>, BenchmarkArchitectAdapterError>>;

    fn get_evidence(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResearchEvidence>, BenchmarkArchitectAdapterError>>;

    /// Atomically publishes the proposal and terminal run projection.
    fn save_proposal_and_run(
        &self,
        proposal: &BenchmarkArchitectureProposal,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>;

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>;

    fn append_review(
        &self,
        review: &BenchmarkArchitectureReview,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>;

    fn get_review(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>;

    fn save_handoff(
        &self,
        handoff: &BenchmarkAcquisitionHandoff,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>>;

    fn get_handoff(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>;

    fn get_handoff_by_id(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>;
}
