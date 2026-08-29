use std::collections::{BTreeMap, BTreeSet};

use analysis_core::{contract::DiagnosticContract, domain::AnalysisReport};
use chrono::Utc;
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, GenerationPlan, PlannedCell},
    planning::explicit_target_plan,
};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    allocation::{AllocationFeasibility, ConstraintIssue, allocate_budget},
    domain::{
        CellRecommendation, OptimizationProposal, OptimizationRecommendation,
        ProposalRebaseLineage, ReviewOnlyRecommendation,
    },
    evidence::OptimizationEvidence,
    protocol::{InfeasibleAllocationBehavior, OptimizationProtocol, RecommendationKind},
    scoring::score_evidence,
    training_candidates::{
        TrainingCandidateError, TrainingCandidateSet, TrainingConfigurationSpace,
        create_training_candidate_set,
    },
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OptimizationError {
    #[error("additional-example budget must be greater than zero")]
    EmptyBudget,
    #[error("minimum support must be greater than zero")]
    MinimumSupport,
    #[error("optimization protocol is invalid: {0}")]
    InvalidProtocol(String),
    #[error("optimization evidence is invalid: {0}")]
    DecisionEvidence(String),
    #[error("allocation left {unallocated_budget} examples unallocated: {issues:?}")]
    InfeasibleAllocation {
        unallocated_budget: u32,
        issues: Vec<ConstraintIssue>,
    },
    #[error("minimum support cannot be lower than the analysis report minimum")]
    MinimumSupportBelowReport,
    #[error("analysis report contains no eligible known generation cells")]
    NoEligibleCells,
    #[error("accepted count or proposed target exceeds the supported range")]
    CountOverflow,
    #[error("proposal does not belong to the supplied dataset")]
    DatasetMismatch,
    #[error("proposal budget is not conserved")]
    BudgetMismatch,
    #[error("proposal repeats a generation cell")]
    DuplicateCell,
    #[error("analysis report contains invalid cell evidence")]
    InvalidEvidence,
    #[error("proposal contains an unknown generation cell")]
    UnknownCell,
    #[error("generation plan is invalid: {0}")]
    InvalidPlan(String),
    #[error("could not fingerprint optimization proposal: {0}")]
    Fingerprint(String),
    #[error("operation requires a decision-grade constrained proposal")]
    LegacyProposal,
    #[error("constrained proposal integrity check failed: {0}")]
    ProposalIntegrity(&'static str),
    #[error("rebasing may only refresh coverage for the same immutable analysis source")]
    RebaseSourceMismatch,
    #[error("rebasing requires coverage to have changed")]
    RebaseCoverageUnchanged,
    #[error("training-candidate construction failed: {0}")]
    TrainingCandidates(String),
}

pub fn create_constrained_proposal(
    evidence: &OptimizationEvidence,
    protocol: &OptimizationProtocol,
) -> Result<OptimizationProposal, OptimizationError> {
    create_constrained_proposal_with_training(evidence, protocol, None)
}

pub fn create_constrained_proposal_with_training(
    evidence: &OptimizationEvidence,
    protocol: &OptimizationProtocol,
    training_space: Option<&TrainingConfigurationSpace>,
) -> Result<OptimizationProposal, OptimizationError> {
    let normalized_evidence = persistence_normalize_decision_evidence(evidence)?;
    let evidence = &normalized_evidence;
    evidence
        .validate(protocol, true)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    let decision_cells = score_evidence(evidence, protocol)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    let allocation = allocate_budget(&decision_cells, &evidence.current_coverage, protocol)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    if allocation.feasibility == AllocationFeasibility::Infeasible
        && protocol.infeasible_allocation_behavior == InfeasibleAllocationBehavior::Reject
    {
        return Err(OptimizationError::InfeasibleAllocation {
            unallocated_budget: allocation.unallocated_budget,
            issues: allocation.issues.clone(),
        });
    }
    let scored_by_cell = decision_cells
        .iter()
        .map(|scored| (scored.cell.key(), scored))
        .collect::<BTreeMap<_, _>>();
    let protocol_fingerprint = protocol
        .fingerprint()
        .map_err(|error| OptimizationError::InvalidProtocol(error.to_string()))?;
    let proposal_id = Uuid::new_v4();
    let training_candidate_set = match (protocol.training_candidates.as_ref(), training_space) {
        (Some(_), Some(space)) => Some(
            create_training_candidate_set(proposal_id, &evidence.source_identity, protocol, space)
                .map_err(training_candidate_error)?,
        ),
        (None, None) => None,
        _ => {
            return Err(OptimizationError::TrainingCandidates(
                "protocol request and explicit configuration space must be supplied together"
                    .into(),
            ));
        }
    };
    let normalized_recommendations = allocation
        .cells
        .iter()
        .map(|allocated| {
            let scored = scored_by_cell
                .get(&allocated.cell.key())
                .ok_or(OptimizationError::InvalidEvidence)?;
            let rationale = format!(
                "allocate {} additional examples to {}: {} errors in {} examples, adjusted rate {:.12}, final {} score {:.12}",
                allocated.additional_count,
                allocated.cell.key(),
                scored.score.evidence_errors,
                scored.score.evidence_support,
                scored.score.adjusted_error_rate,
                scoring_policy_name(scored.score.policy),
                scored.score.final_score,
            );
            let fingerprint = artifact_core::fingerprint(&RecommendationFingerprintInput {
                source_fingerprint: &evidence.fingerprint,
                protocol_fingerprint: &protocol_fingerprint,
                kind: RecommendationKind::DataGeneration,
                cell: &allocated.cell,
                current_accepted: allocated.current_accepted,
                additional_count: allocated.additional_count,
                proposed_target: allocated.proposed_target,
                finding_key: &scored.finding_key,
                finding_fingerprint: &scored.finding_fingerprint,
                evidence_fingerprint: &scored.evidence_fingerprint,
                eligibility: &scored.eligibility,
                score: &scored.score,
                constraints: &allocated.constraints,
                rationale: &rationale,
            })
            .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
            Ok(OptimizationRecommendation {
                id: fingerprint.clone(),
                kind: RecommendationKind::DataGeneration,
                cell: allocated.cell.clone(),
                current_accepted: allocated.current_accepted,
                additional_count: allocated.additional_count,
                proposed_target: allocated.proposed_target,
                finding_key: scored.finding_key.clone(),
                finding_fingerprint: scored.finding_fingerprint.clone(),
                evidence_fingerprint: scored.evidence_fingerprint.clone(),
                eligibility: scored.eligibility.clone(),
                score: scored.score.clone(),
                constraints: allocated.constraints.clone(),
                rationale,
                fingerprint,
            })
        })
        .collect::<Result<Vec<_>, OptimizationError>>()?;
    let recommendations = normalized_recommendations
        .iter()
        .map(|recommendation| CellRecommendation {
            cell: recommendation.cell.clone(),
            current_accepted: recommendation.current_accepted,
            evidence_support: recommendation.score.evidence_support,
            evidence_errors: recommendation.score.evidence_errors,
            evidence_error_rate: recommendation.score.error_rate,
            score: recommendation.score.final_score,
            additional_count: recommendation.additional_count,
            proposed_target: recommendation.proposed_target,
        })
        .collect::<Vec<_>>();
    let evidence_by_finding = evidence
        .diagnostics
        .cells
        .iter()
        .map(|cell| (cell.finding_key.as_str(), cell))
        .collect::<BTreeMap<_, _>>();
    let review_only_recommendations = if protocol
        .recommendation_kinds
        .contains(&RecommendationKind::ReviewOnly)
    {
        decision_cells
            .iter()
            .filter(|decision| decision.score.evidence_errors > 0)
            .map(|decision| {
                let evidence_cell = evidence_by_finding
                    .get(decision.finding_key.as_str())
                    .ok_or(OptimizationError::InvalidEvidence)?;
                let rationale = format!(
                    "review {} before changing labels or schema: {} errors in {} examples, error rate {:.12}",
                    decision.cell.key(),
                    decision.score.evidence_errors,
                    decision.score.evidence_support,
                    decision.score.error_rate,
                );
                let caution =
                    "advisory only; this recommendation does not mutate labels, dimensions, rows, or snapshots"
                        .to_owned();
                let fingerprint = artifact_core::fingerprint(&ReviewRecommendationFingerprintInput {
                    source_fingerprint: &evidence.fingerprint,
                    protocol_fingerprint: &protocol_fingerprint,
                    cell: &decision.cell,
                    finding_key: &decision.finding_key,
                    finding_fingerprint: &decision.finding_fingerprint,
                    evidence_fingerprint: &decision.evidence_fingerprint,
                    score: &decision.score,
                    latest_review: evidence_cell.latest_review.as_ref(),
                    comparison: evidence_cell.comparison.as_ref(),
                    rationale: &rationale,
                    caution: &caution,
                })
                .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
                Ok(ReviewOnlyRecommendation {
                    id: fingerprint.clone(),
                    cell: decision.cell.clone(),
                    finding_key: decision.finding_key.clone(),
                    finding_fingerprint: decision.finding_fingerprint.clone(),
                    evidence_fingerprint: decision.evidence_fingerprint.clone(),
                    score: decision.score.clone(),
                    latest_review: evidence_cell.latest_review.clone(),
                    comparison: evidence_cell.comparison.clone(),
                    rationale,
                    caution,
                    fingerprint,
                })
            })
            .collect::<Result<Vec<_>, OptimizationError>>()?
    } else {
        Vec::new()
    };
    let fingerprint = artifact_core::fingerprint(&ConstrainedProposalFingerprintInput {
        source_identity: &evidence.source_identity,
        evidence_fingerprint: &evidence.fingerprint,
        protocol,
        protocol_fingerprint: &protocol_fingerprint,
        allocation: &allocation,
        decision_cells: &decision_cells,
        recommendations: &normalized_recommendations,
        training_candidate_set: training_candidate_set.as_ref(),
        review_only_recommendations: &review_only_recommendations,
        rebase_lineage: None,
    })
    .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
    Ok(OptimizationProposal {
        id: proposal_id,
        analysis_report_id: evidence.source_identity.analysis_report_id,
        dataset_id: evidence.source_identity.dataset_id,
        additional_example_budget: protocol.additional_example_budget,
        minimum_support: protocol.minimum_evidence_support,
        protocol: Some(protocol.clone()),
        protocol_fingerprint,
        source_identity: Some(evidence.source_identity.clone()),
        evidence_fingerprint: evidence.fingerprint.clone(),
        decision_evidence: Some(evidence.clone()),
        allocation: Some(allocation),
        decision_cells,
        normalized_recommendations,
        training_candidate_set,
        review_only_recommendations,
        rebase_lineage: None,
        recommendations,
        fingerprint,
        created_at: Utc::now(),
    })
}

fn persistence_normalize_decision_evidence(
    evidence: &OptimizationEvidence,
) -> Result<OptimizationEvidence, OptimizationError> {
    let bytes = serde_json::to_vec(evidence)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))
}

