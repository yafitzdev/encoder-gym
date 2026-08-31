//! Deterministic artifact fingerprints and infrastructure-independent provenance shapes.

use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error)]
pub enum FingerprintError {
    #[error("could not serialize fingerprint input: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Hashes compact JSON with sorted object keys. Struct field order and BTreeMap ordering are stable;
/// arbitrary JSON objects are recursively canonicalized before hashing.
pub fn fingerprint<T: Serialize>(value: &T) -> Result<String, FingerprintError> {
    // Normalize through the same byte representation used by JSON persistence.
    // Some IEEE-754 values can be rendered and reparsed to an adjacent value;
    // hashing the direct `Value` representation would then make a freshly
    // persisted artifact fail its own reproduction check.
    let persisted = serde_json::to_vec(value)?;
    let value = canonicalize(serde_json::from_slice(&persisted)?);
    let bytes = serde_json::to_vec(&value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize).collect()),
        Value::Object(fields) => {
            let mut entries = fields.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    ProjectBootstrap,
    ProjectPreparation,
    ProjectConfiguration,
    Dataset,
    SemanticProfile,
    SemanticBinding,
    GenerationSemanticContext,
    ResearchBrief,
    ResearchRun,
    ResearchEvidence,
    ResearchClaim,
    AuthenticityProfile,
    AuthenticityProfileReview,
    AuthenticityProfileBinding,
    GenerationAuthenticityContext,
    DatasetArchitectBrief,
    DatasetArchitectRun,
    DatasetArchitectureProposal,
    DatasetArchitectureReview,
    DatasetArchitectureApplication,
    GenerationStrategyContext,
    InitialAllocation,
    GenerationPlan,
    GenerationJob,
    DatasetImport,
    DatasetSourceRow,
    QualityAuditPlan,
    QualitySemanticGuidance,
    QualityAuditRun,
    QualityEvaluatorAttempt,
    RowQualityAssessment,
    DatasetQualityReport,
    RowQualityReview,
    CurationProposal,
    CurationManifestReview,
    ApprovedCurationManifest,
    CurationApplication,
    Snapshot,
    BaseModel,
    TrainingRun,
    Checkpoint,
    EvaluationRun,
    EvaluationComparison,
    ModelSelection,
    AnalysisReport,
    AnalysisFindingReview,
    OptimizationProposal,
    OptimizationProposalReview,
    OptimizationCampaign,
    OptimizationCampaignLink,
    OptimizationOutcome,
    BenchmarkSuite,
    ContaminationReport,
    BenchmarkBundle,
    TrainingBenchmarkCheck,
    WorkflowDefinition,
    WorkflowRun,
    AcceptanceAssessment,
    AdvisoryAssessment,
    WorkflowApproval,
    StopDecision,
    ModelPromotion,
}

impl ArtifactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectBootstrap => "project_bootstrap",
            Self::ProjectPreparation => "project_preparation",
            Self::ProjectConfiguration => "project_configuration",
            Self::Dataset => "dataset",
            Self::SemanticProfile => "semantic_profile",
            Self::SemanticBinding => "semantic_binding",
            Self::GenerationSemanticContext => "generation_semantic_context",
            Self::ResearchBrief => "research_brief",
            Self::ResearchRun => "research_run",
            Self::ResearchEvidence => "research_evidence",
            Self::ResearchClaim => "research_claim",
            Self::AuthenticityProfile => "authenticity_profile",
            Self::AuthenticityProfileReview => "authenticity_profile_review",
            Self::AuthenticityProfileBinding => "authenticity_profile_binding",
            Self::GenerationAuthenticityContext => "generation_authenticity_context",
            Self::DatasetArchitectBrief => "dataset_architect_brief",
            Self::DatasetArchitectRun => "dataset_architect_run",
            Self::DatasetArchitectureProposal => "dataset_architecture_proposal",
            Self::DatasetArchitectureReview => "dataset_architecture_review",
            Self::DatasetArchitectureApplication => "dataset_architecture_application",
            Self::GenerationStrategyContext => "generation_strategy_context",
            Self::InitialAllocation => "initial_allocation",
            Self::GenerationPlan => "generation_plan",
            Self::GenerationJob => "generation_job",
            Self::DatasetImport => "dataset_import",
            Self::DatasetSourceRow => "dataset_source_row",
            Self::QualityAuditPlan => "quality_audit_plan",
            Self::QualitySemanticGuidance => "quality_semantic_guidance",
            Self::QualityAuditRun => "quality_audit_run",
            Self::QualityEvaluatorAttempt => "quality_evaluator_attempt",
            Self::RowQualityAssessment => "row_quality_assessment",
            Self::DatasetQualityReport => "dataset_quality_report",
            Self::RowQualityReview => "row_quality_review",
            Self::CurationProposal => "curation_proposal",
            Self::CurationManifestReview => "curation_manifest_review",
            Self::ApprovedCurationManifest => "approved_curation_manifest",
            Self::CurationApplication => "curation_application",
            Self::Snapshot => "snapshot",
            Self::BaseModel => "base_model",
            Self::TrainingRun => "training_run",
            Self::Checkpoint => "checkpoint",
            Self::EvaluationRun => "evaluation_run",
            Self::EvaluationComparison => "evaluation_comparison",
            Self::ModelSelection => "model_selection",
            Self::AnalysisReport => "analysis_report",
            Self::AnalysisFindingReview => "analysis_finding_review",
            Self::OptimizationProposal => "optimization_proposal",
            Self::OptimizationProposalReview => "optimization_proposal_review",
            Self::OptimizationCampaign => "optimization_campaign",
            Self::OptimizationCampaignLink => "optimization_campaign_link",
            Self::OptimizationOutcome => "optimization_outcome",
            Self::BenchmarkSuite => "benchmark_suite",
            Self::ContaminationReport => "contamination_report",
            Self::BenchmarkBundle => "benchmark_bundle",
            Self::TrainingBenchmarkCheck => "training_benchmark_check",
            Self::WorkflowDefinition => "workflow_definition",
            Self::WorkflowRun => "workflow_run",
            Self::AcceptanceAssessment => "acceptance_assessment",
            Self::AdvisoryAssessment => "advisory_assessment",
            Self::WorkflowApproval => "workflow_approval",
            Self::StopDecision => "stop_decision",
            Self::ModelPromotion => "model_promotion",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceNode {
    pub kind: ArtifactKind,
    pub id: Uuid,
    pub fingerprint: Option<String>,
    pub attributes: Value,
    pub parents: Vec<ProvenanceNode>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("provenance persistence failed: {0}")]
pub struct ProvenanceStoreError(pub String);

pub trait ProvenanceStore: Send + Sync {
    fn trace_provenance(
        &self,
        kind: ArtifactKind,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProvenanceNode>, ProvenanceStoreError>>;
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::{ArtifactKind, fingerprint};

    #[test]
    fn recursively_sorts_arbitrary_json_object_keys() {
        let left = json!({"outer": {"z": 1, "a": 2}, "b": 3});
        let right = json!({"b": 3, "outer": {"a": 2, "z": 1}});
        assert_eq!(
            fingerprint(&left).expect("hash"),
            fingerprint(&right).expect("hash")
        );
    }

    #[test]
    fn floating_point_fingerprints_survive_json_persistence() {
        #[derive(Serialize, Deserialize)]
        struct Measurement {
            margin: f64,
        }

        let measurement = Measurement {
            margin: 0.014_792_325_024_017_783,
        };
        let persisted = serde_json::to_vec(&measurement).expect("serialize");
        let restored: Measurement = serde_json::from_slice(&persisted).expect("deserialize");

        assert_eq!(
            fingerprint(&measurement).expect("original fingerprint"),
            fingerprint(&restored).expect("restored fingerprint")
        );
    }

    #[test]
    fn dataset_quality_artifact_names_are_stable() {
        let cases = [
            (ArtifactKind::DatasetSourceRow, "dataset_source_row"),
            (ArtifactKind::QualityAuditPlan, "quality_audit_plan"),
            (
                ArtifactKind::QualitySemanticGuidance,
                "quality_semantic_guidance",
            ),
            (ArtifactKind::QualityAuditRun, "quality_audit_run"),
            (
                ArtifactKind::QualityEvaluatorAttempt,
                "quality_evaluator_attempt",
            ),
            (ArtifactKind::RowQualityAssessment, "row_quality_assessment"),
            (ArtifactKind::DatasetQualityReport, "dataset_quality_report"),
            (ArtifactKind::RowQualityReview, "row_quality_review"),
            (ArtifactKind::CurationProposal, "curation_proposal"),
            (
                ArtifactKind::CurationManifestReview,
                "curation_manifest_review",
            ),
            (
                ArtifactKind::ApprovedCurationManifest,
                "approved_curation_manifest",
            ),
            (ArtifactKind::CurationApplication, "curation_application"),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.as_str(), expected);
        }
    }

    #[test]
    fn benchmark_artifact_names_are_stable() {
        let cases = [
            (ArtifactKind::BenchmarkSuite, "benchmark_suite"),
            (ArtifactKind::ContaminationReport, "contamination_report"),
            (ArtifactKind::BenchmarkBundle, "benchmark_bundle"),
            (
                ArtifactKind::TrainingBenchmarkCheck,
                "training_benchmark_check",
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.as_str(), expected);
        }
    }
}
