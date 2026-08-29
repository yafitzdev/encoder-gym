use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::OptimizationProposal,
    evidence::OptimizationEvidence,
    planning::{create_constrained_proposal, verify_constrained_proposal},
    protocol::{
        InfeasibleAllocationBehavior, LabelAllocationBounds, OptimizationProtocol,
        RecommendationKind, RiskPolicy, ScoringPolicy,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioKind {
    Conservative,
    ErrorVolume,
    HighConfidenceRegression,
    BalancedPerLabel,
}

impl ScenarioKind {
    pub const DEFAULTS: [Self; 4] = [
        Self::Conservative,
        Self::ErrorVolume,
        Self::HighConfidenceRegression,
        Self::BalancedPerLabel,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioConcentration {
    pub allocated_by_label: BTreeMap<String, u32>,
    pub allocated_by_dimension_value: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationScenario {
    pub id: String,
    pub kind: ScenarioKind,
    pub protocol: OptimizationProtocol,
    pub protocol_fingerprint: String,
    pub proposal: OptimizationProposal,
    pub concentration: ScenarioConcentration,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensitiveCellAllocation {
    pub cell_key: String,
    pub left_additional_count: u32,
    pub right_additional_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioPairComparison {
    pub left_scenario_id: String,
    pub right_scenario_id: String,
    pub shared_allocated_cells: u32,
    pub union_allocated_cells: u32,
    pub allocation_overlap: f64,
    pub sensitive_cells: Vec<SensitiveCellAllocation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationScenarioGroup {
    pub id: Uuid,
    pub source_evidence_fingerprint: String,
    pub scenarios: Vec<OptimizationScenario>,
    pub comparisons: Vec<ScenarioPairComparison>,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl OptimizationScenarioGroup {
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.source_evidence_fingerprint.trim().is_empty()
            || self.scenarios.is_empty()
            || self
                .scenarios
                .windows(2)
                .any(|pair| pair[0].kind >= pair[1].kind)
        {
            return Err(ScenarioError::ScenarioIntegrity("group identity or order"));
        }
        for scenario in &self.scenarios {
            verify_constrained_proposal(&scenario.proposal)?;
            if scenario.id != scenario.fingerprint {
                return Err(ScenarioError::ScenarioIntegrity("scenario identity"));
            }
            if scenario.protocol_fingerprint != scenario.protocol.fingerprint()?
                || scenario.proposal.protocol.as_ref() != Some(&scenario.protocol)
            {
                return Err(ScenarioError::ScenarioIntegrity("scenario protocol"));
            }
            if scenario.concentration != concentration(&scenario.proposal) {
                return Err(ScenarioError::ScenarioIntegrity("scenario concentration"));
            }
            let fingerprint = artifact_core::fingerprint(&ScenarioFingerprintInput {
                kind: scenario.kind,
                source_evidence_fingerprint: &self.source_evidence_fingerprint,
                protocol_fingerprint: &scenario.protocol_fingerprint,
                proposal_fingerprint: &scenario.proposal.fingerprint,
                concentration: &scenario.concentration,
            })
            .map_err(|error| ScenarioError::Fingerprint(error.to_string()))?;
            if fingerprint != scenario.fingerprint {
                return Err(ScenarioError::ScenarioIntegrity("scenario fingerprint"));
            }
        }
        let first_source = self.scenarios[0]
            .proposal
            .source_identity
            .as_ref()
            .ok_or(ScenarioError::ScenarioIntegrity("scenario source identity"))?;
        if self.scenarios.iter().skip(1).any(|scenario| {
            scenario
                .proposal
                .source_identity
                .as_ref()
                .is_none_or(|source| !same_scenario_source(first_source, source))
        }) {
            return Err(ScenarioError::ScenarioIntegrity(
                "scenarios do not share immutable source facts",
            ));
        }
        if self.comparisons != compare_scenarios(&self.scenarios) {
            return Err(ScenarioError::ScenarioIntegrity("pairwise comparisons"));
        }
        let fingerprint = artifact_core::fingerprint(&ScenarioGroupFingerprintInput {
            source_evidence_fingerprint: &self.source_evidence_fingerprint,
            scenario_fingerprints: self
                .scenarios
                .iter()
                .map(|scenario| scenario.fingerprint.as_str())
                .collect(),
            comparisons: &self.comparisons,
        })
        .map_err(|error| ScenarioError::Fingerprint(error.to_string()))?;
        if fingerprint != self.fingerprint {
            return Err(ScenarioError::ScenarioIntegrity("group fingerprint"));
        }
        Ok(())
    }
}

fn same_scenario_source(
    left: &crate::evidence::OptimizationSourceIdentity,
    right: &crate::evidence::OptimizationSourceIdentity,
) -> bool {
    left.analysis_report_id == right.analysis_report_id
        && left.analysis_fingerprint == right.analysis_fingerprint
        && left.analysis_protocol_fingerprint == right.analysis_protocol_fingerprint
        && left.diagnostic_contract_fingerprint == right.diagnostic_contract_fingerprint
        && left.evaluation_run_id == right.evaluation_run_id
        && left.evaluation_input_fingerprint == right.evaluation_input_fingerprint
        && left.evaluation_protocol_fingerprint == right.evaluation_protocol_fingerprint
        && left.cohort_fingerprint == right.cohort_fingerprint
        && left.dataset_id == right.dataset_id
        && left.dataset_fingerprint == right.dataset_fingerprint
        && left.snapshot_id == right.snapshot_id
        && left.snapshot_fingerprint == right.snapshot_fingerprint
        && left.coverage_fingerprint == right.coverage_fingerprint
        && left.comparison_id == right.comparison_id
        && left.comparison_fingerprint == right.comparison_fingerprint
        && left.training_configuration_space_fingerprint
            == right.training_configuration_space_fingerprint
}

pub fn create_default_scenario_group(
    evidence: &OptimizationEvidence,
    base_protocol: &OptimizationProtocol,
) -> Result<OptimizationScenarioGroup, ScenarioError> {
    create_scenario_group(evidence, base_protocol, &ScenarioKind::DEFAULTS)
}

pub fn create_scenario_group(
    evidence: &OptimizationEvidence,
    base_protocol: &OptimizationProtocol,
    kinds: &[ScenarioKind],
) -> Result<OptimizationScenarioGroup, ScenarioError> {
    base_protocol.validate()?;
    if kinds.is_empty() || kinds.windows(2).any(|values| values[0] >= values[1]) {
        return Err(ScenarioError::ScenarioKinds);
    }
    let labels = evidence
        .current_coverage
        .iter()
        .map(|coverage| coverage.cell.label.clone())
        .collect::<BTreeSet<_>>();
    if labels.is_empty() {
        return Err(ScenarioError::NoLabels);
    }
    let mut scenarios = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let protocol = scenario_protocol(base_protocol, *kind, &labels)?;
        let rebound = evidence.rebind_protocol(&protocol, None)?;
        let proposal = create_constrained_proposal(&rebound, &protocol)?;
        let concentration = concentration(&proposal);
        let protocol_fingerprint = protocol.fingerprint()?;
        let fingerprint = artifact_core::fingerprint(&ScenarioFingerprintInput {
            kind: *kind,
            source_evidence_fingerprint: &evidence.fingerprint,
            protocol_fingerprint: &protocol_fingerprint,
            proposal_fingerprint: &proposal.fingerprint,
            concentration: &concentration,
        })
        .map_err(|error| ScenarioError::Fingerprint(error.to_string()))?;
        scenarios.push(OptimizationScenario {
            id: fingerprint.clone(),
            kind: *kind,
            protocol,
            protocol_fingerprint,
            proposal,
            concentration,
            fingerprint,
        });
    }
    let comparisons = compare_scenarios(&scenarios);
    let fingerprint = artifact_core::fingerprint(&ScenarioGroupFingerprintInput {
        source_evidence_fingerprint: &evidence.fingerprint,
        scenario_fingerprints: scenarios
            .iter()
            .map(|scenario| scenario.fingerprint.as_str())
            .collect::<Vec<_>>(),
        comparisons: &comparisons,
    })
    .map_err(|error| ScenarioError::Fingerprint(error.to_string()))?;
    Ok(OptimizationScenarioGroup {
        id: Uuid::new_v4(),
        source_evidence_fingerprint: evidence.fingerprint.clone(),
        scenarios,
        comparisons,
        fingerprint,
        created_at: Utc::now(),
    })
}

pub fn compare_scenarios(scenarios: &[OptimizationScenario]) -> Vec<ScenarioPairComparison> {
    let mut comparisons = Vec::new();
    for left_index in 0..scenarios.len() {
        for right_index in left_index + 1..scenarios.len() {
            let left = &scenarios[left_index];
            let right = &scenarios[right_index];
            let left_allocations = allocations(left);
            let right_allocations = allocations(right);
            let left_keys = left_allocations.keys().collect::<BTreeSet<_>>();
            let right_keys = right_allocations.keys().collect::<BTreeSet<_>>();
            let shared = left_keys.intersection(&right_keys).count() as u32;
            let union = left_keys.union(&right_keys).count() as u32;
            let allocation_overlap = if union == 0 {
                1.0
            } else {
                round(f64::from(shared) / f64::from(union))
            };
            let mut sensitive_cells = left_allocations
                .keys()
                .chain(right_allocations.keys())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|cell_key| {
                    let left_count = left_allocations.get(&cell_key).copied().unwrap_or(0);
                    let right_count = right_allocations.get(&cell_key).copied().unwrap_or(0);
                    (left_count != right_count).then_some(SensitiveCellAllocation {
                        cell_key,
                        left_additional_count: left_count,
                        right_additional_count: right_count,
                    })
                })
                .collect::<Vec<_>>();
            sensitive_cells.sort_by(|left, right| left.cell_key.cmp(&right.cell_key));
            comparisons.push(ScenarioPairComparison {
                left_scenario_id: left.id.clone(),
                right_scenario_id: right.id.clone(),
                shared_allocated_cells: shared,
                union_allocated_cells: union,
                allocation_overlap,
                sensitive_cells,
            });
        }
    }
    comparisons
}

fn scenario_protocol(
    base: &OptimizationProtocol,
    kind: ScenarioKind,
    labels: &BTreeSet<String>,
) -> Result<OptimizationProtocol, ScenarioError> {
    let mut protocol = base.clone();
    protocol.recommendation_kinds = vec![RecommendationKind::DataGeneration];
    protocol.training_candidates = None;
    protocol.infeasible_allocation_behavior = InfeasibleAllocationBehavior::AllowUnallocated;
    match kind {
        ScenarioKind::Conservative => {
            protocol.scoring_policy = ScoringPolicy::ConservativeComposite;
            protocol.risk_policy = RiskPolicy::WilsonLowerBound { z_score: 1.96 };
        }
        ScenarioKind::ErrorVolume => {
            protocol.scoring_policy = ScoringPolicy::ErrorCount;
            protocol.risk_policy = RiskPolicy::None;
            protocol.use_comparison_regression_evidence = false;
        }
        ScenarioKind::HighConfidenceRegression => {
            protocol.scoring_policy = if protocol.use_comparison_regression_evidence {
                ScoringPolicy::ComparisonRegression
            } else {
                ScoringPolicy::HighConfidenceErrorSeverity
            };
            protocol.risk_policy = RiskPolicy::WilsonLowerBound { z_score: 1.96 };
        }
        ScenarioKind::BalancedPerLabel => {
            protocol.scoring_policy = ScoringPolicy::ConservativeComposite;
            protocol.risk_policy = RiskPolicy::WilsonLowerBound { z_score: 1.96 };
            let budget = f64::from(protocol.additional_example_budget);
            let label_cap = (budget / labels.len() as f64).ceil() / budget;
            let existing = protocol
                .label_bounds
                .iter()
                .map(|bounds| (bounds.label.clone(), bounds.clone()))
                .collect::<BTreeMap<_, _>>();
            protocol.label_bounds = labels
                .iter()
                .map(|label| {
                    let mut bounds =
                        existing
                            .get(label)
                            .cloned()
                            .unwrap_or(LabelAllocationBounds {
                                label: label.clone(),
                                minimum_addition: 0,
                                maximum_addition: None,
                                minimum_budget_share: None,
                                maximum_budget_share: None,
                            });
                    bounds.maximum_budget_share = Some(
                        bounds
                            .maximum_budget_share
                            .map_or(label_cap, |maximum| maximum.min(label_cap)),
                    );
                    bounds
                })
                .collect();
        }
    }
    protocol.normalize().map_err(ScenarioError::from)
}

fn concentration(proposal: &OptimizationProposal) -> ScenarioConcentration {
    let mut allocated_by_label = BTreeMap::new();
    let mut allocated_by_dimension_value = BTreeMap::new();
    for recommendation in &proposal.normalized_recommendations {
        *allocated_by_label
            .entry(recommendation.cell.label.clone())
            .or_insert(0_u32) += recommendation.additional_count;
        for (dimension, value) in &recommendation.cell.dimensions {
            let key = serde_json::to_string(&(dimension, value))
                .expect("dimension concentration key serialization cannot fail");
            *allocated_by_dimension_value.entry(key).or_insert(0_u32) +=
                recommendation.additional_count;
        }
    }
    ScenarioConcentration {
        allocated_by_label,
        allocated_by_dimension_value,
    }
}

fn allocations(scenario: &OptimizationScenario) -> BTreeMap<String, u32> {
    scenario
        .proposal
        .normalized_recommendations
        .iter()
        .map(|recommendation| (recommendation.cell.key(), recommendation.additional_count))
        .collect()
}

fn round(value: f64) -> f64 {
    (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
}

#[derive(Serialize)]
struct ScenarioFingerprintInput<'a> {
    kind: ScenarioKind,
    source_evidence_fingerprint: &'a str,
    protocol_fingerprint: &'a str,
    proposal_fingerprint: &'a str,
    concentration: &'a ScenarioConcentration,
}

#[derive(Serialize)]
struct ScenarioGroupFingerprintInput<'a> {
    source_evidence_fingerprint: &'a str,
    scenario_fingerprints: Vec<&'a str>,
    comparisons: &'a [ScenarioPairComparison],
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error(transparent)]
    Protocol(#[from] crate::protocol::OptimizationProtocolError),
    #[error(transparent)]
    Evidence(#[from] crate::evidence::OptimizationEvidenceError),
    #[error(transparent)]
    Proposal(#[from] crate::planning::OptimizationError),
    #[error("scenario kinds must be non-empty, unique, and in canonical order")]
    ScenarioKinds,
    #[error("scenario generation requires at least one dataset label")]
    NoLabels,
    #[error("optimization scenario group failed its immutable integrity check: {0}")]
    ScenarioIntegrity(&'static str),
    #[error("could not fingerprint optimization scenario: {0}")]
    Fingerprint(String),
}
