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
        ArtifactKindArg::ResearchBrief => ArtifactKind::ResearchBrief,
        ArtifactKindArg::ResearchRun => ArtifactKind::ResearchRun,
        ArtifactKindArg::ResearchEvidence => ArtifactKind::ResearchEvidence,
        ArtifactKindArg::ResearchClaim => ArtifactKind::ResearchClaim,
        ArtifactKindArg::AuthenticityProfile => ArtifactKind::AuthenticityProfile,
        ArtifactKindArg::AuthenticityProfileReview => ArtifactKind::AuthenticityProfileReview,
        ArtifactKindArg::AuthenticityProfileBinding => ArtifactKind::AuthenticityProfileBinding,
        ArtifactKindArg::GenerationAuthenticityContext => {
            ArtifactKind::GenerationAuthenticityContext
        }
        ArtifactKindArg::DatasetArchitectBrief => ArtifactKind::DatasetArchitectBrief,
        ArtifactKindArg::DatasetArchitectRun => ArtifactKind::DatasetArchitectRun,
        ArtifactKindArg::DatasetArchitectureProposal => ArtifactKind::DatasetArchitectureProposal,
        ArtifactKindArg::DatasetArchitectureReview => ArtifactKind::DatasetArchitectureReview,
        ArtifactKindArg::DatasetArchitectureApplication => {
            ArtifactKind::DatasetArchitectureApplication
        }
        ArtifactKindArg::GenerationStrategyContext => ArtifactKind::GenerationStrategyContext,
        ArtifactKindArg::InitialAllocation => ArtifactKind::InitialAllocation,
        ArtifactKindArg::GenerationPlan => ArtifactKind::GenerationPlan,
        ArtifactKindArg::GenerationJob => ArtifactKind::GenerationJob,
        ArtifactKindArg::DatasetImport => ArtifactKind::DatasetImport,
        ArtifactKindArg::DatasetSourceRow => ArtifactKind::DatasetSourceRow,
        ArtifactKindArg::QualityAuditPlan => ArtifactKind::QualityAuditPlan,
        ArtifactKindArg::QualitySemanticGuidance => ArtifactKind::QualitySemanticGuidance,
        ArtifactKindArg::QualityAuditRun => ArtifactKind::QualityAuditRun,
        ArtifactKindArg::QualityEvaluatorAttempt => ArtifactKind::QualityEvaluatorAttempt,
        ArtifactKindArg::RowQualityAssessment => ArtifactKind::RowQualityAssessment,
        ArtifactKindArg::DatasetQualityReport => ArtifactKind::DatasetQualityReport,
        ArtifactKindArg::RowQualityReview => ArtifactKind::RowQualityReview,
        ArtifactKindArg::CurationProposal => ArtifactKind::CurationProposal,
        ArtifactKindArg::CurationManifestReview => ArtifactKind::CurationManifestReview,
        ArtifactKindArg::ApprovedCurationManifest => ArtifactKind::ApprovedCurationManifest,
        ArtifactKindArg::CurationApplication => ArtifactKind::CurationApplication,
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
        ArtifactKindArg::BenchmarkSuite => ArtifactKind::BenchmarkSuite,
        ArtifactKindArg::ContaminationReport => ArtifactKind::ContaminationReport,
        ArtifactKindArg::BenchmarkBundle => ArtifactKind::BenchmarkBundle,
        ArtifactKindArg::BenchmarkQualification => ArtifactKind::BenchmarkQualification,
        ArtifactKindArg::TrainingBenchmarkCheck => ArtifactKind::TrainingBenchmarkCheck,
        ArtifactKindArg::WorkflowDefinition => ArtifactKind::WorkflowDefinition,
        ArtifactKindArg::WorkflowRun => ArtifactKind::WorkflowRun,
        ArtifactKindArg::AcceptanceAssessment => ArtifactKind::AcceptanceAssessment,
        ArtifactKindArg::AdvisoryAssessment => ArtifactKind::AdvisoryAssessment,
        ArtifactKindArg::WorkflowApproval => ArtifactKind::WorkflowApproval,
        ArtifactKindArg::StopDecision => ArtifactKind::StopDecision,
        ArtifactKindArg::ModelPromotion => ArtifactKind::ModelPromotion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_quality_cli_kinds_map_exactly() {
        let mappings = [
            (
                ArtifactKindArg::DatasetSourceRow,
                ArtifactKind::DatasetSourceRow,
            ),
            (
                ArtifactKindArg::QualityAuditPlan,
                ArtifactKind::QualityAuditPlan,
            ),
            (
                ArtifactKindArg::QualitySemanticGuidance,
                ArtifactKind::QualitySemanticGuidance,
            ),
            (
                ArtifactKindArg::QualityAuditRun,
                ArtifactKind::QualityAuditRun,
            ),
            (
                ArtifactKindArg::QualityEvaluatorAttempt,
                ArtifactKind::QualityEvaluatorAttempt,
            ),
            (
                ArtifactKindArg::RowQualityAssessment,
                ArtifactKind::RowQualityAssessment,
            ),
            (
                ArtifactKindArg::DatasetQualityReport,
                ArtifactKind::DatasetQualityReport,
            ),
            (
                ArtifactKindArg::RowQualityReview,
                ArtifactKind::RowQualityReview,
            ),
            (
                ArtifactKindArg::CurationProposal,
                ArtifactKind::CurationProposal,
            ),
            (
                ArtifactKindArg::CurationManifestReview,
                ArtifactKind::CurationManifestReview,
            ),
            (
                ArtifactKindArg::ApprovedCurationManifest,
                ArtifactKind::ApprovedCurationManifest,
            ),
            (
                ArtifactKindArg::CurationApplication,
                ArtifactKind::CurationApplication,
            ),
        ];

        for (argument, expected) in mappings {
            assert_eq!(artifact_kind(argument), expected);
        }
    }

    #[test]
    fn benchmark_cli_kinds_map_exactly() {
        let mappings = [
            (
                ArtifactKindArg::BenchmarkSuite,
                ArtifactKind::BenchmarkSuite,
            ),
            (
                ArtifactKindArg::ContaminationReport,
                ArtifactKind::ContaminationReport,
            ),
            (
                ArtifactKindArg::BenchmarkBundle,
                ArtifactKind::BenchmarkBundle,
            ),
            (
                ArtifactKindArg::BenchmarkQualification,
                ArtifactKind::BenchmarkQualification,
            ),
            (
                ArtifactKindArg::TrainingBenchmarkCheck,
                ArtifactKind::TrainingBenchmarkCheck,
            ),
        ];

        for (argument, expected) in mappings {
            assert_eq!(artifact_kind(argument), expected);
        }
    }
}
