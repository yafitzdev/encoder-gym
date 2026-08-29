use analysis_core::contract::{DiagnosticComparisonEvidence, DiagnosticReviewDisposition};
use chrono::{DateTime, Utc};
use generation_core::domain::GenerationCell;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    allocation::{AllocationResult, AppliedConstraint},
    evidence::{OptimizationEvidence, OptimizationSourceIdentity},
    protocol::{OptimizationProtocol, RecommendationKind},
    scoring::{EligibilityDecision, ScoreBreakdown, ScoredCell},
    training_candidates::TrainingCandidateSet,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellRecommendation {
    pub cell: GenerationCell,
    pub current_accepted: u32,
    pub evidence_support: u64,
    pub evidence_errors: u64,
    pub evidence_error_rate: f64,
    pub score: f64,
    pub additional_count: u32,
    pub proposed_target: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationRecommendation {
    pub id: String,
    pub kind: RecommendationKind,
    pub cell: GenerationCell,
    pub current_accepted: u32,
    pub additional_count: u32,
    pub proposed_target: u32,
    pub finding_key: String,
    pub finding_fingerprint: String,
    pub evidence_fingerprint: String,
    pub eligibility: EligibilityDecision,
    pub score: ScoreBreakdown,
    pub constraints: Vec<AppliedConstraint>,
    pub rationale: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRebaseLineage {
    pub previous_proposal_id: Uuid,
    pub previous_proposal_fingerprint: String,
    pub previous_coverage_fingerprint: String,
    pub refreshed_coverage_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOnlyRecommendation {
    pub id: String,
    pub cell: GenerationCell,
    pub finding_key: String,
    pub finding_fingerprint: String,
    pub evidence_fingerprint: String,
    pub score: ScoreBreakdown,
    pub latest_review: Option<DiagnosticReviewDisposition>,
    pub comparison: Option<DiagnosticComparisonEvidence>,
    pub rationale: String,
    pub caution: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationProposal {
    pub id: Uuid,
    pub analysis_report_id: Uuid,
    pub dataset_id: Uuid,
    pub additional_example_budget: u32,
    pub minimum_support: u64,
    /// Fully resolved protocol for proposals created by Slice 6.1 and later.
    /// Historical proposals deserialize with `None` and retain their original fingerprint.
    #[serde(default)]
    pub protocol: Option<OptimizationProtocol>,
    #[serde(default)]
    pub protocol_fingerprint: String,
    #[serde(default)]
    pub source_identity: Option<OptimizationSourceIdentity>,
    #[serde(default)]
    pub evidence_fingerprint: String,
    #[serde(default)]
    pub decision_evidence: Option<OptimizationEvidence>,
    #[serde(default)]
    pub allocation: Option<AllocationResult>,
    #[serde(default)]
    pub decision_cells: Vec<ScoredCell>,
    #[serde(default)]
    pub normalized_recommendations: Vec<OptimizationRecommendation>,
    #[serde(default)]
    pub training_candidate_set: Option<TrainingCandidateSet>,
    #[serde(default)]
    pub review_only_recommendations: Vec<ReviewOnlyRecommendation>,
    /// Present only when this immutable proposal explicitly refreshes a stale
    /// proposal against newer accepted-row coverage.
    #[serde(default)]
    pub rebase_lineage: Option<ProposalRebaseLineage>,
    pub recommendations: Vec<CellRecommendation>,
    #[serde(default)]
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl OptimizationProposal {
    pub fn allocated_count(&self) -> u64 {
        self.recommendations
            .iter()
            .map(|recommendation| u64::from(recommendation.additional_count))
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalApplication {
    pub proposal_id: Uuid,
    pub generation_plan_id: Uuid,
    #[serde(default)]
    pub approval_review_id: Option<Uuid>,
    #[serde(default)]
    pub approval_fingerprint: Option<String>,
    #[serde(default)]
    pub selected_recommendation_ids: Vec<String>,
    #[serde(default)]
    pub verified_coverage_fingerprint: Option<String>,
    pub applied_at: DateTime<Utc>,
}
