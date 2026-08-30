pub use agent_runtime_core::{
    AgentAdapterError as ArchitectAdapterError, AgentEvent as ArchitectAgentEvent,
    AgentMessage as ArchitectAgentMessage, AgentRequest as ArchitectAgentRequest,
    AgentRuntime as ArchitectAgentRuntime, AgentSession as ArchitectAgentSession, AgentToolRequest,
    AgentToolResult, BoxFuture,
};

use uuid::Uuid;

use crate::{
    brief::ResolvedArchitectBrief,
    lifecycle::{ArchitectRun, ArchitectToolCall},
    proposal::{
        ArchitectProposalReview, DatasetArchitectureApplication, DatasetArchitectureProposal,
    },
};

pub trait ArchitectStore: Send + Sync {
    fn create_run(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedArchitectBrief>, ArchitectAdapterError>>;

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectRun>, ArchitectAdapterError>>;

    fn save_run(&self, run: &ArchitectRun) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn record_tool_call(
        &self,
        call: &ArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ArchitectToolCall>, ArchitectAdapterError>>;

    /// Atomically publishes the proposal and terminal run projection.
    fn save_proposal_and_run(
        &self,
        proposal: &DatasetArchitectureProposal,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>>;

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>>;

    fn append_review(
        &self,
        review: &ArchitectProposalReview,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectProposalReview>, ArchitectAdapterError>>;

    fn save_application(
        &self,
        application: &DatasetArchitectureApplication,
        plan: &generation_core::domain::GenerationPlan,
        strategy: &generation_core::strategy::ResolvedGenerationStrategyContext,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>>;

    fn get_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureApplication>, ArchitectAdapterError>>;
}