/// Re-evaluates an immutable constrained proposal against explicitly refreshed
/// coverage while preserving every other source and protocol identity.
///
/// The previous proposal is never modified. The returned proposal carries a
/// fingerprinted lineage edge to it and receives a new identity.
pub fn rebase_constrained_proposal(
    previous: &OptimizationProposal,
    refreshed_evidence: &OptimizationEvidence,
) -> Result<OptimizationProposal, OptimizationError> {
    verify_constrained_proposal(previous)?;
    let protocol = previous
        .protocol
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    refreshed_evidence
        .validate(protocol, true)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    let previous_source = previous
        .source_identity
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    let refreshed_source = &refreshed_evidence.source_identity;
    if !same_rebase_source(previous_source, refreshed_source) {
        return Err(OptimizationError::RebaseSourceMismatch);
    }
    if previous_source.coverage_fingerprint == refreshed_source.coverage_fingerprint {
        return Err(OptimizationError::RebaseCoverageUnchanged);
    }

    let training_space = previous
        .training_candidate_set
        .as_ref()
        .map(|set| &set.configuration_space);
    let mut proposal =
        create_constrained_proposal_with_training(refreshed_evidence, protocol, training_space)?;
    proposal.rebase_lineage = Some(ProposalRebaseLineage {
        previous_proposal_id: previous.id,
        previous_proposal_fingerprint: previous.fingerprint.clone(),
        previous_coverage_fingerprint: previous_source.coverage_fingerprint.clone(),
        refreshed_coverage_fingerprint: refreshed_source.coverage_fingerprint.clone(),
    });
    proposal.fingerprint = constrained_proposal_fingerprint(&proposal)?;
    Ok(proposal)
}

