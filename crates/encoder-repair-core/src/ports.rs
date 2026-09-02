use std::{future::Future, pin::Pin};

use thiserror::Error;
use uuid::Uuid;

use crate::collection::{CollectedDevelopmentObservations, DevelopmentObservationRequest};
use crate::{
    diagnosis::ComparativeDiagnosis,
    observation::DevelopmentObservationSet,
    proposal::{RepairProposal, RepairProposalApplication, RepairProposalReview},
};
use encoder_experiment_core::domain::{BackendIdentity, ExternalProjectSnapshot};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("encoder repair evidence store failed: {0}")]
pub struct RepairEvidenceStoreError(pub String);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("development observation backend failed: {0}")]
pub struct DevelopmentObservationBackendError(pub String);

/// Replaceable native adapter for complete, text-free development observations.
///
/// The request type can represent `Development` evidence only. Native dataset rows and model
/// execution remain behind this port; the normalized result contains no row text or native label.
pub trait DevelopmentObservationBackend: Send + Sync {
    fn observer_identity(&self) -> BackendIdentity;

    fn collect_development_observations(
        &self,
        project: ExternalProjectSnapshot,
        request: DevelopmentObservationRequest,
    ) -> BoxFuture<'_, Result<CollectedDevelopmentObservations, DevelopmentObservationBackendError>>;
}

/// Append-only persistence for provider-neutral production-repair evidence.
///
/// Implementations must validate artifact integrity and exact source bindings before insert.
/// Stable evidence/derivation fingerprints are idempotency keys; a duplicate key may only
/// resolve to the already persisted identical artifact.
pub trait RepairEvidenceStore: Send + Sync {
    fn create_observation_set(
        &self,
        observation_set: DevelopmentObservationSet,
    ) -> BoxFuture<'_, Result<DevelopmentObservationSet, RepairEvidenceStoreError>>;

    fn get_observation_set(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DevelopmentObservationSet>, RepairEvidenceStoreError>>;

    fn find_observation_set_by_evidence(
        &self,
        evidence_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<DevelopmentObservationSet>, RepairEvidenceStoreError>>;

    fn list_observation_sets_for_campaign(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<DevelopmentObservationSet>, RepairEvidenceStoreError>>;

    fn create_diagnosis(
        &self,
        diagnosis: ComparativeDiagnosis,
    ) -> BoxFuture<'_, Result<ComparativeDiagnosis, RepairEvidenceStoreError>>;

    fn get_diagnosis(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ComparativeDiagnosis>, RepairEvidenceStoreError>>;

    fn find_diagnosis_by_derivation(
        &self,
        derivation_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ComparativeDiagnosis>, RepairEvidenceStoreError>>;

    fn list_diagnoses_for_campaign(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ComparativeDiagnosis>, RepairEvidenceStoreError>>;

    fn create_proposal(
        &self,
        proposal: RepairProposal,
    ) -> BoxFuture<'_, Result<RepairProposal, RepairEvidenceStoreError>>;

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RepairProposal>, RepairEvidenceStoreError>>;

    fn find_proposal_by_specification(
        &self,
        specification_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<RepairProposal>, RepairEvidenceStoreError>>;

    fn append_proposal_review(
        &self,
        review: RepairProposalReview,
    ) -> BoxFuture<'_, Result<RepairProposalReview, RepairEvidenceStoreError>>;

    fn list_proposal_reviews(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RepairProposalReview>, RepairEvidenceStoreError>>;

    fn reserve_proposal_application(
        &self,
        application: RepairProposalApplication,
    ) -> BoxFuture<'_, Result<RepairProposalApplication, RepairEvidenceStoreError>>;

    fn get_proposal_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RepairProposalApplication>, RepairEvidenceStoreError>>;
}
