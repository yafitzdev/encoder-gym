use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use generation_core::{
    domain::{DatasetDefinition, GenerationPlan, PlannedCell},
    planning::explicit_target_plan,
};
use thiserror::Error;

use crate::{
    domain::{OptimizationProposal, ProposalApplication},
    evidence::{canonical_coverage, coverage_fingerprint},
    planning::verify_constrained_proposal,
    reviews::ProposalReviewRecord,
};

pub fn approved_proposal_to_plan(
    proposal: &OptimizationProposal,
    approval: &ProposalReviewRecord,
    dataset: &DatasetDefinition,
    current_accepted_by_cell: &BTreeMap<String, u32>,
) -> Result<(GenerationPlan, ProposalApplication), ApprovedApplicationError> {
    verify_constrained_proposal(proposal)?;
    approval.validate(proposal)?;
    let selected_ids = approval.selected_data_recommendation_ids(proposal)?;
    let source = proposal
        .source_identity
        .as_ref()
        .ok_or(ApprovedApplicationError::LegacyProposal)?;
    if dataset.id != source.dataset_id
        || artifact_core::fingerprint(dataset)? != source.dataset_fingerprint
    {
        return Err(ApprovedApplicationError::DatasetIdentity);
    }
    let current_coverage = canonical_coverage(dataset, current_accepted_by_cell)?;
    let current_coverage_fingerprint = coverage_fingerprint(dataset.id, &current_coverage)?;
    if current_coverage_fingerprint != source.coverage_fingerprint {
        return Err(ApprovedApplicationError::StaleCoverage {
            expected: source.coverage_fingerprint.clone(),
            current: current_coverage_fingerprint,
        });
    }
    let selected = selected_ids.iter().collect::<BTreeSet<_>>();
    let cells = proposal
        .normalized_recommendations
        .iter()
        .filter(|recommendation| selected.contains(&recommendation.id))
        .map(|recommendation| PlannedCell {
            cell: recommendation.cell.clone(),
            target_count: recommendation.proposed_target,
        })
        .collect::<Vec<_>>();
    if cells.is_empty() || cells.len() != selected_ids.len() {
        return Err(ApprovedApplicationError::Selection);
    }
    let plan = explicit_target_plan(dataset, cells)
        .map_err(|error| ApprovedApplicationError::Plan(error.to_string()))?;
    let application = ProposalApplication {
        proposal_id: proposal.id,
        generation_plan_id: plan.id,
        approval_review_id: Some(approval.id),
        approval_fingerprint: Some(approval.fingerprint.clone()),
        selected_recommendation_ids: selected_ids,
        verified_coverage_fingerprint: Some(source.coverage_fingerprint.clone()),
        applied_at: Utc::now(),
    };
    Ok((plan, application))
}

#[derive(Debug, Error)]
pub enum ApprovedApplicationError {
    #[error(transparent)]
    Proposal(#[from] crate::planning::OptimizationError),
    #[error(transparent)]
    Review(#[from] crate::reviews::ProposalReviewError),
    #[error(transparent)]
    Evidence(#[from] crate::evidence::OptimizationEvidenceError),
    #[error(transparent)]
    Fingerprint(#[from] artifact_core::FingerprintError),
    #[error("approved application requires a decision-grade proposal")]
    LegacyProposal,
    #[error("current dataset definition does not match the proposal source identity")]
    DatasetIdentity,
    #[error("proposal coverage is stale: expected {expected}, current {current}")]
    StaleCoverage { expected: String, current: String },
    #[error("approval selection did not resolve to a non-empty unique recommendation set")]
    Selection,
    #[error("approved recommendations could not form a generation plan: {0}")]
    Plan(String),
}