pub fn verify_constrained_proposal(
    proposal: &OptimizationProposal,
) -> Result<(), OptimizationError> {
    let protocol = proposal
        .protocol
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    let source_identity = proposal
        .source_identity
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    let allocation = proposal
        .allocation
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    let evidence = proposal
        .decision_evidence
        .as_ref()
        .ok_or(OptimizationError::LegacyProposal)?;
    evidence
        .validate(protocol, true)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    if &evidence.source_identity != source_identity
        || evidence.fingerprint != proposal.evidence_fingerprint
    {
        return Err(OptimizationError::ProposalIntegrity(
            "decision evidence identity mismatch",
        ));
    }
    let reproduced_decisions = score_evidence(evidence, protocol)
        .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    let reproduced_allocation =
        allocate_budget(&reproduced_decisions, &evidence.current_coverage, protocol)
            .map_err(|error| OptimizationError::DecisionEvidence(error.to_string()))?;
    if reproduced_decisions != proposal.decision_cells || &reproduced_allocation != allocation {
        return Err(OptimizationError::ProposalIntegrity(
            "decision scores or allocation do not reproduce",
        ));
    }
    let protocol_fingerprint = protocol
        .fingerprint()
        .map_err(|error| OptimizationError::InvalidProtocol(error.to_string()))?;
    if protocol_fingerprint != proposal.protocol_fingerprint
        || protocol_fingerprint != source_identity.optimization_protocol_fingerprint
        || proposal.analysis_report_id != source_identity.analysis_report_id
        || proposal.dataset_id != source_identity.dataset_id
        || proposal.additional_example_budget != protocol.additional_example_budget
        || proposal.minimum_support != protocol.minimum_evidence_support
        || allocation.requested_budget != protocol.additional_example_budget
        || allocation
            .allocated_budget
            .checked_add(allocation.unallocated_budget)
            != Some(allocation.requested_budget)
    {
        return Err(OptimizationError::ProposalIntegrity(
            "identity or budget mismatch",
        ));
    }
    for recommendation in &proposal.normalized_recommendations {
        let fingerprint = artifact_core::fingerprint(&RecommendationFingerprintInput {
            source_fingerprint: &proposal.evidence_fingerprint,
            protocol_fingerprint: &proposal.protocol_fingerprint,
            kind: recommendation.kind,
            cell: &recommendation.cell,
            current_accepted: recommendation.current_accepted,
            additional_count: recommendation.additional_count,
            proposed_target: recommendation.proposed_target,
            finding_key: &recommendation.finding_key,
            finding_fingerprint: &recommendation.finding_fingerprint,
            evidence_fingerprint: &recommendation.evidence_fingerprint,
            eligibility: &recommendation.eligibility,
            score: &recommendation.score,
            constraints: &recommendation.constraints,
            rationale: &recommendation.rationale,
        })
        .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
        if recommendation.id != fingerprint
            || recommendation.fingerprint != fingerprint
            || !recommendation.eligibility.eligible
            || recommendation
                .current_accepted
                .checked_add(recommendation.additional_count)
                != Some(recommendation.proposed_target)
        {
            return Err(OptimizationError::ProposalIntegrity(
                "recommendation fingerprint or target mismatch",
            ));
        }
    }
    match (
        &protocol.training_candidates,
        &proposal.training_candidate_set,
    ) {
        (Some(_), Some(set)) => set
            .validate(proposal.id, source_identity, protocol)
            .map_err(training_candidate_error)?,
        (None, None) => {}
        _ => {
            return Err(OptimizationError::ProposalIntegrity(
                "training candidate request or artifact mismatch",
            ));
        }
    }
    let review_requested = protocol
        .recommendation_kinds
        .contains(&RecommendationKind::ReviewOnly);
    if !review_requested && !proposal.review_only_recommendations.is_empty() {
        return Err(OptimizationError::ProposalIntegrity(
            "review-only request or artifact mismatch",
        ));
    }
    for recommendation in &proposal.review_only_recommendations {
        let fingerprint = artifact_core::fingerprint(&ReviewRecommendationFingerprintInput {
            source_fingerprint: &proposal.evidence_fingerprint,
            protocol_fingerprint: &proposal.protocol_fingerprint,
            cell: &recommendation.cell,
            finding_key: &recommendation.finding_key,
            finding_fingerprint: &recommendation.finding_fingerprint,
            evidence_fingerprint: &recommendation.evidence_fingerprint,
            score: &recommendation.score,
            latest_review: recommendation.latest_review.as_ref(),
            comparison: recommendation.comparison.as_ref(),
            rationale: &recommendation.rationale,
            caution: &recommendation.caution,
        })
        .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
        if recommendation.id != fingerprint || recommendation.fingerprint != fingerprint {
            return Err(OptimizationError::ProposalIntegrity(
                "review-only recommendation fingerprint mismatch",
            ));
        }
    }
    let fingerprint = artifact_core::fingerprint(&ConstrainedProposalFingerprintInput {
        source_identity,
        evidence_fingerprint: &proposal.evidence_fingerprint,
        protocol,
        protocol_fingerprint: &proposal.protocol_fingerprint,
        allocation,
        decision_cells: &proposal.decision_cells,
        recommendations: &proposal.normalized_recommendations,
        training_candidate_set: proposal.training_candidate_set.as_ref(),
        review_only_recommendations: &proposal.review_only_recommendations,
        rebase_lineage: proposal.rebase_lineage.as_ref(),
    })
    .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
    if fingerprint != proposal.fingerprint {
        return Err(OptimizationError::ProposalIntegrity(
            "proposal fingerprint mismatch",
        ));
    }
    Ok(())
}

