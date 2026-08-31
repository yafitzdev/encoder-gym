use std::collections::{BTreeMap, BTreeSet};

use analysis_core::ports::AnalysisStore;
use artifact_core::{
    ArtifactKind, BoxFuture, ProvenanceNode, ProvenanceStore, ProvenanceStoreError,
};
use dataset_architect_core::{
    brief::ResolvedArchitectBrief,
    lifecycle::ArchitectRun,
    proposal::{
        ArchitectProposalReview, DatasetArchitectureApplication, DatasetArchitectureProposal,
    },
};
use dataset_core::{
    domain::{DatasetImport, DatasetSnapshot, SnapshotMember, SourceProvenance, SourceRow},
    ports::{ImportStore, SnapshotStore},
};
use dataset_quality_core::{
    assessment::{
        AuthenticityEvaluatorGuidance, BlindEvaluatorRequest, RowQualityAssessment,
        SemanticEvaluatorGuidance,
    },
    curation::{
        ApprovedCurationManifest, CurationApplication, CurationDecision, CurationDecisionBasis,
        CurationDisposition, CurationManifestMember, CurationManifestReview,
        CurationManifestReviewDecision, CurationProposal, CurationProposalEntry,
        DatasetQualityReport, RowQualityReview,
    },
    lifecycle::{EvaluatorAttempt, EvaluatorAttemptState, QualityAuditRun},
    population::{AuditPlan, AuditPlanItem},
    ports::DatasetQualityStore,
};
use evaluation_core::ports::EvaluationStore;
use generation_core::strategy::ResolvedGenerationStrategyContext;
use generation_core::{
    deduplication::normalize_text,
    domain::{GenerationCell, GenerationPlan, ValidationStatus},
    jobs::GenerationJob,
    ports::{DatasetStore, GenerationExecutionStore, JobStore, PlanStore},
};
use optimization_core::{
    campaigns::{CampaignArtifactKind, CampaignArtifactLink, CampaignOutcomeAssessment},
    ports::OptimizationStore,
    reviews::ProposalReviewRecord,
};
use project_preparation::{BootstrapStore, PreparationStore};
use research_core::{
    evidence::{ResearchClaim, ResearchEvidence},
    ports::ResearchStore,
    profile::{ProfileBinding, ProfileReview, ProfileReviewDecision},
};
use semantic_catalog::{
    SemanticBindingDecision, SemanticCatalogStore, SemanticLayer, SemanticScope,
};
use serde::Serialize;
use serde_json::json;
use sqlx::{FromRow, Row};
use training_core::ports::{EncoderRegistry, TrainingStore};
use uuid::Uuid;
use workflow_core::ports::{
    AdvisorStore, BenchmarkBundleStore, BenchmarkQualificationStore, BenchmarkStore,
    ContaminationStore, InitialAllocationStore, PromotionStore, StopDecisionStore,
    TrainingBenchmarkCheckStore, WorkflowApprovalStore, WorkflowRunStore,
};

use super::SqliteStore;

