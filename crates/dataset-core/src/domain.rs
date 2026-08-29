use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum DatasetError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("split ratios must be finite and non-negative")]
    InvalidSplitRatio,
    #[error("split ratios must sum to 1.0, got {0}")]
    SplitRatioSum(f64),
    #[error("source row {row_id} belongs to dataset {actual}, expected {expected}")]
    SourceDatasetMismatch {
        row_id: Uuid,
        expected: Uuid,
        actual: Uuid,
    },
    #[error("source row appears more than once: {0}")]
    DuplicateSourceRow(Uuid),
    #[error("a snapshot must contain at least one source row")]
    EmptySnapshot,
    #[error("could not fingerprint dataset artifact: {0}")]
    Fingerprint(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SplitRatios {
    pub train: f64,
    pub validation: f64,
    pub test: f64,
}

impl SplitRatios {
    pub fn new(train: f64, validation: f64, test: f64) -> Result<Self, DatasetError> {
        let values = [train, validation, test];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(DatasetError::InvalidSplitRatio);
        }
        let sum = train + validation + test;
        if (sum - 1.0).abs() > 1e-9 {
            return Err(DatasetError::SplitRatioSum(sum));
        }
        Ok(Self {
            train,
            validation,
            test,
        })
    }
}

impl Default for SplitRatios {
    fn default() -> Self {
        Self {
            train: 0.8,
            validation: 0.1,
            test: 0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SplitConfiguration {
    pub ratios: SplitRatios,
    pub seed: u64,
}

impl SplitConfiguration {
    pub fn new(ratios: SplitRatios, seed: u64) -> Self {
        Self { ratios, seed }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSplit {
    Train,
    Validation,
    Test,
}

impl SnapshotSplit {
    pub const ALL: [Self; 3] = [Self::Train, Self::Validation, Self::Test];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Validation => "validation",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRow {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub text: String,
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
    pub provenance: SourceProvenance,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceProvenance {
    Generated {
        generation_job_id: Uuid,
        backend: String,
        model: String,
    },
    Imported {
        import_id: Uuid,
        source_path: String,
        source_row_number: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    Jsonl,
    Csv,
}

impl ImportFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::Csv => "csv",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFieldMapping {
    pub text_field: String,
    pub label_field: String,
    pub dimension_fields: BTreeMap<String, String>,
}

impl ImportFieldMapping {
    pub fn new(
        text_field: impl Into<String>,
        label_field: impl Into<String>,
        dimension_fields: BTreeMap<String, String>,
    ) -> Result<Self, DatasetError> {
        let text_field = required(text_field.into(), "text field")?;
        let label_field = required(label_field.into(), "label field")?;
        let dimension_fields = dimension_fields
            .into_iter()
            .map(|(name, field)| {
                Ok((
                    required(name, "dimension name")?,
                    required(field, "dimension field")?,
                ))
            })
            .collect::<Result<_, DatasetError>>()?;
        Ok(Self {
            text_field,
            label_field,
            dimension_fields,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportState {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetImport {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub source_path: String,
    pub format: ImportFormat,
    pub mapping: ImportFieldMapping,
    pub state: ImportState,
    pub processed_rows: u64,
    pub accepted_rows: u64,
    pub rejected_rows: u64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DatasetImport {
    pub fn queued(
        dataset_id: Uuid,
        source_path: impl Into<String>,
        format: ImportFormat,
        mapping: ImportFieldMapping,
    ) -> Result<Self, DatasetError> {
        let source_path = required(source_path.into(), "source path")?;
        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4(),
            dataset_id,
            source_path,
            format,
            mapping,
            state: ImportState::Queued,
            processed_rows: 0,
            accepted_rows: 0,
            rejected_rows: 0,
            error_message: None,
            created_at: now,
            updated_at: now,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRowStatus {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedRow {
    pub id: Uuid,
    pub import_id: Uuid,
    pub dataset_id: Uuid,
    pub source_row_number: u64,
    pub text: String,
    pub normalized_text: String,
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
    pub cell_key: Option<String>,
    pub status: ImportRowStatus,
    pub issues: Vec<ImportIssue>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRejection {
    pub source_row_number: u64,
    pub issues: Vec<ImportIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub import_id: Option<Uuid>,
    pub source_path: String,
    pub dry_run: bool,
    pub processed_rows: u64,
    pub accepted_rows: u64,
    pub rejected_rows: u64,
    pub rejection_samples: Vec<ImportRejection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetSnapshot {
    pub id: Uuid,
    pub source_dataset_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub split_configuration: SplitConfiguration,
    pub member_count: u64,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl DatasetSnapshot {
    pub fn new(
        source_dataset_id: Uuid,
        name: impl Into<String>,
        description: Option<String>,
        split_configuration: SplitConfiguration,
        member_count: u64,
    ) -> Result<Self, DatasetError> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(DatasetError::EmptyField { field: "name" });
        }
        if member_count == 0 {
            return Err(DatasetError::EmptySnapshot);
        }
        let description = description.and_then(|value| {
            let trimmed = value.trim().to_owned();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        Ok(Self {
            id: Uuid::new_v4(),
            source_dataset_id,
            name,
            description,
            split_configuration,
            member_count,
            fingerprint: String::new(),
            created_at: Utc::now(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMember {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub source_row_id: Uuid,
    pub split: SnapshotSplit,
    pub text: String,
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
    pub source_provenance: SourceProvenance,
    pub source_created_at: DateTime<Utc>,
}

fn required(value: String, field: &'static str) -> Result<String, DatasetError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(DatasetError::EmptyField { field })
    } else {
        Ok(value)
    }
}