fn scoring_policy_name(policy: crate::protocol::ScoringPolicy) -> &'static str {
    use crate::protocol::ScoringPolicy;
    match policy {
        ScoringPolicy::ErrorCount => "error-count",
        ScoringPolicy::ErrorRate => "error-rate",
        ScoringPolicy::ErrorRateLift => "error-rate-lift",
        ScoringPolicy::HighConfidenceErrorSeverity => "high-confidence-severity",
        ScoringPolicy::MarginalErrorCoverage => "marginal-error-coverage",
        ScoringPolicy::ComparisonRegression => "comparison-regression",
        ScoringPolicy::ConservativeComposite => "conservative-composite",
    }
}

#[derive(Serialize)]
struct RecommendationFingerprintInput<'a> {
    source_fingerprint: &'a str,
    protocol_fingerprint: &'a str,
    kind: RecommendationKind,
    cell: &'a generation_core::domain::GenerationCell,
    current_accepted: u32,
    additional_count: u32,
    proposed_target: u32,
    finding_key: &'a str,
    finding_fingerprint: &'a str,
    evidence_fingerprint: &'a str,
    eligibility: &'a crate::scoring::EligibilityDecision,
    score: &'a crate::scoring::ScoreBreakdown,
    constraints: &'a [crate::allocation::AppliedConstraint],
    rationale: &'a str,
}

#[derive(Serialize)]
struct ReviewRecommendationFingerprintInput<'a> {
    source_fingerprint: &'a str,
    protocol_fingerprint: &'a str,
    cell: &'a generation_core::domain::GenerationCell,
    finding_key: &'a str,
    finding_fingerprint: &'a str,
    evidence_fingerprint: &'a str,
    score: &'a crate::scoring::ScoreBreakdown,
    latest_review: Option<&'a analysis_core::contract::DiagnosticReviewDisposition>,
    comparison: Option<&'a analysis_core::contract::DiagnosticComparisonEvidence>,
    rationale: &'a str,
    caution: &'a str,
}

#[derive(Serialize)]
struct ConstrainedProposalFingerprintInput<'a> {
    source_identity: &'a crate::evidence::OptimizationSourceIdentity,
    evidence_fingerprint: &'a str,
    protocol: &'a OptimizationProtocol,
    protocol_fingerprint: &'a str,
    allocation: &'a crate::allocation::AllocationResult,
    decision_cells: &'a [crate::scoring::ScoredCell],
    recommendations: &'a [OptimizationRecommendation],
    training_candidate_set: Option<&'a TrainingCandidateSet>,
    review_only_recommendations: &'a [ReviewOnlyRecommendation],
    rebase_lineage: Option<&'a ProposalRebaseLineage>,
}