impl ProvenanceStore for SqliteStore {
    fn trace_provenance(
        &self,
        kind: ArtifactKind,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProvenanceNode>, ProvenanceStoreError>> {
        Box::pin(async move {
            match kind {
                ArtifactKind::ProjectBootstrap => self.project_bootstrap_node(id).await,
                ArtifactKind::ProjectPreparation => self.project_preparation_node(id).await,
                ArtifactKind::ProjectConfiguration => self.configuration_node(id).await,
                ArtifactKind::Dataset => self.dataset_node(id).await,
                ArtifactKind::SemanticProfile => self.semantic_profile_node(id).await,
                ArtifactKind::SemanticBinding => self.semantic_binding_node(id).await,
                ArtifactKind::GenerationSemanticContext => self.semantic_context_node(id).await,
                ArtifactKind::ResearchBrief => self.research_brief_node(id).await,
                ArtifactKind::ResearchRun => self.research_run_node(id).await,
                ArtifactKind::ResearchEvidence => self.research_evidence_node(id).await,
                ArtifactKind::ResearchClaim => self.research_claim_node(id).await,
                ArtifactKind::AuthenticityProfile => self.authenticity_profile_node(id).await,
                ArtifactKind::AuthenticityProfileReview => self.authenticity_review_node(id).await,
                ArtifactKind::AuthenticityProfileBinding => {
                    self.authenticity_binding_node(id).await
                }
                ArtifactKind::GenerationAuthenticityContext => {
                    self.generation_authenticity_node(id).await
                }
                ArtifactKind::DatasetArchitectBrief => {
                    Box::pin(self.architect_brief_node(id)).await
                }
                ArtifactKind::DatasetArchitectRun => Box::pin(self.architect_run_node(id)).await,
                ArtifactKind::DatasetArchitectureProposal => {
                    Box::pin(self.architecture_proposal_node(id)).await
                }
                ArtifactKind::DatasetArchitectureReview => {
                    Box::pin(self.architecture_review_node(id)).await
                }
                ArtifactKind::DatasetArchitectureApplication => {
                    Box::pin(self.architecture_application_node(id)).await
                }
                ArtifactKind::GenerationStrategyContext => {
                    Box::pin(self.generation_strategy_node(id)).await
                }
                ArtifactKind::InitialAllocation => self.initial_allocation_node(id).await,
                ArtifactKind::GenerationPlan => self.plan_node(id).await,
                ArtifactKind::GenerationJob => self.job_node(id).await,
                ArtifactKind::DatasetImport => self.import_node(id).await,
                ArtifactKind::DatasetSourceRow => self.source_row_node(id, None).await,
                ArtifactKind::QualityAuditPlan => self.quality_audit_plan_node(id).await,
                ArtifactKind::QualitySemanticGuidance => {
                    self.quality_semantic_guidance_node(id).await
                }
                ArtifactKind::QualityAuditRun => self.quality_audit_run_node(id).await,
                ArtifactKind::QualityEvaluatorAttempt => {
                    Box::pin(self.quality_evaluator_attempt_node(id)).await
                }
                ArtifactKind::RowQualityAssessment => {
                    Box::pin(self.row_quality_assessment_node(id)).await
                }
                ArtifactKind::DatasetQualityReport => {
                    Box::pin(self.dataset_quality_report_node(id)).await
                }
                ArtifactKind::RowQualityReview => Box::pin(self.row_quality_review_node(id)).await,
                ArtifactKind::CurationProposal => Box::pin(self.curation_proposal_node(id)).await,
                ArtifactKind::CurationManifestReview => {
                    Box::pin(self.curation_manifest_review_node(id)).await
                }
                ArtifactKind::ApprovedCurationManifest => {
                    Box::pin(self.approved_curation_manifest_node(id)).await
                }
                ArtifactKind::CurationApplication => {
                    Box::pin(self.curation_application_node(id)).await
                }
                ArtifactKind::Snapshot => self.snapshot_node(id).await,
                ArtifactKind::BaseModel => self.encoder_node(id).await,
                ArtifactKind::TrainingRun => self.training_node(id).await,
                ArtifactKind::Checkpoint => self.checkpoint_node(id).await,
                ArtifactKind::EvaluationRun => self.evaluation_node(id).await,
                ArtifactKind::EvaluationComparison => self.comparison_node(id).await,
                ArtifactKind::ModelSelection => self.selection_node(id).await,
                ArtifactKind::AnalysisReport => self.analysis_node(id).await,
                ArtifactKind::AnalysisFindingReview => self.analysis_review_node(id).await,
                ArtifactKind::OptimizationProposal => self.optimization_node(id).await,
                ArtifactKind::OptimizationProposalReview => self.optimization_review_node(id).await,
                ArtifactKind::OptimizationCampaign => self.campaign_node(id).await,
                ArtifactKind::OptimizationCampaignLink => self.campaign_link_node(id).await,
                ArtifactKind::OptimizationOutcome => self.optimization_outcome_node(id).await,
                ArtifactKind::BenchmarkSuite => self.benchmark_suite_node(id).await,
                ArtifactKind::ContaminationReport => self.contamination_report_node(id).await,
                ArtifactKind::BenchmarkBundle => self.benchmark_bundle_node(id).await,
                ArtifactKind::BenchmarkQualification => self.benchmark_qualification_node(id).await,
                ArtifactKind::TrainingBenchmarkCheck => {
                    self.training_benchmark_check_node(id).await
                }
                ArtifactKind::WorkflowDefinition => self.workflow_definition_node(id).await,
                ArtifactKind::WorkflowRun => self.workflow_run_node(id).await,
                ArtifactKind::AcceptanceAssessment => self.acceptance_node(id).await,
                ArtifactKind::AdvisoryAssessment => self.advisory_node(id).await,
                ArtifactKind::WorkflowApproval => self.workflow_approval_node(id).await,
                ArtifactKind::StopDecision => self.stop_decision_node(id).await,
                ArtifactKind::ModelPromotion => self.promotion_node(id).await,
            }
        })
    }
}

impl SqliteStore {
    async fn architect_brief_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT brief_json FROM dataset_architect_briefs WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: ResolvedArchitectBrief = serde_json::from_str(&value).map_err(store_error)?;
        let parents = self
            .dataset_node(artifact.dataset.id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::DatasetArchitectBrief,
            id,
            Some(artifact.fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn architect_run_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT run_json FROM dataset_architect_runs WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: ArchitectRun = serde_json::from_str(&value).map_err(store_error)?;
        let parents = Box::pin(self.architect_brief_node(artifact.brief_id))
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::DatasetArchitectRun,
            id,
            Some(artifact.specification_fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn architecture_proposal_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT proposal_json FROM dataset_architecture_proposals WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: DatasetArchitectureProposal =
            serde_json::from_str(&value).map_err(store_error)?;
        let parents = Box::pin(self.architect_run_node(artifact.run_id))
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::DatasetArchitectureProposal,
            id,
            Some(artifact.fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn architecture_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT review_json FROM dataset_architecture_reviews WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: ArchitectProposalReview =
            serde_json::from_str(&value).map_err(store_error)?;
        let mut parents = Box::pin(self.architecture_proposal_node(artifact.proposal_id))
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(predecessor) = artifact.predecessor_id
            && let Some(node) = Box::pin(self.architecture_review_node(predecessor)).await?
        {
            parents.push(node);
        }
        Ok(Some(node(
            ArtifactKind::DatasetArchitectureReview,
            id,
            Some(artifact.fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn architecture_application_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT application_json FROM dataset_architecture_applications WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: DatasetArchitectureApplication =
            serde_json::from_str(&value).map_err(store_error)?;
        let mut parents = Box::pin(self.architecture_proposal_node(artifact.proposal_id))
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(review) = Box::pin(self.architecture_review_node(artifact.approval_id)).await? {
            parents.push(review);
        }
        Ok(Some(node(
            ArtifactKind::DatasetArchitectureApplication,
            id,
            Some(artifact.fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn generation_strategy_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT context_json FROM generation_strategy_contexts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(value) = value else { return Ok(None) };
        let artifact: ResolvedGenerationStrategyContext =
            serde_json::from_str(&value).map_err(store_error)?;
        let application_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM dataset_architecture_applications WHERE strategy_context_id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let parents = match application_id {
            Some(id) => Box::pin(self.architecture_application_node(id))
                .await?
                .into_iter()
                .collect(),
            None => vec![],
        };
        Ok(Some(node(
            ArtifactKind::GenerationStrategyContext,
            id,
            Some(artifact.fingerprint.clone()),
            &artifact,
            parents,
        )?))
    }

    async fn semantic_profile_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(profile) = self.get_semantic_profile(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        if let Some(predecessor_id) = profile.predecessor_id
            && let Some(predecessor) = Box::pin(self.semantic_profile_node(predecessor_id)).await?
        {
            parents.push(predecessor);
        }
        Ok(Some(node(
            ArtifactKind::SemanticProfile,
            id,
            Some(profile.fingerprint.clone()),
            &profile,
            parents,
        )?))
    }

    async fn semantic_binding_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(payload) = sqlx::query_scalar::<_, String>(
            "SELECT artifact_json FROM dataset_semantic_binding_decisions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let binding: SemanticBindingDecision =
            serde_json::from_str(&payload).map_err(store_error)?;
        let mut parents = self
            .dataset_node(binding.dataset_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(profile_id) = binding.profile_id
            && let Some(profile) = self.semantic_profile_node(profile_id).await?
        {
            parents.push(profile);
        }
        if let Some(predecessor_id) = binding.predecessor_id
            && let Some(predecessor) = Box::pin(self.semantic_binding_node(predecessor_id)).await?
        {
            parents.push(predecessor);
        }
        Ok(Some(node(
            ArtifactKind::SemanticBinding,
            id,
            Some(binding.fingerprint.clone()),
            &binding,
            parents,
        )?))
    }

    async fn semantic_context_node(
        &self,
        job_id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(assignment) = self
            .get_generation_semantics(job_id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        for source in &assignment.context.sources {
            if let Some(binding) = self.semantic_binding_node(source.binding_id).await? {
                parents.push(binding);
            }
        }
        Ok(Some(node(
            ArtifactKind::GenerationSemanticContext,
            job_id,
            Some(assignment.fingerprint.clone()),
            &assignment,
            parents,
        )?))
    }

    async fn research_brief_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(brief) = self.get_brief(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .dataset_node(brief.dataset.id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::ResearchBrief,
            id,
            Some(brief.fingerprint.clone()),
            &brief,
            parents,
        )?))
    }

    async fn research_run_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(run) = self.get_run(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .research_brief_node(run.brief_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::ResearchRun,
            id,
            Some(run.specification_fingerprint.clone()),
            &run,
            parents,
        )?))
    }

    async fn research_evidence_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(evidence) = self
            .research_artifact::<ResearchEvidence>(
                "SELECT evidence_json FROM research_evidence WHERE id = ?",
                id,
            )
            .await?
        else {
            return Ok(None);
        };
        if evidence.reproduce_fingerprint().map_err(store_error)? != evidence.fingerprint {
            return Err(store_error("research evidence fingerprint mismatch"));
        }
        let parents = self
            .research_run_node(evidence.run_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::ResearchEvidence,
            id,
            Some(evidence.fingerprint.clone()),
            &evidence,
            parents,
        )?))
    }

    async fn research_claim_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(claim) = self
            .research_artifact::<ResearchClaim>(
                "SELECT claim_json FROM research_claims WHERE id = ?",
                id,
            )
            .await?
        else {
            return Ok(None);
        };
        if claim.reproduce_fingerprint().map_err(store_error)? != claim.fingerprint {
            return Err(store_error("research claim fingerprint mismatch"));
        }
        let mut parents = self
            .research_run_node(claim.run_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for evidence_id in claim
            .supporting_evidence_ids
            .iter()
            .chain(claim.conflicting_evidence_ids.iter())
        {
            if let Some(evidence) = self.research_evidence_node(*evidence_id).await? {
                parents.push(evidence);
            }
        }
        Ok(Some(node(
            ArtifactKind::ResearchClaim,
            id,
            Some(claim.fingerprint.clone()),
            &claim,
            parents,
        )?))
    }

    async fn authenticity_profile_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(profile) = self.get_profile(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .research_run_node(profile.run_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for claim in &profile.claims {
            if let Some(claim) = self.research_claim_node(claim.id).await? {
                parents.push(claim);
            }
        }
        if let Some(predecessor_id) = profile.predecessor_id
            && let Some(predecessor) =
                Box::pin(self.authenticity_profile_node(predecessor_id)).await?
        {
            parents.push(predecessor);
        }
        Ok(Some(node(
            ArtifactKind::AuthenticityProfile,
            id,
            Some(profile.fingerprint.clone()),
            &profile,
            parents,
        )?))
    }

    async fn authenticity_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(review) = self
            .research_artifact::<ProfileReview>(
                "SELECT review_json FROM authenticity_profile_reviews WHERE id = ?",
                id,
            )
            .await?
        else {
            return Ok(None);
        };
        if review.reproduce_fingerprint().map_err(store_error)? != review.fingerprint {
            return Err(store_error("authenticity review fingerprint mismatch"));
        }
        let mut parents = self
            .authenticity_profile_node(review.profile_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(predecessor_id) = review.predecessor_id
            && let Some(predecessor) =
                Box::pin(self.authenticity_review_node(predecessor_id)).await?
        {
            parents.push(predecessor);
        }
        Ok(Some(node(
            ArtifactKind::AuthenticityProfileReview,
            id,
            Some(review.fingerprint.clone()),
            &review,
            parents,
        )?))
    }

    async fn authenticity_binding_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(binding) = self
            .research_artifact::<ProfileBinding>(
                "SELECT binding_json FROM authenticity_profile_bindings WHERE id = ?",
                id,
            )
            .await?
        else {
            return Ok(None);
        };
        if binding.reproduce_fingerprint().map_err(store_error)? != binding.fingerprint {
            return Err(store_error("authenticity binding fingerprint mismatch"));
        }
        let mut parents = self
            .dataset_node(binding.dataset_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(profile) = self.authenticity_profile_node(binding.profile_id).await? {
            parents.push(profile);
        }
        if let Some(review) = self.authenticity_review_node(binding.approval_id).await? {
            parents.push(review);
        }
        if let Some(predecessor_id) = binding.predecessor_id
            && let Some(predecessor) =
                Box::pin(self.authenticity_binding_node(predecessor_id)).await?
        {
            parents.push(predecessor);
        }
        Ok(Some(node(
            ArtifactKind::AuthenticityProfileBinding,
            id,
            Some(binding.fingerprint.clone()),
            &binding,
            parents,
        )?))
    }

    async fn generation_authenticity_node(
        &self,
        job_id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(assignment) = self
            .get_generation_authenticity(job_id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let parents = self
            .authenticity_binding_node(assignment.context.binding_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::GenerationAuthenticityContext,
            job_id,
            Some(assignment.fingerprint.clone()),
            &assignment,
            parents,
        )?))
    }

    async fn research_artifact<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        id: Uuid,
    ) -> Result<Option<T>, ProvenanceStoreError> {
        let value = sqlx::query_scalar::<_, String>(query)
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
        value
            .map(|value| serde_json::from_str(&value).map_err(store_error))
            .transpose()
    }

    async fn project_bootstrap_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_bootstrap(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .project_preparation_node(value.preparation_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for source in &value.sources {
            if let Some(snapshot) = self.snapshot_node(source.snapshot_id).await? {
                parents.push(snapshot);
            }
        }
        Ok(Some(node(
            ArtifactKind::ProjectBootstrap,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn project_preparation_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_preparation(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .workflow_definition_node(value.workflow_definition_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::ProjectPreparation,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn benchmark_suite_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_benchmark_suite(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        Ok(Some(node(
            ArtifactKind::BenchmarkSuite,
            id,
            Some(value.fingerprint.clone()),
            &value,
            vec![],
        )?))
    }

    async fn contamination_report_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_contamination_report(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        Ok(Some(node(
            ArtifactKind::ContaminationReport,
            id,
            Some(value.fingerprint.clone()),
            &value,
            vec![],
        )?))
    }

    async fn benchmark_bundle_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_benchmark_bundle(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let development = required_provenance_parent(
            self.benchmark_suite_node(value.development_suite_id)
                .await?,
            "benchmark bundle development suite",
        )?;
        require_node_fingerprint(
            &development,
            &value.development_suite_fingerprint,
            "benchmark bundle development suite",
        )?;
        let mut parents = vec![development];
        if let (Some(suite_id), Some(fingerprint)) = (
            value.sealed_suite_id,
            value.sealed_suite_fingerprint.as_deref(),
        ) {
            let sealed = required_provenance_parent(
                self.benchmark_suite_node(suite_id).await?,
                "benchmark bundle sealed suite",
            )?;
            require_node_fingerprint(&sealed, fingerprint, "benchmark bundle sealed suite")?;
            parents.push(sealed);
        }
        let report = required_provenance_parent(
            self.contamination_report_node(value.contamination_report_id)
                .await?,
            "benchmark bundle contamination report",
        )?;
        require_node_fingerprint(
            &report,
            &value.contamination_report_fingerprint,
            "benchmark bundle contamination report",
        )?;
        parents.push(report);
        Ok(Some(node(
            ArtifactKind::BenchmarkBundle,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn training_benchmark_check_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_training_benchmark_check(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        // This check is an authority edge, not a second ownership path for the
        // entire snapshot lineage. Keep the parent shallow: workflow
        // provenance already contains the snapshot itself, often through the
        // training run and checkpoint as well. Expanding the same qualified
        // snapshot recursively through every one of those DAG edges can make
        // an otherwise finite provenance graph exceed the process stack while
        // serializing it.
        let snapshot = required_provenance_parent(
            Box::pin(self.snapshot_reference_node(value.training_snapshot_id)).await?,
            "training-benchmark check snapshot",
        )?;
        require_node_fingerprint(
            &snapshot,
            &value.training_snapshot_fingerprint,
            "training-benchmark check snapshot",
        )?;
        let bundle = required_provenance_parent(
            self.benchmark_bundle_node(value.benchmark_bundle_id)
                .await?,
            "training-benchmark check bundle",
        )?;
        require_node_fingerprint(
            &bundle,
            &value.benchmark_bundle_fingerprint,
            "training-benchmark check bundle",
        )?;
        let report = required_provenance_parent(
            self.contamination_report_node(value.contamination_report_id)
                .await?,
            "training-benchmark check contamination report",
        )?;
        require_node_fingerprint(
            &report,
            &value.contamination_report_fingerprint,
            "training-benchmark check contamination report",
        )?;
        Ok(Some(node(
            ArtifactKind::TrainingBenchmarkCheck,
            id,
            Some(value.fingerprint.clone()),
            &value,
            vec![snapshot, bundle, report],
        )?))
    }

    async fn benchmark_qualification_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_benchmark_qualification(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let bundle = required_provenance_parent(
            self.benchmark_bundle_node(value.benchmark_bundle_id)
                .await?,
            "benchmark qualification bundle",
        )?;
        require_node_fingerprint(
            &bundle,
            &value.benchmark_bundle_fingerprint,
            "benchmark qualification bundle",
        )?;
        Ok(Some(node(
            ArtifactKind::BenchmarkQualification,
            id,
            Some(value.fingerprint.clone()),
            &value,
            vec![bundle],
        )?))
    }

    async fn workflow_definition_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_workflow_definition(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        if let Some(dataset) = self.dataset_node(value.dataset_id).await? {
            parents.push(dataset);
        }
        if let Some(config) = self
            .configuration_node(value.project_configuration_id)
            .await?
        {
            parents.push(config);
        }
        if let Some(binding) = &value.benchmark_bundle {
            let bundle = required_provenance_parent(
                self.benchmark_bundle_node(binding.bundle_id).await?,
                "workflow definition benchmark bundle",
            )?;
            require_node_fingerprint(
                &bundle,
                &binding.bundle_fingerprint,
                "workflow definition benchmark bundle",
            )?;
            parents.push(bundle);
        }
        Ok(Some(node(
            ArtifactKind::WorkflowDefinition,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn workflow_run_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(run) = self.get_workflow_run(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let attempts = self.list_workflow_attempts(id).await.map_err(store_error)?;
        let mut child_executions = Vec::new();
        for attempt in &attempts {
            child_executions.extend(
                self.list_workflow_child_executions(attempt.id)
                    .await
                    .map_err(store_error)?,
            );
        }
        let mut parents = self
            .workflow_definition_node(run.definition_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for artifact in attempts.iter().flat_map(|attempt| &attempt.artifacts) {
            if let Some(kind) = workflow_artifact_kind(&artifact.kind) {
                // A workflow is already an immutable index over completed
                // stage artifacts. Represent those edges as references instead
                // of recursively expanding the same provenance DAG once per
                // link. Direct artifact traces remain the checked, full view.
                parents.push(ProvenanceNode {
                    kind,
                    id: artifact.artifact_id,
                    fingerprint: Some(artifact.artifact_fingerprint.clone()),
                    attributes: serde_json::json!({
                        "workflow_artifact_kind": artifact.kind,
                        "reference_only": true,
                    }),
                    parents: Vec::new(),
                });
            }
        }
        Ok(Some(node(
            ArtifactKind::WorkflowRun,
            id,
            run.latest_attempt_fingerprint.clone(),
            &serde_json::json!({
                "run": run,
                "attempts": attempts,
                "child_executions": child_executions,
            }),
            parents,
        )?))
    }

    async fn acceptance_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_acceptance_assessment(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        for run_id in value.evaluation_run_ids.values() {
            if let Some(parent) = self.evaluation_node(*run_id).await? {
                parents.push(parent);
            }
        }
        for comparison_id in value.comparison_ids.values() {
            if let Some(parent) = self.comparison_node(*comparison_id).await? {
                parents.push(parent);
            }
        }
        Ok(Some(node(
            ArtifactKind::AcceptanceAssessment,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn advisory_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self
            .get_advisory_assessment(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = self
            .analysis_node(value.request.analysis_report_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(job_id) = value.request.semantic_context_job_id
            && let Some(context) = self.semantic_context_node(job_id).await?
        {
            parents.push(context);
        }
        Ok(Some(node(
            ArtifactKind::AdvisoryAssessment,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn workflow_approval_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_workflow_approval(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        if let Some(parent) = self.optimization_node(value.proposal_id).await? {
            parents.push(parent);
        }
        if let Some(parent) = self
            .optimization_review_node(value.proposal_review_id)
            .await?
        {
            parents.push(parent);
        }
        Ok(Some(node(
            ArtifactKind::WorkflowApproval,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn stop_decision_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_stop_decision(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .acceptance_node(value.acceptance_assessment_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for comparison_id in &value.comparison_ids {
            if let Some(parent) = self.comparison_node(*comparison_id).await? {
                parents.push(parent);
            }
        }
        Ok(Some(node(
            ArtifactKind::StopDecision,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn promotion_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(value) = self.get_promotion(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        if let Some(parent) = self.checkpoint_node(value.checkpoint_id).await? {
            parents.push(parent);
        }
        if let Some(parent) = self.snapshot_node(value.training_snapshot_id).await? {
            parents.push(parent);
        }
        if let (Some(check_id), Some(check_fingerprint)) = (
            value.training_benchmark_check_id,
            value.training_benchmark_check_fingerprint.as_deref(),
        ) {
            let check = required_provenance_parent(
                self.training_benchmark_check_node(check_id).await?,
                "model promotion training-benchmark check",
            )?;
            require_node_fingerprint(
                &check,
                check_fingerprint,
                "model promotion training-benchmark check",
            )?;
            parents.push(check);
        }
        if let Some(parent) = self
            .acceptance_node(value.development_assessment_id)
            .await?
        {
            parents.push(parent);
        }
        if let Some(parent) = self.acceptance_node(value.sealed_assessment_id).await? {
            parents.push(parent);
        }
        Ok(Some(node(
            ArtifactKind::ModelPromotion,
            id,
            Some(value.fingerprint.clone()),
            &value,
            parents,
        )?))
    }

    async fn optimization_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(payload) = sqlx::query_scalar::<_, String>(
            "SELECT review_json FROM optimization_proposal_reviews WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let review: ProposalReviewRecord = serde_json::from_str(&payload).map_err(store_error)?;
        let mut parents = self
            .optimization_node(review.proposal_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(superseding_id) = review.superseding_proposal_id {
            if let Some(proposal) = self.optimization_node(superseding_id).await? {
                parents.push(proposal);
            }
        }
        Ok(Some(node(
            ArtifactKind::OptimizationProposalReview,
            id,
            Some(review.fingerprint.clone()),
            &review,
            parents,
        )?))
    }

    async fn campaign_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(campaign) = self.get_campaign(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .optimization_node(campaign.proposal_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(review) = self
            .optimization_review_node(campaign.approval_review_id)
            .await?
        {
            parents.push(review);
        }
        Ok(Some(node(
            ArtifactKind::OptimizationCampaign,
            id,
            Some(campaign.fingerprint.clone()),
            &campaign,
            parents,
        )?))
    }

    async fn campaign_link_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(payload) = sqlx::query_scalar::<_, String>(
            "SELECT link_json FROM optimization_campaign_links WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let link: CampaignArtifactLink = serde_json::from_str(&payload).map_err(store_error)?;
        let mut parents = self
            .campaign_node(link.campaign_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        let artifact = match link.artifact_kind {
            CampaignArtifactKind::GenerationPlan => self.plan_node(link.artifact_id).await?,
            CampaignArtifactKind::GenerationJob => self.job_node(link.artifact_id).await?,
            CampaignArtifactKind::DatasetSnapshot => self.snapshot_node(link.artifact_id).await?,
            CampaignArtifactKind::TrainingRun => self.training_node(link.artifact_id).await?,
            CampaignArtifactKind::TrainingCheckpoint => {
                self.checkpoint_node(link.artifact_id).await?
            }
            CampaignArtifactKind::CandidateEvaluation => {
                self.evaluation_node(link.artifact_id).await?
            }
            CampaignArtifactKind::EvaluationComparison => {
                self.comparison_node(link.artifact_id).await?
            }
            CampaignArtifactKind::FollowUpAnalysis => self.analysis_node(link.artifact_id).await?,
        };
        if let Some(artifact) = artifact {
            parents.push(artifact);
        }
        Ok(Some(node(
            ArtifactKind::OptimizationCampaignLink,
            id,
            Some(link.fingerprint.clone()),
            &link,
            parents,
        )?))
    }

    async fn optimization_outcome_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(payload) = sqlx::query_scalar::<_, String>(
            "SELECT outcome_json FROM optimization_campaign_outcomes WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let outcome: CampaignOutcomeAssessment =
            serde_json::from_str(&payload).map_err(store_error)?;
        let mut parents = self
            .campaign_node(outcome.campaign_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(comparison) = self.comparison_node(outcome.comparison_id).await? {
            parents.push(comparison);
        }
        Ok(Some(node(
            ArtifactKind::OptimizationOutcome,
            id,
            Some(outcome.fingerprint.clone()),
            &outcome,
            parents,
        )?))
    }

    async fn analysis_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(review) = sqlx::query_as::<_, AnalysisReviewProvenanceRecord>(
            "SELECT analysis_report_id, finding_key, state, note, \
             resolution_evaluation_run_id, resolution_comparison_id, created_at \
             FROM analysis_finding_reviews WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = self
            .analysis_node(review.analysis_report_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(run_id) = review.resolution_evaluation_run_id {
            if let Some(run) = self.evaluation_node(run_id).await? {
                parents.push(run);
            }
        }
        if let Some(comparison_id) = review.resolution_comparison_id {
            if let Some(comparison) = self.comparison_node(comparison_id).await? {
                parents.push(comparison);
            }
        }
        Ok(Some(node(
            ArtifactKind::AnalysisFindingReview,
            id,
            None,
            &review,
            parents,
        )?))
    }

    async fn comparison_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(report) = self.get_comparison(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        if let Some(node) = self.evaluation_node(report.left_run_id).await? {
            parents.push(node);
        }
        if let Some(node) = self.evaluation_node(report.right_run_id).await? {
            parents.push(node);
        }
        Ok(Some(node(
            ArtifactKind::EvaluationComparison,
            id,
            Some(report.fingerprint.clone()),
            &report,
            parents,
        )?))
    }

    async fn selection_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(report) = self.get_selection(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Vec::new();
        for run_id in &report.candidate_run_ids {
            if let Some(node) = self.evaluation_node(*run_id).await? {
                parents.push(node);
            }
        }
        Ok(Some(node(
            ArtifactKind::ModelSelection,
            id,
            Some(report.fingerprint.clone()),
            &report,
            parents,
        )?))
    }
    async fn encoder_node(&self, id: Uuid) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(encoder) = self.get_encoder(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        Ok(Some(node(
            ArtifactKind::BaseModel,
            id,
            Some(encoder.fingerprint.clone()),
            &encoder,
            vec![],
        )?))
    }

    async fn configuration_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ConfigurationRecord>(
            "SELECT id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json \
             FROM project_configurations WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        record.map(ConfigurationRecord::into_node).transpose()
    }

    async fn configuration_for_dataset(
        &self,
        dataset_id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ConfigurationRecord>(
            "SELECT id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json \
             FROM project_configurations WHERE dataset_id = ?",
        )
        .bind(dataset_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        record.map(ConfigurationRecord::into_node).transpose()
    }

    async fn dataset_node(&self, id: Uuid) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(dataset) = self.get_dataset(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .configuration_for_dataset(id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::Dataset,
            id,
            None,
            &dataset,
            parents,
        )?))
    }

    async fn plan_node(&self, id: Uuid) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(plan) = self.get_plan(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let configured = sqlx::query_as::<_, ConfigurationRecord>(
            "SELECT id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json \
             FROM project_configurations WHERE generation_plan_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(ConfigurationRecord::into_node)
        .transpose()?;
        let mut parents = configured.into_iter().collect::<Vec<_>>();
        if parents.is_empty() {
            if let Some(proposal) = sqlx::query_as::<_, ShallowProposalRecord>(
                "SELECT p.id, p.fingerprint, p.analysis_report_id, p.dataset_id \
                 FROM optimization_proposals p \
                 JOIN optimization_proposal_applications a ON a.proposal_id = p.id \
                 WHERE a.generation_plan_id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            {
                parents.push(proposal.into_node());
            }
        }
        let allocation_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM workflow_initial_allocations WHERE generation_plan_id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        if let Some(allocation_id) = allocation_id
            && let Some(allocation) = self.initial_allocation_node(allocation_id).await?
        {
            parents.push(allocation);
        }
        let architecture_application_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM dataset_architecture_applications WHERE plan_id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        if let Some(application_id) = architecture_application_id
            && let Some(application) =
                Box::pin(self.architecture_application_node(application_id)).await?
        {
            parents.push(application);
        }
        Ok(Some(node(
            ArtifactKind::GenerationPlan,
            id,
            None,
            &plan,
            parents,
        )?))
    }

    async fn initial_allocation_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(allocation) = self.get_initial_allocation(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .dataset_node(allocation.result.dataset_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::InitialAllocation,
            id,
            Some(allocation.fingerprint.clone()),
            &allocation,
            parents,
        )?))
    }

    async fn job_node(&self, id: Uuid) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(job) = self.get_job(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .plan_node(job.plan_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(context) = self.semantic_context_node(id).await? {
            parents.push(context);
        }
        if let Some(context) = Box::pin(self.generation_authenticity_node(id)).await? {
            parents.push(context);
        }
        let strategy_context_id: Option<Uuid> =
            sqlx::query_scalar("SELECT context_id FROM generation_job_strategies WHERE job_id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        if let Some(context_id) = strategy_context_id
            && let Some(context) = Box::pin(self.generation_strategy_node(context_id)).await?
        {
            parents.push(context);
        }
        let execution = self
            .get_generation_execution_spec(id)
            .await
            .map_err(store_error)?;
        let attempts = self
            .list_generation_attempts(id)
            .await
            .map_err(store_error)?;
        let fingerprint = execution
            .as_ref()
            .map(|execution| execution.fingerprint.clone());
        Ok(Some(node(
            ArtifactKind::GenerationJob,
            id,
            fingerprint,
            &serde_json::json!({
                "job": job,
                "execution": execution,
                "attempts": attempts,
            }),
            parents,
        )?))
    }

    async fn import_node(&self, id: Uuid) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(dataset_import) = self.get_import(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        Ok(Some(node(
            ArtifactKind::DatasetImport,
            id,
            None,
            &dataset_import,
            vec![],
        )?))
    }

    async fn source_row_node(
        &self,
        id: Uuid,
        expected: Option<&AuditPlanItem>,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ProvenanceSourceRowRecord>(
            "SELECT id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, \
             label, dimensions_json, fields_json, provenance_json, created_at \
             FROM dataset_source_rows WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let stored_source_kind = record.source_kind.clone();
        let stored_source_ref = record.source_ref;
        let stored_cell_key = record.cell_key.clone();
        let stored_normalized_text = record.normalized_text.clone();
        let row = record.into_domain()?;
        let expected_cell_key = GenerationCell {
            label: row.label.clone(),
            dimensions: row.dimensions.clone(),
        }
        .key();
        let expected_source_kind = match &row.provenance {
            SourceProvenance::Generated { .. } => "generated",
            SourceProvenance::Imported { .. } => "imported",
        };
        if stored_source_kind != expected_source_kind
            || stored_source_ref != row.id
            || stored_cell_key != expected_cell_key
            || stored_normalized_text != normalize_text(&row.text)
        {
            return Err(store_error(format!(
                "source row {} normalized columns disagree with its durable content",
                row.id
            )));
        }
        let row_fingerprint = artifact_core::fingerprint(&row).map_err(store_error)?;
        if let Some(expected) = expected {
            if expected.source_row_id != row.id
                || expected.source_row_fingerprint != row_fingerprint
                || expected.cell.label != row.label
                || expected.cell.dimensions != row.dimensions
            {
                return Err(store_error(format!(
                    "source row {} no longer matches its quality audit plan item",
                    row.id
                )));
            }
        }
        let producer = self.checked_source_producer_parent(&row).await?;
        Ok(Some(redacted_source_row_node(
            &row,
            row_fingerprint,
            vec![producer],
        )))
    }

    async fn checked_source_producer_parent(
        &self,
        row: &SourceRow,
    ) -> Result<ProvenanceNode, ProvenanceStoreError> {
        match &row.provenance {
            SourceProvenance::Generated {
                generation_job_id,
                backend,
                model,
                construction_plan_fingerprint,
            } => {
                let job = self
                    .checked_generation_producer(*generation_job_id, row.dataset_id, backend, model)
                    .await?;
                let generated = sqlx::query_as::<_, super::RowRecord>(
                    "SELECT id, dataset_id, plan_id, generation_job_id, cell_key, text, \
                     normalized_text, label, dimensions_json, generator_backend, generator_model, \
                     created_at, validation_status, validation_errors_json, \
                     generation_metadata_json, fields_json, construction_json \
                     FROM generated_rows WHERE id = ?",
                )
                .bind(row.id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?
                .ok_or_else(|| store_error("source row generated origin is missing"))?
                .into_domain()
                .map_err(store_error)?;
                let expected_cell_key = GenerationCell {
                    label: row.label.clone(),
                    dimensions: row.dimensions.clone(),
                }
                .key();
                let actual_construction_fingerprint = generated
                    .construction
                    .as_ref()
                    .map(|trace| trace.plan_fingerprint.as_str());
                if job.id != *generation_job_id
                    || job.dataset_id != row.dataset_id
                    || job.plan_id != generated.plan_id
                    || job.backend_name != *backend
                    || job.backend_model != *model
                    || generated.id != row.id
                    || generated.dataset_id != row.dataset_id
                    || generated.generation_job_id != *generation_job_id
                    || generated.cell_key != expected_cell_key
                    || generated.text != row.text
                    || generated.normalized_text != normalize_text(&row.text)
                    || generated.label != row.label
                    || generated.dimensions != row.dimensions
                    || generated.fields != row.fields
                    || generated.generator_backend != *backend
                    || generated.generator_model != *model
                    || generated.created_at != row.created_at
                    || generated.validation_status != ValidationStatus::Accepted
                    || actual_construction_fingerprint != construction_plan_fingerprint.as_deref()
                {
                    return Err(store_error(format!(
                        "source row {} disagrees with its generated origin or producer",
                        row.id
                    )));
                }
                Ok(redacted_generation_job_node(&job))
            }
            SourceProvenance::Imported {
                import_id,
                source_path,
                source_row_number,
            } => {
                let dataset_import = self
                    .checked_import_producer(*import_id, row.dataset_id, source_path)
                    .await?;
                let imported = sqlx::query_as::<_, ProvenanceImportedRowRecord>(
                    "SELECT id, import_id, dataset_id, source_row_number, cell_key, text, \
                     normalized_text, label, dimensions_json, validation_status, created_at \
                     FROM imported_rows WHERE id = ?",
                )
                .bind(row.id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?
                .ok_or_else(|| store_error("source row imported origin is missing"))?;
                let imported_dimensions = imported.dimensions()?;
                let imported_source_row_number = u64::try_from(imported.source_row_number)
                    .map_err(|_| store_error("imported source row number is invalid"))?;
                let expected_cell_key = GenerationCell {
                    label: row.label.clone(),
                    dimensions: row.dimensions.clone(),
                }
                .key();
                if dataset_import.id != *import_id
                    || dataset_import.dataset_id != row.dataset_id
                    || dataset_import.source_path != *source_path
                    || imported.id != row.id
                    || imported.import_id != *import_id
                    || imported.dataset_id != row.dataset_id
                    || imported_source_row_number != *source_row_number
                    || imported.cell_key.as_deref() != Some(expected_cell_key.as_str())
                    || imported.text != row.text
                    || imported.normalized_text != normalize_text(&row.text)
                    || imported.label != row.label
                    || imported_dimensions != row.dimensions
                    || imported.validation_status != "accepted"
                    || imported.created_at != row.created_at
                    || !row.fields.is_empty()
                {
                    return Err(store_error(format!(
                        "source row {} disagrees with its imported origin or producer",
                        row.id
                    )));
                }
                Ok(redacted_import_node(&dataset_import))
            }
        }
    }

    async fn checked_generation_producer(
        &self,
        id: Uuid,
        expected_dataset_id: Uuid,
        expected_backend: &str,
        expected_model: &str,
    ) -> Result<GenerationJob, ProvenanceStoreError> {
        let job = self
            .get_job(id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("source provenance generation job is missing"))?;
        if job.dataset_id != expected_dataset_id
            || job.backend_name != expected_backend
            || job.backend_model != expected_model
        {
            return Err(store_error(
                "source provenance generation job ownership or identity mismatch",
            ));
        }
        Ok(job)
    }

    async fn checked_generation_plan_for_job(
        &self,
        job: &GenerationJob,
    ) -> Result<GenerationPlan, ProvenanceStoreError> {
        let plan = self
            .get_plan(job.plan_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("source provenance generation plan is missing"))?;
        if plan.id != job.plan_id || plan.dataset_id != job.dataset_id {
            return Err(store_error(
                "source provenance generation plan ownership mismatch",
            ));
        }
        Ok(plan)
    }

    async fn checked_import_producer(
        &self,
        id: Uuid,
        expected_dataset_id: Uuid,
        expected_source_path: &str,
    ) -> Result<DatasetImport, ProvenanceStoreError> {
        let dataset_import = self
            .get_import(id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("source provenance dataset import is missing"))?;
        if dataset_import.dataset_id != expected_dataset_id
            || dataset_import.source_path != expected_source_path
        {
            return Err(store_error(
                "source provenance dataset import ownership or identity mismatch",
            ));
        }
        Ok(dataset_import)
    }

    async fn quality_audit_plan_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(plan) = self.get_audit_plan(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut dataset = required_provenance_parent(
            self.dataset_node(plan.dataset_schema.dataset_definition_id)
                .await?,
            "quality audit plan dataset definition",
        )?;
        let stored_dataset = self
            .get_dataset(plan.dataset_schema.dataset_definition_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality audit plan dataset definition is missing"))?;
        let dataset_fingerprint =
            artifact_core::fingerprint(&stored_dataset).map_err(store_error)?;
        if dataset_fingerprint != plan.dataset_schema.dataset_definition_fingerprint {
            return Err(store_error(
                "quality audit plan dataset definition fingerprint mismatch",
            ));
        }
        dataset.fingerprint = Some(dataset_fingerprint);

        let mut parents = Vec::with_capacity(plan.items.len().saturating_add(3));
        parents.push(dataset);
        let expected_dataset_id = plan.dataset_schema.dataset_definition_id.to_string();
        for item in &plan.items {
            let source = required_provenance_parent(
                self.source_row_node(item.source_row_id, Some(item)).await?,
                "quality audit plan source row",
            )?;
            if source
                .attributes
                .get("dataset_id")
                .and_then(serde_json::Value::as_str)
                != Some(expected_dataset_id.as_str())
            {
                return Err(store_error(
                    "quality audit plan source row belongs to another dataset",
                ));
            }
            parents.push(source);
        }
        let guidance = self
            .get_audit_guidance(plan.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality audit plan pinned guidance is missing"))?;
        guidance.verify_against(&plan).map_err(store_error)?;
        if let Some(semantic) = &guidance.semantic {
            parents.push(
                self.checked_quality_semantic_guidance_node(&plan, semantic)
                    .await?,
            );
        }
        if let Some(authenticity) = &guidance.authenticity {
            parents.push(
                self.checked_quality_authenticity_guidance_parent(&plan, authenticity)
                    .await?,
            );
        }
        Ok(Some(quality_plan_node(&plan, parents, false)))
    }

    async fn quality_semantic_guidance_node(
        &self,
        plan_id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(plan) = self.get_audit_plan(plan_id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let guidance = self
            .get_audit_guidance(plan.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality audit plan pinned guidance is missing"))?;
        guidance.verify_against(&plan).map_err(store_error)?;
        let Some(semantic) = &guidance.semantic else {
            return Ok(None);
        };
        Ok(Some(
            self.checked_quality_semantic_guidance_node(&plan, semantic)
                .await?,
        ))
    }

    async fn checked_quality_semantic_guidance_node(
        &self,
        plan: &AuditPlan,
        guidance: &SemanticEvaluatorGuidance,
    ) -> Result<ProvenanceNode, ProvenanceStoreError> {
        if guidance.reference.id != plan.id {
            return Err(store_error(
                "quality semantic guidance reference does not use its audit plan identity",
            ));
        }
        let mut parents = Vec::with_capacity(guidance.sources.len());
        for source in &guidance.sources {
            parents.push(
                self.checked_quality_semantic_binding_parent(plan, source)
                    .await?,
            );
        }
        Ok(quality_semantic_guidance_node_value(guidance, parents))
    }

    async fn checked_quality_semantic_binding_parent(
        &self,
        plan: &AuditPlan,
        source: &dataset_quality_core::population::GuidanceReference,
    ) -> Result<ProvenanceNode, ProvenanceStoreError> {
        let binding = self
            .load_semantic_binding_for_quality(source.id)
            .await?
            .ok_or_else(|| store_error("quality semantic guidance source binding is missing"))?;
        if binding.fingerprint != source.fingerprint
            || binding.dataset_id != plan.dataset_schema.dataset_definition_id
        {
            return Err(store_error(
                "quality semantic guidance source binding is foreign or stale",
            ));
        }
        let (profile_id, profile_fingerprint) = binding
            .profile_id
            .zip(binding.profile_fingerprint.as_deref())
            .ok_or_else(|| {
                store_error("quality semantic guidance source is not a bound semantic profile")
            })?;
        let profile = self
            .get_semantic_profile(profile_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality semantic guidance source profile is missing"))?;
        if profile.reproduce_fingerprint().map_err(store_error)? != profile.fingerprint
            || profile.fingerprint != profile_fingerprint
            || profile.target != binding.target
            || SemanticLayer::for_scope(&profile.scope) != binding.layer
            || matches!(
                profile.scope,
                SemanticScope::Dataset { dataset_id }
                    if dataset_id != plan.dataset_schema.dataset_definition_id
            )
        {
            return Err(store_error(
                "quality semantic guidance source profile is foreign, stale, or invalid",
            ));
        }
        let mut parents = vec![redacted_artifact_reference_node(
            ArtifactKind::SemanticProfile,
            profile.id,
            Some(profile.fingerprint),
            json!({
                "target_key": profile.target.key(),
                "scope": profile.scope,
                "content_redacted": true,
                "reference_only": true,
            }),
        )];
        if let Some(predecessor_id) = binding.predecessor_id {
            let predecessor = self
                .load_semantic_binding_for_quality(predecessor_id)
                .await?
                .ok_or_else(|| {
                    store_error("quality semantic guidance binding predecessor is missing")
                })?;
            if predecessor.dataset_id != binding.dataset_id
                || predecessor.target != binding.target
                || predecessor.layer != binding.layer
            {
                return Err(store_error(
                    "quality semantic guidance binding predecessor is foreign",
                ));
            }
            parents.push(redacted_artifact_reference_node(
                ArtifactKind::SemanticBinding,
                predecessor.id,
                Some(predecessor.fingerprint),
                json!({
                    "content_redacted": true,
                    "reference_only": true,
                }),
            ));
        }
        let mut parent = redacted_artifact_reference_node(
            ArtifactKind::SemanticBinding,
            binding.id,
            Some(binding.fingerprint),
            json!({
                "dataset_id": binding.dataset_id,
                "target_key": binding.target.key(),
                "layer": binding.layer,
                "profile_id": binding.profile_id,
                "predecessor_id": binding.predecessor_id,
                "content_redacted": true,
                "reference_only": true,
            }),
        );
        parent.parents = parents;
        Ok(parent)
    }

    async fn load_semantic_binding_for_quality(
        &self,
        id: Uuid,
    ) -> Result<Option<SemanticBindingDecision>, ProvenanceStoreError> {
        sqlx::query_as::<_, ProvenanceSemanticBindingRecord>(
            "SELECT id, dataset_id, target_key, layer, profile_id, profile_fingerprint, \
             predecessor_id, fingerprint, artifact_json, created_at \
             FROM dataset_semantic_binding_decisions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .map(ProvenanceSemanticBindingRecord::into_checked)
        .transpose()
    }

    async fn checked_quality_authenticity_guidance_parent(
        &self,
        plan: &AuditPlan,
        guidance: &AuthenticityEvaluatorGuidance,
    ) -> Result<ProvenanceNode, ProvenanceStoreError> {
        let binding = self
            .load_authenticity_binding_for_quality(guidance.reference.id)
            .await?
            .ok_or_else(|| store_error("quality authenticity guidance binding is missing"))?;
        if binding.fingerprint != guidance.reference.fingerprint
            || binding.dataset_id != plan.dataset_schema.dataset_definition_id
            || binding.dataset_fingerprint != plan.dataset_schema.dataset_definition_fingerprint
        {
            return Err(store_error(
                "quality authenticity guidance binding is foreign or stale",
            ));
        }
        let profile = self
            .get_profile(binding.profile_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality authenticity guidance profile is missing"))?;
        if profile.id != binding.profile_id
            || profile.version != binding.profile_version
            || profile.fingerprint != binding.profile_fingerprint
            || profile.dataset_id != binding.dataset_id
            || profile.dataset_fingerprint != binding.dataset_fingerprint
            || profile.reproduce_fingerprint().map_err(store_error)? != profile.fingerprint
        {
            return Err(store_error(
                "quality authenticity guidance profile is foreign, stale, or invalid",
            ));
        }
        let approval = sqlx::query_as::<_, ProvenanceAuthenticityReviewRecord>(
            "SELECT id, profile_id, predecessor_id, decision, fingerprint, review_json, \
             created_at FROM authenticity_profile_reviews WHERE id = ?",
        )
        .bind(binding.approval_id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .ok_or_else(|| store_error("quality authenticity guidance approval is missing"))?
        .into_checked()?;
        if approval.profile_id != profile.id
            || approval.profile_fingerprint != profile.fingerprint
            || approval.fingerprint != binding.approval_fingerprint
            || approval.decision != ProfileReviewDecision::Approve
        {
            return Err(store_error(
                "quality authenticity guidance approval is foreign, stale, or not approving",
            ));
        }
        let mut parents = vec![
            redacted_artifact_reference_node(
                ArtifactKind::AuthenticityProfile,
                profile.id,
                Some(profile.fingerprint),
                json!({
                    "dataset_id": profile.dataset_id,
                    "version": profile.version,
                    "content_redacted": true,
                    "reference_only": true,
                }),
            ),
            redacted_artifact_reference_node(
                ArtifactKind::AuthenticityProfileReview,
                approval.id,
                Some(approval.fingerprint),
                json!({
                    "profile_id": approval.profile_id,
                    "decision": approval.decision,
                    "reason_redacted": true,
                    "reference_only": true,
                }),
            ),
        ];
        if let Some(predecessor_id) = binding.predecessor_id {
            let predecessor = self
                .load_authenticity_binding_for_quality(predecessor_id)
                .await?
                .ok_or_else(|| {
                    store_error("quality authenticity guidance binding predecessor is missing")
                })?;
            if predecessor.dataset_id != binding.dataset_id {
                return Err(store_error(
                    "quality authenticity guidance binding predecessor is foreign",
                ));
            }
            parents.push(redacted_artifact_reference_node(
                ArtifactKind::AuthenticityProfileBinding,
                predecessor.id,
                Some(predecessor.fingerprint),
                json!({
                    "content_redacted": true,
                    "reference_only": true,
                }),
            ));
        }
        let mut parent = redacted_artifact_reference_node(
            ArtifactKind::AuthenticityProfileBinding,
            binding.id,
            Some(binding.fingerprint),
            json!({
                "dataset_id": binding.dataset_id,
                "profile_id": binding.profile_id,
                "profile_version": binding.profile_version,
                "approval_id": binding.approval_id,
                "predecessor_id": binding.predecessor_id,
                "content_redacted": true,
                "reference_only": true,
            }),
        );
        parent.parents = parents;
        Ok(parent)
    }

    async fn load_authenticity_binding_for_quality(
        &self,
        id: Uuid,
    ) -> Result<Option<ProfileBinding>, ProvenanceStoreError> {
        sqlx::query_as::<_, ProvenanceAuthenticityBindingRecord>(
            "SELECT id, dataset_id, profile_id, approval_id, predecessor_id, fingerprint, \
             binding_json, created_at FROM authenticity_profile_bindings WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .map(ProvenanceAuthenticityBindingRecord::into_checked)
        .transpose()
    }

    async fn quality_audit_run_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(run) = self.get_audit_run(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let plan = self
            .get_audit_plan(run.plan_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality audit run plan is missing"))?;
        run.verify_integrity(&plan).map_err(store_error)?;
        let plan_node = required_provenance_parent(
            Box::pin(self.quality_audit_plan_node(plan.id)).await?,
            "quality audit run plan",
        )?;
        Ok(Some(quality_run_node(&run, vec![plan_node], false)))
    }

    async fn checked_quality_report_evidence(
        &self,
        report: &DatasetQualityReport,
    ) -> Result<CheckedQualityReportEvidence, ProvenanceStoreError> {
        let plan = self
            .get_audit_plan(report.plan_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("dataset quality report audit plan is missing"))?;
        let run = self
            .get_audit_run(report.run_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("dataset quality report audit run is missing"))?;
        let requests = self
            .list_evaluator_requests(run.id)
            .await
            .map_err(store_error)?;
        let attempts = self.list_attempts(run.id).await.map_err(store_error)?;
        let assessments = self.list_assessments(run.id).await.map_err(store_error)?;
        report
            .verify_against(&plan, &run, &requests, &attempts, &assessments)
            .map_err(store_error)?;

        let mut plan_items = BTreeMap::new();
        for item in &plan.items {
            if plan_items
                .insert(item.source_row_id, item.clone())
                .is_some()
            {
                return Err(store_error(
                    "dataset quality report audit plan repeats a source row",
                ));
            }
        }
        let mut requests_by_id = BTreeMap::new();
        for request in requests {
            if requests_by_id.insert(request.id, request).is_some() {
                return Err(store_error(
                    "dataset quality report evidence repeats an evaluator request",
                ));
            }
        }
        let mut attempts_by_id = BTreeMap::new();
        for attempt in attempts {
            if attempts_by_id.insert(attempt.id, attempt).is_some() {
                return Err(store_error(
                    "dataset quality report evidence repeats an evaluator attempt",
                ));
            }
        }
        let mut assessments_by_id = BTreeMap::new();
        for assessment in assessments {
            if assessments_by_id
                .insert(assessment.id, assessment)
                .is_some()
            {
                return Err(store_error(
                    "dataset quality report evidence repeats a row assessment",
                ));
            }
        }
        Ok(CheckedQualityReportEvidence {
            plan,
            run,
            plan_items,
            requests_by_id,
            attempts_by_id,
            assessments_by_id,
        })
    }

    async fn load_quality_attempt(
        &self,
        id: Uuid,
    ) -> Result<
        Option<(
            EvaluatorAttempt,
            BlindEvaluatorRequest,
            QualityAuditRun,
            AuditPlan,
        )>,
        ProvenanceStoreError,
    > {
        let run_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT run_id FROM dataset_quality_evaluator_attempts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let run = self
            .get_audit_run(run_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality evaluator attempt run is missing"))?;
        let plan = self
            .get_audit_plan(run.plan_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality evaluator attempt plan is missing"))?;
        let attempt = self
            .list_attempts(run.id)
            .await
            .map_err(store_error)?
            .into_iter()
            .find(|attempt| attempt.id == id)
            .ok_or_else(|| {
                store_error("quality evaluator attempt normalized evidence is missing")
            })?;
        let request = self
            .get_evaluator_request(attempt.request_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("quality evaluator attempt request is missing"))?;
        attempt
            .verify_integrity(&plan, &run, &request)
            .map_err(store_error)?;
        Ok(Some((attempt, request, run, plan)))
    }

    async fn quality_evaluator_attempt_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some((attempt, _, run, plan)) = self.load_quality_attempt(id).await? else {
            return Ok(None);
        };
        let run_parent = required_provenance_parent(
            Box::pin(self.quality_audit_run_node(run.id)).await?,
            "quality evaluator attempt run",
        )?;
        let mut parents = Vec::with_capacity(attempt.source_row_ids.len().saturating_add(1));
        parents.push(run_parent);
        for source_row_id in &attempt.source_row_ids {
            let item = plan
                .items
                .iter()
                .find(|item| item.source_row_id == *source_row_id)
                .ok_or_else(|| {
                    store_error("quality evaluator attempt source row is absent from its plan")
                })?;
            parents.push(required_provenance_parent(
                self.source_row_node(*source_row_id, Some(item)).await?,
                "quality evaluator attempt source row",
            )?);
        }
        Ok(Some(quality_attempt_node(&attempt, parents, false)))
    }

    async fn load_row_quality_assessment(
        &self,
        id: Uuid,
    ) -> Result<
        Option<(
            RowQualityAssessment,
            EvaluatorAttempt,
            BlindEvaluatorRequest,
            QualityAuditRun,
            AuditPlan,
        )>,
        ProvenanceStoreError,
    > {
        let run_id: Option<Uuid> =
            sqlx::query_scalar("SELECT run_id FROM dataset_quality_row_assessments WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let assessment = self
            .list_assessments(run_id)
            .await
            .map_err(store_error)?
            .into_iter()
            .find(|assessment| assessment.id == id)
            .ok_or_else(|| store_error("row quality assessment normalized evidence is missing"))?;
        let Some((attempt, request, run, plan)) =
            self.load_quality_attempt(assessment.attempt_id).await?
        else {
            return Err(store_error("row quality assessment attempt is missing"));
        };
        if attempt.state != EvaluatorAttemptState::Succeeded {
            return Err(store_error(
                "row quality assessment belongs to a non-successful evaluator attempt",
            ));
        }
        assessment
            .verify_request_binding(&plan, &request)
            .map_err(store_error)?;
        Ok(Some((assessment, attempt, request, run, plan)))
    }

    async fn row_quality_assessment_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some((assessment, attempt, _, _, plan)) = self.load_row_quality_assessment(id).await?
        else {
            return Ok(None);
        };
        let attempt_parent = required_provenance_parent(
            Box::pin(self.quality_evaluator_attempt_node(attempt.id)).await?,
            "row quality assessment evaluator attempt",
        )?;
        let plan_parent = quality_plan_node(&plan, vec![], true);
        let item = plan
            .items
            .iter()
            .find(|item| item.source_row_id == assessment.source_row_id)
            .ok_or_else(|| store_error("row quality assessment plan item is missing"))?;
        let source_parent = required_provenance_parent(
            self.source_row_node(assessment.source_row_id, Some(item))
                .await?,
            "row quality assessment source row",
        )?;
        Ok(Some(quality_assessment_node(
            &assessment,
            vec![attempt_parent, plan_parent, source_parent],
            false,
        )))
    }

    async fn dataset_quality_report_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(report) = self.get_report(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let evidence = self.checked_quality_report_evidence(&report).await?;
        let plan_parent = required_provenance_parent(
            Box::pin(self.quality_audit_plan_node(evidence.plan.id)).await?,
            "dataset quality report audit plan",
        )?;
        let run_parent = quality_run_node(&evidence.run, vec![plan_parent], false);
        let mut parents = vec![run_parent];
        let mut assessment_ids = BTreeSet::new();
        let mut invalid_attempt_ids = BTreeSet::new();
        for row in &report.rows {
            for reference in &row.assessment_references {
                if !assessment_ids.insert(reference.id) {
                    continue;
                }
                let assessment = evidence
                    .assessments_by_id
                    .get(&reference.id)
                    .ok_or_else(|| store_error("dataset quality report assessment is missing"))?;
                if assessment.fingerprint != reference.fingerprint {
                    return Err(store_error(
                        "dataset quality report assessment fingerprint mismatch",
                    ));
                }
                let attempt = evidence
                    .attempts_by_id
                    .get(&assessment.attempt_id)
                    .ok_or_else(|| {
                        store_error("dataset quality report assessment attempt is missing")
                    })?;
                let request = evidence
                    .requests_by_id
                    .get(&assessment.request_id)
                    .ok_or_else(|| {
                        store_error("dataset quality report assessment request is missing")
                    })?;
                if attempt.state != EvaluatorAttemptState::Succeeded
                    || attempt.request_id != request.id
                {
                    return Err(store_error(
                        "dataset quality report assessment has invalid evaluator evidence",
                    ));
                }
                attempt
                    .verify_integrity(&evidence.plan, &evidence.run, request)
                    .map_err(store_error)?;
                assessment
                    .verify_request_binding(&evidence.plan, request)
                    .map_err(store_error)?;
                let item = evidence
                    .plan_items
                    .get(&assessment.source_row_id)
                    .ok_or_else(|| {
                        store_error("dataset quality report assessment plan item is missing")
                    })?;
                let source_parent = required_provenance_parent(
                    self.source_row_node(assessment.source_row_id, Some(item))
                        .await?,
                    "dataset quality report assessment source row",
                )?;
                parents.push(quality_assessment_node(
                    assessment,
                    vec![
                        quality_attempt_node(attempt, vec![], true),
                        quality_plan_node(&evidence.plan, vec![], true),
                        source_parent,
                    ],
                    true,
                ));
            }
            for reference in &row.invalid_attempt_references {
                if !invalid_attempt_ids.insert(reference.id) {
                    continue;
                }
                let attempt = evidence.attempts_by_id.get(&reference.id).ok_or_else(|| {
                    store_error("dataset quality report invalid evaluator attempt is missing")
                })?;
                if attempt.fingerprint != reference.fingerprint
                    || attempt.state != EvaluatorAttemptState::InvalidResponse
                {
                    return Err(store_error(
                        "dataset quality report invalid evaluator attempt does not match its reference",
                    ));
                }
                let request = evidence
                    .requests_by_id
                    .get(&attempt.request_id)
                    .ok_or_else(|| {
                        store_error(
                            "dataset quality report invalid evaluator attempt request is missing",
                        )
                    })?;
                attempt
                    .verify_integrity(&evidence.plan, &evidence.run, request)
                    .map_err(store_error)?;
                parents.push(quality_attempt_node(attempt, vec![], true));
            }
        }
        Ok(Some(quality_report_node(&report, parents, false)))
    }

    async fn load_row_quality_review(
        &self,
        id: Uuid,
    ) -> Result<Option<(RowQualityReview, DatasetQualityReport)>, ProvenanceStoreError> {
        let report_id: Option<Uuid> =
            sqlx::query_scalar("SELECT report_id FROM dataset_quality_row_reviews WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?;
        let Some(report_id) = report_id else {
            return Ok(None);
        };
        let report = self
            .get_report(report_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("row quality review report is missing"))?;
        let review = self
            .list_row_reviews(report_id)
            .await
            .map_err(store_error)?
            .into_iter()
            .find(|review| review.id == id)
            .ok_or_else(|| store_error("row quality review normalized evidence is missing"))?;
        Ok(Some((review, report)))
    }

    async fn row_quality_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some((review, report)) = self.load_row_quality_review(id).await? else {
            return Ok(None);
        };
        let report_parent = required_provenance_parent(
            Box::pin(self.dataset_quality_report_node(report.id)).await?,
            "row quality review report",
        )?;
        let mut parents = vec![report_parent];
        if let Some(predecessor_id) = review.predecessor_id {
            let Some((predecessor, _)) = self.load_row_quality_review(predecessor_id).await? else {
                return Err(store_error("row quality review predecessor is missing"));
            };
            if review.predecessor_fingerprint.as_deref() != Some(predecessor.fingerprint.as_str()) {
                return Err(store_error(
                    "row quality review predecessor fingerprint mismatch",
                ));
            }
            parents.push(row_review_node(&predecessor, vec![], true));
        }
        Ok(Some(row_review_node(&review, parents, false)))
    }

    async fn curation_proposal_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(proposal) = Box::pin(self.load_curation_proposal_for_provenance(id)).await? else {
            return Ok(None);
        };
        let report = self
            .get_report(proposal.report_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("curation proposal report is missing"))?;
        if report.fingerprint != proposal.report_fingerprint {
            return Err(store_error("curation proposal report fingerprint mismatch"));
        }
        let report_parent = required_provenance_parent(
            Box::pin(self.dataset_quality_report_node(report.id)).await?,
            "curation proposal report",
        )?;
        let all_reviews = self
            .list_row_reviews(report.id)
            .await
            .map_err(store_error)?;
        let predecessor = match proposal.predecessor_id {
            Some(predecessor_id) => Some(
                Box::pin(self.load_curation_proposal_for_provenance(predecessor_id))
                    .await?
                    .ok_or_else(|| store_error("curation proposal predecessor is missing"))?,
            ),
            None => None,
        };
        let referenced_reviews = proposal
            .row_review_references
            .iter()
            .map(|reference| {
                all_reviews
                    .iter()
                    .find(|review| {
                        review.id == reference.id && review.fingerprint == reference.fingerprint
                    })
                    .cloned()
                    .ok_or_else(|| store_error("curation proposal row review is missing or stale"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        proposal
            .verify_against(&report, predecessor.as_ref(), &referenced_reviews)
            .map_err(store_error)?;
        let mut parents = vec![report_parent];
        for reference in &proposal.row_review_references {
            let review = all_reviews
                .iter()
                .find(|review| review.id == reference.id)
                .ok_or_else(|| store_error("curation proposal row review is missing"))?;
            if review.fingerprint != reference.fingerprint {
                return Err(store_error(
                    "curation proposal row review fingerprint mismatch",
                ));
            }
            parents.push(row_review_node(review, vec![], true));
        }
        if let Some(predecessor) = predecessor {
            if proposal.predecessor_fingerprint.as_deref() != Some(predecessor.fingerprint.as_str())
            {
                return Err(store_error(
                    "curation proposal predecessor fingerprint mismatch",
                ));
            }
            parents.push(curation_proposal_node_value(&predecessor, vec![], true));
        }
        Ok(Some(curation_proposal_node_value(
            &proposal, parents, false,
        )))
    }

    async fn load_curation_manifest_review(
        &self,
        id: Uuid,
    ) -> Result<Option<CurationManifestReview>, ProvenanceStoreError> {
        let proposal_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT proposal_id FROM dataset_curation_manifest_reviews WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(proposal_id) = proposal_id else {
            return Ok(None);
        };
        let proposal_fingerprint: String =
            sqlx::query_scalar("SELECT fingerprint FROM dataset_curation_proposals WHERE id = ?")
                .bind(proposal_id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?
                .ok_or_else(|| store_error("curation manifest review proposal is missing"))?;
        let records = sqlx::query_as::<_, ProvenanceManifestReviewRecord>(
            "SELECT id, proposal_id, predecessor_id, decision, fingerprint, review_json, \
             created_at FROM dataset_curation_manifest_reviews WHERE proposal_id = ? \
             ORDER BY rowid",
        )
        .bind(proposal_id)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        let mut latest: Option<(Uuid, String)> = None;
        let mut target = None;
        for record in records {
            let review = record.into_checked()?;
            if review.proposal_id != proposal_id
                || review.proposal_fingerprint != proposal_fingerprint
                || review.reproduce_fingerprint().map_err(store_error)? != review.fingerprint
                || review.predecessor_id != latest.as_ref().map(|(id, _)| *id)
                || review.predecessor_fingerprint.as_deref()
                    != latest.as_ref().map(|(_, fingerprint)| fingerprint.as_str())
            {
                return Err(store_error(
                    "curation manifest review chain or proposal binding is invalid",
                ));
            }
            latest = Some((review.id, review.fingerprint.clone()));
            if review.id == id {
                target = Some(review);
            }
        }
        target
            .map(Some)
            .ok_or_else(|| store_error("curation manifest review normalized evidence is missing"))
    }

    async fn curation_manifest_review_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(review) = Box::pin(self.load_curation_manifest_review(id)).await? else {
            return Ok(None);
        };
        let proposal_parent = required_provenance_parent(
            Box::pin(self.curation_proposal_node(review.proposal_id)).await?,
            "curation manifest review proposal",
        )?;
        require_node_fingerprint(
            &proposal_parent,
            &review.proposal_fingerprint,
            "curation manifest review proposal",
        )?;
        let mut parents = vec![proposal_parent];
        if let Some(predecessor_id) = review.predecessor_id {
            let Some(predecessor) =
                Box::pin(self.load_curation_manifest_review(predecessor_id)).await?
            else {
                return Err(store_error(
                    "curation manifest review predecessor is missing",
                ));
            };
            if review.predecessor_fingerprint.as_deref() != Some(predecessor.fingerprint.as_str()) {
                return Err(store_error(
                    "curation manifest review predecessor fingerprint mismatch",
                ));
            }
            parents.push(manifest_review_node(&predecessor, vec![], true));
        }
        Ok(Some(manifest_review_node(&review, parents, false)))
    }

    async fn approved_curation_manifest_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(manifest) = self.load_curation_manifest_for_provenance(id).await? else {
            return Ok(None);
        };
        let (proposal, report, approval) = self.checked_manifest_evidence(&manifest).await?;
        let proposal_parent = required_provenance_parent(
            Box::pin(self.curation_proposal_node(proposal.id)).await?,
            "approved curation manifest proposal",
        )?;
        let parents = vec![
            manifest_review_node(&approval, vec![], true),
            proposal_parent,
            quality_report_node(&report, vec![], true),
        ];
        Ok(Some(approved_manifest_node(&manifest, parents, false)))
    }

    async fn checked_manifest_evidence(
        &self,
        manifest: &ApprovedCurationManifest,
    ) -> Result<
        (
            CurationProposal,
            DatasetQualityReport,
            CurationManifestReview,
        ),
        ProvenanceStoreError,
    > {
        let proposal = Box::pin(self.load_curation_proposal_for_provenance(manifest.proposal_id))
            .await?
            .ok_or_else(|| store_error("approved curation manifest proposal is missing"))?;
        let report = self
            .get_report(manifest.report_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("approved curation manifest report is missing"))?;
        let all_reviews = self
            .list_row_reviews(report.id)
            .await
            .map_err(store_error)?;
        let referenced_reviews = proposal
            .row_review_references
            .iter()
            .map(|reference| {
                all_reviews
                    .iter()
                    .find(|review| {
                        review.id == reference.id && review.fingerprint == reference.fingerprint
                    })
                    .cloned()
                    .ok_or_else(|| {
                        store_error(
                            "approved curation manifest proposal review is missing or stale",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let predecessor = match proposal.predecessor_id {
            Some(predecessor_id) => Some(
                Box::pin(self.load_curation_proposal_for_provenance(predecessor_id))
                    .await?
                    .ok_or_else(|| {
                        store_error("approved curation manifest proposal predecessor is missing")
                    })?,
            ),
            None => None,
        };
        proposal
            .verify_against(&report, predecessor.as_ref(), &referenced_reviews)
            .map_err(store_error)?;
        let approval = Box::pin(self.load_curation_manifest_review(manifest.approval_id))
            .await?
            .ok_or_else(|| store_error("approved curation manifest approval review is missing"))?;
        let latest_approval_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM dataset_curation_manifest_reviews WHERE proposal_id = ? \
             ORDER BY rowid DESC LIMIT 1",
        )
        .bind(proposal.id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .ok_or_else(|| store_error("approved curation manifest review chain is missing"))?;
        let complete_members = manifest.members.len() == proposal.entries.len()
            && manifest
                .members
                .iter()
                .zip(&proposal.entries)
                .all(|(member, entry)| {
                    member.source_row_id == entry.source_row_id
                        && member.source_row_fingerprint == entry.source_row_fingerprint
                        && member.proposal_entry_fingerprint == entry.fingerprint
                        && matches!(
                            (member.disposition, entry.decision),
                            (CurationDisposition::Include, CurationDecision::Include)
                                | (CurationDisposition::Exclude, CurationDecision::Exclude)
                        )
                });
        if manifest.report_id != report.id
            || manifest.report_fingerprint != report.fingerprint
            || manifest.proposal_id != proposal.id
            || manifest.proposal_fingerprint != proposal.fingerprint
            || manifest.dataset_definition_id != report.dataset_definition_id
            || manifest.dataset_definition_fingerprint != report.dataset_definition_fingerprint
            || manifest.approval_id != approval.id
            || manifest.approval_fingerprint != approval.fingerprint
            || approval.id != latest_approval_id
            || approval.proposal_id != proposal.id
            || approval.proposal_fingerprint != proposal.fingerprint
            || approval.decision != CurationManifestReviewDecision::Approve
            || proposal.counts.needs_review_rows != 0
            || !complete_members
            || manifest
                .reproduce_complete_member_fingerprint()
                .map_err(store_error)?
                != manifest.complete_member_fingerprint
            || manifest
                .reproduce_selected_member_fingerprint()
                .map_err(store_error)?
                != manifest.selected_member_fingerprint
        {
            return Err(store_error(
                "approved curation manifest failed checked provenance relationships",
            ));
        }
        Ok((proposal, report, approval))
    }

    async fn load_curation_proposal_for_provenance(
        &self,
        id: Uuid,
    ) -> Result<Option<CurationProposal>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ProvenanceCurationProposalRecord>(
            "SELECT id, report_id, dataset_id, predecessor_id, row_review_set_fingerprint, \
             fingerprint, proposal_json, created_at FROM dataset_curation_proposals WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let proposal = record.into_checked()?;
        let entries = sqlx::query(
            "SELECT source_row_id, decision, basis, fingerprint, entry_json \
             FROM dataset_curation_proposal_entries WHERE proposal_id = ? ORDER BY source_row_id",
        )
        .bind(id)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        if entries.len() != proposal.entries.len() {
            return Err(store_error(
                "curation proposal normalized entry manifest is incomplete",
            ));
        }
        for (record, expected) in entries.into_iter().zip(&proposal.entries) {
            let source_row_id: Uuid = record.try_get("source_row_id").map_err(store_error)?;
            let decision: String = record.try_get("decision").map_err(store_error)?;
            let basis: String = record.try_get("basis").map_err(store_error)?;
            let fingerprint: String = record.try_get("fingerprint").map_err(store_error)?;
            let entry_json: String = record.try_get("entry_json").map_err(store_error)?;
            let entry: CurationProposalEntry =
                serde_json::from_str(&entry_json).map_err(store_error)?;
            let expected_decision = match expected.decision {
                CurationDecision::Include => "include",
                CurationDecision::Exclude => "exclude",
                CurationDecision::NeedsReview => "needs_review",
            };
            let expected_basis = match expected.basis {
                CurationDecisionBasis::QualifiedAssessment => "qualified_assessment",
                CurationDecisionBasis::QuarantinedAssessment => "quarantined_assessment",
                CurationDecisionBasis::InvalidEvaluatorOutput => "invalid_evaluator_output",
                CurationDecisionBasis::Unaudited => "unaudited",
                CurationDecisionBasis::ConflictingEvidence => "conflicting_evidence",
                CurationDecisionBasis::BorderlineAssessment => "borderline_assessment",
                CurationDecisionBasis::HumanIncludeOverride => "human_include_override",
                CurationDecisionBasis::HumanExclude => "human_exclude",
                CurationDecisionBasis::ReassessmentRequested => "reassessment_requested",
            };
            if &entry != expected
                || source_row_id != expected.source_row_id
                || decision != expected_decision
                || basis != expected_basis
                || fingerprint != expected.fingerprint
            {
                return Err(store_error(
                    "curation proposal entry normalized facts disagree with JSON",
                ));
            }
        }
        Ok(Some(proposal))
    }

    async fn load_curation_manifest_for_provenance(
        &self,
        id: Uuid,
    ) -> Result<Option<ApprovedCurationManifest>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ProvenanceManifestRecord>(
            "SELECT id, report_id, proposal_id, approval_id, dataset_id, \
             complete_member_fingerprint, selected_member_fingerprint, fingerprint, \
             manifest_json, created_at FROM dataset_curation_manifests WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let manifest = record.into_checked()?;
        let members = sqlx::query(
            "SELECT source_row_id, disposition, source_row_fingerprint, \
             proposal_entry_fingerprint, member_json FROM dataset_curation_manifest_members \
             WHERE manifest_id = ? ORDER BY source_row_id",
        )
        .bind(id)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        if members.len() != manifest.members.len() {
            return Err(store_error(
                "curation manifest normalized membership is incomplete",
            ));
        }
        for (record, expected) in members.into_iter().zip(&manifest.members) {
            let source_row_id: Uuid = record.try_get("source_row_id").map_err(store_error)?;
            let disposition: String = record.try_get("disposition").map_err(store_error)?;
            let source_row_fingerprint: String = record
                .try_get("source_row_fingerprint")
                .map_err(store_error)?;
            let proposal_entry_fingerprint: String = record
                .try_get("proposal_entry_fingerprint")
                .map_err(store_error)?;
            let member_json: String = record.try_get("member_json").map_err(store_error)?;
            let member: CurationManifestMember =
                serde_json::from_str(&member_json).map_err(store_error)?;
            let expected_disposition = match expected.disposition {
                CurationDisposition::Include => "include",
                CurationDisposition::Exclude => "exclude",
            };
            if &member != expected
                || source_row_id != expected.source_row_id
                || disposition != expected_disposition
                || source_row_fingerprint != expected.source_row_fingerprint
                || proposal_entry_fingerprint != expected.proposal_entry_fingerprint
            {
                return Err(store_error(
                    "curation manifest member normalized facts disagree with JSON",
                ));
            }
        }
        Ok(Some(manifest))
    }

    async fn load_curation_application(
        &self,
        id: Uuid,
    ) -> Result<Option<CurationApplication>, ProvenanceStoreError> {
        let record = sqlx::query_as::<_, ProvenanceApplicationRecord>(
            "SELECT id, manifest_id, approval_id, snapshot_id, manifest_fingerprint, \
             snapshot_fingerprint, selected_member_fingerprint, \
             snapshot_membership_fingerprint, fingerprint, application_json, applied_at \
             FROM dataset_curation_applications WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let Some(record) = record else {
            return Ok(None);
        };
        Ok(Some(record.into_checked()?))
    }

    async fn curation_application_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(application) = Box::pin(self.load_curation_application(id)).await? else {
            return Ok(None);
        };
        let manifest =
            Box::pin(self.load_curation_manifest_for_provenance(application.manifest_id))
                .await?
                .ok_or_else(|| store_error("curation application manifest is missing"))?;
        let (proposal, report, approval) =
            Box::pin(self.checked_manifest_evidence(&manifest)).await?;
        let snapshot = self
            .get_snapshot(application.snapshot_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| store_error("curation application snapshot is missing"))?;
        let snapshot_members = self
            .list_snapshot_members(application.snapshot_id)
            .await
            .map_err(store_error)?;
        let membership_fingerprint =
            verify_curated_snapshot_for_provenance(&manifest, &snapshot, &snapshot_members)?;
        let snapshot_parent = required_provenance_parent(
            Box::pin(self.snapshot_reference_node(application.snapshot_id)).await?,
            "curation application snapshot",
        )?;
        if snapshot_parent.fingerprint.as_deref() != Some(application.snapshot_fingerprint.as_str())
        {
            return Err(store_error(
                "curation application snapshot fingerprint mismatch",
            ));
        }
        if application.manifest_fingerprint != manifest.fingerprint
            || application.approval_id != approval.id
            || application.approval_fingerprint != approval.fingerprint
            || application.snapshot_id != snapshot.id
            || application.snapshot_fingerprint != snapshot.fingerprint
            || application.selected_member_fingerprint != manifest.selected_member_fingerprint
            || application.snapshot_membership_fingerprint != membership_fingerprint
        {
            return Err(store_error(
                "curation application failed checked provenance relationships",
            ));
        }
        let proposal_parent = required_provenance_parent(
            Box::pin(self.curation_proposal_node(proposal.id)).await?,
            "curation application proposal",
        )?;
        let manifest_parent = approved_manifest_node(
            &manifest,
            vec![
                manifest_review_node(&approval, vec![], true),
                curation_proposal_node_value(&proposal, vec![], true),
                quality_report_node(&report, vec![], true),
            ],
            true,
        );
        Ok(Some(curation_application_node_value(
            &application,
            vec![snapshot_parent, manifest_parent, proposal_parent],
            false,
        )))
    }

    async fn snapshot_reference_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(snapshot) = self.get_snapshot(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let members = self.list_snapshot_members(id).await.map_err(store_error)?;
        dataset_core::splitting::verify_snapshot(&snapshot, &members).map_err(store_error)?;
        Ok(Some(node(
            ArtifactKind::Snapshot,
            id,
            Some(snapshot.fingerprint.clone()),
            &json!({
                "reference_only": true,
                "source_dataset_id": snapshot.source_dataset_id,
                "member_count": snapshot.member_count,
                "source_row_ids": members
                    .iter()
                    .map(|member| member.source_row_id)
                    .collect::<Vec<_>>(),
                "created_at": snapshot.created_at,
            }),
            vec![],
        )?))
    }

    async fn snapshot_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(snapshot) = self.get_snapshot(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let members = self.list_snapshot_members(id).await.map_err(store_error)?;
        let mut parents = self
            .dataset_node(snapshot.source_dataset_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        let curation_application_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM dataset_curation_applications WHERE snapshot_id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        let qualified = curation_application_id.is_some();
        let mut generated = BTreeMap::<Uuid, (String, String)>::new();
        let mut imported = BTreeMap::<Uuid, String>::new();
        for member in &members {
            match &member.source_provenance {
                SourceProvenance::Generated {
                    generation_job_id,
                    backend,
                    model,
                    ..
                } => {
                    let identity = (backend.clone(), model.clone());
                    if let Some(previous) = generated.insert(*generation_job_id, identity.clone())
                        && previous != identity
                    {
                        return Err(store_error(
                            "snapshot repeats a generation producer with conflicting identity",
                        ));
                    }
                }
                SourceProvenance::Imported {
                    import_id,
                    source_path,
                    ..
                } => {
                    if let Some(previous) = imported.insert(*import_id, source_path.clone())
                        && previous.as_str() != source_path
                    {
                        return Err(store_error(
                            "snapshot repeats an import producer with conflicting identity",
                        ));
                    }
                }
            }
        }
        for (job_id, (backend, model)) in generated {
            let job = self
                .checked_generation_producer(job_id, snapshot.source_dataset_id, &backend, &model)
                .await?;
            let plan = self.checked_generation_plan_for_job(&job).await?;
            if qualified {
                let mut producer = redacted_generation_job_node(&job);
                producer.parents.push(redacted_generation_plan_node(&plan));
                parents.push(producer);
            } else {
                parents.push(required_provenance_parent(
                    Box::pin(self.job_node(job.id)).await?,
                    "snapshot generation job",
                )?);
            }
        }
        for (import_id, source_path) in imported {
            let dataset_import = self
                .checked_import_producer(import_id, snapshot.source_dataset_id, &source_path)
                .await?;
            if qualified {
                parents.push(redacted_import_node(&dataset_import));
            } else {
                parents.push(required_provenance_parent(
                    self.import_node(dataset_import.id).await?,
                    "snapshot dataset import",
                )?);
            }
        }
        if let Some(application_id) = curation_application_id {
            parents.push(required_provenance_parent(
                Box::pin(self.curation_application_node(application_id)).await?,
                "qualified snapshot curation application",
            )?);
        }
        Ok(Some(node(
            ArtifactKind::Snapshot,
            id,
            Some(snapshot.fingerprint.clone()),
            &json!({
                "snapshot": snapshot,
                "source_row_ids": members.iter().map(|member| member.source_row_id).collect::<Vec<_>>(),
            }),
            parents,
        )?))
    }

    async fn training_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(run) = self.get_training_run(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        if run.input_binding.is_some() {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            crate::training::validate_training_input_authority(&mut connection, &run, false)
                .await
                .map_err(store_error)?;
        }
        let mut parents = self
            .snapshot_node(run.snapshot_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(base_model_id) = run.base_model_id {
            if let Some(base_model) = self.encoder_node(base_model_id).await? {
                parents.push(base_model);
            }
        }
        if let Some(parent_checkpoint_id) = run.parent_checkpoint_id {
            if let Some(parent_checkpoint) =
                Box::pin(self.checkpoint_node(parent_checkpoint_id)).await?
            {
                parents.push(parent_checkpoint);
            }
        }
        if let Some(binding) = &run.input_binding {
            if binding.authority_kind != "training_benchmark_check" {
                return Err(store_error(format!(
                    "unsupported training input authority in provenance: {}",
                    binding.authority_kind
                )));
            }
            let check = required_provenance_parent(
                self.training_benchmark_check_node(binding.authority_id)
                    .await?,
                "training run input authority",
            )?;
            require_node_fingerprint(
                &check,
                &binding.authority_fingerprint,
                "training run input authority",
            )?;
            parents.push(check);
        }
        Ok(Some(node(
            ArtifactKind::TrainingRun,
            id,
            None,
            &run,
            parents,
        )?))
    }

    async fn checkpoint_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(checkpoint) = self.get_checkpoint(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let parents = self
            .training_node(checkpoint.run_id)
            .await?
            .into_iter()
            .collect();
        Ok(Some(node(
            ArtifactKind::Checkpoint,
            id,
            Some(checkpoint.artifact_checksum.clone()),
            &checkpoint,
            parents,
        )?))
    }

    async fn evaluation_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(run) = self.get_evaluation_run(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = self
            .checkpoint_node(run.checkpoint_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(snapshot) = self.snapshot_node(run.snapshot_id).await? {
            parents.push(snapshot);
        }
        Ok(Some(node(
            ArtifactKind::EvaluationRun,
            id,
            Some(run.input_fingerprint.clone()),
            &run,
            parents,
        )?))
    }

    async fn analysis_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(report) = self.get_analysis_report(id).await.map_err(store_error)? else {
            return Ok(None);
        };
        let mut parents = Box::pin(self.evaluation_node(report.evaluation_run_id))
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(comparison_id) = report
            .source_identity
            .as_ref()
            .and_then(|source| source.comparison_id)
        {
            if let Some(comparison) = self.comparison_node(comparison_id).await? {
                parents.push(comparison);
            }
        }
        Ok(Some(node(
            ArtifactKind::AnalysisReport,
            id,
            Some(report.fingerprint.clone()),
            &report,
            parents,
        )?))
    }

    async fn optimization_node(
        &self,
        id: Uuid,
    ) -> Result<Option<ProvenanceNode>, ProvenanceStoreError> {
        let Some(proposal) = self
            .get_optimization_proposal(id)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let mut parents = Box::pin(self.analysis_node(proposal.analysis_report_id))
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(dataset) = self.dataset_node(proposal.dataset_id).await? {
            parents.push(dataset);
        }
        Ok(Some(node(
            ArtifactKind::OptimizationProposal,
            id,
            Some(proposal.fingerprint.clone()),
            &proposal,
            parents,
        )?))
    }
}

#[derive(Debug, FromRow)]
struct ConfigurationRecord {
    id: Uuid,
    fingerprint: String,
    dataset_id: Uuid,
    generation_plan_id: Uuid,
    resolved_toml_json: String,
}

#[derive(Debug, FromRow, Serialize)]
struct AnalysisReviewProvenanceRecord {
    analysis_report_id: Uuid,
    finding_key: String,
    state: String,
    note: Option<String>,
    resolution_evaluation_run_id: Option<Uuid>,
    resolution_comparison_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ConfigurationRecord {
    fn into_node(self) -> Result<ProvenanceNode, ProvenanceStoreError> {
        let resolved: serde_json::Value =
            serde_json::from_str(&self.resolved_toml_json).map_err(store_error)?;
        Ok(ProvenanceNode {
            kind: ArtifactKind::ProjectConfiguration,
            id: self.id,
            fingerprint: Some(self.fingerprint),
            attributes: json!({
                "dataset_id": self.dataset_id,
                "generation_plan_id": self.generation_plan_id,
                "resolved_configuration": resolved,
            }),
            parents: vec![],
        })
    }
}

#[derive(Debug, FromRow)]
struct ShallowProposalRecord {
    id: Uuid,
    fingerprint: Option<String>,
    analysis_report_id: Uuid,
    dataset_id: Uuid,
}

struct CheckedQualityReportEvidence {
    plan: AuditPlan,
    run: QualityAuditRun,
    plan_items: BTreeMap<Uuid, AuditPlanItem>,
    requests_by_id: BTreeMap<Uuid, BlindEvaluatorRequest>,
    attempts_by_id: BTreeMap<Uuid, EvaluatorAttempt>,
    assessments_by_id: BTreeMap<Uuid, RowQualityAssessment>,
}

#[derive(Debug, FromRow)]
struct ProvenanceSourceRowRecord {
    id: Uuid,
    dataset_id: Uuid,
    source_kind: String,
    source_ref: Uuid,
    cell_key: String,
    text: String,
    normalized_text: String,
    label: String,
    dimensions_json: String,
    fields_json: String,
    provenance_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ProvenanceSemanticBindingRecord {
    id: Uuid,
    dataset_id: Uuid,
    target_key: String,
    layer: String,
    profile_id: Option<Uuid>,
    profile_fingerprint: Option<String>,
    predecessor_id: Option<Uuid>,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceSemanticBindingRecord {
    fn into_checked(self) -> Result<SemanticBindingDecision, ProvenanceStoreError> {
        let binding: SemanticBindingDecision =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        let layer = match binding.layer {
            SemanticLayer::Reusable => "reusable",
            SemanticLayer::DatasetOverride => "dataset_override",
        };
        if self.id != binding.id
            || self.dataset_id != binding.dataset_id
            || self.target_key != binding.target.key()
            || self.layer != layer
            || self.profile_id != binding.profile_id
            || self.profile_fingerprint != binding.profile_fingerprint
            || self.predecessor_id != binding.predecessor_id
            || self.fingerprint != binding.fingerprint
            || self.created_at != binding.created_at
            || binding.reproduce_fingerprint().map_err(store_error)? != binding.fingerprint
        {
            return Err(store_error(
                "semantic binding normalized columns disagree with JSON",
            ));
        }
        Ok(binding)
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceAuthenticityBindingRecord {
    id: Uuid,
    dataset_id: Uuid,
    profile_id: Uuid,
    approval_id: Uuid,
    predecessor_id: Option<Uuid>,
    fingerprint: String,
    binding_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceAuthenticityBindingRecord {
    fn into_checked(self) -> Result<ProfileBinding, ProvenanceStoreError> {
        let binding: ProfileBinding =
            serde_json::from_str(&self.binding_json).map_err(store_error)?;
        if self.id != binding.id
            || self.dataset_id != binding.dataset_id
            || self.profile_id != binding.profile_id
            || self.approval_id != binding.approval_id
            || self.predecessor_id != binding.predecessor_id
            || self.fingerprint != binding.fingerprint
            || self.created_at != binding.created_at
            || binding.reproduce_fingerprint().map_err(store_error)? != binding.fingerprint
        {
            return Err(store_error(
                "authenticity binding normalized columns disagree with JSON",
            ));
        }
        Ok(binding)
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceAuthenticityReviewRecord {
    id: Uuid,
    profile_id: Uuid,
    predecessor_id: Option<Uuid>,
    decision: String,
    fingerprint: String,
    review_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceAuthenticityReviewRecord {
    fn into_checked(self) -> Result<ProfileReview, ProvenanceStoreError> {
        let review: ProfileReview = serde_json::from_str(&self.review_json).map_err(store_error)?;
        let decision = match review.decision {
            ProfileReviewDecision::Approve => "approve",
            ProfileReviewDecision::Reject => "reject",
            ProfileReviewDecision::RequestRevision => "request_revision",
        };
        if self.id != review.id
            || self.profile_id != review.profile_id
            || self.predecessor_id != review.predecessor_id
            || self.decision != decision
            || self.fingerprint != review.fingerprint
            || self.created_at != review.created_at
            || review.reproduce_fingerprint().map_err(store_error)? != review.fingerprint
        {
            return Err(store_error(
                "authenticity review normalized columns disagree with JSON",
            ));
        }
        Ok(review)
    }
}

impl ProvenanceSourceRowRecord {
    fn into_domain(self) -> Result<SourceRow, ProvenanceStoreError> {
        Ok(SourceRow {
            id: self.id,
            dataset_id: self.dataset_id,
            text: self.text,
            label: self.label,
            dimensions: serde_json::from_str(&self.dimensions_json).map_err(store_error)?,
            fields: serde_json::from_str(&self.fields_json).map_err(store_error)?,
            provenance: serde_json::from_str(&self.provenance_json).map_err(store_error)?,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceImportedRowRecord {
    id: Uuid,
    import_id: Uuid,
    dataset_id: Uuid,
    source_row_number: i64,
    cell_key: Option<String>,
    text: String,
    normalized_text: String,
    label: String,
    dimensions_json: String,
    validation_status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ProvenanceCurationProposalRecord {
    id: Uuid,
    report_id: Uuid,
    dataset_id: Uuid,
    predecessor_id: Option<Uuid>,
    row_review_set_fingerprint: String,
    fingerprint: String,
    proposal_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceCurationProposalRecord {
    fn into_checked(self) -> Result<CurationProposal, ProvenanceStoreError> {
        let proposal: CurationProposal =
            serde_json::from_str(&self.proposal_json).map_err(store_error)?;
        if self.id != proposal.id
            || self.report_id != proposal.report_id
            || self.dataset_id != proposal.dataset_definition_id
            || self.predecessor_id != proposal.predecessor_id
            || self.row_review_set_fingerprint != proposal.row_review_set_fingerprint
            || self.fingerprint != proposal.fingerprint
            || self.created_at != proposal.created_at
            || proposal.reproduce_fingerprint().map_err(store_error)? != proposal.fingerprint
        {
            return Err(store_error(
                "curation proposal normalized columns disagree with JSON",
            ));
        }
        Ok(proposal)
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceManifestReviewRecord {
    id: Uuid,
    proposal_id: Uuid,
    predecessor_id: Option<Uuid>,
    decision: String,
    fingerprint: String,
    review_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceManifestReviewRecord {
    fn into_checked(self) -> Result<CurationManifestReview, ProvenanceStoreError> {
        let review: CurationManifestReview =
            serde_json::from_str(&self.review_json).map_err(store_error)?;
        let expected_decision = match review.decision {
            CurationManifestReviewDecision::Approve => "approve",
            CurationManifestReviewDecision::Reject => "reject",
            CurationManifestReviewDecision::RequestRevision => "request_revision",
        };
        if self.id != review.id
            || self.proposal_id != review.proposal_id
            || self.predecessor_id != review.predecessor_id
            || self.decision != expected_decision
            || self.fingerprint != review.fingerprint
            || self.created_at != review.created_at
        {
            return Err(store_error(
                "curation manifest review normalized columns disagree with JSON",
            ));
        }
        Ok(review)
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceManifestRecord {
    id: Uuid,
    report_id: Uuid,
    proposal_id: Uuid,
    approval_id: Uuid,
    dataset_id: Uuid,
    complete_member_fingerprint: String,
    selected_member_fingerprint: String,
    fingerprint: String,
    manifest_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceManifestRecord {
    fn into_checked(self) -> Result<ApprovedCurationManifest, ProvenanceStoreError> {
        let manifest: ApprovedCurationManifest =
            serde_json::from_str(&self.manifest_json).map_err(store_error)?;
        if self.id != manifest.id
            || self.report_id != manifest.report_id
            || self.proposal_id != manifest.proposal_id
            || self.approval_id != manifest.approval_id
            || self.dataset_id != manifest.dataset_definition_id
            || self.complete_member_fingerprint != manifest.complete_member_fingerprint
            || self.selected_member_fingerprint != manifest.selected_member_fingerprint
            || self.fingerprint != manifest.fingerprint
            || self.created_at != manifest.created_at
            || manifest.reproduce_fingerprint().map_err(store_error)? != manifest.fingerprint
        {
            return Err(store_error(
                "curation manifest normalized columns disagree with JSON",
            ));
        }
        Ok(manifest)
    }
}

#[derive(Debug, FromRow)]
struct ProvenanceApplicationRecord {
    id: Uuid,
    manifest_id: Uuid,
    approval_id: Uuid,
    snapshot_id: Uuid,
    manifest_fingerprint: String,
    snapshot_fingerprint: String,
    selected_member_fingerprint: String,
    snapshot_membership_fingerprint: String,
    fingerprint: String,
    application_json: String,
    applied_at: chrono::DateTime<chrono::Utc>,
}

impl ProvenanceApplicationRecord {
    fn into_checked(self) -> Result<CurationApplication, ProvenanceStoreError> {
        let application: CurationApplication =
            serde_json::from_str(&self.application_json).map_err(store_error)?;
        if self.id != application.id
            || self.manifest_id != application.manifest_id
            || self.approval_id != application.approval_id
            || self.snapshot_id != application.snapshot_id
            || self.manifest_fingerprint != application.manifest_fingerprint
            || self.snapshot_fingerprint != application.snapshot_fingerprint
            || self.selected_member_fingerprint != application.selected_member_fingerprint
            || self.snapshot_membership_fingerprint != application.snapshot_membership_fingerprint
            || self.fingerprint != application.fingerprint
            || self.applied_at != application.applied_at
            || application.reproduce_fingerprint().map_err(store_error)? != application.fingerprint
        {
            return Err(store_error(
                "curation application normalized columns disagree with JSON",
            ));
        }
        Ok(application)
    }
}

impl ProvenanceImportedRowRecord {
    fn dimensions(&self) -> Result<BTreeMap<String, String>, ProvenanceStoreError> {
        serde_json::from_str(&self.dimensions_json).map_err(store_error)
    }
}

impl ShallowProposalRecord {
    fn into_node(self) -> ProvenanceNode {
        ProvenanceNode {
            kind: ArtifactKind::OptimizationProposal,
            id: self.id,
            fingerprint: self.fingerprint,
            attributes: json!({
                "analysis_report_id": self.analysis_report_id,
                "dataset_id": self.dataset_id,
                "note": "shallow reference prevents a cyclic applied-plan trace",
            }),
            parents: vec![],
        }
    }
}

// Quality traces use checked shallow references at high-fan-out and cyclic
// boundaries. This keeps a snapshot trace linear in its evidence set instead
// of duplicating the complete audit plan below every assessment, and prevents
// the qualified snapshot <-> curation application edge from recursing.

fn redacted_artifact_reference_node(
    kind: ArtifactKind,
    id: Uuid,
    fingerprint: Option<String>,
    attributes: serde_json::Value,
) -> ProvenanceNode {
    ProvenanceNode {
        kind,
        id,
        fingerprint,
        attributes,
        parents: vec![],
    }
}

fn redacted_source_row_node(
    row: &SourceRow,
    fingerprint: String,
    parents: Vec<ProvenanceNode>,
) -> ProvenanceNode {
    let provenance = match &row.provenance {
        SourceProvenance::Generated {
            generation_job_id,
            backend,
            model,
            construction_plan_fingerprint,
        } => json!({
            "kind": "generated",
            "generation_job_id": generation_job_id,
            "backend": backend,
            "model": model,
            "construction_plan_fingerprint": construction_plan_fingerprint,
        }),
        SourceProvenance::Imported {
            import_id,
            source_row_number,
            ..
        } => json!({
            "kind": "imported",
            "import_id": import_id,
            "source_row_number": source_row_number,
        }),
    };
    ProvenanceNode {
        kind: ArtifactKind::DatasetSourceRow,
        id: row.id,
        fingerprint: Some(fingerprint),
        attributes: json!({
            "dataset_id": row.dataset_id,
            "cell": {
                "label": row.label,
                "dimensions": row.dimensions,
            },
            "provenance": provenance,
            "created_at": row.created_at,
            "content_redacted": true,
        }),
        parents,
    }
}

fn redacted_generation_plan_node(plan: &GenerationPlan) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::GenerationPlan,
        id: plan.id,
        fingerprint: None,
        attributes: json!({
            "dataset_id": plan.dataset_id,
            "cell_count": plan.cells.len(),
            "total_target_count": plan.total_target_count(),
            "created_at": plan.created_at,
            "content_redacted": true,
            "reference_only": true,
        }),
        parents: vec![],
    }
}

fn redacted_generation_job_node(job: &GenerationJob) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::GenerationJob,
        id: job.id,
        fingerprint: None,
        attributes: json!({
            "dataset_id": job.dataset_id,
            "plan_id": job.plan_id,
            "backend_name": job.backend_name,
            "backend_model": job.backend_model,
            "state": job.state,
            "requested_rows": job.requested_rows,
            "generated_rows": job.generated_rows,
            "accepted_rows": job.accepted_rows,
            "rejected_rows": job.rejected_rows,
            "failed_requests": job.failed_requests,
            "cancel_requested": job.cancel_requested,
            "has_error_message": job.error_message.is_some(),
            "created_at": job.created_at,
            "updated_at": job.updated_at,
            "content_redacted": true,
            "reference_only": true,
        }),
        parents: vec![],
    }
}

fn redacted_import_node(dataset_import: &DatasetImport) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::DatasetImport,
        id: dataset_import.id,
        fingerprint: None,
        attributes: json!({
            "dataset_id": dataset_import.dataset_id,
            "format": dataset_import.format,
            "state": dataset_import.state,
            "processed_rows": dataset_import.processed_rows,
            "accepted_rows": dataset_import.accepted_rows,
            "rejected_rows": dataset_import.rejected_rows,
            "has_error_message": dataset_import.error_message.is_some(),
            "created_at": dataset_import.created_at,
            "updated_at": dataset_import.updated_at,
            "source_path_redacted": true,
            "mapping_redacted": true,
            "content_redacted": true,
            "reference_only": true,
        }),
        parents: vec![],
    }
}

fn quality_plan_node(
    plan: &AuditPlan,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::QualityAuditPlan,
        id: plan.id,
        fingerprint: Some(plan.fingerprint.clone()),
        attributes: json!({
            "schema_version": plan.schema_version,
            "dataset_definition_id": plan.dataset_schema.dataset_definition_id,
            "dataset_definition_fingerprint": plan.dataset_schema.dataset_definition_fingerprint,
            "policy_fingerprint": plan.policy.fingerprint,
            "source_set_fingerprint": plan.source_set_fingerprint,
            "resolved_guidance_fingerprint": plan.resolved_guidance_fingerprint,
            "guidance": plan.guidance,
            "evaluator_protocol_version": plan.evaluator_protocol_version,
            "population_rows": plan.population_count(),
            "selected_rows": plan.selected_count(),
            "created_at": plan.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn quality_semantic_guidance_node_value(
    guidance: &SemanticEvaluatorGuidance,
    parents: Vec<ProvenanceNode>,
) -> ProvenanceNode {
    let label_entry_count = guidance
        .labels
        .as_ref()
        .map_or(0, |labels| labels.entries.len());
    let dimension_entry_count = guidance
        .dimensions
        .values()
        .map(|dimension| dimension.entries.len())
        .sum::<usize>();
    ProvenanceNode {
        kind: ArtifactKind::QualitySemanticGuidance,
        id: guidance.reference.id,
        fingerprint: Some(guidance.reference.fingerprint.clone()),
        attributes: json!({
            "reference": guidance.reference,
            "sources": guidance.sources,
            "has_label_guidance": guidance.labels.is_some(),
            "label_entry_count": label_entry_count,
            "dimension_count": guidance.dimensions.len(),
            "dimension_entry_count": dimension_entry_count,
            "payload_redacted": true,
        }),
        parents,
    }
}

fn quality_run_node(
    run: &QualityAuditRun,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::QualityAuditRun,
        id: run.id,
        fingerprint: Some(run.specification_fingerprint.clone()),
        attributes: json!({
            "schema_version": run.schema_version,
            "plan_id": run.plan_id,
            "plan_fingerprint": run.plan_fingerprint,
            "source_set_fingerprint": run.source_set_fingerprint,
            "policy_fingerprint": run.policy_fingerprint,
            "primary_evaluator": run.primary_evaluator,
            "independent_reviewers": run.independent_reviewers,
            "state": run.state,
            "progress": run.progress,
            "usage": run.usage,
            "cancel_requested": run.cancel_requested,
            "stop_reason": run.stop_reason,
            "has_error_message": run.error_message.is_some(),
            "created_at": run.created_at,
            "started_at": run.started_at,
            "finished_at": run.finished_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn quality_attempt_node(
    attempt: &EvaluatorAttempt,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::QualityEvaluatorAttempt,
        id: attempt.id,
        fingerprint: Some(attempt.fingerprint.clone()),
        attributes: json!({
            "schema_version": attempt.schema_version,
            "run_id": attempt.run_id,
            "request_id": attempt.request_id,
            "request_sequence": attempt.request_sequence,
            "attempt_number": attempt.attempt_number,
            "evaluator": attempt.evaluator,
            "source_row_ids": attempt.source_row_ids,
            "request_fingerprint": attempt.request_fingerprint,
            "retry_payload_fingerprint": attempt.retry_payload_fingerprint,
            "state": attempt.state,
            "provider_usage": attempt.provider_usage,
            "invalid_source_row_ids": attempt.invalid_source_row_ids,
            "failure_kind": attempt.failure_kind,
            "has_error_message": attempt.error_message.is_some(),
            "metadata_redacted": true,
            "started_at": attempt.started_at,
            "finished_at": attempt.finished_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn quality_assessment_node(
    assessment: &RowQualityAssessment,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::RowQualityAssessment,
        id: assessment.id,
        fingerprint: Some(assessment.fingerprint.clone()),
        attributes: json!({
            "schema_version": assessment.schema_version,
            "audit_plan_id": assessment.audit_plan_id,
            "audit_plan_fingerprint": assessment.audit_plan_fingerprint,
            "audit_run_id": assessment.audit_run_id,
            "plan_item_fingerprint": assessment.plan_item_fingerprint,
            "source_row_id": assessment.source_row_id,
            "source_row_fingerprint": assessment.source_row_fingerprint,
            "request_id": assessment.request_id,
            "request_fingerprint": assessment.request_fingerprint,
            "attempt_id": assessment.attempt_id,
            "request_sequence": assessment.request_sequence,
            "attempt_number": assessment.attempt_number,
            "evaluator": assessment.evaluator,
            "generator_relationship": assessment.generator_relationship,
            "label_scores": assessment.label_scores,
            "assigned_label_score": assessment.assigned_label_score,
            "strongest_competing_label": assessment.strongest_competing_label,
            "strongest_competing_score": assessment.strongest_competing_score,
            "assigned_label_margin": assessment.assigned_label_margin,
            "dimension_scores": assessment.dimension_scores,
            "assigned_dimension_scores": assessment.assigned_dimension_scores,
            "authenticity_score": assessment.authenticity_score,
            "label_leakage_risk": assessment.label_leakage_risk,
            "shortcut_risk": assessment.shortcut_risk,
            "confidence": assessment.confidence,
            "issue_codes": assessment.issue_codes,
            "outcomes": assessment.outcomes,
            "verdict": assessment.verdict,
            "rationale_redacted": true,
            "created_at": assessment.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn quality_report_node(
    report: &DatasetQualityReport,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    let rows = report
        .rows
        .iter()
        .map(|row| {
            json!({
                "source_row_id": row.source_row_id,
                "source_row_fingerprint": row.source_row_fingerprint,
                "cell": row.cell,
                "provenance_stratum": row.provenance_stratum,
                "selection": row.selection,
                "assessment_references": row.assessment_references,
                "invalid_attempt_references": row.invalid_attempt_references,
                "verdict": row.verdict,
                "conflicting_evidence": row.conflicting_evidence,
                "issue_codes": row.issue_codes,
                "fingerprint": row.fingerprint,
            })
        })
        .collect::<Vec<_>>();
    ProvenanceNode {
        kind: ArtifactKind::DatasetQualityReport,
        id: report.id,
        fingerprint: Some(report.fingerprint.clone()),
        attributes: json!({
            "schema_version": report.schema_version,
            "plan_id": report.plan_id,
            "plan_fingerprint": report.plan_fingerprint,
            "run_id": report.run_id,
            "run_specification_fingerprint": report.run_specification_fingerprint,
            "dataset_definition_id": report.dataset_definition_id,
            "dataset_definition_fingerprint": report.dataset_definition_fingerprint,
            "source_set_fingerprint": report.source_set_fingerprint,
            "policy_fingerprint": report.policy_fingerprint,
            "assessment_set_fingerprint": report.assessment_set_fingerprint,
            "invalid_attempt_set_fingerprint": report.invalid_attempt_set_fingerprint,
            "totals": report.totals,
            "by_label": report.by_label,
            "by_cell": report.by_cell,
            "by_provenance": report.by_provenance,
            "by_issue": report.by_issue,
            "rows": rows,
            "created_at": report.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn row_review_node(
    review: &RowQualityReview,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::RowQualityReview,
        id: review.id,
        fingerprint: Some(review.fingerprint.clone()),
        attributes: json!({
            "report_id": review.report_id,
            "report_fingerprint": review.report_fingerprint,
            "source_row_id": review.source_row_id,
            "report_row_fingerprint": review.report_row_fingerprint,
            "predecessor_id": review.predecessor_id,
            "predecessor_fingerprint": review.predecessor_fingerprint,
            "decision": review.decision,
            "reviewer": review.reviewer,
            "reason_redacted": true,
            "created_at": review.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn curation_proposal_node_value(
    proposal: &CurationProposal,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    let entries = proposal
        .entries
        .iter()
        .map(|entry| {
            json!({
                "source_row_id": entry.source_row_id,
                "source_row_fingerprint": entry.source_row_fingerprint,
                "report_row_fingerprint": entry.report_row_fingerprint,
                "assessment_references": entry.assessment_references,
                "invalid_attempt_references": entry.invalid_attempt_references,
                "decision": entry.decision,
                "basis": entry.basis,
                "applied_row_review_id": entry.applied_row_review_id,
                "applied_row_review_fingerprint": entry.applied_row_review_fingerprint,
                "fingerprint": entry.fingerprint,
                "reasons_redacted": true,
            })
        })
        .collect::<Vec<_>>();
    ProvenanceNode {
        kind: ArtifactKind::CurationProposal,
        id: proposal.id,
        fingerprint: Some(proposal.fingerprint.clone()),
        attributes: json!({
            "schema_version": proposal.schema_version,
            "report_id": proposal.report_id,
            "report_fingerprint": proposal.report_fingerprint,
            "dataset_definition_id": proposal.dataset_definition_id,
            "dataset_definition_fingerprint": proposal.dataset_definition_fingerprint,
            "predecessor_id": proposal.predecessor_id,
            "predecessor_fingerprint": proposal.predecessor_fingerprint,
            "row_review_references": proposal.row_review_references,
            "row_review_set_fingerprint": proposal.row_review_set_fingerprint,
            "entries": entries,
            "counts": proposal.counts,
            "created_at": proposal.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn manifest_review_node(
    review: &CurationManifestReview,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::CurationManifestReview,
        id: review.id,
        fingerprint: Some(review.fingerprint.clone()),
        attributes: json!({
            "proposal_id": review.proposal_id,
            "proposal_fingerprint": review.proposal_fingerprint,
            "predecessor_id": review.predecessor_id,
            "predecessor_fingerprint": review.predecessor_fingerprint,
            "decision": review.decision,
            "reviewer": review.reviewer,
            "reason_redacted": true,
            "created_at": review.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn approved_manifest_node(
    manifest: &ApprovedCurationManifest,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::ApprovedCurationManifest,
        id: manifest.id,
        fingerprint: Some(manifest.fingerprint.clone()),
        attributes: json!({
            "schema_version": manifest.schema_version,
            "report_id": manifest.report_id,
            "report_fingerprint": manifest.report_fingerprint,
            "proposal_id": manifest.proposal_id,
            "proposal_fingerprint": manifest.proposal_fingerprint,
            "approval_id": manifest.approval_id,
            "approval_fingerprint": manifest.approval_fingerprint,
            "dataset_definition_id": manifest.dataset_definition_id,
            "dataset_definition_fingerprint": manifest.dataset_definition_fingerprint,
            "members": manifest.members,
            "complete_member_fingerprint": manifest.complete_member_fingerprint,
            "selected_member_fingerprint": manifest.selected_member_fingerprint,
            "created_at": manifest.created_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn curation_application_node_value(
    application: &CurationApplication,
    parents: Vec<ProvenanceNode>,
    reference_only: bool,
) -> ProvenanceNode {
    ProvenanceNode {
        kind: ArtifactKind::CurationApplication,
        id: application.id,
        fingerprint: Some(application.fingerprint.clone()),
        attributes: json!({
            "schema_version": application.schema_version,
            "manifest_id": application.manifest_id,
            "manifest_fingerprint": application.manifest_fingerprint,
            "approval_id": application.approval_id,
            "approval_fingerprint": application.approval_fingerprint,
            "snapshot_id": application.snapshot_id,
            "snapshot_fingerprint": application.snapshot_fingerprint,
            "selected_member_fingerprint": application.selected_member_fingerprint,
            "snapshot_membership_fingerprint": application.snapshot_membership_fingerprint,
            "applied_at": application.applied_at,
            "reference_only": reference_only,
        }),
        parents,
    }
}

fn verify_curated_snapshot_for_provenance(
    manifest: &ApprovedCurationManifest,
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<String, ProvenanceStoreError> {
    dataset_core::splitting::verify_snapshot(snapshot, members).map_err(store_error)?;
    if snapshot.source_dataset_id != manifest.dataset_definition_id
        || snapshot.member_count != members.len() as u64
        || members
            .iter()
            .any(|member| member.snapshot_id != snapshot.id)
    {
        return Err(store_error(
            "curation application snapshot identity or member count mismatch",
        ));
    }
    let selected = manifest
        .members
        .iter()
        .filter(|member| member.disposition == CurationDisposition::Include)
        .map(|member| (member.source_row_id, member))
        .collect::<BTreeMap<_, _>>();
    let mut source_row_ids = members
        .iter()
        .map(|member| member.source_row_id)
        .collect::<Vec<_>>();
    source_row_ids.sort_unstable();
    if source_row_ids.windows(2).any(|pair| pair[0] == pair[1])
        || source_row_ids != manifest.selected_source_row_ids()
    {
        return Err(store_error(
            "curation application snapshot membership differs from its manifest",
        ));
    }
    for member in members {
        let expected = selected.get(&member.source_row_id).ok_or_else(|| {
            store_error("curation application snapshot contains an unapproved source row")
        })?;
        let source = SourceRow {
            id: member.source_row_id,
            dataset_id: manifest.dataset_definition_id,
            text: member.text.clone(),
            label: member.label.clone(),
            dimensions: member.dimensions.clone(),
            fields: member.fields.clone(),
            provenance: member.source_provenance.clone(),
            created_at: member.source_created_at,
        };
        if artifact_core::fingerprint(&source).map_err(store_error)?
            != expected.source_row_fingerprint
        {
            return Err(store_error(
                "curation application snapshot content differs from its approved source row",
            ));
        }
    }
    let mut membership = members
        .iter()
        .map(|member| (member.source_row_id, member.split))
        .collect::<Vec<_>>();
    membership.sort();
    artifact_core::fingerprint(&membership).map_err(store_error)
}

fn required_provenance_parent(
    node: Option<ProvenanceNode>,
    description: &str,
) -> Result<ProvenanceNode, ProvenanceStoreError> {
    node.ok_or_else(|| store_error(format!("{description} is missing")))
}

fn require_node_fingerprint(
    node: &ProvenanceNode,
    expected: &str,
    description: &str,
) -> Result<(), ProvenanceStoreError> {
    if node.fingerprint.as_deref() == Some(expected) {
        Ok(())
    } else {
        Err(store_error(format!("{description} fingerprint mismatch")))
    }
}

fn workflow_artifact_kind(value: &str) -> Option<ArtifactKind> {
    match value {
        "generation_plan" | "iteration_generation_plan" => Some(ArtifactKind::GenerationPlan),
        "initial_allocation" => Some(ArtifactKind::InitialAllocation),
        "generation_job" | "dataset_diff_generation_job" => Some(ArtifactKind::GenerationJob),
        "quality_audit_plan" | "iteration_quality_audit_plan" => {
            Some(ArtifactKind::QualityAuditPlan)
        }
        "quality_audit_run" | "iteration_quality_audit_run" => Some(ArtifactKind::QualityAuditRun),
        "quality_report" | "iteration_quality_report" => Some(ArtifactKind::DatasetQualityReport),
        "curation_proposal" | "iteration_curation_proposal" => Some(ArtifactKind::CurationProposal),
        "curation_manifest_review" => Some(ArtifactKind::CurationManifestReview),
        "quality_manifest" | "iteration_quality_manifest" => {
            Some(ArtifactKind::ApprovedCurationManifest)
        }
        "curation_application" | "iteration_curation_application" => {
            Some(ArtifactKind::CurationApplication)
        }
        "snapshot" | "iteration_snapshot" => Some(ArtifactKind::Snapshot),
        "training_run" | "iteration_training_run" => Some(ArtifactKind::TrainingRun),
        "training_benchmark_check" | "iteration_training_benchmark_check" => {
            Some(ArtifactKind::TrainingBenchmarkCheck)
        }
        "checkpoint" | "iteration_checkpoint" => Some(ArtifactKind::Checkpoint),
        "evaluation_run" | "iteration_evaluation_run" | "sealed_evaluation_run" => {
            Some(ArtifactKind::EvaluationRun)
        }
        "evaluation_comparison" => Some(ArtifactKind::EvaluationComparison),
        "acceptance_assessment"
        | "development_acceptance_assessment"
        | "sealed_acceptance_assessment"
        | "development_acceptance_pass" => Some(ArtifactKind::AcceptanceAssessment),
        "advisory_assessment" => Some(ArtifactKind::AdvisoryAssessment),
        "analysis_report" | "followup_analysis_report" => Some(ArtifactKind::AnalysisReport),
        "optimization_proposal" => Some(ArtifactKind::OptimizationProposal),
        "proposal_review" => Some(ArtifactKind::OptimizationProposalReview),
        "workflow_approval" => Some(ArtifactKind::WorkflowApproval),
        "stop_decision" => Some(ArtifactKind::StopDecision),
        "model_promotion" => Some(ArtifactKind::ModelPromotion),
        _ => None,
    }
}

fn node(
    kind: ArtifactKind,
    id: Uuid,
    fingerprint: Option<String>,
    attributes: &impl Serialize,
    parents: Vec<ProvenanceNode>,
) -> Result<ProvenanceNode, ProvenanceStoreError> {
    Ok(ProvenanceNode {
        kind,
        id,
        fingerprint,
        attributes: serde_json::to_value(attributes).map_err(store_error)?,
        parents,
    })
}

fn store_error(error: impl std::fmt::Display) -> ProvenanceStoreError {
    ProvenanceStoreError(error.to_string())
}

#[cfg(test)]
mod quality_provenance_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use artifact_core::{ArtifactKind, ProvenanceNode, ProvenanceStore};
    use chrono::{TimeZone, Utc};
    use dataset_core::{
        domain::{
            DatasetImport, ImportFieldMapping, ImportFormat, ImportRowStatus, ImportState,
            ImportedRow, SourceProvenance, SourceRow, SplitConfiguration, SplitRatios,
        },
        ports::{AcceptedRowSource, ImportStore},
        splitting::build_snapshot,
    };
    use dataset_quality_core::{
        assessment::{
            BlindEvaluatorRequest, EvaluatorExecutionLocation, EvaluatorGuidance,
            EvaluatorIdentity, EvaluatorIndependence, EvaluatorRequestBudget, RowAssessmentDraft,
            RowQualityAssessment, SemanticEvaluatorGuidance,
        },
        curation::{
            ApprovedCurationManifest, CurationApplication, CurationManifestReview,
            CurationManifestReviewDecision, CurationProposal, DatasetQualityReport,
            RowQualityReview, RowQualityReviewDecision,
        },
        lifecycle::{AuditProgress, EvaluatorAttempt, ProviderUsage, QualityAuditRun},
        policy::{
            AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, CheckedAuditPlan, GuidanceReference, GuidanceReferences},
        ports::DatasetQualityStore,
    };
    use generation_core::{
        domain::{DatasetDefinition, DimensionDefinition, GenerationCell},
        ports::DatasetStore,
    };
    use serde_json::json;
    use uuid::Uuid;

    use crate::{SqliteStore, insert_dataset};

    struct QualifiedFixture {
        store: SqliteStore,
        snapshot_id: Uuid,
        application_id: Uuid,
        assessment_id: Uuid,
        plan_id: Uuid,
        source_row_id: Uuid,
    }

    async fn qualified_fixture() -> QualifiedFixture {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("temporary store");
        let dataset = DatasetDefinition::with_identity(
            Uuid::from_u128(10),
            "support",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset");
        let row = SourceRow {
            id: Uuid::from_u128(11),
            dataset_id: dataset.id,
            text: "charged twice on my card".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Imported {
                import_id: Uuid::from_u128(12),
                source_path: "fixture-secret-source.jsonl".into(),
                source_row_number: 1,
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 1)
                .single()
                .expect("time"),
        };
        let mut transaction = store.pool().begin().await.expect("transaction");
        insert_dataset(&mut transaction, &dataset)
            .await
            .expect("dataset");
        transaction.commit().await.expect("commit source fixture");
        let mut dataset_import = DatasetImport::queued(
            dataset.id,
            "fixture-secret-source.jsonl",
            ImportFormat::Jsonl,
            ImportFieldMapping::new(
                "sensitive-import-text-mapping",
                "label",
                BTreeMap::from([("difficulty".into(), "difficulty".into())]),
            )
            .expect("mapping"),
        )
        .expect("dataset import");
        dataset_import.id = Uuid::from_u128(12);
        dataset_import.state = ImportState::Completed;
        dataset_import.processed_rows = 1;
        dataset_import.accepted_rows = 1;
        dataset_import.error_message = Some("sensitive-import-error-marker".into());
        store
            .create_import(&dataset_import)
            .await
            .expect("create import");
        let imported_row = ImportedRow {
            id: row.id,
            import_id: dataset_import.id,
            dataset_id: dataset.id,
            source_row_number: 1,
            text: row.text.clone(),
            normalized_text: row.text.to_lowercase(),
            label: row.label.clone(),
            dimensions: row.dimensions.clone(),
            cell_key: Some(
                GenerationCell {
                    label: row.label.clone(),
                    dimensions: row.dimensions.clone(),
                }
                .key(),
            ),
            status: ImportRowStatus::Accepted,
            issues: Vec::new(),
            created_at: row.created_at,
        };
        store
            .insert_imported_rows(&dataset_import, &[imported_row])
            .await
            .expect("insert imported row");

        let policy = QualityPreset::Fast
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: Some(10_000),
            })
            .expect("policy");
        let guidance = EvaluatorGuidance::default();
        let guidance_fingerprint = guidance
            .reproduce_fingerprint()
            .expect("guidance fingerprint");
        let plan = AuditPlan::with_identity(
            Uuid::from_u128(20),
            &dataset,
            policy,
            GuidanceReferences::default(),
            guidance_fingerprint.clone(),
            "quality-evaluator-v1",
            vec![row.clone()],
            Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("plan");
        let evaluator = EvaluatorIdentity::new(
            "fake-quality",
            "fixture-v1",
            "quality-evaluator-v1",
            "sha256:fake-quality-config",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator");
        let mut run = QualityAuditRun::queue(&plan, evaluator.clone(), Vec::new()).expect("run");
        store
            .create_audit(&plan, &run, &guidance)
            .await
            .expect("create audit");
        run.start(&plan).expect("start run");
        store.save_audit_run(&run).await.expect("save run");
        let request = BlindEvaluatorRequest::create(
            &plan,
            Uuid::from_u128(30),
            run.id,
            Uuid::from_u128(31),
            1,
            1,
            &evaluator,
            vec![row.clone()],
            guidance,
            guidance_fingerprint,
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 10_000,
                maximum_total_tokens: 20_000,
                maximum_cost_microusd: Some(1_000),
            },
        )
        .expect("request");
        let mut attempt =
            EvaluatorAttempt::start(&plan, &run, &request, evaluator.clone()).expect("attempt");
        run.reserve_attempt(&plan, &request, &evaluator)
            .expect("reserve attempt");
        let checked = CheckedAuditPlan::new(&plan).expect("checked plan");
        let lease = store
            .acquire_audit_execution_lease(run.id, Uuid::new_v4())
            .await
            .expect("acquire execution lease");
        store
            .record_attempt(&checked, &lease, &attempt, &request, &run)
            .await
            .expect("record attempt");

        let high = BasisPoints::new(9_500).expect("score");
        let low = BasisPoints::new(500).expect("score");
        let assessment = RowQualityAssessment::create(
            &plan,
            &plan.items[0],
            &request,
            evaluator,
            RowAssessmentDraft {
                source_row_id: row.id,
                source_row_fingerprint: plan.items[0].source_row_fingerprint.clone(),
                label_scores: BTreeMap::from([("billing".into(), high), ("fraud".into(), low)]),
                dimension_scores: BTreeMap::from([(
                    "difficulty".into(),
                    BTreeMap::from([("easy".into(), high), ("hard".into(), low)]),
                )]),
                authenticity_score: None,
                label_leakage_risk: low,
                shortcut_risk: low,
                confidence: high,
                issue_codes: Vec::new(),
                rationale: "charged twice is clearly a billing request".into(),
            },
            &[],
            Utc::now(),
        )
        .expect("assessment");
        attempt
            .succeed(
                ProviderUsage::default(),
                json!({
                    "provider_echo": "charged twice on my card",
                    "secret": "sensitive-provider-marker"
                }),
            )
            .expect("finish attempt");
        run.reconcile_progress(AuditProgress {
            population_rows: 1,
            selected_rows: 1,
            assessed_rows: 1,
            qualified_rows: 1,
            ..AuditProgress::default()
        })
        .expect("progress");
        run.complete(&plan).expect("complete run");
        store
            .finish_attempt(
                &checked,
                &lease,
                &attempt,
                std::slice::from_ref(&assessment),
                &run,
            )
            .await
            .expect("finish attempt atomically");
        store
            .release_audit_execution_lease(&lease)
            .await
            .expect("release execution lease");
        let report = DatasetQualityReport::create(
            &plan,
            &run,
            std::slice::from_ref(&request),
            std::slice::from_ref(&attempt),
            std::slice::from_ref(&assessment),
        )
        .expect("report");
        store.save_report(&report).await.expect("save report");
        let row_review = RowQualityReview::create(
            &report,
            row.id,
            None,
            RowQualityReviewDecision::Include,
            "operator",
            "manual confirmation mentions charged twice and sensitive-review-marker",
        )
        .expect("row review");
        store
            .append_row_review(&row_review)
            .await
            .expect("save row review");
        let proposal = CurationProposal::create(&report, None, std::slice::from_ref(&row_review))
            .expect("proposal");
        store
            .save_curation_proposal(&proposal)
            .await
            .expect("save proposal");
        let approval = CurationManifestReview::create(
            &proposal,
            None,
            CurationManifestReviewDecision::Approve,
            "approver",
            "approve charged twice row with sensitive-approval-marker",
        )
        .expect("approval");
        store
            .append_manifest_review(&approval)
            .await
            .expect("save approval");
        let manifest = ApprovedCurationManifest::create(
            &report,
            &proposal,
            None,
            std::slice::from_ref(&row_review),
            std::slice::from_ref(&approval),
        )
        .expect("manifest");
        store.save_manifest(&manifest).await.expect("save manifest");
        let (snapshot, members) = build_snapshot(
            dataset.id,
            "qualified-v1",
            None,
            SplitConfiguration::new(SplitRatios::new(1.0, 0.0, 0.0).expect("ratios"), 42),
            vec![row.clone()],
        )
        .expect("snapshot");
        let application = CurationApplication::create(
            &manifest,
            &report,
            &proposal,
            None,
            std::slice::from_ref(&row_review),
            std::slice::from_ref(&approval),
            &snapshot,
            &members,
        )
        .expect("application");
        store
            .apply_manifest(&application, &snapshot, &members)
            .await
            .expect("apply manifest");
        QualifiedFixture {
            store,
            snapshot_id: snapshot.id,
            application_id: application.id,
            assessment_id: assessment.id,
            plan_id: plan.id,
            source_row_id: row.id,
        }
    }

    #[tokio::test]
    async fn qualified_snapshot_provenance_is_complete_checked_and_redacted() {
        let fixture = qualified_fixture().await;
        let trace = fixture
            .store
            .trace_provenance(ArtifactKind::Snapshot, fixture.snapshot_id)
            .await
            .expect("trace snapshot")
            .expect("snapshot provenance");
        let mut kinds = BTreeSet::new();
        collect_kinds(&trace, &mut kinds);
        for expected in [
            ArtifactKind::Snapshot,
            ArtifactKind::CurationApplication,
            ArtifactKind::ApprovedCurationManifest,
            ArtifactKind::CurationManifestReview,
            ArtifactKind::CurationProposal,
            ArtifactKind::RowQualityReview,
            ArtifactKind::DatasetQualityReport,
            ArtifactKind::RowQualityAssessment,
            ArtifactKind::QualityEvaluatorAttempt,
            ArtifactKind::QualityAuditRun,
            ArtifactKind::QualityAuditPlan,
            ArtifactKind::DatasetSourceRow,
            ArtifactKind::DatasetImport,
            ArtifactKind::Dataset,
        ] {
            assert!(kinds.contains(expected.as_str()), "missing {expected:?}");
        }
        let rendered = serde_json::to_string(&trace).expect("serialize trace");
        for forbidden in [
            "charged twice on my card",
            "charged twice is clearly",
            "fixture-secret-source.jsonl",
            "sensitive-import-text-mapping",
            "sensitive-import-error-marker",
            "sensitive-provider-marker",
            "sensitive-review-marker",
            "sensitive-approval-marker",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "provenance leaked redacted value {forbidden:?}"
            );
        }

        let assessment = fixture
            .store
            .trace_provenance(ArtifactKind::RowQualityAssessment, fixture.assessment_id)
            .await
            .expect("trace assessment")
            .expect("assessment provenance");
        let parent_kinds = assessment
            .parents
            .iter()
            .map(|parent| parent.kind)
            .collect::<Vec<_>>();
        assert_eq!(parent_kinds.len(), 3);
        assert!(parent_kinds.contains(&ArtifactKind::QualityEvaluatorAttempt));
        assert!(parent_kinds.contains(&ArtifactKind::QualityAuditPlan));
        assert!(parent_kinds.contains(&ArtifactKind::DatasetSourceRow));

        let application = fixture
            .store
            .trace_provenance(ArtifactKind::CurationApplication, fixture.application_id)
            .await
            .expect("trace curation application")
            .expect("curation application provenance");
        let rendered_application =
            serde_json::to_vec(&application).expect("serialize curation application trace");
        assert!(rendered_application.len() < 1_000_000);
    }

    #[tokio::test]
    async fn quality_provenance_rejects_tampering_and_returns_none_for_missing_roots() {
        let fixture = qualified_fixture().await;
        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::CurationApplication, Uuid::new_v4())
                .await
                .expect("missing query")
                .is_none()
        );
        sqlx::query(
            "UPDATE dataset_quality_row_assessments SET fingerprint = 'sha256:tampered' \
             WHERE id = ?",
        )
        .bind(fixture.assessment_id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper assessment");
        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::Snapshot, fixture.snapshot_id)
                .await
                .is_err()
        );
        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::CurationApplication, fixture.application_id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn source_row_provenance_rejects_normalized_column_and_producer_tampering() {
        let fixture = qualified_fixture().await;
        sqlx::query("UPDATE dataset_source_rows SET normalized_text = 'tampered' WHERE id = ?")
            .bind(fixture.source_row_id)
            .execute(fixture.store.pool())
            .await
            .expect("tamper normalized text");
        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::DatasetSourceRow, fixture.source_row_id)
                .await
                .is_err()
        );

        let fixture = qualified_fixture().await;
        sqlx::query("UPDATE dataset_imports SET source_path = 'tampered' WHERE id = ?")
            .bind(Uuid::from_u128(12))
            .execute(fixture.store.pool())
            .await
            .expect("tamper import ownership");
        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::QualityAuditPlan, fixture.plan_id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn quality_plan_requires_every_pinned_semantic_guidance_source() {
        let fixture = qualified_fixture().await;
        let dataset = fixture
            .store
            .get_dataset(Uuid::from_u128(10))
            .await
            .expect("load dataset")
            .expect("dataset");
        let rows = fixture
            .store
            .list_accepted_source_rows(dataset.id)
            .await
            .expect("load source rows");
        let plan_id = Uuid::new_v4();
        let context_reference = GuidanceReference::new(
            plan_id,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("context reference");
        let missing_source = GuidanceReference::new(
            Uuid::new_v4(),
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .expect("source reference");
        let guidance = EvaluatorGuidance {
            semantic: Some(SemanticEvaluatorGuidance {
                reference: context_reference.clone(),
                sources: vec![missing_source],
                labels: None,
                dimensions: BTreeMap::new(),
            }),
            authenticity: None,
        };
        let policy = QualityPreset::Fast
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: Some(10_000),
            })
            .expect("policy");
        let plan = AuditPlan::with_identity(
            plan_id,
            &dataset,
            policy,
            GuidanceReferences {
                semantic_context: Some(context_reference),
                authenticity_context: None,
            },
            guidance
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            "quality-evaluator-v1",
            rows,
            Utc::now(),
        )
        .expect("plan");
        let evaluator = EvaluatorIdentity::new(
            "fake-quality",
            "fixture-v1",
            "quality-evaluator-v1",
            "sha256:fake-quality-config",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator");
        let run = QualityAuditRun::queue(&plan, evaluator, Vec::new()).expect("run");
        fixture
            .store
            .create_audit(&plan, &run, &guidance)
            .await
            .expect("create audit");

        assert!(
            fixture
                .store
                .trace_provenance(ArtifactKind::QualityAuditPlan, plan.id)
                .await
                .is_err()
        );
    }

    fn collect_kinds(node: &ProvenanceNode, kinds: &mut BTreeSet<&'static str>) {
        kinds.insert(node.kind.as_str());
        for parent in &node.parents {
            collect_kinds(parent, kinds);
        }
    }
}
