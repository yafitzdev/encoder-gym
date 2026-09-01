use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use research_core::brief::SourcePolicy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::{
    benchmark::BenchmarkSuiteKind,
    governance::{AdaptiveRiskLevel, CohortRole, DisclosureLevel, ExposureRiskSummary},
};

use crate::{BenchmarkArchitectError, canonical_fingerprint, fingerprint, required};

pub const BENCHMARK_ARCHITECT_BRIEF_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelSemantic {
    pub label: String,
    pub meaning: String,
    #[serde(default)]
    pub inclusions: Vec<String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentContext {
    pub summary: String,
    #[serde(default)]
    pub users: Vec<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub time_horizon: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentRisk {
    pub key: String,
    pub description: String,
    pub consequence: String,
    pub likelihood: RiskLevel,
    pub severity: RiskLevel,
    /// Relative operator preference from 1 through 100.
    pub weight: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkObjective {
    pub key: String,
    pub description: String,
    pub weight: u8,
}

/// Row-free summary of a candidate or current benchmark cohort. This is the
/// richest cohort shape the agent is allowed to inspect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateCohortSummary {
    pub key: String,
    #[serde(default)]
    pub cohort_id: Option<Uuid>,
    #[serde(default)]
    pub cohort_fingerprint: Option<String>,
    pub suite_kind: BenchmarkSuiteKind,
    pub role: CohortRole,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub total_support: u64,
    pub label_support: BTreeMap<String, u64>,
    #[serde(default)]
    pub required_slice_support: BTreeMap<String, u64>,
    #[serde(default)]
    pub dimension_value_support: BTreeMap<String, BTreeMap<String, u64>>,
    pub distinct_producers: u64,
    pub maximum_single_producer_rows: u64,
    pub normalized_duplicate_rows: u64,
    pub reference_distribution_bound: bool,
    #[serde(default)]
    pub exposure: Option<ExposureRiskSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingBenchmarkSummary {
    pub bundle_id: Uuid,
    pub bundle_fingerprint: String,
    pub qualification_id: Uuid,
    pub qualification_fingerprint: String,
    pub cohorts: Vec<AggregateCohortSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkArchitectBudgets {
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_searches: u32,
    pub max_fetched_pages: u32,
    pub max_fetched_bytes: u64,
    pub max_blueprint_previews: u32,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_cost_microusd: u64,
    pub max_wall_clock_seconds: u64,
}

impl BenchmarkArchitectBudgets {
    pub fn validate(&self) -> Result<(), BenchmarkArchitectError> {
        let bounded = [
            ("max_model_turns", u64::from(self.max_model_turns), 128),
            ("max_tool_calls", u64::from(self.max_tool_calls), 1_024),
            ("max_searches", u64::from(self.max_searches), 256),
            (
                "max_fetched_pages",
                u64::from(self.max_fetched_pages),
                1_024,
            ),
            ("max_fetched_bytes", self.max_fetched_bytes, 268_435_456),
            (
                "max_blueprint_previews",
                u64::from(self.max_blueprint_previews),
                128,
            ),
            ("max_input_tokens", self.max_input_tokens, 10_000_000),
            ("max_output_tokens", self.max_output_tokens, 2_000_000),
            (
                "max_wall_clock_seconds",
                self.max_wall_clock_seconds,
                86_400,
            ),
        ];
        for (name, value, ceiling) in bounded {
            if value == 0 || value > ceiling {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "budgets.{name} must be in 1..={ceiling}"
                )));
            }
        }
        if self.max_cost_microusd > 100_000_000 {
            return Err(BenchmarkArchitectError::Validation(
                "budgets.max_cost_microusd exceeds 100000000".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkArchitectProvider {
    pub runtime: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkArchitectBriefDraft {
    pub task: String,
    pub labels: Vec<String>,
    pub label_semantics: Vec<LabelSemantic>,
    pub deployment: DeploymentContext,
    pub risks: Vec<DeploymentRisk>,
    pub objectives: Vec<BenchmarkObjective>,
    #[serde(default)]
    pub candidate_sources: Vec<AggregateCohortSummary>,
    #[serde(default)]
    pub existing_benchmark: Option<ExistingBenchmarkSummary>,
    #[serde(default)]
    pub source_policy: SourcePolicy,
    pub budgets: BenchmarkArchitectBudgets,
    pub provider: BenchmarkArchitectProvider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBenchmarkArchitectBrief {
    pub id: Uuid,
    pub schema_version: u32,
    pub task: String,
    pub labels: Vec<String>,
    pub label_semantics: Vec<LabelSemantic>,
    pub deployment: DeploymentContext,
    pub risks: Vec<DeploymentRisk>,
    pub objectives: Vec<BenchmarkObjective>,
    pub candidate_sources: Vec<AggregateCohortSummary>,
    pub existing_benchmark: Option<ExistingBenchmarkSummary>,
    pub source_policy: SourcePolicy,
    pub budgets: BenchmarkArchitectBudgets,
    pub provider: BenchmarkArchitectProvider,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ResolvedBenchmarkArchitectBrief {
    pub fn create(draft: BenchmarkArchitectBriefDraft) -> Result<Self, BenchmarkArchitectError> {
        draft.budgets.validate()?;
        let labels = normalized_unique(draft.labels, "labels")?;
        if labels.len() < 2 {
            return Err(BenchmarkArchitectError::Validation(
                "at least two labels are required".into(),
            ));
        }
        let task = required(draft.task, "task")?;
        let label_semantics = normalize_semantics(draft.label_semantics, &labels)?;
        let deployment = normalize_deployment(draft.deployment)?;
        let risks = normalize_risks(draft.risks)?;
        let objectives = normalize_objectives(draft.objectives)?;
        let candidate_sources = normalize_cohorts(draft.candidate_sources, &labels)?;
        let existing_benchmark = draft
            .existing_benchmark
            .map(|value| normalize_existing(value, &labels))
            .transpose()?;
        let provider = normalize_provider(draft.provider)?;
        let source_policy = draft
            .source_policy
            .normalize()
            .map_err(|error| BenchmarkArchitectError::Validation(error.to_string()))?;
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: BENCHMARK_ARCHITECT_BRIEF_SCHEMA_VERSION,
            task,
            labels,
            label_semantics,
            deployment,
            risks,
            objectives,
            candidate_sources,
            existing_benchmark,
            source_policy,
            budgets: draft.budgets,
            provider,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn normalize_semantics(
    values: Vec<LabelSemantic>,
    labels: &[String],
) -> Result<Vec<LabelSemantic>, BenchmarkArchitectError> {
    if values.len() != labels.len() {
        return Err(BenchmarkArchitectError::Validation(
            "label_semantics must describe every label exactly once".into(),
        ));
    }
    let mut by_label = BTreeMap::new();
    for value in values {
        let label = required(value.label, "label_semantics.label")?;
        if !labels.contains(&label) || by_label.contains_key(&label) {
            return Err(BenchmarkArchitectError::Validation(format!(
                "unknown or duplicate label semantic {label:?}"
            )));
        }
        by_label.insert(
            label.clone(),
            LabelSemantic {
                label,
                meaning: required(value.meaning, "label_semantics.meaning")?,
                inclusions: normalized_unique(value.inclusions, "label_semantics.inclusions")?,
                exclusions: normalized_unique(value.exclusions, "label_semantics.exclusions")?,
            },
        );
    }
    labels
        .iter()
        .map(|label| {
            by_label.remove(label).ok_or_else(|| {
                BenchmarkArchitectError::Validation(format!("missing label semantic for {label:?}"))
            })
        })
        .collect()
}

fn normalize_deployment(
    value: DeploymentContext,
) -> Result<DeploymentContext, BenchmarkArchitectError> {
    Ok(DeploymentContext {
        summary: required(value.summary, "deployment.summary")?,
        users: normalized_unique(value.users, "deployment.users")?,
        channels: normalized_unique(value.channels, "deployment.channels")?,
        languages: normalized_unique(value.languages, "deployment.languages")?,
        regions: normalized_unique(value.regions, "deployment.regions")?,
        time_horizon: value
            .time_horizon
            .map(|text| required(text, "deployment.time_horizon"))
            .transpose()?,
        constraints: normalized_unique(value.constraints, "deployment.constraints")?,
    })
}

fn normalize_risks(
    values: Vec<DeploymentRisk>,
) -> Result<Vec<DeploymentRisk>, BenchmarkArchitectError> {
    if values.is_empty() {
        return Err(BenchmarkArchitectError::Validation(
            "at least one deployment risk is required".into(),
        ));
    }
    let mut keys = BTreeSet::new();
    values
        .into_iter()
        .map(|value| {
            let key = required(value.key, "risks.key")?;
            if !keys.insert(key.clone()) || value.weight == 0 || value.weight > 100 {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "risk key {key:?} must be unique and weight must be in 1..=100"
                )));
            }
            Ok(DeploymentRisk {
                key,
                description: required(value.description, "risks.description")?,
                consequence: required(value.consequence, "risks.consequence")?,
                ..value
            })
        })
        .collect()
}

fn normalize_objectives(
    values: Vec<BenchmarkObjective>,
) -> Result<Vec<BenchmarkObjective>, BenchmarkArchitectError> {
    if values.is_empty() {
        return Err(BenchmarkArchitectError::Validation(
            "at least one benchmark objective is required".into(),
        ));
    }
    let mut keys = BTreeSet::new();
    values
        .into_iter()
        .map(|value| {
            let key = required(value.key, "objectives.key")?;
            if !keys.insert(key.clone()) || value.weight == 0 || value.weight > 100 {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "objective key {key:?} must be unique and weight must be in 1..=100"
                )));
            }
            Ok(BenchmarkObjective {
                key,
                description: required(value.description, "objectives.description")?,
                weight: value.weight,
            })
        })
        .collect()
}

fn normalize_cohorts(
    values: Vec<AggregateCohortSummary>,
    labels: &[String],
) -> Result<Vec<AggregateCohortSummary>, BenchmarkArchitectError> {
    let mut keys = BTreeSet::new();
    values
        .into_iter()
        .map(|mut value| {
            value.key = required(value.key, "cohort.key")?;
            if !keys.insert(value.key.clone()) {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "duplicate cohort summary {:?}",
                    value.key
                )));
            }
            if value.cohort_id.is_some() != value.cohort_fingerprint.is_some()
                || value
                    .cohort_fingerprint
                    .as_deref()
                    .is_some_and(|fingerprint| !canonical_fingerprint(fingerprint))
                || value
                    .label_support
                    .keys()
                    .any(|label| !labels.contains(label))
                || value.label_support.values().sum::<u64>() > value.total_support
                || value.maximum_single_producer_rows > value.total_support
                || value.normalized_duplicate_rows > value.total_support
            {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "aggregate cohort summary {:?} is inconsistent",
                    value.key
                )));
            }
            if value.suite_kind == BenchmarkSuiteKind::SealedAcceptance
                && (value.disclosure != DisclosureLevel::Aggregate
                    || value.adaptation_eligible
                    || value
                        .exposure
                        .as_ref()
                        .is_some_and(|risk| risk.adaptive_exposures > 0))
            {
                return Err(BenchmarkArchitectError::Validation(format!(
                    "sealed cohort summary {:?} exposes adaptive evidence",
                    value.key
                )));
            }
            if let Some(exposure) = &value.exposure {
                if value.cohort_id != Some(exposure.cohort_id)
                    || matches!(exposure.risk, AdaptiveRiskLevel::Compromised)
                        && value.suite_kind == BenchmarkSuiteKind::SealedAcceptance
                {
                    return Err(BenchmarkArchitectError::Validation(format!(
                        "cohort exposure summary {:?} does not match safe authority",
                        value.key
                    )));
                }
            }
            Ok(value)
        })
        .collect()
}

