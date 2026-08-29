use std::collections::{BTreeMap, BTreeSet};

use generation_core::domain::GenerationCell;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    DataGeneration,
    TrainingConfiguration,
    ReviewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringPolicy {
    ErrorCount,
    ErrorRate,
    ErrorRateLift,
    HighConfidenceErrorSeverity,
    MarginalErrorCoverage,
    ComparisonRegression,
    ConservativeComposite,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RiskPolicy {
    None,
    WilsonLowerBound { z_score: f64 },
    BaselineShrinkage { prior_strength: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TieBreakPolicy {
    CanonicalCellIdentity,
    SeededCanonicalCellIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedReviewDisposition {
    AcceptedLimitation,
    ResolvedByLaterEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfeasibleAllocationBehavior {
    Reject,
    AllowUnallocated,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionValueSelection {
    pub dimension: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellAllocationBounds {
    pub cell: GenerationCell,
    #[serde(default)]
    pub minimum_addition: u32,
    pub maximum_addition: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelAllocationBounds {
    pub label: String,
    #[serde(default)]
    pub minimum_addition: u32,
    pub maximum_addition: Option<u32>,
    pub minimum_budget_share: Option<f64>,
    pub maximum_budget_share: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingCandidateRequest {
    pub maximum_candidates: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationProtocol {
    pub recommendation_kinds: Vec<RecommendationKind>,
    pub additional_example_budget: u32,
    pub minimum_evidence_support: u64,
    pub scoring_policy: ScoringPolicy,
    pub risk_policy: RiskPolicy,
    pub tie_break_policy: TieBreakPolicy,
    #[serde(default)]
    pub allowed_labels: Vec<String>,
    #[serde(default)]
    pub excluded_labels: Vec<String>,
    #[serde(default)]
    pub allowed_cells: Vec<GenerationCell>,
    #[serde(default)]
    pub excluded_cells: Vec<GenerationCell>,
    #[serde(default)]
    pub allowed_dimension_values: Vec<DimensionValueSelection>,
    #[serde(default)]
    pub excluded_dimension_values: Vec<DimensionValueSelection>,
    #[serde(default)]
    pub label_bounds: Vec<LabelAllocationBounds>,
    #[serde(default)]
    pub cell_bounds: Vec<CellAllocationBounds>,
    pub maximum_cell_budget_share: Option<f64>,
    pub minimum_allocation_increment: u32,
    pub use_comparison_regression_evidence: bool,
    #[serde(default)]
    pub excluded_review_dispositions: Vec<ExcludedReviewDisposition>,
    pub training_candidates: Option<TrainingCandidateRequest>,
    pub deterministic_seed: u64,
    pub infeasible_allocation_behavior: InfeasibleAllocationBehavior,
}

impl OptimizationProtocol {
    pub fn legacy(additional_example_budget: u32, minimum_evidence_support: u64) -> Self {
        Self {
            recommendation_kinds: vec![RecommendationKind::DataGeneration],
            additional_example_budget,
            minimum_evidence_support,
            scoring_policy: ScoringPolicy::ErrorCount,
            risk_policy: RiskPolicy::None,
            tie_break_policy: TieBreakPolicy::CanonicalCellIdentity,
            allowed_labels: Vec::new(),
            excluded_labels: Vec::new(),
            allowed_cells: Vec::new(),
            excluded_cells: Vec::new(),
            allowed_dimension_values: Vec::new(),
            excluded_dimension_values: Vec::new(),
            label_bounds: Vec::new(),
            cell_bounds: Vec::new(),
            maximum_cell_budget_share: None,
            minimum_allocation_increment: 1,
            use_comparison_regression_evidence: false,
            excluded_review_dispositions: Vec::new(),
            training_candidates: None,
            deterministic_seed: 42,
            infeasible_allocation_behavior: InfeasibleAllocationBehavior::Reject,
        }
    }

    pub fn normalize(mut self) -> Result<Self, OptimizationProtocolError> {
        normalize_strings(&mut self.allowed_labels, "allowed labels")?;
        normalize_strings(&mut self.excluded_labels, "excluded labels")?;
        normalize_cells(&mut self.allowed_cells, "allowed cells")?;
        normalize_cells(&mut self.excluded_cells, "excluded cells")?;
        normalize_dimension_values(
            &mut self.allowed_dimension_values,
            "allowed dimension values",
        )?;
        normalize_dimension_values(
            &mut self.excluded_dimension_values,
            "excluded dimension values",
        )?;
        normalize_label_bounds(&mut self.label_bounds)?;
        normalize_cell_bounds(&mut self.cell_bounds)?;
        self.recommendation_kinds.sort_unstable();
        self.excluded_review_dispositions.sort_unstable();
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), OptimizationProtocolError> {
        if self.recommendation_kinds.is_empty()
            || has_duplicates_or_unsorted(&self.recommendation_kinds)
        {
            return Err(OptimizationProtocolError::RecommendationKinds);
        }
        if !self
            .recommendation_kinds
            .contains(&RecommendationKind::DataGeneration)
        {
            return Err(OptimizationProtocolError::DataGenerationKindMissing);
        }
        if self.additional_example_budget == 0 {
            return Err(OptimizationProtocolError::Budget);
        }
        if self.minimum_evidence_support == 0 {
            return Err(OptimizationProtocolError::MinimumSupport);
        }
        validate_risk_policy(self.risk_policy)?;
        validate_canonical_strings(&self.allowed_labels, "allowed labels")?;
        validate_canonical_strings(&self.excluded_labels, "excluded labels")?;
        validate_cells(&self.allowed_cells, "allowed cells")?;
        validate_cells(&self.excluded_cells, "excluded cells")?;
        validate_dimension_values(&self.allowed_dimension_values, "allowed dimension values")?;
        validate_dimension_values(&self.excluded_dimension_values, "excluded dimension values")?;
        reject_overlap(
            &self.allowed_labels,
            &self.excluded_labels,
            OptimizationProtocolError::ContradictoryLabels,
        )?;
        reject_cell_overlap(&self.allowed_cells, &self.excluded_cells)?;
        reject_overlap(
            &self.allowed_dimension_values,
            &self.excluded_dimension_values,
            OptimizationProtocolError::ContradictoryDimensionValues,
        )?;
        validate_label_bounds(&self.label_bounds, self.additional_example_budget)?;
        validate_cell_bounds(&self.cell_bounds, self.additional_example_budget)?;
        validate_bounds_against_exclusions(self)?;
        validate_optional_share(self.maximum_cell_budget_share, false)?;
        if self.minimum_allocation_increment == 0
            || self.minimum_allocation_increment > self.additional_example_budget
        {
            return Err(OptimizationProtocolError::AllocationIncrement);
        }
        if self.infeasible_allocation_behavior == InfeasibleAllocationBehavior::Reject
            && self.additional_example_budget % self.minimum_allocation_increment != 0
        {
            return Err(OptimizationProtocolError::IncrementCannotConserveBudget);
        }
        if has_duplicates_or_unsorted(&self.excluded_review_dispositions) {
            return Err(OptimizationProtocolError::ReviewDispositions);
        }
        if self.scoring_policy == ScoringPolicy::ComparisonRegression
            && !self.use_comparison_regression_evidence
        {
            return Err(OptimizationProtocolError::ComparisonPolicyDisabled);
        }
        if let Some(request) = &self.training_candidates {
            if request.maximum_candidates == 0 || request.maximum_candidates > 64 {
                return Err(OptimizationProtocolError::TrainingCandidateLimit);
            }
            if !self
                .recommendation_kinds
                .contains(&RecommendationKind::TrainingConfiguration)
            {
                return Err(OptimizationProtocolError::TrainingKindMissing);
            }
        } else if self
            .recommendation_kinds
            .contains(&RecommendationKind::TrainingConfiguration)
        {
            return Err(OptimizationProtocolError::TrainingRequestMissing);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, OptimizationProtocolError> {
        self.validate()?;
        artifact_core::fingerprint(self)
            .map_err(|error| OptimizationProtocolError::Fingerprint(error.to_string()))
    }
}

fn validate_risk_policy(policy: RiskPolicy) -> Result<(), OptimizationProtocolError> {
    match policy {
        RiskPolicy::None => Ok(()),
        RiskPolicy::WilsonLowerBound { z_score }
            if z_score.is_finite() && z_score > 0.0 && z_score <= 10.0 =>
        {
            Ok(())
        }
        RiskPolicy::BaselineShrinkage { prior_strength }
            if prior_strength.is_finite()
                && prior_strength > 0.0
                && prior_strength <= 1_000_000.0 =>
        {
            Ok(())
        }
        RiskPolicy::WilsonLowerBound { .. } => Err(OptimizationProtocolError::WilsonZScore),
        RiskPolicy::BaselineShrinkage { .. } => {
            Err(OptimizationProtocolError::ShrinkagePriorStrength)
        }
    }
}

fn validate_label_bounds(
    bounds: &[LabelAllocationBounds],
    budget: u32,
) -> Result<(), OptimizationProtocolError> {
    let mut previous = None;
    let mut total_minimum = 0_u64;
    for bound in bounds {
        let label = bound.label.trim();
        if label.is_empty() || label != bound.label || previous.is_some_and(|value| value >= label)
        {
            return Err(OptimizationProtocolError::LabelBounds);
        }
        validate_count_bounds(bound.minimum_addition, bound.maximum_addition, budget)?;
        validate_optional_share(bound.minimum_budget_share, true)?;
        validate_optional_share(bound.maximum_budget_share, false)?;
        if matches!(
            (bound.minimum_budget_share, bound.maximum_budget_share),
            (Some(minimum), Some(maximum)) if minimum > maximum
        ) {
            return Err(OptimizationProtocolError::LabelBounds);
        }
        total_minimum = total_minimum
            .checked_add(u64::from(bound.minimum_addition))
            .ok_or(OptimizationProtocolError::BoundsOverflow)?;
        previous = Some(label);
    }
    if total_minimum > u64::from(budget) {
        return Err(OptimizationProtocolError::MinimumsExceedBudget);
    }
    Ok(())
}

fn validate_cell_bounds(
    bounds: &[CellAllocationBounds],
    budget: u32,
) -> Result<(), OptimizationProtocolError> {
    let mut previous = None;
    let mut total_minimum = 0_u64;
    for bound in bounds {
        validate_cell(&bound.cell)?;
        let key = bound.cell.key();
        if previous.as_ref().is_some_and(|value| value >= &key) {
            return Err(OptimizationProtocolError::CellBounds);
        }
        validate_count_bounds(bound.minimum_addition, bound.maximum_addition, budget)?;
        total_minimum = total_minimum
            .checked_add(u64::from(bound.minimum_addition))
            .ok_or(OptimizationProtocolError::BoundsOverflow)?;
        previous = Some(key);
    }
    if total_minimum > u64::from(budget) {
        return Err(OptimizationProtocolError::MinimumsExceedBudget);
    }
    Ok(())
}

fn validate_count_bounds(
    minimum: u32,
    maximum: Option<u32>,
    budget: u32,
) -> Result<(), OptimizationProtocolError> {
    if minimum > budget || maximum.is_some_and(|value| value < minimum || value > budget) {
        return Err(OptimizationProtocolError::AllocationBounds);
    }
    Ok(())
}

fn validate_optional_share(
    value: Option<f64>,
    allow_zero: bool,
) -> Result<(), OptimizationProtocolError> {
    if value.is_some_and(|share| {
        !share.is_finite()
            || share > 1.0
            || if allow_zero {
                share < 0.0
            } else {
                share <= 0.0
            }
    }) {
        return Err(OptimizationProtocolError::BudgetShare);
    }
    Ok(())
}

fn validate_bounds_against_exclusions(
    protocol: &OptimizationProtocol,
) -> Result<(), OptimizationProtocolError> {
    let excluded_labels = protocol.excluded_labels.iter().collect::<BTreeSet<_>>();
    if protocol.label_bounds.iter().any(|bound| {
        excluded_labels.contains(&bound.label)
            && (bound.minimum_addition > 0
                || bound.minimum_budget_share.is_some_and(|share| share > 0.0))
    }) {
        return Err(OptimizationProtocolError::ExcludedMinimum);
    }
    let excluded_cells = protocol
        .excluded_cells
        .iter()
        .map(GenerationCell::key)
        .collect::<BTreeSet<_>>();
    if protocol
        .cell_bounds
        .iter()
        .any(|bound| excluded_cells.contains(&bound.cell.key()) && bound.minimum_addition > 0)
    {
        return Err(OptimizationProtocolError::ExcludedMinimum);
    }
    Ok(())
}

fn normalize_strings(
    values: &mut [String],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    for value in values.iter_mut() {
        *value = value.trim().to_owned();
        if value.is_empty() {
            return Err(OptimizationProtocolError::NonCanonical(field));
        }
    }
    values.sort_unstable();
    if values.windows(2).any(|values| values[0] == values[1]) {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn normalize_cells(
    cells: &mut [GenerationCell],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    for cell in cells.iter_mut() {
        normalize_cell(cell)?;
    }
    cells.sort_by_key(GenerationCell::key);
    if cells
        .windows(2)
        .any(|cells| cells[0].key() == cells[1].key())
    {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn normalize_dimension_values(
    values: &mut [DimensionValueSelection],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    for selection in values.iter_mut() {
        selection.dimension = selection.dimension.trim().to_owned();
        selection.value = selection.value.trim().to_owned();
        if selection.dimension.is_empty() || selection.value.is_empty() {
            return Err(OptimizationProtocolError::NonCanonical(field));
        }
    }
    values.sort_unstable();
    if values.windows(2).any(|values| values[0] == values[1]) {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn normalize_label_bounds(
    bounds: &mut [LabelAllocationBounds],
) -> Result<(), OptimizationProtocolError> {
    for bound in bounds.iter_mut() {
        bound.label = bound.label.trim().to_owned();
    }
    bounds.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(())
}

fn normalize_cell_bounds(
    bounds: &mut [CellAllocationBounds],
) -> Result<(), OptimizationProtocolError> {
    for bound in bounds.iter_mut() {
        normalize_cell(&mut bound.cell)?;
    }
    bounds.sort_by_key(|bound| bound.cell.key());
    Ok(())
}

fn normalize_cell(cell: &mut GenerationCell) -> Result<(), OptimizationProtocolError> {
    cell.label = cell.label.trim().to_owned();
    let mut dimensions = BTreeMap::new();
    for (name, value) in std::mem::take(&mut cell.dimensions) {
        if dimensions
            .insert(name.trim().to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(OptimizationProtocolError::CellIdentity);
        }
    }
    cell.dimensions = dimensions;
    validate_cell(cell)
}

fn validate_canonical_strings(
    values: &[String],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    if values
        .iter()
        .any(|value| value.trim().is_empty() || value.trim() != value)
        || has_duplicates_or_unsorted(values)
    {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn validate_cells(
    cells: &[GenerationCell],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    for cell in cells {
        validate_cell(cell)?;
    }
    if cells
        .windows(2)
        .any(|cells| cells[0].key() >= cells[1].key())
    {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn validate_cell(cell: &GenerationCell) -> Result<(), OptimizationProtocolError> {
    if cell.label.trim().is_empty()
        || cell.label.trim() != cell.label
        || cell.dimensions.iter().any(|(name, value)| {
            name.trim().is_empty()
                || value.trim().is_empty()
                || name.trim() != name
                || value.trim() != value
        })
    {
        return Err(OptimizationProtocolError::CellIdentity);
    }
    Ok(())
}

fn validate_dimension_values(
    values: &[DimensionValueSelection],
    field: &'static str,
) -> Result<(), OptimizationProtocolError> {
    if values.iter().any(|selection| {
        selection.dimension.trim().is_empty()
            || selection.value.trim().is_empty()
            || selection.dimension.trim() != selection.dimension
            || selection.value.trim() != selection.value
    }) || has_duplicates_or_unsorted(values)
    {
        return Err(OptimizationProtocolError::NonCanonical(field));
    }
    Ok(())
}

fn reject_overlap<T: Ord>(
    allowed: &[T],
    excluded: &[T],
    error: OptimizationProtocolError,
) -> Result<(), OptimizationProtocolError> {
    if allowed
        .iter()
        .any(|value| excluded.binary_search(value).is_ok())
    {
        return Err(error);
    }
    Ok(())
}

fn reject_cell_overlap(
    allowed: &[GenerationCell],
    excluded: &[GenerationCell],
) -> Result<(), OptimizationProtocolError> {
    let excluded = excluded
        .iter()
        .map(GenerationCell::key)
        .collect::<BTreeSet<_>>();
    if allowed.iter().any(|cell| excluded.contains(&cell.key())) {
        return Err(OptimizationProtocolError::ContradictoryCells);
    }
    Ok(())
}

fn has_duplicates_or_unsorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).any(|values| values[0] >= values[1])
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OptimizationProtocolError {
    #[error("recommendation kinds must be non-empty, unique, and in canonical order")]
    RecommendationKinds,
    #[error(
        "the current bounded allocation protocol requires the data-generation recommendation kind"
    )]
    DataGenerationKindMissing,
    #[error("additional-example budget must be greater than zero")]
    Budget,
    #[error("minimum evidence support must be greater than zero")]
    MinimumSupport,
    #[error("Wilson z-score must be finite and inside (0, 10]")]
    WilsonZScore,
    #[error("shrinkage prior strength must be finite and inside (0, 1000000]")]
    ShrinkagePriorStrength,
    #[error("{0} must contain trimmed, unique values in canonical order")]
    NonCanonical(&'static str),
    #[error(
        "cell identities must contain a non-empty label and non-empty dimension names and values"
    )]
    CellIdentity,
    #[error("a label cannot be both allowed and excluded")]
    ContradictoryLabels,
    #[error("a cell cannot be both allowed and excluded")]
    ContradictoryCells,
    #[error("a dimension value cannot be both allowed and excluded")]
    ContradictoryDimensionValues,
    #[error("label allocation bounds must be unique and in canonical label order")]
    LabelBounds,
    #[error("cell allocation bounds must be unique and in canonical cell order")]
    CellBounds,
    #[error("allocation bounds must satisfy 0 <= minimum <= maximum <= budget")]
    AllocationBounds,
    #[error("allocation bounds overflowed the supported range")]
    BoundsOverflow,
    #[error("the sum of explicit minimum allocations exceeds the budget")]
    MinimumsExceedBudget,
    #[error("budget shares must be finite and inside the supported zero-to-one interval")]
    BudgetShare,
    #[error("an excluded label or cell cannot have a positive minimum allocation")]
    ExcludedMinimum,
    #[error("minimum allocation increment must be between one and the budget")]
    AllocationIncrement,
    #[error(
        "strict budget conservation requires the budget to be divisible by the allocation increment"
    )]
    IncrementCannotConserveBudget,
    #[error("excluded review dispositions must be unique and in canonical order")]
    ReviewDispositions,
    #[error("comparison-regression scoring requires comparison evidence to be enabled")]
    ComparisonPolicyDisabled,
    #[error("training candidate maximum must be between one and 64")]
    TrainingCandidateLimit,
    #[error("a training-candidate request requires the training-configuration recommendation kind")]
    TrainingKindMissing,
    #[error("the training-configuration recommendation kind requires a bounded candidate request")]
    TrainingRequestMissing,
    #[error("could not fingerprint optimization protocol: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use generation_core::domain::GenerationCell;

    use super::{
        CellAllocationBounds, InfeasibleAllocationBehavior, OptimizationProtocol,
        OptimizationProtocolError, RecommendationKind, RiskPolicy, ScoringPolicy,
        TrainingCandidateRequest,
    };

    #[test]
    fn normalizes_and_reproduces_protocol_fingerprint() {
        let protocol = OptimizationProtocol {
            allowed_labels: vec![" fraud ".into(), "billing".into()],
            risk_policy: RiskPolicy::WilsonLowerBound { z_score: 1.96 },
            ..OptimizationProtocol::legacy(500, 20)
        }
        .normalize()
        .expect("normalized protocol");

        assert_eq!(protocol.allowed_labels, ["billing", "fraud"]);
        assert_eq!(
            protocol.fingerprint().expect("fingerprint"),
            protocol.fingerprint().expect("repeat fingerprint")
        );
    }

    #[test]
    fn rejects_contradictory_filters_and_impossible_minimums() {
        let contradiction = OptimizationProtocol {
            allowed_labels: vec!["billing".into()],
            excluded_labels: vec!["billing".into()],
            ..OptimizationProtocol::legacy(10, 1)
        };
        assert_eq!(
            contradiction.validate(),
            Err(OptimizationProtocolError::ContradictoryLabels)
        );

        let impossible = OptimizationProtocol {
            cell_bounds: vec![
                CellAllocationBounds {
                    cell: cell("billing"),
                    minimum_addition: 6,
                    maximum_addition: None,
                },
                CellAllocationBounds {
                    cell: cell("fraud"),
                    minimum_addition: 5,
                    maximum_addition: None,
                },
            ],
            ..OptimizationProtocol::legacy(10, 1)
        }
        .normalize()
        .expect_err("minimums exceed budget");
        assert_eq!(impossible, OptimizationProtocolError::MinimumsExceedBudget);
    }

    #[test]
    fn requires_explicit_unallocated_behavior_for_indivisible_increments() {
        let strict = OptimizationProtocol {
            minimum_allocation_increment: 4,
            ..OptimizationProtocol::legacy(10, 1)
        };
        assert_eq!(
            strict.validate(),
            Err(OptimizationProtocolError::IncrementCannotConserveBudget)
        );

        OptimizationProtocol {
            infeasible_allocation_behavior: InfeasibleAllocationBehavior::AllowUnallocated,
            ..strict
        }
        .validate()
        .expect("explicitly permits an unallocated remainder");
    }

    #[test]
    fn training_candidates_are_bounded_and_explicit() {
        let missing_kind = OptimizationProtocol {
            training_candidates: Some(TrainingCandidateRequest {
                maximum_candidates: 4,
            }),
            ..OptimizationProtocol::legacy(10, 1)
        };
        assert_eq!(
            missing_kind.validate(),
            Err(OptimizationProtocolError::TrainingKindMissing)
        );

        OptimizationProtocol {
            recommendation_kinds: vec![
                RecommendationKind::DataGeneration,
                RecommendationKind::TrainingConfiguration,
            ],
            scoring_policy: ScoringPolicy::ConservativeComposite,
            training_candidates: Some(TrainingCandidateRequest {
                maximum_candidates: 4,
            }),
            ..OptimizationProtocol::legacy(10, 1)
        }
        .validate()
        .expect("bounded candidate request");
    }

    #[test]
    fn rejects_protocols_that_would_allocate_unrequested_data() {
        let protocol = OptimizationProtocol {
            recommendation_kinds: vec![RecommendationKind::ReviewOnly],
            ..OptimizationProtocol::legacy(10, 1)
        };
        assert_eq!(
            protocol.validate(),
            Err(OptimizationProtocolError::DataGenerationKindMissing)
        );
    }

    fn cell(label: &str) -> GenerationCell {
        GenerationCell {
            label: label.into(),
            dimensions: Default::default(),
        }
    }
}
