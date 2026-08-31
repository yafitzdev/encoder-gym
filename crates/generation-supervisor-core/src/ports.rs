//! Persistence boundary used by the bounded diagnosis runner.

use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::{
    SupervisorError,
    advisor::{AdvisorModelCall, AdvisorSession, AdvisorToolCall, GenerationQualityDiagnosisBrief},
    contract::GenerationQualityContract,
    decision::DeterministicQualityDecision,
    lifecycle::{ChildOutcome, ChildReservation, SupervisorRun, SupervisorRunEvent},
    observation::{BatchQualityObservation, QualityEvidenceManifest, RowQualityObservation},
    revision::{
        PromptGuidanceVersion, PromptRevisionActivation, PromptRevisionAuthorization,
        PromptRevisionProposal, PromptRevisionReview, SupervisorDiagnosis,
    },
    strategy::StrategyAssignmentSet,
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

    fn list_sessions(
        &self,
        supervisor_run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<AdvisorSession>, SupervisorError>>;

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

    fn get_proposal(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionProposal>, SupervisorError>>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisedRowTrace {
    pub contract: GenerationQualityContract,
    pub run: SupervisorRun,
    pub prompt_version: PromptGuidanceVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_set: Option<StrategyAssignmentSet>,
    pub row_observation: RowQualityObservation,
    pub windows: Vec<BatchQualityObservation>,
    pub decisions: Vec<DeterministicQualityDecision>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorIntegrityReport {
    pub contracts: u64,
    pub runs: u64,
    pub prompt_versions: u64,
    pub strategy_assignments: u64,
    pub row_observations: u64,
    pub quality_windows: u64,
    pub decisions: u64,
    pub advisor_sessions: u64,
    pub revision_proposals: u64,
    pub activations: u64,
    #[serde(default)]
    pub errors: Vec<String>,
}

impl SupervisorIntegrityReport {
    pub fn healthy(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Complete durable boundary for supervisor facts. Methods that create a
/// parent and first child together are intentionally transactional.
pub trait GenerationSupervisorStore: Send + Sync {
    fn create_contract(
        &self,
        contract: &GenerationQualityContract,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_contract(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationQualityContract>, SupervisorError>>;

    fn create_run(
        &self,
        run: &SupervisorRun,
        initial_prompt: &PromptGuidanceVersion,
        assignments: Option<&StrategyAssignmentSet>,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_supervisor_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisorRun>, SupervisorError>>;

    fn request_supervisor_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, SupervisorError>>;

    fn supervisor_cancel_requested(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<bool>, SupervisorError>>;

    fn append_run_event(
        &self,
        event: &SupervisorRunEvent,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_run_events(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SupervisorRunEvent>, SupervisorError>>;

    fn reserve_child(
        &self,
        reservation: &ChildReservation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn finish_child(&self, outcome: &ChildOutcome) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_child_reservations(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ChildReservation>, SupervisorError>>;

    fn list_child_outcomes(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ChildOutcome>, SupervisorError>>;

    fn get_strategy_assignments(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<StrategyAssignmentSet>, SupervisorError>>;

    fn save_prompt_version(
        &self,
        version: &PromptGuidanceVersion,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_prompt_version(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptGuidanceVersion>, SupervisorError>>;

    fn list_prompt_versions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<PromptGuidanceVersion>, SupervisorError>>;

    fn save_row_observations(
        &self,
        rows: &[RowQualityObservation],
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_row_observations(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityObservation>, SupervisorError>>;

    fn save_quality_window(
        &self,
        manifest: &QualityEvidenceManifest,
        observation: &BatchQualityObservation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_quality_window(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BatchQualityObservation>, SupervisorError>>;

    fn list_quality_windows(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BatchQualityObservation>, SupervisorError>>;

    fn save_decision(
        &self,
        decision: &DeterministicQualityDecision,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn list_decisions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<DeterministicQualityDecision>, SupervisorError>>;

    fn append_revision_review(
        &self,
        review: &PromptRevisionReview,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn latest_revision_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionReview>, SupervisorError>>;

    fn save_revision_authorization(
        &self,
        authorization: &PromptRevisionAuthorization,
        candidate_version: &PromptGuidanceVersion,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_revision_authorization(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionAuthorization>, SupervisorError>>;

    fn save_revision_activation(
        &self,
        activation: &PromptRevisionActivation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>>;

    fn get_revision_activation(
        &self,
        prompt_version_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionActivation>, SupervisorError>>;

    fn trace_supervised_row(
        &self,
        generated_row_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisedRowTrace>, SupervisorError>>;

    fn verify_supervisor_integrity(
        &self,
    ) -> BoxFuture<'_, Result<SupervisorIntegrityReport, SupervisorError>>;
}