fn normalize_existing(
    mut value: ExistingBenchmarkSummary,
    labels: &[String],
) -> Result<ExistingBenchmarkSummary, BenchmarkArchitectError> {
    if value.bundle_id.is_nil()
        || value.qualification_id.is_nil()
        || !canonical_fingerprint(&value.bundle_fingerprint)
        || !canonical_fingerprint(&value.qualification_fingerprint)
    {
        return Err(BenchmarkArchitectError::Validation(
            "existing benchmark authority pins are malformed".into(),
        ));
    }
    value.cohorts = normalize_cohorts(value.cohorts, labels)?;
    if value.cohorts.is_empty() {
        return Err(BenchmarkArchitectError::Validation(
            "existing benchmark summary requires aggregate cohorts".into(),
        ));
    }
    Ok(value)
}

fn normalize_provider(
    value: BenchmarkArchitectProvider,
) -> Result<BenchmarkArchitectProvider, BenchmarkArchitectError> {
    let runtime = required(value.runtime, "provider.runtime")?.to_ascii_lowercase();
    if runtime != "pi" {
        return Err(BenchmarkArchitectError::Validation(
            "provider.runtime must be \"pi\"".into(),
        ));
    }
    let api_key_env = value
        .api_key_env
        .map(|name| required(name, "provider.api_key_env"))
        .transpose()?;
    if api_key_env.as_ref().is_some_and(|name| {
        !name.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    }) {
        return Err(BenchmarkArchitectError::Validation(
            "provider.api_key_env must name an uppercase environment variable".into(),
        ));
    }
    Ok(BenchmarkArchitectProvider {
        runtime,
        provider: required(value.provider, "provider.provider")?,
        model: required(value.model, "provider.model")?,
        api_key_env,
    })
}

