use std::collections::BTreeSet;

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
    domain::SourceProvenance,
    ports::{ImportStore, SnapshotStore},
};
use evaluation_core::ports::EvaluationStore;
use generation_core::ports::{DatasetStore, GenerationExecutionStore, JobStore, PlanStore};
use generation_core::strategy::ResolvedGenerationStrategyContext;
use optimization_core::{
    campaigns::{CampaignArtifactKind, CampaignArtifactLink, CampaignOutcomeAssessment},
    ports::OptimizationStore,
    reviews::ProposalReviewRecord,
};
use project_preparation::{BootstrapStore, PreparationStore};
use research_core::{
    evidence::{ResearchClaim, ResearchEvidence},
    ports::ResearchStore,
    profile::{ProfileBinding, ProfileReview},
};
use semantic_catalog::{SemanticBindingDecision, SemanticCatalogStore};
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use training_core::ports::{EncoderRegistry, TrainingStore};
use uuid::Uuid;
use workflow_core::ports::{
    AdvisorStore, BenchmarkStore, InitialAllocationStore, PromotionStore, StopDecisionStore,
    WorkflowApprovalStore, WorkflowRunStore,
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
                ArtifactKind::DatasetArchitectBrief => self.architect_brief_node(id).await,
                ArtifactKind::DatasetArchitectRun => self.architect_run_node(id).await,
                ArtifactKind::DatasetArchitectureProposal => {
                    self.architecture_proposal_node(id).await
                }
                ArtifactKind::DatasetArchitectureReview => self.architecture_review_node(id).await,
                ArtifactKind::DatasetArchitectureApplication => {
                    self.architecture_application_node(id).await
                }
                ArtifactKind::GenerationStrategyContext => self.generation_strategy_node(id).await,
                ArtifactKind::InitialAllocation => self.initial_allocation_node(id).await,
                ArtifactKind::GenerationPlan => self.plan_node(id).await,
                ArtifactKind::GenerationJob => self.job_node(id).await,
                ArtifactKind::DatasetImport => self.import_node(id).await,
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
        let parents = self
            .architect_brief_node(artifact.brief_id)
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
        let parents = self
            .architect_run_node(artifact.run_id)
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
        let mut parents = self
            .architecture_proposal_node(artifact.proposal_id)
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
        let mut parents = self
            .architecture_proposal_node(artifact.proposal_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(review) = self.architecture_review_node(artifact.approval_id).await? {
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
            Some(id) => self
                .architecture_application_node(id)
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
        let mut parents = self
            .workflow_definition_node(run.definition_id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        for artifact in attempts.iter().flat_map(|attempt| &attempt.artifacts) {
            let parent = match artifact.kind.as_str() {
                "generation_plan" | "iteration_generation_plan" => {
                    self.plan_node(artifact.artifact_id).await?
                }
                "initial_allocation" => self.initial_allocation_node(artifact.artifact_id).await?,
                "generation_job" | "dataset_diff_generation_job" => {
                    self.job_node(artifact.artifact_id).await?
                }
                "snapshot" | "iteration_snapshot" => {
                    self.snapshot_node(artifact.artifact_id).await?
                }
                "training_run" | "iteration_training_run" => {
                    self.training_node(artifact.artifact_id).await?
                }
                "checkpoint" | "iteration_checkpoint" => {
                    self.checkpoint_node(artifact.artifact_id).await?
                }
                "evaluation_run" | "iteration_evaluation_run" | "sealed_evaluation_run" => {
                    self.evaluation_node(artifact.artifact_id).await?
                }
                "evaluation_comparison" => self.comparison_node(artifact.artifact_id).await?,
                "analysis_report" | "followup_analysis_report" => {
                    self.analysis_node(artifact.artifact_id).await?
                }
                "optimization_proposal" => self.optimization_node(artifact.artifact_id).await?,
                "proposal_review" => self.optimization_review_node(artifact.artifact_id).await?,
                _ => None,
            };
            if let Some(parent) = parent {
                parents.push(parent);
            }
        }
        Ok(Some(node(
            ArtifactKind::WorkflowRun,
            id,
            run.latest_attempt_fingerprint.clone(),
            &serde_json::json!({"run": run, "attempts": attempts}),
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
            && let Some(application) = self.architecture_application_node(application_id).await?
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
        let mut generated = BTreeSet::new();
        let mut imported = BTreeSet::new();
        for member in &members {
            match member.source_provenance {
                SourceProvenance::Generated {
                    generation_job_id, ..
                } => {
                    generated.insert(generation_job_id);
                }
                SourceProvenance::Imported { import_id, .. } => {
                    imported.insert(import_id);
                }
            }
        }
        for job_id in generated {
            if let Some(parent) = self.job_node(job_id).await? {
                parents.push(parent);
            }
        }
        for import_id in imported {
            if let Some(parent) = self.import_node(import_id).await? {
                parents.push(parent);
            }
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
        let mut parents = self
            .evaluation_node(report.evaluation_run_id)
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
        let mut parents = self
            .analysis_node(proposal.analysis_report_id)
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
