use std::collections::BTreeMap;

use dataset_architect_core::proposal::{
    ArchitectProposalDraft, CellAllocationRecommendation, ExpectedBenefit,
    GenerationStrategyDirective, RecommendationConfidence, StrategyKind,
};
use generation_core::domain::GenerationCell;
use serde::Deserialize;
use uuid::Uuid;
use workflow_core::allocation::{CellSelector, ExplicitCellTarget};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PreviewAllocationInput {
    pub allocations: Vec<PreviewCellInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreviewCellInput {
    pub cell: GenerationCell,
    pub target: u32,
}

impl PreviewAllocationInput {
    pub fn into_targets(self) -> Vec<ExplicitCellTarget> {
        self.allocations
            .into_iter()
            .map(|value| ExplicitCellTarget {
                cell: value.cell,
                target: value.target,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EstimateCostInput {
    pub additional_rows: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProposalSubmission {
    pub summary: String,
    pub allocations: Vec<AllocationSubmission>,
    #[serde(default)]
    pub strategies: Vec<StrategySubmission>,
    #[serde(default)]
    pub tradeoffs: Vec<String>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AllocationSubmission {
    pub cell: GenerationCell,
    pub target: u32,
    pub rationale: String,
    pub confidence: RecommendationConfidence,
    pub expected_benefits: Vec<ExpectedBenefit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StrategySubmission {
    pub id: Option<Uuid>,
    pub selector: SelectorSubmission,
    pub kind: StrategyKind,
    pub share_basis_points: u16,
    pub instructions: Vec<String>,
    #[serde(default)]
    pub related_labels: Vec<String>,
    pub rationale: String,
    pub confidence: RecommendationConfidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectorSubmission {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}

impl ProposalSubmission {
    pub fn into_draft(self) -> ArchitectProposalDraft {
        ArchitectProposalDraft {
            summary: self.summary,
            allocations: self
                .allocations
                .into_iter()
                .map(|value| CellAllocationRecommendation {
                    cell: value.cell,
                    target: value.target,
                    rationale: value.rationale,
                    confidence: value.confidence,
                    expected_benefits: value.expected_benefits,
                })
                .collect(),
            strategies: self
                .strategies
                .into_iter()
                .map(|value| GenerationStrategyDirective {
                    id: value.id.unwrap_or_else(Uuid::new_v4),
                    selector: CellSelector {
                        label: value.selector.label,
                        dimensions: value.selector.dimensions,
                    },
                    kind: value.kind,
                    share_basis_points: value.share_basis_points,
                    instructions: value.instructions,
                    related_labels: value.related_labels,
                    rationale: value.rationale,
                    confidence: value.confidence,
                })
                .collect(),
            tradeoffs: self.tradeoffs,
            uncertainties: self.uncertainties,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FinishReason {
    ProposalSubmitted,
    BudgetExhausted,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FinishInput {
    pub reason: FinishReason,
    pub summary: String,
    pub confidence: RecommendationConfidence,
}