fn normalized_unique(
    values: Vec<String>,
    field: &str,
) -> Result<Vec<String>, BenchmarkArchitectError> {
    let mut result = BTreeSet::new();
    for value in values {
        let value = required(value, field)?;
        if !result.insert(value.clone()) {
            return Err(BenchmarkArchitectError::Validation(format!(
                "{field} contains duplicate {value:?}"
            )));
        }
    }
    Ok(result.into_iter().collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn draft() -> BenchmarkArchitectBriefDraft {
        BenchmarkArchitectBriefDraft {
            task: "Classify support messages".into(),
            labels: vec!["billing".into(), "fraud".into()],
            label_semantics: vec![
                LabelSemantic {
                    label: "billing".into(),
                    meaning: "Charges and invoices".into(),
                    inclusions: vec![],
                    exclusions: vec![],
                },
                LabelSemantic {
                    label: "fraud".into(),
                    meaning: "Unauthorized activity".into(),
                    inclusions: vec![],
                    exclusions: vec![],
                },
            ],
            deployment: DeploymentContext {
                summary: "Consumer support intake".into(),
                users: vec!["customers".into()],
                channels: vec!["chat".into()],
                languages: vec!["English".into()],
                regions: vec![],
                time_horizon: Some("next 12 months".into()),
                constraints: vec![],
            },
            risks: vec![DeploymentRisk {
                key: "fraud_as_billing".into(),
                description: "Fraud is mistaken for billing".into(),
                consequence: "Delayed account protection".into(),
                likelihood: RiskLevel::Medium,
                severity: RiskLevel::Critical,
                weight: 100,
            }],
            objectives: vec![BenchmarkObjective {
                key: "boundary_recall".into(),
                description: "Measure the fraud/billing boundary".into(),
                weight: 100,
            }],
            candidate_sources: vec![],
            existing_benchmark: None,
            source_policy: SourcePolicy::default(),
            budgets: BenchmarkArchitectBudgets {
                max_model_turns: 10,
                max_tool_calls: 50,
                max_searches: 5,
                max_fetched_pages: 10,
                max_fetched_bytes: 100_000,
                max_blueprint_previews: 5,
                max_input_tokens: 100_000,
                max_output_tokens: 20_000,
                max_cost_microusd: 1_000_000,
                max_wall_clock_seconds: 600,
            },
            provider: BenchmarkArchitectProvider {
                runtime: "pi".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
            },
        }
    }

    pub(crate) fn brief() -> ResolvedBenchmarkArchitectBrief {
        ResolvedBenchmarkArchitectBrief::create(draft()).expect("brief")
    }

    #[test]
    fn brief_is_row_free_and_reproducible() {
        let brief = brief();
        let json = serde_json::to_string(&brief).expect("json");
        assert!(!json.contains("row_text"));
        assert!(!json.contains("predictions"));
        assert_eq!(brief.reproduce_fingerprint().unwrap(), brief.fingerprint);
    }

    #[test]
    fn raw_secret_cannot_masquerade_as_environment_name() {
        let mut value = draft();
        value.provider.api_key_env = Some("sk-secret".into());
        assert!(ResolvedBenchmarkArchitectBrief::create(value).is_err());
    }
}
