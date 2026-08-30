use std::collections::BTreeSet;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{SnapshotMember, SnapshotSplit, SourceProvenance};

#[derive(Debug, Error)]
pub enum SnapshotExportError {
    #[error("could not serialize snapshot export: {0}")]
    Serialization(String),
    #[error("snapshot export was not UTF-8: {0}")]
    Utf8(String),
}

#[derive(Serialize)]
struct JsonlMember<'a> {
    snapshot_id: Uuid,
    source_row_id: Uuid,
    split: SnapshotSplit,
    text: &'a str,
    label: &'a str,
    dimensions: &'a std::collections::BTreeMap<String, String>,
    fields: &'a std::collections::BTreeMap<String, serde_json::Value>,
    source_provenance: &'a SourceProvenance,
    source_created_at: chrono::DateTime<chrono::Utc>,
}

pub fn to_jsonl(members: &[SnapshotMember]) -> Result<String, SnapshotExportError> {
    let mut output = String::new();
    for member in members {
        let exported = JsonlMember {
            snapshot_id: member.snapshot_id,
            source_row_id: member.source_row_id,
            split: member.split,
            text: &member.text,
            label: &member.label,
            dimensions: &member.dimensions,
            fields: &member.fields,
            source_provenance: &member.source_provenance,
            source_created_at: member.source_created_at,
        };
        output.push_str(
            &serde_json::to_string(&exported)
                .map_err(|error| SnapshotExportError::Serialization(error.to_string()))?,
        );
        output.push('\n');
    }
    Ok(output)
}

pub fn to_csv(members: &[SnapshotMember]) -> Result<String, SnapshotExportError> {
    let dimensions = members
        .iter()
        .flat_map(|member| member.dimensions.keys().cloned())
        .collect::<BTreeSet<_>>();
    let fields = members
        .iter()
        .flat_map(|member| member.fields.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut writer = csv::Writer::from_writer(Vec::new());
    let mut header = vec![
        "snapshot_id".to_owned(),
        "source_row_id".to_owned(),
        "split".to_owned(),
        "text".to_owned(),
        "label".to_owned(),
    ];
    header.extend(dimensions.iter().map(|name| format!("dimension:{name}")));
    header.extend(fields.iter().map(|name| format!("field:{name}")));
    header.push("source_created_at".to_owned());
    header.extend(["source_kind".to_owned(), "source_provenance".to_owned()]);
    writer
        .write_record(header)
        .map_err(|error| SnapshotExportError::Serialization(error.to_string()))?;

    for member in members {
        let mut record = vec![
            member.snapshot_id.to_string(),
            member.source_row_id.to_string(),
            member.split.as_str().to_owned(),
            member.text.clone(),
            member.label.clone(),
        ];
        record.extend(
            dimensions
                .iter()
                .map(|name| member.dimensions.get(name).cloned().unwrap_or_default()),
        );
        record.extend(fields.iter().map(|name| {
            member.fields.get(name).map_or_else(String::new, |value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            })
        }));
        record.push(member.source_created_at.to_rfc3339());
        let source_kind = match member.source_provenance {
            SourceProvenance::Generated { .. } => "generated",
            SourceProvenance::Imported { .. } => "imported",
        };
        record.push(source_kind.into());
        record.push(
            serde_json::to_string(&member.source_provenance)
                .map_err(|error| SnapshotExportError::Serialization(error.to_string()))?,
        );
        writer
            .write_record(record)
            .map_err(|error| SnapshotExportError::Serialization(error.to_string()))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|error| SnapshotExportError::Serialization(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| SnapshotExportError::Utf8(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use uuid::Uuid;

    use super::{to_csv, to_jsonl};
    use crate::domain::{SnapshotMember, SnapshotSplit, SourceProvenance};

    #[test]
    fn exports_split_dimensions_and_source_provenance() {
        let member = SnapshotMember {
            id: Uuid::new_v4(),
            snapshot_id: Uuid::new_v4(),
            source_row_id: Uuid::new_v4(),
            split: SnapshotSplit::Validation,
            text: "charged twice".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("difficulty".into(), "hard".into())]),
            fields: BTreeMap::from([("ticket_id".into(), serde_json::json!("T-1"))]),
            source_provenance: SourceProvenance::Imported {
                import_id: Uuid::new_v4(),
                source_path: "fixtures/support.jsonl".into(),
                source_row_number: 7,
            },
            source_created_at: Utc::now(),
        };
        let jsonl = to_jsonl(std::slice::from_ref(&member)).expect("JSONL");
        assert!(jsonl.contains("\"split\":\"validation\""));
        assert!(jsonl.contains(&member.source_row_id.to_string()));
        let csv = to_csv(&[member]).expect("CSV");
        assert!(csv.contains("dimension:difficulty"));
        assert!(csv.contains("validation"));
    }
}
