//! Replaceable evaluator, source, and persistence ports.

use std::{future::Future, pin::Pin};

use dataset_core::domain::{DatasetSnapshot, SnapshotMember, SourceRow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    assessment::{
        BlindEvaluatorRequest, EvaluatorGuidance, EvaluatorIdentity, RowAssessmentDraft,
        RowQualityAssessment,
    },
    curation::{
        ApprovedCurationManifest, CurationApplication, CurationManifestReview, CurationProposal,
        DatasetQualityReport, RowQualityReview,
    },
    lifecycle::{
        AuditExecutionLease, AuditStatusProjection, EvaluatorAttempt, EvaluatorFailureKind,
        ProviderUsage, QualityAuditRun,
    },
    native_assessment::{
        NativeBlindAssessmentDraft, NativeBlindAssessmentRequest, NativeReviewUsage,
        NativeTargetFitDraft, NativeTargetFitRequest,
    },
    population::{AuditPlan, CheckedAuditPlan},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("dataset quality adapter failed: {0}")]
pub struct QualityAdapterError(pub String);

/// Loads authoritative immutable training-candidate rows. Implementations must
/// return rows in canonical source-row ID order and withhold rows assigned to a
/// non-training evaluation cohort's declared snapshot split. An explicitly
/// training-role cohort may remain eligible; categorically sealed or external
/// contents never do, even after a later role decision. Missing, ineligible,
/// or cross-dataset identities must be rejected rather than returned as a
/// partial set.
pub trait QualityCandidateSource: Send + Sync {
    fn list_source_rows(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>>;

    fn get_source_rows(
        &self,
        dataset_id: Uuid,
        source_row_ids: Vec<Uuid>,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorBatchOutput {
    pub assessments: Vec<RowAssessmentDraft>,
    pub usage: ProviderUsage,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityEvaluationErrorKind {
    Configuration,
    Authentication,
    InvalidResponse,
    RateLimit,
    Transport,
    Provider,
}

impl QualityEvaluationErrorKind {
    /// Retry only failures which can plausibly be transient. Invalid output is
    /// durable row evidence and must follow the audit's explicit invalid-output
    /// policy instead of being hidden by an unbounded normalization retry.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::RateLimit | Self::Transport | Self::Provider)
    }

    pub const fn attempt_failure_kind(self) -> Option<EvaluatorFailureKind> {
        match self {
            Self::Configuration => Some(EvaluatorFailureKind::Configuration),
            Self::Authentication => Some(EvaluatorFailureKind::Authentication),
            Self::InvalidResponse => None,
            Self::RateLimit => Some(EvaluatorFailureKind::RateLimit),
            Self::Transport => Some(EvaluatorFailureKind::Transport),
            Self::Provider => Some(EvaluatorFailureKind::Provider),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("quality evaluator {kind:?} failure: {message}")]
pub struct QualityEvaluationError {
    pub kind: QualityEvaluationErrorKind,
    pub message: String,
    /// Provider guidance only. The runner still applies its own finite retry
    /// policy and never treats this as permission to exceed the audit budget.
    pub retry_after_millis: Option<u64>,
    /// Actual provider usage observed before normalization failed. Transport
    /// failures normally leave this at zero; invalid content may still carry
    /// billable usage which must not disappear from durable evidence.
    pub observed_usage: ProviderUsage,
    /// Bounded, redacted provider metadata. Raw response bodies and secrets do
    /// not belong here; `EvaluatorAttempt` applies the final size limit.
    pub metadata: Value,
}

impl QualityEvaluationError {
    pub fn new(kind: QualityEvaluationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after_millis: None,
            observed_usage: ProviderUsage::default(),
            metadata: Value::Null,
        }
    }

    pub fn with_retry_after_millis(mut self, value: u64) -> Self {
        self.retry_after_millis = Some(value);
        self
    }

    pub fn with_observed_evidence(mut self, usage: ProviderUsage, metadata: Value) -> Self {
        self.observed_usage = usage;
        self.metadata = metadata;
        self
    }
}

/// Provider-neutral semantic-quality evaluator. Prompt construction and strict
/// response normalization belong on the application side of this port; HTTP
/// or model-runtime types must not cross it.
pub trait QualityEvaluator: Send + Sync {
    fn identity(&self) -> EvaluatorIdentity;

    fn evaluate(
        &self,
        request: BlindEvaluatorRequest,
    ) -> BoxFuture<'_, Result<EvaluatorBatchOutput, QualityEvaluationError>>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBlindBatchOutput {
    pub assessments: Vec<NativeBlindAssessmentDraft>,
    pub usage: NativeReviewUsage,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTargetFitBatchOutput {
    pub assessments: Vec<NativeTargetFitDraft>,
    pub usage: NativeReviewUsage,
    #[serde(default)]
    pub metadata: Value,
}

/// Provider-neutral two-pass native reviewer. Implementations receive separate
/// label-blind and target-fit request types; inherited labels never cross this
/// port.
pub trait NativeSemanticReviewer: Send + Sync {
    fn native_identity(&self) -> EvaluatorIdentity;

    fn assess_blind(
        &self,
        request: NativeBlindAssessmentRequest,
    ) -> BoxFuture<'_, Result<NativeBlindBatchOutput, QualityEvaluationError>>;

    fn assess_target_fit(
        &self,
        request: NativeTargetFitRequest,
    ) -> BoxFuture<'_, Result<NativeTargetFitBatchOutput, QualityEvaluationError>>;
}

/// Durable quality evidence and curation storage. Transactional methods make
/// the important boundaries explicit: an evaluator attempt and its normalized
/// assessments advance together, and a curated snapshot is linked to its
/// approved manifest in the same commit.
pub trait DatasetQualityStore: Send + Sync {
    fn create_audit(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        guidance: &EvaluatorGuidance,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn get_audit_plan(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuditPlan>, QualityAdapterError>>;

    /// Loads the exact normalized guidance persisted with the immutable plan.
    /// Historical execution must not resolve a newer "current" binding.
    fn get_audit_guidance(
        &self,
        plan_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluatorGuidance>, QualityAdapterError>>;

    fn get_audit_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<QualityAuditRun>, QualityAdapterError>>;

    /// Reads only the durable cancellation flag used between evaluator calls.
    /// Full run loading remains the checked status/report boundary.
    fn audit_cancel_requested(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<bool>, QualityAdapterError>>;

    /// Loads bounded, checked run/plan/counter facts for high-frequency
    /// status polling without hydrating full population evidence.
    fn get_audit_status(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuditStatusProjection>, QualityAdapterError>>;

    fn save_audit_run(
        &self,
        run: &QualityAuditRun,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    /// Atomically claims one audit for this exact local invocation. A live
    /// owner, including another task in the same process, must be rejected.
    fn acquire_audit_execution_lease(
        &self,
        run_id: Uuid,
        invocation_token: Uuid,
    ) -> BoxFuture<'_, Result<AuditExecutionLease, QualityAdapterError>>;

    /// Releases ownership only when every persisted owner fact, especially
    /// the per-invocation fencing token, still matches.
    fn release_audit_execution_lease(
        &self,
        lease: &AuditExecutionLease,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    /// Persists one attempt reservation using a plan that the application
    /// already verified and indexed for this execution. Implementations must
    /// still compare the checked plan identity with durable plan facts inside
    /// the write transaction; this path exists to avoid rehydrating the full
    /// immutable population for every bounded evaluator batch.
    fn record_attempt<'a>(
        &'a self,
        checked: &'a CheckedAuditPlan<'a>,
        lease: &'a AuditExecutionLease,
        attempt: &'a EvaluatorAttempt,
        request: &'a BlindEvaluatorRequest,
        run: &'a QualityAuditRun,
    ) -> BoxFuture<'a, Result<(), QualityAdapterError>>;

    fn get_evaluator_request(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BlindEvaluatorRequest>, QualityAdapterError>>;

    fn list_evaluator_requests(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BlindEvaluatorRequest>, QualityAdapterError>>;

    /// Atomically closes one bounded attempt using the execution's already
    /// checked plan index and exact row-local predecessor evidence.
    fn finish_attempt<'a>(
        &'a self,
        checked: &'a CheckedAuditPlan<'a>,
        lease: &'a AuditExecutionLease,
        attempt: &'a EvaluatorAttempt,
        assessments: &'a [RowQualityAssessment],
        run: &'a QualityAuditRun,
    ) -> BoxFuture<'a, Result<(), QualityAdapterError>>;

    fn list_attempts(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<EvaluatorAttempt>, QualityAdapterError>>;

    fn list_assessments(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityAssessment>, QualityAdapterError>>;

    fn get_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RowQualityAssessment>, QualityAdapterError>>;

    fn save_report(
        &self,
        report: &DatasetQualityReport,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn get_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>>;

    fn report_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>>;

    fn append_row_review(
        &self,
        review: &RowQualityReview,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn list_row_reviews(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityReview>, QualityAdapterError>>;

    fn save_curation_proposal(
        &self,
        proposal: &CurationProposal,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn get_curation_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>>;

    fn latest_curation_proposal(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>>;

    fn append_manifest_review(
        &self,
        review: &CurationManifestReview,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn list_manifest_reviews(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CurationManifestReview>, QualityAdapterError>>;

    fn save_manifest(
        &self,
        manifest: &ApprovedCurationManifest,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn get_manifest(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>>;

    fn manifest_for_proposal(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>>;

    fn apply_manifest(
        &self,
        application: &CurationApplication,
        snapshot: &DatasetSnapshot,
        members: &[SnapshotMember],
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>>;

    fn curation_application_for_snapshot(
        &self,
        snapshot_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>>;

    fn curation_application_for_manifest(
        &self,
        manifest_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>>;
}