fn constrained_proposal_fingerprint(
    proposal: &OptimizationProposal,
) -> Result<String, OptimizationError> {
    artifact_core::fingerprint(&ConstrainedProposalFingerprintInput {
        source_identity: proposal
            .source_identity
            .as_ref()
            .ok_or(OptimizationError::LegacyProposal)?,
        evidence_fingerprint: &proposal.evidence_fingerprint,
        protocol: proposal
            .protocol
            .as_ref()
            .ok_or(OptimizationError::LegacyProposal)?,
        protocol_fingerprint: &proposal.protocol_fingerprint,
        allocation: proposal
            .allocation
            .as_ref()
            .ok_or(OptimizationError::LegacyProposal)?,
        decision_cells: &proposal.decision_cells,
        recommendations: &proposal.normalized_recommendations,
        training_candidate_set: proposal.training_candidate_set.as_ref(),
        review_only_recommendations: &proposal.review_only_recommendations,
        rebase_lineage: proposal.rebase_lineage.as_ref(),
    })
    .map_err(|error| OptimizationError::Fingerprint(error.to_string()))
}

fn training_candidate_error(error: TrainingCandidateError) -> OptimizationError {
    OptimizationError::TrainingCandidates(error.to_string())
}

fn same_rebase_source(
    previous: &crate::evidence::OptimizationSourceIdentity,
    refreshed: &crate::evidence::OptimizationSourceIdentity,
) -> bool {
    previous.analysis_report_id == refreshed.analysis_report_id
        && previous.analysis_fingerprint == refreshed.analysis_fingerprint
        && previous.analysis_protocol_fingerprint == refreshed.analysis_protocol_fingerprint
        && previous.diagnostic_contract_fingerprint == refreshed.diagnostic_contract_fingerprint
        && previous.evaluation_run_id == refreshed.evaluation_run_id
        && previous.evaluation_input_fingerprint == refreshed.evaluation_input_fingerprint
        && previous.evaluation_protocol_fingerprint == refreshed.evaluation_protocol_fingerprint
        && previous.cohort_fingerprint == refreshed.cohort_fingerprint
        && previous.dataset_id == refreshed.dataset_id
        && previous.dataset_fingerprint == refreshed.dataset_fingerprint
        && previous.snapshot_id == refreshed.snapshot_id
        && previous.snapshot_fingerprint == refreshed.snapshot_fingerprint
        && previous.comparison_id == refreshed.comparison_id
        && previous.comparison_fingerprint == refreshed.comparison_fingerprint
        && previous.optimization_protocol_fingerprint == refreshed.optimization_protocol_fingerprint
        && previous.training_configuration_space_fingerprint
            == refreshed.training_configuration_space_fingerprint
}

pub fn create_proposal(
    report: &AnalysisReport,
    dataset: &DatasetDefinition,
    accepted_by_cell: &BTreeMap<String, u32>,
    additional_example_budget: u32,
    minimum_support: u64,
) -> Result<OptimizationProposal, OptimizationError> {
    let diagnostics =
        DiagnosticContract::try_from(report).map_err(|_| OptimizationError::InvalidEvidence)?;
    create_proposal_from_diagnostics(
        &diagnostics,
        dataset,
        accepted_by_cell,
        additional_example_budget,
        minimum_support,
    )
}

