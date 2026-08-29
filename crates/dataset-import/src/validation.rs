use std::collections::{BTreeMap, HashSet};

use chrono::Utc;
use dataset_core::domain::{ImportIssue, ImportRowStatus, ImportedRow};
use generation_core::{
    deduplication::normalize_text,
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, GenerationCell},
};
use thiserror::Error;
use uuid::Uuid;

use crate::MappedRecord;

pub trait ImportValidator: Send {
    fn validate(&mut self, dataset: &DatasetDefinition, record: &MappedRecord) -> Vec<ImportIssue>;
}

pub struct ValidationPipeline {
    validators: Vec<Box<dyn ImportValidator>>,
}

impl ValidationPipeline {
    pub fn new(validators: Vec<Box<dyn ImportValidator>>) -> Self {
        Self { validators }
    }

    pub fn standard(existing_normalized: impl IntoIterator<Item = String>) -> Self {
        Self::new(vec![
            Box::new(NonEmptyTextValidator),
            Box::new(LabelValidator),
            Box::new(DimensionValidator),
            Box::new(NormalizedDuplicateValidator::new(existing_normalized)),
        ])
    }

    fn validate(&mut self, dataset: &DatasetDefinition, record: &MappedRecord) -> Vec<ImportIssue> {
        self.validators
            .iter_mut()
            .flat_map(|validator| validator.validate(dataset, record))
            .collect()
    }
}

pub struct NonEmptyTextValidator;

impl ImportValidator for NonEmptyTextValidator {
    fn validate(
        &mut self,
        _dataset: &DatasetDefinition,
        record: &MappedRecord,
    ) -> Vec<ImportIssue> {
        if record.text.trim().is_empty() {
            vec![issue("empty_text", "text must not be empty")]
        } else {
            vec![]
        }
    }
}

pub struct LabelValidator;

impl ImportValidator for LabelValidator {
    fn validate(&mut self, dataset: &DatasetDefinition, record: &MappedRecord) -> Vec<ImportIssue> {
        if dataset.labels.contains(&record.label) {
            vec![]
        } else {
            vec![issue(
                "invalid_label",
                format!("unknown label: {}", record.label),
            )]
        }
    }
}

pub struct DimensionValidator;

impl ImportValidator for DimensionValidator {
    fn validate(&mut self, dataset: &DatasetDefinition, record: &MappedRecord) -> Vec<ImportIssue> {
        let expected = dataset
            .dimensions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<HashSet<_>>();
        let mut issues = record
            .dimensions
            .keys()
            .filter(|name| !expected.contains(name.as_str()))
            .map(|name| issue("unknown_dimension", format!("unknown dimension: {name}")))
            .collect::<Vec<_>>();
        for definition in &dataset.dimensions {
            match record.dimensions.get(&definition.name) {
                Some(value) if definition.values.contains(value) => {}
                Some(value) => issues.push(issue(
                    "invalid_dimension_value",
                    format!("invalid value {value} for dimension {}", definition.name),
                )),
                None => issues.push(issue(
                    "missing_dimension",
                    format!("missing dimension {}", definition.name),
                )),
            }
        }
        issues
    }
}

pub struct NormalizedDuplicateValidator {
    seen: HashSet<String>,
}

impl NormalizedDuplicateValidator {
    pub fn new(existing_normalized: impl IntoIterator<Item = String>) -> Self {
        Self {
            seen: existing_normalized.into_iter().collect(),
        }
    }
}

impl ImportValidator for NormalizedDuplicateValidator {
    fn validate(
        &mut self,
        _dataset: &DatasetDefinition,
        record: &MappedRecord,
    ) -> Vec<ImportIssue> {
        if record.text.trim().is_empty() {
            return vec![];
        }
        if self.seen.insert(normalize_text(&record.text)) {
            vec![]
        } else {
            vec![issue(
                "duplicate_text",
                "normalized text already exists in this dataset or import",
            )]
        }
    }
}

pub struct ImportProcessor {
    dataset: DatasetDefinition,
    import_id: Uuid,
    pipeline: ValidationPipeline,
    valid_cells: BTreeMap<String, GenerationCell>,
}

impl ImportProcessor {
    pub fn new(
        dataset: DatasetDefinition,
        import_id: Uuid,
        mapping_dimensions: impl IntoIterator<Item = String>,
        existing_normalized: impl IntoIterator<Item = String>,
    ) -> Result<Self, ImportValidationError> {
        let expected = dataset
            .dimensions
            .iter()
            .map(|definition| definition.name.clone())
            .collect::<HashSet<_>>();
        let mapped = mapping_dimensions.into_iter().collect::<HashSet<_>>();
        if mapped != expected {
            return Err(ImportValidationError::DimensionMapping {
                expected: sorted(expected),
                actual: sorted(mapped),
            });
        }
        let valid_cells = expand_generation_cells(&dataset)
            .into_iter()
            .map(|cell| (cell.key(), cell))
            .collect();
        Ok(Self {
            dataset,
            import_id,
            pipeline: ValidationPipeline::standard(existing_normalized),
            valid_cells,
        })
    }

    pub fn process(&mut self, record: MappedRecord) -> ImportedRow {
        let mut issues = record.parse_issues.clone();
        if issues.is_empty() {
            issues.extend(self.pipeline.validate(&self.dataset, &record));
        }
        let candidate = GenerationCell {
            label: record.label.clone(),
            dimensions: record.dimensions.clone(),
        };
        let candidate_key = candidate.key();
        let cell_key = self
            .valid_cells
            .contains_key(&candidate_key)
            .then_some(candidate_key);
        ImportedRow {
            id: Uuid::new_v4(),
            import_id: self.import_id,
            dataset_id: self.dataset.id,
            source_row_number: record.source_row_number,
            normalized_text: normalize_text(&record.text),
            text: record.text,
            label: record.label,
            dimensions: record.dimensions,
            cell_key,
            status: if issues.is_empty() {
                ImportRowStatus::Accepted
            } else {
                ImportRowStatus::Rejected
            },
            issues,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ImportValidationError {
    #[error(
        "dimension mapping must exactly match dataset dimensions; expected {expected:?}, got {actual:?}"
    )]
    DimensionMapping {
        expected: Vec<String>,
        actual: Vec<String>,
    },
}

fn sorted(values: HashSet<String>) -> Vec<String> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn issue(code: impl Into<String>, message: impl Into<String>) -> ImportIssue {
    ImportIssue {
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dataset_core::domain::ImportRowStatus;
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use uuid::Uuid;

    use super::ImportProcessor;
    use crate::MappedRecord;

    #[test]
    fn composes_schema_and_duplicate_validation() {
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
        let mut processor = ImportProcessor::new(
            dataset,
            Uuid::new_v4(),
            ["style".into()],
            ["already exists".into()],
        )
        .expect("processor");
        let row = processor.process(MappedRecord {
            source_row_number: 3,
            text: "  Already   EXISTS ".into(),
            label: "unknown".into(),
            dimensions: BTreeMap::from([("style".into(), "other".into())]),
            parse_issues: vec![],
        });
        assert_eq!(row.status, ImportRowStatus::Rejected);
        assert!(row.issues.iter().any(|issue| issue.code == "invalid_label"));
        assert!(
            row.issues
                .iter()
                .any(|issue| issue.code == "invalid_dimension_value")
        );
        assert!(
            row.issues
                .iter()
                .any(|issue| issue.code == "duplicate_text")
        );
        assert!(row.cell_key.is_none());
    }
}
