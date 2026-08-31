//! Persistence boundary used by the bounded diagnosis runner.

use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::{
    SupervisorError,
    advisor::{AdvisorModelCall, AdvisorSession, AdvisorToolCall, GenerationQualityDiagnosisBrief},
    revision::{PromptRevisionProposal, SupervisorDiagnosis},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SupervisorAdvisorStore: Send + Sync {
    fn create_session(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_brief(
        &self,
        brief_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationQualityDiagnosisBrief>, SupervisorError>>;

    fn get_session(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AdvisorSession>, SupervisorError>>;

    fn save_session(&self, session: &AdvisorSession) -> BoxFuture<'_, Result<(), SupervisorError>>;

    /// Reserves every possible model turn before the runtime process can make
    /// provider I/O. Unused reservations remain explicit facts.
    fn reserve_model_calls(
        &self,
        calls: &[AdvisorModelCall],
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_model_calls(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<AdvisorModelCall>, SupervisorError>>;

    fn save_model_call_and_session(
        &self,
        call: &AdvisorModelCall,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn save_tool_call(&self, call: &AdvisorToolCall) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn save_tool_call_and_session(
        &self,
        call: &AdvisorToolCall,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_tool_calls(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<AdvisorToolCall>, SupervisorError>>;

    fn save_diagnosis(
        &self,
        session_id: Uuid,
        diagnosis: &SupervisorDiagnosis,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn save_proposal(
        &self,
        session_id: Uuid,
        proposal: &PromptRevisionProposal,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn latest_diagnosis(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisorDiagnosis>, SupervisorError>>;

    fn latest_proposal(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionProposal>, SupervisorError>>;
}
