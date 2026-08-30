use anyhow::Context;
use artifact_core::{ArtifactKind, ProvenanceStore};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::ArtifactKindArg;

pub async fn execute(
    kind: ArtifactKindArg,
    id: uuid::Uuid,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    let kind = artifact_kind(kind);
    let trace = store
        .trace_provenance(kind, id)
        .await?
        .with_context(|| format!("{} artifact not found: {id}", kind.as_str()))?;
    crate::presentation::print(&trace)
}

const fn artifact_kind(kind: ArtifactKindArg) -> ArtifactKind {
    match kind {
        ArtifactKindArg::ProjectBootstrap => ArtifactKind::ProjectBootstrap,
        ArtifactKindArg::ProjectPreparation => ArtifactKind::ProjectPreparation,
        ArtifactKindArg::ProjectConfiguration => ArtifactKind::ProjectConfiguration,
        ArtifactKindArg::Dataset => ArtifactKind::Dataset,
        ArtifactKindArg::SemanticProfile => ArtifactKind::SemanticProfile,
        ArtifactKindArg::SemanticBinding => ArtifactKind::SemanticBinding,
        ArtifactKindArg::GenerationSemanticContext => ArtifactKind::GenerationSemanticContext,
        ArtifactKindArg::GenerationPlan => ArtifactKind::GenerationPlan,
        ArtifactKindArg::GenerationJob => ArtifactKind::GenerationJob,
        ArtifactKindArg::DatasetImport => ArtifactKind::DatasetImport,
        ArtifactKindArg::Snapshot => ArtifactKind::Snapshot,
        ArtifactKindArg::BaseModel => ArtifactKind::BaseModel,
        ArtifactKindArg::TrainingRun => ArtifactKind::TrainingRun,
        ArtifactKindArg::Checkpoint => ArtifactKind::Checkpoint,
        ArtifactKindArg::EvaluationRun => ArtifactKind::EvaluationRun,
        ArtifactKindArg::EvaluationComparison => ArtifactKind::EvaluationComparison,
        ArtifactKindArg::ModelSelection => ArtifactKind::ModelSelection,
        ArtifactKindArg::AnalysisReport => ArtifactKind::AnalysisReport,
        ArtifactKindArg::AnalysisFindingReview => ArtifactKind::AnalysisFindingReview,
        ArtifactKindArg::OptimizationProposal => ArtifactKind::OptimizationProposal,
        ArtifactKindArg::OptimizationProposalReview => ArtifactKind::OptimizationProposalReview,
        ArtifactKindArg::OptimizationCampaign => ArtifactKind::OptimizationCampaign,
        ArtifactKindArg::OptimizationCampaignLink => ArtifactKind::OptimizationCampaignLink,
        ArtifactKindArg::OptimizationOutcome => ArtifactKind::OptimizationOutcome,
        ArtifactKindArg::WorkflowDefinition => ArtifactKind::WorkflowDefinition,
        ArtifactKindArg::WorkflowRun => ArtifactKind::WorkflowRun,
        ArtifactKindArg::AcceptanceAssessment => ArtifactKind::AcceptanceAssessment,
        ArtifactKindArg::AdvisoryAssessment => ArtifactKind::AdvisoryAssessment,
        ArtifactKindArg::WorkflowApproval => ArtifactKind::WorkflowApproval,
        ArtifactKindArg::StopDecision => ArtifactKind::StopDecision,
        ArtifactKindArg::ModelPromotion => ArtifactKind::ModelPromotion,
    }
}
