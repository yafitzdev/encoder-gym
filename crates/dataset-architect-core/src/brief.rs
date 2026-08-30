use std::collections::{BTreeMap, BTreeSet};

use analysis_core::contract::DiagnosticContract;
use chrono::{DateTime, Utc};
use generation_core::{dimensions::expand_generation_cells, domain::DatasetDefinition};
use research_core::profile::ResolvedAuthenticityContext;
use semantic_catalog::ResolvedSemanticContext;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::{
    allocation::{InitialCellConstraint, InitialCellCoverage},
    governance::{CohortDisposition, CohortRole},
};

use crate::{ArchitectError, fingerprint, required};

pub const ARCHITECT_BRIEF_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningPriority {
    pub name: String,
    pub description: String,
    /// Relative user preference from 1 through 100. It guides Pi but grants no authority.
    pub weight: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectBudgets {
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_allocation_previews: u32,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_cost_microusd: u64,
    pub max_wall_clock_seconds: u64,
}

impl ArchitectBudgets {
    pub fn validate(&self) -> Result<(), ArchitectError> {
        if self.max_model_turns == 0
            || self.max_tool_calls == 0
            || self.max_allocation_previews == 0
            || self.max_input_tokens == 0
            || self.max_output_tokens == 0
            || self.max_wall_clock_seconds == 0
        {
            return Err(ArchitectError::Validation(
                "architect budgets must be finite and positive".into(),
            ));
        }
        if self.max_model_turns > 100
            || self.max_tool_calls > 500
            || self.max_allocation_previews > 100
            || self.max_input_tokens > 10_000_000
            || self.max_output_tokens > 2_000_000
            || self.max_wall_clock_seconds > 86_400
            || self.max_cost_microusd > 100_000_000
        {
            return Err(ArchitectError::Validation(
                "architect budgets exceed local safety ceilings".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectProviderConfiguration {
    pub runtime: String,
    pub provider: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationCostModel {
    pub rows_per_request: u32,
    pub estimated_input_tokens_per_request: u64,
    pub estimated_output_tokens_per_row: u64,
    pub input_cost_microusd_per_million_tokens: Option<u64>,
    pub output_cost_microusd_per_million_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GovernedDiagnosticContext {
    pub cohort_id: Uuid,
    pub cohort_fingerprint: String,
    pub role_decision_id: Uuid,
    pub role_decision_fingerprint: String,
    pub role: CohortRole,
    pub disposition: CohortDisposition,
    pub diagnostic: DiagnosticContract,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedArchitectBrief {
    pub id: Uuid,
    pub schema_version: u32,
    pub dataset: DatasetDefinition,
    pub dataset_fingerprint: String,
    pub target_total_rows: u32,
    #[serde(default)]
    pub reserved_rows: u32,
    pub current_coverage: Vec<InitialCellCoverage>,
    #[serde(default)]
    pub constraints: Vec<InitialCellConstraint>,
    pub priorities: Vec<PlanningPriority>,
    pub semantic_context: Option<ResolvedSemanticContext>,
    pub authenticity_context: Option<ResolvedAuthenticityContext>,
    pub development_evidence: Option<GovernedDiagnosticContext>,
    pub cost_model: GenerationCostModel,
    pub budgets: ArchitectBudgets,
    pub provider: ArchitectProviderConfiguration,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[allow(clippy::too_many_arguments)]
impl ResolvedArchitectBrief {
    pub fn create(
        dataset: DatasetDefinition,
        target_total_rows: u32,
        reserved_rows: u32,
        current_coverage: Vec<InitialCellCoverage>,
        constraints: Vec<InitialCellConstraint>,
        priorities: Vec<PlanningPriority>,
        semantic_context: Option<ResolvedSemanticContext>,
        authenticity_context: Option<ResolvedAuthenticityContext>,
        development_evidence: Option<GovernedDiagnosticContext>,
        cost_model: GenerationCostModel,
        budgets: ArchitectBudgets,
        provider: ArchitectProviderConfiguration,
    ) -> Result<Self, ArchitectError> {
        if target_total_rows == 0 || reserved_rows > target_total_rows {
            return Err(ArchitectError::Validation(
                "target total must be positive and reserve cannot exceed it".into(),
            ));
        }
        budgets.validate()?;
        validate_cost_model(&cost_model)?;
        let provider = validate_provider(provider)?;
        let priorities = validate_priorities(priorities)?;
        let dataset_fingerprint = fingerprint(&dataset)?;
        validate_contexts(
            &dataset,
            &dataset_fingerprint,
            semantic_context.as_ref(),
            authenticity_context.as_ref(),
            development_evidence.as_ref(),
        )?;
        validate_cells(&dataset, &current_coverage, &constraints)?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: ARCHITECT_BRIEF_SCHEMA_VERSION,
            dataset,
            dataset_fingerprint,
            target_total_rows,
            reserved_rows,
            current_coverage,
            constraints,
            priorities,
            semantic_context,
            authenticity_context,
            development_evidence,
            cost_model,
            budgets,
            provider,
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

    pub fn current_coverage_map(&self) -> BTreeMap<String, u32> {
        self.current_coverage
            .iter()
            .map(|coverage| (coverage.cell.key(), coverage.accepted))
            .collect()
    }
}

fn validate_priorities(
    values: Vec<PlanningPriority>,
) -> Result<Vec<PlanningPriority>, ArchitectError> {
    if values.is_empty() {
        return Err(ArchitectError::Validation(
            "at least one dataset-planning priority is required".into(),
        ));
    }
    let mut names = BTreeSet::new();
    values
        .into_iter()
        .map(|value| {
            let value = PlanningPriority {
                name: required(value.name, "priority.name")?,
                description: required(value.description, "priority.description")?,
                weight: value.weight,
            };
            if value.weight == 0 || value.weight > 100 {
                return Err(ArchitectError::Validation(
                    "priority weight must be between 1 and 100".into(),
                ));
            }
            if !names.insert(value.name.clone()) {
                return Err(ArchitectError::Validation(format!(
                    "duplicate priority {:?}",
                    value.name
                )));
            }
            Ok(value)
        })
        .collect()
}

fn validate_provider(
    value: ArchitectProviderConfiguration,
) -> Result<ArchitectProviderConfiguration, ArchitectError> {
    let value = ArchitectProviderConfiguration {
        runtime: required(value.runtime, "provider.runtime")?,
        provider: required(value.provider, "provider.provider")?,
        model: required(value.model, "provider.model")?,
        api_key_env: value
            .api_key_env
            .map(|name| required(name, "provider.api_key_env"))
            .transpose()?,
    };
    if let Some(name) = &value.api_key_env
        && (name.starts_with("sk-")
            || name.contains(' ')
            || !name
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric()))
    {
        return Err(ArchitectError::Validation(
            "api_key_env must be an environment-variable name, never a secret value".into(),
        ));
    }
    Ok(value)
}

fn validate_cost_model(value: &GenerationCostModel) -> Result<(), ArchitectError> {
    if value.rows_per_request == 0 || value.estimated_output_tokens_per_row == 0 {
        return Err(ArchitectError::Validation(
            "cost model rows per request and output tokens per row must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_contexts(
    dataset: &DatasetDefinition,
    dataset_fingerprint: &str,
    semantics: Option<&ResolvedSemanticContext>,
    authenticity: Option<&ResolvedAuthenticityContext>,
    diagnostics: Option<&GovernedDiagnosticContext>,
) -> Result<(), ArchitectError> {
    if let Some(context) = semantics
        && (context.dataset_id != dataset.id
            || context
                .reproduce_fingerprint()
                .map_err(|error| ArchitectError::Integrity(error.to_string()))?
                != context.fingerprint)
    {
        return Err(ArchitectError::Integrity(
            "semantic context does not match the dataset or its fingerprint".into(),
        ));
    }
    if let Some(context) = authenticity
        && (context.dataset_id != dataset.id
            || context.dataset_fingerprint != dataset_fingerprint
            || context
                .reproduce_fingerprint()
                .map_err(|error| ArchitectError::Integrity(error.to_string()))?
                != context.fingerprint)
    {
        return Err(ArchitectError::Integrity(
            "authenticity context does not match the dataset or its fingerprint".into(),
        ));
    }
    if let Some(context) = diagnostics {
        context
            .diagnostic
            .validate(true)
            .map_err(|error| ArchitectError::Integrity(error.to_string()))?;
        if context.disposition != CohortDisposition::Active
            || !matches!(
                context.role,
                CohortRole::Development | CohortRole::Diagnostic
            )
            || context.cohort_fingerprint.trim().is_empty()
            || context.role_decision_fingerprint.trim().is_empty()
        {
            return Err(ArchitectError::Validation(
                "dataset architecture accepts only active development or diagnostic evidence"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn validate_cells(
    dataset: &DatasetDefinition,
    coverage: &[InitialCellCoverage],
    constraints: &[InitialCellConstraint],
) -> Result<(), ArchitectError> {
    let keys = expand_generation_cells(dataset)
        .into_iter()
        .map(|cell| cell.key())
        .collect::<BTreeSet<_>>();
    let mut coverage_keys = BTreeSet::new();
    for value in coverage {
        let key = value.cell.key();
        if !keys.contains(&key) || !coverage_keys.insert(key.clone()) {
            return Err(ArchitectError::Validation(format!(
                "coverage contains an unknown or duplicate cell {key}"
            )));
        }
    }
    let mut constraint_keys = BTreeSet::new();
    for value in constraints {
        let key = value.cell.key();
        if !keys.contains(&key) || !constraint_keys.insert(key.clone()) {
            return Err(ArchitectError::Validation(format!(
                "constraints contain an unknown or duplicate cell {key}"
            )));
        }
        if value
            .maximum_target
            .is_some_and(|maximum| value.minimum_target > maximum)
        {
            return Err(ArchitectError::Validation(format!(
                "constraint minimum exceeds maximum for {key}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use analysis_core::contract::{DiagnosticContract, DiagnosticSourceIdentity};
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use workflow_core::governance::{CohortDisposition, CohortRole};

    use super::*;

    pub(crate) fn dataset() -> DatasetDefinition {
        DatasetDefinition::new(
            "support",
            "Classify support messages",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("dimension"),
            ],
        )
        .expect("dataset")
    }

    pub(crate) fn brief() -> ResolvedArchitectBrief {
        ResolvedArchitectBrief::create(
            dataset(),
            40,
            0,
            vec![],
            vec![],
            vec![PlanningPriority {
                name: "robustness".into(),
                description: "Prefer class boundaries".into(),
                weight: 100,
            }],
            None,
            None,
            None,
            GenerationCostModel {
                rows_per_request: 10,
                estimated_input_tokens_per_request: 500,
                estimated_output_tokens_per_row: 50,
                input_cost_microusd_per_million_tokens: Some(100_000),
                output_cost_microusd_per_million_tokens: Some(200_000),
            },
            ArchitectBudgets {
                max_model_turns: 10,
                max_tool_calls: 30,
                max_allocation_previews: 5,
                max_input_tokens: 50_000,
                max_output_tokens: 10_000,
                max_cost_microusd: 1_000_000,
                max_wall_clock_seconds: 600,
            },
            ArchitectProviderConfiguration {
                runtime: "pi".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
            },
        )
        .expect("brief")
    }

    #[test]
    fn resolves_and_fingerprints_complete_planning_inputs() {
        let brief = brief();
        assert_eq!(brief.dataset.generation_cell_count(), 4);
        assert_eq!(brief.reproduce_fingerprint().unwrap(), brief.fingerprint);
    }

    #[test]
    fn sealed_diagnostics_are_structurally_ineligible() {
        let dataset = dataset();
        let evaluation_run_id = Uuid::new_v4();
        let source_identity = DiagnosticSourceIdentity {
            evaluation_run_id,
            evaluation_input_fingerprint: "sha256:evaluation".into(),
            evaluation_protocol_fingerprint: "sha256:protocol".into(),
            cohort_fingerprint: "sha256:cohort".into(),
            prediction_count: 10,
            comparison_id: None,
            comparison_fingerprint: None,
            legacy: false,
        };
        let mut diagnostic = DiagnosticContract {
            analysis_report_id: Uuid::new_v4(),
            analysis_fingerprint: "sha256:analysis".into(),
            analysis_protocol_fingerprint: "sha256:analysis-protocol".into(),
            source_identity,
            minimum_support: 1,
            prediction_count: 10,
            error_count: 2,
            baseline_error_rate: 0.2,
            cells: vec![],
            fingerprint: String::new(),
        };
        diagnostic.fingerprint = diagnostic.reproduce_fingerprint().unwrap();
        let result = ResolvedArchitectBrief::create(
            dataset,
            40,
            0,
            vec![],
            vec![],
            vec![PlanningPriority {
                name: "coverage".into(),
                description: "Cover cells".into(),
                weight: 10,
            }],
            None,
            None,
            Some(GovernedDiagnosticContext {
                cohort_id: Uuid::new_v4(),
                cohort_fingerprint: "sha256:cohort".into(),
                role_decision_id: Uuid::new_v4(),
                role_decision_fingerprint: "sha256:role".into(),
                role: CohortRole::SealedAcceptance,
                disposition: CohortDisposition::Active,
                diagnostic,
            }),
            GenerationCostModel {
                rows_per_request: 10,
                estimated_input_tokens_per_request: 100,
                estimated_output_tokens_per_row: 10,
                input_cost_microusd_per_million_tokens: None,
                output_cost_microusd_per_million_tokens: None,
            },
            ArchitectBudgets {
                max_model_turns: 1,
                max_tool_calls: 1,
                max_allocation_previews: 1,
                max_input_tokens: 1,
                max_output_tokens: 1,
                max_cost_microusd: 0,
                max_wall_clock_seconds: 1,
            },
            ArchitectProviderConfiguration {
                runtime: "pi".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
            },
        );
        assert!(
            matches!(result, Err(ArchitectError::Validation(message)) if message.contains("development or diagnostic"))
        );
    }
}