pub fn create_proposal_from_diagnostics(
    diagnostics: &DiagnosticContract,
    dataset: &DatasetDefinition,
    accepted_by_cell: &BTreeMap<String, u32>,
    additional_example_budget: u32,
    minimum_support: u64,
) -> Result<OptimizationProposal, OptimizationError> {
    diagnostics
        .validate(false)
        .map_err(|_| OptimizationError::InvalidEvidence)?;
    let protocol = OptimizationProtocol::legacy(additional_example_budget, minimum_support)
        .normalize()
        .map_err(|error| OptimizationError::InvalidProtocol(error.to_string()))?;
    let protocol_fingerprint = protocol
        .fingerprint()
        .map_err(|error| OptimizationError::InvalidProtocol(error.to_string()))?;
    if minimum_support < diagnostics.minimum_support {
        return Err(OptimizationError::MinimumSupportBelowReport);
    }
    let valid_cells = expand_generation_cells(dataset)
        .into_iter()
        .map(|cell| (cell.key(), cell))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = diagnostics
        .cells
        .iter()
        .filter(|finding| finding.support >= minimum_support && finding.error_count > 0)
        .filter_map(|finding| {
            let label = finding.identity.get("label")?.clone();
            let dimensions = finding
                .identity
                .iter()
                .filter(|(name, _)| name.as_str() != "label")
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            let candidate = generation_core::domain::GenerationCell { label, dimensions };
            let cell = valid_cells.get(&candidate.key())?.clone();
            Some(Candidate {
                cell,
                support: finding.support,
                errors: finding.error_count,
                error_rate: finding.error_rate,
                additional: 0,
                remainder: 0,
            })
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(OptimizationError::NoEligibleCells);
    }
    if candidates.iter().any(|candidate| {
        candidate.support == 0
            || candidate.errors > candidate.support
            || !candidate.error_rate.is_finite()
            || !(0.0..=1.0).contains(&candidate.error_rate)
    }) {
        return Err(OptimizationError::InvalidEvidence);
    }
    let candidate_keys = candidates
        .iter()
        .map(|candidate| candidate.cell.key())
        .collect::<BTreeSet<_>>();
    if candidate_keys.len() != candidates.len() {
        return Err(OptimizationError::DuplicateCell);
    }
    candidates.sort_by(|left, right| {
        right
            .error_rate
            .total_cmp(&left.error_rate)
            .then_with(|| right.errors.cmp(&left.errors))
            .then_with(|| left.cell.key().cmp(&right.cell.key()))
    });

    let total_weight = candidates
        .iter()
        .map(|candidate| candidate.errors)
        .sum::<u64>();
    let budget = u64::from(additional_example_budget);
    let mut allocated = 0_u64;
    for candidate in &mut candidates {
        let numerator = budget
            .checked_mul(candidate.errors)
            .ok_or(OptimizationError::CountOverflow)?;
        let share = numerator / total_weight;
        candidate.additional =
            u32::try_from(share).map_err(|_| OptimizationError::CountOverflow)?;
        candidate.remainder = numerator % total_weight;
        allocated += share;
    }
    let remaining =
        usize::try_from(budget - allocated).map_err(|_| OptimizationError::CountOverflow)?;
    let mut remainder_order = (0..candidates.len()).collect::<Vec<_>>();
    remainder_order.sort_by(|&left, &right| {
        candidates[right]
            .remainder
            .cmp(&candidates[left].remainder)
            .then_with(|| left.cmp(&right))
    });
    for index in remainder_order.into_iter().take(remaining) {
        candidates[index].additional += 1;
    }

    let recommendations = candidates
        .into_iter()
        .filter(|candidate| candidate.additional > 0)
        .map(|candidate| {
            let current_accepted = accepted_by_cell
                .get(&candidate.cell.key())
                .copied()
                .unwrap_or(0);
            let proposed_target = current_accepted
                .checked_add(candidate.additional)
                .ok_or(OptimizationError::CountOverflow)?;
            Ok(CellRecommendation {
                cell: candidate.cell,
                current_accepted,
                evidence_support: candidate.support,
                evidence_errors: candidate.errors,
                evidence_error_rate: candidate.error_rate,
                score: candidate.error_rate,
                additional_count: candidate.additional,
                proposed_target,
            })
        })
        .collect::<Result<Vec<_>, OptimizationError>>()?;
    let fingerprint = artifact_core::fingerprint(&ProposalFingerprintInput {
        analysis_report_id: diagnostics.analysis_report_id,
        analysis_fingerprint: &diagnostics.analysis_fingerprint,
        dataset_id: dataset.id,
        additional_example_budget,
        minimum_support,
        protocol_fingerprint: &protocol_fingerprint,
        recommendations: &recommendations,
    })
    .map_err(|error| OptimizationError::Fingerprint(error.to_string()))?;
    Ok(OptimizationProposal {
        id: Uuid::new_v4(),
        analysis_report_id: diagnostics.analysis_report_id,
        dataset_id: dataset.id,
        additional_example_budget,
        minimum_support,
        protocol: Some(protocol),
        protocol_fingerprint,
        source_identity: None,
        evidence_fingerprint: diagnostics.fingerprint.clone(),
        decision_evidence: None,
        allocation: None,
        decision_cells: Vec::new(),
        normalized_recommendations: Vec::new(),
        training_candidate_set: None,
        review_only_recommendations: Vec::new(),
        rebase_lineage: None,
        recommendations,
        fingerprint,
        created_at: Utc::now(),
    })
}

#[derive(Serialize)]
struct ProposalFingerprintInput<'a> {
    analysis_report_id: Uuid,
    analysis_fingerprint: &'a str,
    dataset_id: Uuid,
    additional_example_budget: u32,
    minimum_support: u64,
    protocol_fingerprint: &'a str,
    recommendations: &'a [CellRecommendation],
}

pub fn proposal_to_plan(
    proposal: &OptimizationProposal,
    dataset: &DatasetDefinition,
) -> Result<GenerationPlan, OptimizationError> {
    if proposal.dataset_id != dataset.id {
        return Err(OptimizationError::DatasetMismatch);
    }
    if proposal.allocated_count() != u64::from(proposal.additional_example_budget) {
        return Err(OptimizationError::BudgetMismatch);
    }
    let valid_keys = expand_generation_cells(dataset)
        .into_iter()
        .map(|cell| cell.key())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let cells = proposal
        .recommendations
        .iter()
        .map(|recommendation| {
            let key = recommendation.cell.key();
            if !seen.insert(key.clone()) {
                return Err(OptimizationError::DuplicateCell);
            }
            if !valid_keys.contains(&key) {
                return Err(OptimizationError::UnknownCell);
            }
            if recommendation
                .current_accepted
                .checked_add(recommendation.additional_count)
                != Some(recommendation.proposed_target)
            {
                return Err(OptimizationError::CountOverflow);
            }
            Ok(PlannedCell {
                cell: recommendation.cell.clone(),
                target_count: recommendation.proposed_target,
            })
        })
        .collect::<Result<Vec<_>, OptimizationError>>()?;
    explicit_target_plan(dataset, cells)
        .map_err(|error| OptimizationError::InvalidPlan(error.to_string()))
}

