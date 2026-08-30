//! Pure accepted-row export formatting.

use std::collections::BTreeSet;

use crate::domain::{GeneratedRow, ValidationStatus};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("could not encode CSV export: {0}")]
    Csv(#[from] csv::Error),
    #[error("could not finish CSV export: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode JSONL export: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CSV export was not UTF-8")]
    Utf8,
}

pub fn to_jsonl(rows: &[GeneratedRow]) -> Result<String, ExportError> {
    let accepted = rows
        .iter()
        .filter(|row| row.validation_status == ValidationStatus::Accepted)
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    if accepted.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}\n", accepted.join("\n")))
    }
}

pub fn to_csv(rows: &[GeneratedRow]) -> Result<String, ExportError> {
    let accepted = rows
        .iter()
        .filter(|row| row.validation_status == ValidationStatus::Accepted)
        .collect::<Vec<_>>();
    let dimension_names = accepted
        .iter()
        .flat_map(|row| row.dimensions.keys().cloned())
        .collect::<BTreeSet<_>>();
    let field_names = accepted
        .iter()
        .flat_map(|row| row.fields.keys().cloned())
        .collect::<BTreeSet<_>>();

    let mut writer = csv::Writer::from_writer(Vec::new());
    let mut header = vec!["id".to_owned(), "text".to_owned(), "label".to_owned()];
    header.extend(
        dimension_names
            .iter()
            .map(|name| format!("dimension.{name}")),
    );
    header.extend(field_names.iter().map(|name| format!("field.{name}")));
    header.extend([
        "generation_job_id".to_owned(),
        "generator_backend".to_owned(),
        "generator_model".to_owned(),
        "construction_provenance".to_owned(),
        "created_at".to_owned(),
    ]);
    writer.write_record(&header)?;

    for row in accepted {
        let mut record = vec![row.id.to_string(), row.text.clone(), row.label.clone()];
        record.extend(
            dimension_names
                .iter()
                .map(|name| row.dimensions.get(name).cloned().unwrap_or_default()),
        );
        record.extend(field_names.iter().map(|name| {
            row.fields.get(name).map_or_else(String::new, |value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            })
        }));
        record.extend([
            row.generation_job_id.to_string(),
            row.generator_backend.clone(),
            row.generator_model.clone(),
            row.construction
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
                .unwrap_or_default(),
            row.created_at.to_rfc3339(),
        ]);
        writer.write_record(&record)?;
    }

    let bytes = writer.into_inner().map_err(|error| error.into_error())?;
    String::from_utf8(bytes).map_err(|_| ExportError::Utf8)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use uuid::Uuid;

    use super::{to_csv, to_jsonl};
    use crate::domain::{GeneratedRow, ValidationStatus};

    #[test]
    fn exports_only_accepted_rows_and_flattens_arbitrary_dimensions() {
        let accepted = row(ValidationStatus::Accepted);
        let rejected = row(ValidationStatus::Rejected);

        let jsonl = to_jsonl(&[accepted.clone(), rejected.clone()]).expect("JSONL export");
        assert_eq!(jsonl.lines().count(), 1);
        assert!(jsonl.contains(&accepted.id.to_string()));
        assert!(!jsonl.contains(&rejected.id.to_string()));

        let csv = to_csv(&[accepted, rejected]).expect("CSV export");
        assert!(
            csv.lines()
                .next()
                .expect("header")
                .contains("dimension.style")
        );
        assert_eq!(csv.lines().count(), 2);
    }

    fn row(status: ValidationStatus) -> GeneratedRow {
        GeneratedRow {
            id: Uuid::new_v4(),
            dataset_id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            generation_job_id: Uuid::new_v4(),
            cell_key: "cell".into(),
            text: "Why was I charged twice?".into(),
            normalized_text: "why was i charged twice?".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("style".into(), "clean".into())]),
            fields: BTreeMap::from([("ticket_id".into(), serde_json::json!("T-1"))]),
            construction: None,
            generator_backend: "fake".into(),
            generator_model: "fake-v1".into(),
            created_at: Utc::now(),
            validation_status: status,
            validation_errors: vec![],
            generation_metadata: serde_json::json!({}),
        }
    }
}
