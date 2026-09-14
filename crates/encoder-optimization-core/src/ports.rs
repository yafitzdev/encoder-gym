use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::{
    OptimizationError,
    agent::{AgentAnalysisScope, AgentCallReservation, AgentTurnRecord, InspectionPage},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, OptimizationError>> + Send + 'a>>;

/// Adapters return only the pinned development report's persisted failures and
/// the selected training version. There is deliberately no arbitrary read tool.
pub trait OptimizationInspection: Send + Sync {
    fn development_failures(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage>;
    fn training_rows(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
        query: Option<String>,
    ) -> BoxFuture<'_, InspectionPage>;
}

/// Implementations atomically enforce cumulative launch-provider budgets,
/// including pending/unknown calls. A reservation must precede runtime.start.
pub trait OptimizationAgentStore: Send + Sync {
    fn history(&self, scope: AgentAnalysisScope) -> BoxFuture<'_, Vec<AgentTurnRecord>>;
    fn reserve(&self, scope: AgentAnalysisScope, call: AgentCallReservation) -> BoxFuture<'_, ()>;
    fn finish(&self, scope: AgentAnalysisScope, record: AgentTurnRecord) -> BoxFuture<'_, ()>;
    fn stopped(&self, run_id: Uuid) -> BoxFuture<'_, bool>;
    fn public_explanation(
        &self,
        scope: AgentAnalysisScope,
        call_id: Uuid,
        text: String,
    ) -> BoxFuture<'_, ()>;
}

/// Implements task-owned native schema and local duplicate admission. Complete
/// training-to-benchmark qualification is a separate gate before training.
/// It receives only a pinned training template and generated output, never an
/// arbitrary file path or an agent-selected evaluation population.
pub trait OptimizationGenerationAdmission: Send + Sync {
    fn admit(
        &self,
        task: crate::generation::GenerationTask,
        content: String,
    ) -> BoxFuture<'_, crate::generation::GenerationAdmission>;
}

pub trait OptimizationGenerationStore: Send + Sync {
    fn history(
        &self,
        task: crate::generation::GenerationTask,
    ) -> BoxFuture<'_, Vec<crate::generation::GenerationOutcome>>;
    /// Atomically enforce launch budgets and in-flight concurrency before I/O.
    fn reserve(
        &self,
        task: crate::generation::GenerationTask,
        call: crate::generation::GenerationReservation,
    ) -> BoxFuture<'_, ()>;
    fn finish(
        &self,
        task: crate::generation::GenerationTask,
        outcome: crate::generation::GenerationOutcome,
    ) -> BoxFuture<'_, ()>;
    fn stopped(&self, run_id: Uuid) -> BoxFuture<'_, bool>;
}