struct Candidate {
    cell: generation_core::domain::GenerationCell,
    support: u64,
    errors: u64,
    error_rate: f64,
    additional: u32,
    remainder: u64,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use analysis_core::{
        domain::{
            AnalysisFinding, AnalysisReport, AnalysisSourceIdentity, FindingIdentity, FindingKind,
        },
        protocol::AnalysisProtocol,
    };
    use chrono::Utc;
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use uuid::Uuid;

    use crate::{
        application::{ApprovedApplicationError, approved_proposal_to_plan},
        evidence::OptimizationEvidence,
        protocol::OptimizationProtocol,
        reviews::{ProposalReviewRecord, ProposalReviewState},
        scenarios::create_default_scenario_group,
    };

    use super::{
        create_constrained_proposal, create_proposal, proposal_to_plan,
        rebase_constrained_proposal, verify_constrained_proposal,
    };

    #[test]
    fn conserves_budget_ignores_unknown_cells_and_builds_absolute_targets() {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("dimension"),
            ],
        )
        .expect("dataset");
        let evaluation_run_id = Uuid::new_v4();
        let report = AnalysisReport {
            id: Uuid::new_v4(),
            evaluation_run_id,
            minimum_support: 1,
            prediction_count: 30,
            error_count: 16,
            findings: vec![
                cell_finding("billing", "clean", 10, 4, 30, 16),
                cell_finding("fraud", "messy", 10, 2, 30, 16),
                cell_finding("unknown", "clean", 10, 10, 30, 16),
            ],
            errors: vec![],
            protocol: AnalysisProtocol::default(),
            protocol_fingerprint: "sha256:fixture-protocol".into(),
            source_identity: Some(AnalysisSourceIdentity {
                evaluation_run_id,
                evaluation_input_fingerprint: "sha256:evaluation".into(),
                evaluation_protocol_fingerprint: "sha256:evaluation-protocol".into(),
                cohort_fingerprint: "sha256:cohort".into(),
                prediction_count: 30,
                comparison_id: None,
                comparison_fingerprint: None,
            }),
            finding_evidence: BTreeMap::new(),
            comparison_diagnosis: None,
            fingerprint: "sha256:fixture-analysis".into(),
            created_at: Utc::now(),
        };
        let valid_cells = generation_core::dimensions::expand_generation_cells(&dataset);
        let accepted = BTreeMap::from([(valid_cells[0].key(), 12), (valid_cells[3].key(), 7)]);
        analysis_core::contract::DiagnosticContract::try_from(&report)
            .expect("valid diagnostic contract");

        let first = create_proposal(&report, &dataset, &accepted, 10, 1).expect("proposal");
        let second = create_proposal(&report, &dataset, &accepted, 10, 1).expect("proposal");
        assert_eq!(first.allocated_count(), 10);
        assert_eq!(first.recommendations.len(), 2);
        assert_eq!(
            first
                .recommendations
                .iter()
                .map(|item| item.additional_count)
                .collect::<Vec<_>>(),
            vec![7, 3]
        );
        assert_eq!(
            first
                .recommendations
                .iter()
                .map(|item| item.additional_count)
                .collect::<Vec<_>>(),
            second
                .recommendations
                .iter()
                .map(|item| item.additional_count)
                .collect::<Vec<_>>()
        );
        assert_eq!(first.fingerprint, second.fingerprint);
        let plan = proposal_to_plan(&first, &dataset).expect("plan");
        assert_eq!(plan.cells[0].target_count, 19);
        assert_eq!(plan.cells[1].target_count, 10);

        let diagnostics = analysis_core::contract::DiagnosticContract::try_from(&report)
            .expect("diagnostic contract");
        let protocol = OptimizationProtocol::legacy(10, 1);
        let snapshot_id = Uuid::new_v4();
        let evidence = OptimizationEvidence::new(
            diagnostics,
            &dataset,
            snapshot_id,
            "sha256:snapshot",
            &accepted,
            &protocol,
            None,
            true,
        )
        .expect("evidence");
        let constrained =
            create_constrained_proposal(&evidence, &protocol).expect("constrained proposal");
        verify_constrained_proposal(&constrained).expect("proposal verifies");
        let mut tampered_score = constrained.clone();
        tampered_score.decision_cells[0].score.final_score += 1.0;
        assert!(verify_constrained_proposal(&tampered_score).is_err());
        let mut tampered_allocation = constrained.clone();
        tampered_allocation
            .allocation
            .as_mut()
            .expect("allocation")
            .cells[0]
            .additional_count += 1;
        assert!(verify_constrained_proposal(&tampered_allocation).is_err());
        assert_eq!(constrained.allocated_count(), 10);
        assert_eq!(constrained.normalized_recommendations.len(), 2);
        assert!(constrained.source_identity.is_some());
        assert_eq!(
            constrained
                .allocation
                .as_ref()
                .map(|allocation| allocation.unallocated_budget),
            Some(0)
        );
        let scenarios = create_default_scenario_group(&evidence, &protocol)
            .expect("default alternative scenarios");
        let repeated_scenarios = create_default_scenario_group(&evidence, &protocol)
            .expect("repeated alternative scenarios");
        assert_eq!(scenarios.scenarios.len(), 4);
        assert_eq!(scenarios.comparisons.len(), 6);
        assert_eq!(scenarios.fingerprint, repeated_scenarios.fingerprint);
        scenarios.validate().expect("scenario group verifies");
        let mut tampered_scenarios = scenarios.clone();
        tampered_scenarios.comparisons[0].allocation_overlap = 0.123;
        assert!(tampered_scenarios.validate().is_err());
        let full_approval = ProposalReviewRecord::new(
            &constrained,
            ProposalReviewState::ApprovedForPlanCreation,
            Vec::new(),
            Some(" approved ".into()),
            None,
            None,
        )
        .expect("full approval");
        assert_eq!(full_approval.note.as_deref(), Some("approved"));
        assert_eq!(
            full_approval
                .selected_data_recommendation_ids(&constrained)
                .expect("approved IDs")
                .len(),
            constrained.normalized_recommendations.len()
        );
        let selected_id = constrained.normalized_recommendations[0].id.clone();
        let partial = ProposalReviewRecord::new(
            &constrained,
            ProposalReviewState::PartiallyAccepted,
            vec![selected_id.clone()],
            None,
            None,
            None,
        )
        .expect("partial approval");
        assert_eq!(
            partial
                .selected_data_recommendation_ids(&constrained)
                .expect("partial IDs"),
            vec![selected_id]
        );
        let (approved_plan, application) =
            approved_proposal_to_plan(&constrained, &partial, &dataset, &accepted)
                .expect("approved plan");
        assert_eq!(approved_plan.cells.len(), 1);
        assert_eq!(application.approval_review_id, Some(partial.id));
        let mut changed_coverage = accepted.clone();
        *changed_coverage
            .entry(constrained.normalized_recommendations[0].cell.key())
            .or_insert(0) += 1;
        assert!(matches!(
            approved_proposal_to_plan(&constrained, &partial, &dataset, &changed_coverage),
            Err(ApprovedApplicationError::StaleCoverage { .. })
        ));
        let refreshed_evidence = OptimizationEvidence::new(
            evidence.diagnostics.clone(),
            &dataset,
            snapshot_id,
            "sha256:snapshot",
            &changed_coverage,
            &protocol,
            None,
            true,
        )
        .expect("refreshed evidence");
        let rebased = rebase_constrained_proposal(&constrained, &refreshed_evidence)
            .expect("immutable rebase");
        verify_constrained_proposal(&rebased).expect("rebased proposal verifies");
        let lineage = rebased.rebase_lineage.as_ref().expect("rebase lineage");
        assert_eq!(lineage.previous_proposal_id, constrained.id);
        assert_eq!(
            lineage.previous_proposal_fingerprint,
            constrained.fingerprint
        );
        assert_ne!(
            lineage.previous_coverage_fingerprint,
            lineage.refreshed_coverage_fingerprint
        );
        assert_ne!(rebased.fingerprint, constrained.fingerprint);
        assert!(matches!(
            rebase_constrained_proposal(&constrained, &evidence),
            Err(super::OptimizationError::RebaseCoverageUnchanged)
        ));
        let mut review_protocol = protocol.clone();
        review_protocol.recommendation_kinds = vec![
            crate::protocol::RecommendationKind::DataGeneration,
            crate::protocol::RecommendationKind::ReviewOnly,
        ];
        let review_protocol = review_protocol.normalize().expect("review protocol");
        let review_evidence = evidence
            .rebind_protocol(&review_protocol, None)
            .expect("review-bound evidence");
        let review_proposal = create_constrained_proposal(&review_evidence, &review_protocol)
            .expect("proposal with review-only recommendations");
        assert!(review_proposal.review_only_recommendations.len() >= 2);
        let review_acceptance = ProposalReviewRecord::new(
            &review_proposal,
            ProposalReviewState::AcceptedReviewOnlyCandidates,
            vec![review_proposal.review_only_recommendations[0].id.clone()],
            Some("inspect schema".into()),
            None,
            None,
        )
        .expect("review-only acceptance");
        review_acceptance
            .validate(&review_proposal)
            .expect("review-only acceptance validates");
    }

    fn cell_finding(
        label: &str,
        style: &str,
        support: u64,
        errors: u64,
        total_support: u64,
        total_errors: u64,
    ) -> AnalysisFinding {
        let attributes = BTreeMap::from([
            ("label".into(), label.into()),
            ("style".into(), style.into()),
        ]);
        let error_rate = rounded(errors as f64 / support as f64);
        let baseline_error_rate = rounded(total_errors as f64 / total_support as f64);
        let mut finding = AnalysisFinding {
            rank: 1,
            kind: FindingKind::Cell,
            key: FindingIdentity {
                kind: FindingKind::Cell,
                attributes: attributes.clone(),
            }
            .key(),
            attributes,
            support,
            error_count: errors,
            error_rate,
            mean_error_confidence: 0.0,
            median_error_confidence: 0.0,
            mean_expected_probability: 0.0,
            mean_prediction_margin: 0.0,
            mean_entropy: 0.0,
            error_rate_lift: rounded(error_rate - baseline_error_rate),
            error_share: rounded(errors as f64 / total_errors as f64),
            high_confidence_error_severity: 0.0,
            marginal_error_count: 0,
            cumulative_error_count: 0,
            cumulative_error_coverage: 0.0,
            fingerprint: String::new(),
        };
        finding.refresh_fingerprint().expect("finding fingerprint");
        finding
    }

    fn rounded(value: f64) -> f64 {
        (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
    }
}
