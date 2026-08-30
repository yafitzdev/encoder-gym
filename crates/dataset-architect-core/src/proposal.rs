use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{GenerationCell, GenerationPlan},
    strategy::{
        GenerationStrategyDirective as AppliedStrategyDirective, ResolvedGenerationStrategyContext,
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::allocation::{
    CellSelector, ExplicitCellTarget, InitialAllocationFeasibility, InitialAllocationPolicy,
    InitialAllocationRequest, InitialAllocationResult, allocate_initial_budget,
};

use crate::{
    ArchitectError,
    brief::{GenerationCostModel, ResolvedArchitectBrief},
    fingerprint,
    lifecycle::{ArchitectRun, ArchitectRunState},
    required,
};

pub const ARCHITECT_PROPOSAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedBenefit {
    Coverage,
    ClassBoundary,
    Robustness,
    Authenticity,
    Ambiguity,
    RarePattern,
    ErrorReduction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    HardNegative,
    BoundaryCase,
    Ambiguity,
    Noise,
    RarePattern,
    ChannelVariation,
    LengthVariation,
    Custom,
}

impl StrategyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HardNegative => "hard_negative",
            Self::BoundaryCase => "boundary_case",
            Self::Ambiguity => "ambiguity",
            Self::Noise => "noise",
            Self::RarePattern => "rare_pattern",
            Self::ChannelVariation => "channel_variation",
            Self::LengthVariation => "length_variation",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellAllocationRecommendation {
    pub cell: GenerationCell,
    pub target: u32,
    pub rationale: String,
    pub confidence: RecommendationConfidence,
    pub expected_benefits: Vec<ExpectedBenefit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationStrategyDirective {
    pub id: Uuid,
    pub selector: CellSelector,
    pub kind: StrategyKind,
    /// Approximate share of examples inside every matched cell, in basis points.
    pub share_basis_points: u16,
    pub instructions: Vec<String>,
    #[serde(default)]
    pub related_labels: Vec<String>,
    pub rationale: String,
    pub confidence: RecommendationConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationCostEstimate {
    pub additional_rows: u64,
    pub estimated_requests: u64,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub estimated_cost_microusd: Option<u64>,
    pub assumptions_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectProposalDraft {
    pub summary: String,
    pub allocations: Vec<CellAllocationRecommendation>,
    #[serde(default)]
    pub strategies: Vec<GenerationStrategyDirective>,
    #[serde(default)]
    pub tradeoffs: Vec<String>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetArchitectureProposal {
    pub id: Uuid,
    pub schema_version: u32,
    pub run_id: Uuid,
    pub run_fingerprint: String,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub summary: String,
    pub allocations: Vec<CellAllocationRecommendation>,
    pub strategies: Vec<GenerationStrategyDirective>,
    pub tradeoffs: Vec<String>,
    pub uncertainties: Vec<String>,
    pub validated_allocation: InitialAllocationResult,
    pub coverage_fingerprint: String,
    pub cost_estimate: GenerationCostEstimate,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DatasetArchitectureProposal {
    pub fn create(
        brief: &ResolvedArchitectBrief,
        run: &ArchitectRun,
        draft: ArchitectProposalDraft,
    ) -> Result<Self, ArchitectError> {
        if brief.reproduce_fingerprint()? != brief.fingerprint
            || run.reproduce_specification_fingerprint()? != run.specification_fingerprint
            || run.brief_id != brief.id
            || run.brief_fingerprint != brief.fingerprint
            || run.state != ArchitectRunState::Running
        {
            return Err(ArchitectError::Integrity(
                "proposal inputs do not reproduce or belong together".into(),
            ));
        }
        let allocations = validate_allocations(brief, draft.allocations)?;
        let strategies = validate_strategies(brief, draft.strategies)?;
        let validated_allocation = allocate_initial_budget(
            &brief.dataset,
            InitialAllocationRequest {
                total_rows: brief.target_total_rows,
                reserved_rows: brief.reserved_rows,
                policy: InitialAllocationPolicy::Explicit {
                    targets: allocations
                        .iter()
                        .map(|value| ExplicitCellTarget {
                            cell: value.cell.clone(),
                            target: value.target,
                        })
                        .collect(),
                },
                current_coverage: brief.current_coverage.clone(),
                constraints: brief.constraints.clone(),
            },
        )
        .map_err(|error| ArchitectError::Allocation(error.to_string()))?;
        if validated_allocation.feasibility != InitialAllocationFeasibility::Feasible {
            return Err(ArchitectError::Allocation(format!(
                "proposal is infeasible: {:?}",
                validated_allocation.issues
            )));
        }
        let coverage_fingerprint = fingerprint(&brief.current_coverage)?;
        let cost_estimate = estimate_cost(
            validated_allocation
                .cells
                .iter()
                .map(|cell| u64::from(cell.additional_required))
                .sum(),
            &brief.cost_model,
        )?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: ARCHITECT_PROPOSAL_SCHEMA_VERSION,
            run_id: run.id,
            run_fingerprint: run.specification_fingerprint.clone(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            dataset_id: brief.dataset.id,
            dataset_fingerprint: brief.dataset_fingerprint.clone(),
            summary: required(draft.summary, "proposal.summary")?,
            allocations,
            strategies,
            tradeoffs: normalized_list(draft.tradeoffs, "proposal.tradeoffs")?,
            uncertainties: normalized_list(draft.uncertainties, "proposal.uncertainties")?,
            validated_allocation,
            coverage_fingerprint,
            cost_estimate,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

// A proposal intentionally does not retain a second copy of the complete dataset.
// Application therefore receives the authoritative dataset explicitly.
pub fn proposal_to_generation_plan(
    proposal: &DatasetArchitectureProposal,
    brief: &ResolvedArchitectBrief,
) -> Result<GenerationPlan, ArchitectError> {
    validate_proposal_context(proposal, brief)?;
    proposal
        .validated_allocation
        .to_generation_plan(&brief.dataset)
        .map_err(|error| ArchitectError::Allocation(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetArchitectureApplication {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub strategy_context_id: Uuid,
    pub strategy_context_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DatasetArchitectureApplication {
    pub fn reproduce_fingerprint(&self) -> Result<String, ArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedArchitecture {
    pub plan: GenerationPlan,
    pub strategy_context: ResolvedGenerationStrategyContext,
    pub application: DatasetArchitectureApplication,
}

/// Converts a human-approved advisory proposal into ordinary generation inputs.
/// Coverage is checked again at application time so stale advice cannot be applied.
pub fn apply_approved_proposal(
    proposal: &DatasetArchitectureProposal,
    brief: &ResolvedArchitectBrief,
    approval: &ArchitectProposalReview,
    current_coverage: &[workflow_core::allocation::InitialCellCoverage],
) -> Result<AppliedArchitecture, ArchitectError> {
    validate_proposal_context(proposal, brief)?;
    if approval.reproduce_fingerprint()? != approval.fingerprint
        || approval.proposal_id != proposal.id
        || approval.proposal_fingerprint != proposal.fingerprint
        || approval.decision != ArchitectReviewDecision::Approve
    {
        return Err(ArchitectError::Integrity(
            "proposal requires its latest reproducible human approval".into(),
        ));
    }
    if fingerprint(&current_coverage.to_vec())? != proposal.coverage_fingerprint {
        return Err(ArchitectError::Integrity(
            "dataset coverage changed after the proposal was produced".into(),
        ));
    }
    let plan = proposal_to_generation_plan(proposal, brief)?;
    let per_cell = plan
        .cells
        .iter()
        .filter_map(|planned| {
            let directives = proposal
                .strategies
                .iter()
                .filter(|directive| selector_matches(&directive.selector, &planned.cell))
                .map(|directive| AppliedStrategyDirective {
                    source_directive_id: directive.id,
                    kind: directive.kind.as_str().into(),
                    share_basis_points: directive.share_basis_points,
                    instructions: directive.instructions.clone(),
                    related_labels: directive.related_labels.clone(),
                    rationale: directive.rationale.clone(),
                    confidence: format!("{:?}", directive.confidence).to_lowercase(),
                })
                .collect::<Vec<_>>();
            (!directives.is_empty()).then(|| (planned.cell.key(), directives))
        })
        .collect();
    let strategy_context = ResolvedGenerationStrategyContext::create(
        &brief.dataset,
        &plan,
        proposal.id,
        proposal.fingerprint.clone(),
        approval.id,
        approval.fingerprint.clone(),
        per_cell,
    )
    .map_err(|error| ArchitectError::Integrity(error.to_string()))?;
    let mut application = DatasetArchitectureApplication {
        id: Uuid::new_v4(),
        proposal_id: proposal.id,
        proposal_fingerprint: proposal.fingerprint.clone(),
        approval_id: approval.id,
        approval_fingerprint: approval.fingerprint.clone(),
        plan_id: plan.id,
        plan_fingerprint: fingerprint(&plan)?,
        strategy_context_id: strategy_context.id,
        strategy_context_fingerprint: strategy_context.fingerprint.clone(),
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    application.fingerprint = application.reproduce_fingerprint()?;
    Ok(AppliedArchitecture {
        plan,
        strategy_context,
        application,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectProposalReview {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub decision: ArchitectReviewDecision,
    pub reviewer: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ArchitectProposalReview {
    pub fn create(
        proposal: &DatasetArchitectureProposal,
        predecessor: Option<&Self>,
        decision: ArchitectReviewDecision,
        reviewer: String,
        reason: String,
    ) -> Result<Self, ArchitectError> {
        if proposal.reproduce_fingerprint()? != proposal.fingerprint
            || predecessor.is_some_and(|value| value.proposal_id != proposal.id)
        {
            return Err(ArchitectError::Integrity(
                "proposal review inputs do not reproduce or belong together".into(),
            ));
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            decision,
            reviewer: required(reviewer, "review.reviewer")?,
            reason: required(reason, "review.reason")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn validate_allocations(
    brief: &ResolvedArchitectBrief,
    values: Vec<CellAllocationRecommendation>,
) -> Result<Vec<CellAllocationRecommendation>, ArchitectError> {
    let expected = expand_generation_cells(&brief.dataset)
        .into_iter()
        .map(|cell| (cell.key(), cell))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    for mut value in values {
        let key = value.cell.key();
        if expected.get(&key) != Some(&value.cell) || actual.contains_key(&key) {
            return Err(ArchitectError::Validation(format!(
                "proposal allocation contains an unknown or duplicate cell {key}"
            )));
        }
        value.rationale = required(value.rationale, "allocation.rationale")?;
        if value.expected_benefits.is_empty() {
            return Err(ArchitectError::Validation(format!(
                "allocation {key} requires at least one expected benefit"
            )));
        }
        let unique = value
            .expected_benefits
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if unique.len() != value.expected_benefits.len() {
            return Err(ArchitectError::Validation(format!(
                "allocation {key} repeats an expected benefit"
            )));
        }
        actual.insert(key, value);
    }
    if actual.len() != expected.len() {
        let missing = expected
            .keys()
            .filter(|key| !actual.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        return Err(ArchitectError::Validation(format!(
            "proposal must explicitly allocate every generation cell; missing {missing:?}"
        )));
    }
    Ok(actual.into_values().collect())
}

fn validate_strategies(
    brief: &ResolvedArchitectBrief,
    values: Vec<GenerationStrategyDirective>,
) -> Result<Vec<GenerationStrategyDirective>, ArchitectError> {
    let cells = expand_generation_cells(&brief.dataset);
    let labels = brief.dataset.labels.iter().collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    values
        .into_iter()
        .map(|mut value| {
            if value.id == Uuid::nil() || !ids.insert(value.id) {
                return Err(ArchitectError::Validation(
                    "strategy directive IDs must be unique and non-nil".into(),
                ));
            }
            if value.share_basis_points == 0 || value.share_basis_points > 10_000 {
                return Err(ArchitectError::Validation(
                    "strategy share must be between 1 and 10000 basis points".into(),
                ));
            }
            if !cells
                .iter()
                .any(|cell| selector_matches(&value.selector, cell))
            {
                return Err(ArchitectError::Validation(
                    "strategy selector matches no generation cells".into(),
                ));
            }
            for label in &value.related_labels {
                if !labels.contains(label) {
                    return Err(ArchitectError::Validation(format!(
                        "strategy references unknown related label {label:?}"
                    )));
                }
            }
            value.instructions = normalized_list(value.instructions, "strategy.instructions")?;
            if value.instructions.is_empty() {
                return Err(ArchitectError::Validation(
                    "strategy requires at least one generation instruction".into(),
                ));
            }
            value.rationale = required(value.rationale, "strategy.rationale")?;
            Ok(value)
        })
        .collect()
}

fn selector_matches(selector: &CellSelector, cell: &GenerationCell) -> bool {
    selector
        .label
        .as_ref()
        .is_none_or(|label| label == &cell.label)
        && selector.dimensions.iter().all(|(dimension, value)| {
            cell.dimensions
                .get(dimension)
                .is_some_and(|actual| actual == value)
        })
}

pub fn estimate_cost(
    additional_rows: u64,
    model: &GenerationCostModel,
) -> Result<GenerationCostEstimate, ArchitectError> {
    if model.rows_per_request == 0 || model.estimated_output_tokens_per_row == 0 {
        return Err(ArchitectError::Validation(
            "invalid generation cost model".into(),
        ));
    }
    let estimated_requests = additional_rows.div_ceil(u64::from(model.rows_per_request));
    let estimated_input_tokens = estimated_requests
        .checked_mul(model.estimated_input_tokens_per_request)
        .ok_or_else(|| ArchitectError::Validation("cost estimate overflowed".into()))?;
    let estimated_output_tokens = additional_rows
        .checked_mul(model.estimated_output_tokens_per_row)
        .ok_or_else(|| ArchitectError::Validation("cost estimate overflowed".into()))?;
    let estimated_cost_microusd = match (
        model.input_cost_microusd_per_million_tokens,
        model.output_cost_microusd_per_million_tokens,
    ) {
        (Some(input), Some(output)) => Some(
            estimated_input_tokens
                .checked_mul(input)
                .and_then(|value| {
                    estimated_output_tokens
                        .checked_mul(output)
                        .and_then(|right| value.checked_add(right))
                })
                .map(|value| value.div_ceil(1_000_000))
                .ok_or_else(|| ArchitectError::Validation("cost estimate overflowed".into()))?,
        ),
        _ => None,
    };
    Ok(GenerationCostEstimate {
        additional_rows,
        estimated_requests,
        estimated_input_tokens,
        estimated_output_tokens,
        estimated_cost_microusd,
        assumptions_fingerprint: fingerprint(model)?,
    })
}

fn normalized_list(values: Vec<String>, field: &str) -> Result<Vec<String>, ArchitectError> {
    let mut result = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = required(value, field)?;
        if !seen.insert(value.clone()) {
            return Err(ArchitectError::Validation(format!(
                "{field} contains a duplicate"
            )));
        }
        result.push(value);
    }
    Ok(result)
}

fn validate_proposal_context(
    proposal: &DatasetArchitectureProposal,
    brief: &ResolvedArchitectBrief,
) -> Result<(), ArchitectError> {
    if proposal.reproduce_fingerprint()? != proposal.fingerprint
        || brief.reproduce_fingerprint()? != brief.fingerprint
        || proposal.brief_id != brief.id
        || proposal.brief_fingerprint != brief.fingerprint
        || proposal.dataset_id != brief.dataset.id
        || proposal.dataset_fingerprint != brief.dataset_fingerprint
        || proposal.coverage_fingerprint != fingerprint(&brief.current_coverage)?
    {
        return Err(ArchitectError::Integrity(
            "proposal cannot be applied to changed brief, dataset, or coverage facts".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use generation_core::dimensions::expand_generation_cells;

    use crate::{brief::tests::brief, lifecycle::ArchitectRun};

    use super::*;

    fn draft(brief: &ResolvedArchitectBrief) -> ArchitectProposalDraft {
        ArchitectProposalDraft {
            summary: "Emphasize messy boundary cases while retaining complete coverage.".into(),
            allocations: expand_generation_cells(&brief.dataset)
                .into_iter()
                .map(|cell| CellAllocationRecommendation {
                    cell,
                    target: 10,
                    rationale: "Balanced baseline with explicit future refinement.".into(),
                    confidence: RecommendationConfidence::Medium,
                    expected_benefits: vec![ExpectedBenefit::Coverage],
                })
                .collect(),
            strategies: vec![GenerationStrategyDirective {
                id: Uuid::new_v4(),
                selector: CellSelector {
                    label: None,
                    dimensions: BTreeMap::from([("style".into(), "messy".into())]),
                },
                kind: StrategyKind::Noise,
                share_basis_points: 3_000,
                instructions: vec!["Use plausible typos and fragmented punctuation.".into()],
                related_labels: vec![],
                rationale: "The target channel contains hurried messages.".into(),
                confidence: RecommendationConfidence::High,
            }],
            tradeoffs: vec!["More edge cases can reduce easy-example share.".into()],
            uncertainties: vec!["No development diagnostics are available yet.".into()],
        }
    }

    #[test]
    fn proposal_is_validated_by_the_existing_allocator_and_builds_a_normal_plan() {
        let brief = brief();
        let mut run = ArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let proposal = DatasetArchitectureProposal::create(&brief, &run, draft(&brief)).unwrap();
        assert_eq!(proposal.validated_allocation.allocated_target_rows, 40);
        assert_eq!(proposal.cost_estimate.additional_rows, 40);
        assert_eq!(
            proposal.reproduce_fingerprint().unwrap(),
            proposal.fingerprint
        );
        let plan = proposal_to_generation_plan(&proposal, &brief).unwrap();
        assert_eq!(plan.total_target_count(), 40);
    }

    #[test]
    fn agent_cannot_omit_a_cell_or_bypass_feasibility() {
        let brief = brief();
        let mut run = ArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let mut value = draft(&brief);
        value.allocations.pop();
        assert!(matches!(
            DatasetArchitectureProposal::create(&brief, &run, value),
            Err(ArchitectError::Validation(message)) if message.contains("every generation cell")
        ));
    }

    #[test]
    fn only_an_approved_current_proposal_compiles_to_generation_inputs() {
        let brief = brief();
        let mut run = ArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let proposal = DatasetArchitectureProposal::create(&brief, &run, draft(&brief)).unwrap();
        let approval = ArchitectProposalReview::create(
            &proposal,
            None,
            ArchitectReviewDecision::Approve,
            "operator".into(),
            "The budget and tradeoffs are acceptable.".into(),
        )
        .unwrap();
        let applied =
            apply_approved_proposal(&proposal, &brief, &approval, &brief.current_coverage).unwrap();
        assert_eq!(applied.plan.total_target_count(), 40);
        assert_eq!(applied.strategy_context.per_cell.len(), 2);
        assert_eq!(
            applied.application.strategy_context_fingerprint,
            applied.strategy_context.fingerprint
        );

        let mut stale = brief.current_coverage.clone();
        stale.push(workflow_core::allocation::InitialCellCoverage {
            cell: expand_generation_cells(&brief.dataset)[0].clone(),
            accepted: 1,
        });
        assert!(apply_approved_proposal(&proposal, &brief, &approval, &stale).is_err());
    }
}
