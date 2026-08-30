//! Domain objects and invariants for Slice 1.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("{field} contains a duplicate value: {value}")]
    DuplicateValue { field: &'static str, value: String },
    #[error("dimension name is reserved: {0}")]
    ReservedDimensionName(String),
    #[error("generation plan contains a cell that is not part of the dataset: {0}")]
    UnknownCell(String),
    #[error("generation plan contains the same cell more than once: {0}")]
    DuplicateCell(String),
    #[error("dataset defines {count} generation cells; maximum supported is {maximum}")]
    TooManyGenerationCells { count: u64, maximum: u64 },
    #[error("dataset generation-cell count overflowed")]
    GenerationCellCountOverflow,
}

/// Prevents accidental Cartesian explosions from exhausting a local process.
pub const MAX_GENERATION_CELLS: u64 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionDefinition {
    pub name: String,
    pub values: Vec<String>,
}

impl DimensionDefinition {
    pub fn new(name: impl Into<String>, values: Vec<String>) -> Result<Self, DomainError> {
        let name = clean_required(name.into(), "dimension name")?;
        if matches!(name.as_str(), "label" | "text" | "dimensions") {
            return Err(DomainError::ReservedDimensionName(name));
        }

        let values = clean_unique(values, "dimension values")?;
        if values.is_empty() {
            return Err(DomainError::EmptyField {
                field: "dimension values",
            });
        }

        Ok(Self { name, values })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetDefinition {
    pub id: Uuid,
    pub name: String,
    pub task_description: String,
    pub labels: Vec<String>,
    pub dimensions: Vec<DimensionDefinition>,
    pub created_at: DateTime<Utc>,
}

impl DatasetDefinition {
    pub fn new(
        name: impl Into<String>,
        task_description: impl Into<String>,
        labels: Vec<String>,
        dimensions: Vec<DimensionDefinition>,
    ) -> Result<Self, DomainError> {
        Self::with_identity(
            Uuid::new_v4(),
            name,
            task_description,
            labels,
            dimensions,
            Utc::now(),
        )
    }

    pub fn with_identity(
        id: Uuid,
        name: impl Into<String>,
        task_description: impl Into<String>,
        labels: Vec<String>,
        dimensions: Vec<DimensionDefinition>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = clean_required(name.into(), "dataset name")?;
        let task_description = clean_required(task_description.into(), "task description")?;
        let labels = clean_unique(labels, "labels")?;
        if labels.is_empty() {
            return Err(DomainError::EmptyField { field: "labels" });
        }

        let dimensions = dimensions
            .into_iter()
            .map(|dimension| DimensionDefinition::new(dimension.name, dimension.values))
            .collect::<Result<Vec<_>, _>>()?;
        let mut dimension_names = BTreeSet::new();
        for dimension in &dimensions {
            if !dimension_names.insert(dimension.name.clone()) {
                return Err(DomainError::DuplicateValue {
                    field: "dimension names",
                    value: dimension.name.clone(),
                });
            }
        }
        let cell_count = dimensions.iter().try_fold(
            u64::try_from(labels.len()).map_err(|_| DomainError::GenerationCellCountOverflow)?,
            |count, dimension| {
                count
                    .checked_mul(
                        u64::try_from(dimension.values.len())
                            .map_err(|_| DomainError::GenerationCellCountOverflow)?,
                    )
                    .ok_or(DomainError::GenerationCellCountOverflow)
            },
        )?;
        if cell_count > MAX_GENERATION_CELLS {
            return Err(DomainError::TooManyGenerationCells {
                count: cell_count,
                maximum: MAX_GENERATION_CELLS,
            });
        }

        Ok(Self {
            id,
            name,
            task_description,
            labels,
            dimensions,
            created_at,
        })
    }

    pub fn generation_cell_count(&self) -> u64 {
        self.dimensions
            .iter()
            .fold(self.labels.len() as u64, |count, dimension| {
                count * dimension.values.len() as u64
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GenerationCell {
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
}

impl GenerationCell {
    pub fn key(&self) -> String {
        let mut key = length_prefixed(&self.label);
        for (name, value) in &self.dimensions {
            key.push('|');
            key.push_str(&length_prefixed(name));
            key.push('=');
            key.push_str(&length_prefixed(value));
        }
        key
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedCell {
    pub cell: GenerationCell,
    pub target_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationPlan {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub cells: Vec<PlannedCell>,
    pub created_at: DateTime<Utc>,
}

impl GenerationPlan {
    pub fn new(dataset_id: Uuid, cells: Vec<PlannedCell>) -> Result<Self, DomainError> {
        Self::with_identity(Uuid::new_v4(), dataset_id, cells, Utc::now())
    }

    pub fn with_identity(
        id: Uuid,
        dataset_id: Uuid,
        cells: Vec<PlannedCell>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let mut keys = BTreeSet::new();
        for planned in &cells {
            let key = planned.cell.key();
            if !keys.insert(key.clone()) {
                return Err(DomainError::DuplicateCell(key));
            }
        }
        Ok(Self {
            id,
            dataset_id,
            cells,
            created_at,
        })
    }

    pub fn total_target_count(&self) -> u64 {
        self.cells
            .iter()
            .map(|planned| u64::from(planned.target_count))
            .sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParameters {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub target: GenerationCell,
    pub requested_count: u32,
    pub parameters: GenerationParameters,
    #[serde(default)]
    pub construction: Option<crate::construction::PreparedConstructionBatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedCandidate {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub construction: Option<crate::construction::RowConstructionTrace>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageMetadata {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationResult {
    pub rows: Vec<GeneratedCandidate>,
    pub usage: Option<UsageMetadata>,
    #[serde(default)]
    pub backend_metadata: Value,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendConfiguration {
    pub name: String,
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub parameters: GenerationParameters,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedRow {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub plan_id: Uuid,
    pub generation_job_id: Uuid,
    pub cell_key: String,
    pub text: String,
    pub normalized_text: String,
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub construction: Option<crate::construction::RowConstructionTrace>,
    pub generator_backend: String,
    pub generator_model: String,
    pub created_at: DateTime<Utc>,
    pub validation_status: ValidationStatus,
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub generation_metadata: Value,
}

fn clean_required(value: String, field: &'static str) -> Result<String, DomainError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(DomainError::EmptyField { field })
    } else {
        Ok(value)
    }
}

fn clean_unique(values: Vec<String>, field: &'static str) -> Result<Vec<String>, DomainError> {
    let mut cleaned = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = clean_required(value, field)?;
        if !seen.insert(value.clone()) {
            return Err(DomainError::DuplicateValue { field, value });
        }
        cleaned.push(value);
    }
    Ok(cleaned)
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{value}", value.len())
}

#[cfg(test)]
mod tests {
    use super::{DatasetDefinition, DimensionDefinition, DomainError, GenerationCell};
    use std::collections::BTreeMap;

    #[test]
    fn dataset_rejects_duplicate_labels_after_trimming() {
        let result = DatasetDefinition::new(
            "support",
            "Classify support requests",
            vec!["billing".into(), " billing ".into()],
            vec![],
        );

        assert_eq!(
            result,
            Err(DomainError::DuplicateValue {
                field: "labels",
                value: "billing".into()
            })
        );
    }

    #[test]
    fn dataset_rejects_accidental_cartesian_explosions() {
        let dimensions = (0..6)
            .map(|index| {
                DimensionDefinition::new(
                    format!("dimension_{index}"),
                    (0..10).map(|value| format!("value_{value}")).collect(),
                )
                .expect("dimension")
            })
            .collect();
        assert!(matches!(
            DatasetDefinition::new("huge", "classify", vec!["label".into()], dimensions),
            Err(super::DomainError::TooManyGenerationCells { .. })
        ));
    }

    #[test]
    fn dimension_rejects_reserved_names() {
        let result = DimensionDefinition::new("label", vec!["x".into()]);
        assert_eq!(
            result,
            Err(DomainError::ReservedDimensionName("label".into()))
        );
    }

    #[test]
    fn cell_keys_do_not_collide_on_delimiters() {
        let first = GenerationCell {
            label: "a|b".into(),
            dimensions: BTreeMap::from([("c".into(), "d".into())]),
        };
        let second = GenerationCell {
            label: "a".into(),
            dimensions: BTreeMap::from([("b|c".into(), "d".into())]),
        };
        assert_ne!(first.key(), second.key());
    }
}
