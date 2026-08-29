use std::{
    collections::BTreeMap,
    io::{BufRead, Lines, Read},
};

use dataset_core::domain::{ImportFieldMapping, ImportIssue};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedRecord {
    pub source_row_number: u64,
    pub text: String,
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
    pub parse_issues: Vec<ImportIssue>,
}

impl MappedRecord {
    fn rejected(source_row_number: u64, error: impl std::fmt::Display) -> Self {
        Self {
            source_row_number,
            text: String::new(),
            label: String::new(),
            dimensions: BTreeMap::new(),
            parse_issues: vec![ImportIssue {
                code: "parse_error".into(),
                message: error.to_string(),
            }],
        }
    }
}

pub struct JsonlRecordReader<R: BufRead> {
    lines: Lines<R>,
    mapping: ImportFieldMapping,
    line_number: u64,
}

impl<R: BufRead> JsonlRecordReader<R> {
    pub fn new(reader: R, mapping: ImportFieldMapping) -> Self {
        Self {
            lines: reader.lines(),
            mapping,
            line_number: 0,
        }
    }
}

impl<R: BufRead> Iterator for JsonlRecordReader<R> {
    type Item = MappedRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        self.line_number += 1;
        Some(match line {
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(value) => map_json(self.line_number, &value, &self.mapping),
                Err(error) => MappedRecord::rejected(self.line_number, error),
            },
            Err(error) => MappedRecord::rejected(self.line_number, error),
        })
    }
}

pub struct CsvRecordReader<R: Read> {
    headers: csv::StringRecord,
    records: csv::StringRecordsIntoIter<R>,
    mapping: ImportFieldMapping,
    record_number: u64,
}

impl<R: Read> CsvRecordReader<R> {
    pub fn new(reader: R, mapping: ImportFieldMapping) -> Result<Self, ImportReaderError> {
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
        let headers = reader.headers().map_err(reader_error)?.clone();
        Ok(Self {
            headers,
            records: reader.into_records(),
            mapping,
            record_number: 1,
        })
    }
}

impl<R: Read> Iterator for CsvRecordReader<R> {
    type Item = MappedRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let record = self.records.next()?;
        self.record_number += 1;
        Some(match record {
            Ok(record) => map_csv(self.record_number, &self.headers, &record, &self.mapping),
            Err(error) => MappedRecord::rejected(self.record_number, error),
        })
    }
}

#[derive(Debug, Error)]
#[error("could not read import source: {0}")]
pub struct ImportReaderError(String);

fn map_json(row_number: u64, value: &Value, mapping: &ImportFieldMapping) -> MappedRecord {
    let mut issues = Vec::new();
    let text = json_string(value, &mapping.text_field, &mut issues);
    let label = json_string(value, &mapping.label_field, &mut issues);
    let dimensions = mapping
        .dimension_fields
        .iter()
        .map(|(name, field)| (name.clone(), json_string(value, field, &mut issues)))
        .collect();
    MappedRecord {
        source_row_number: row_number,
        text,
        label,
        dimensions,
        parse_issues: issues,
    }
}

fn json_string(value: &Value, path: &str, issues: &mut Vec<ImportIssue>) -> String {
    let selected = path
        .split('.')
        .try_fold(value, |current, segment| current.get(segment));
    match selected.and_then(Value::as_str) {
        Some(value) => value.to_owned(),
        None => {
            issues.push(ImportIssue {
                code: "missing_or_non_string_field".into(),
                message: format!("field {path} must contain a string"),
            });
            String::new()
        }
    }
}

fn map_csv(
    row_number: u64,
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
    mapping: &ImportFieldMapping,
) -> MappedRecord {
    let fields = headers
        .iter()
        .zip(record.iter())
        .collect::<BTreeMap<_, _>>();
    let mut issues = Vec::new();
    let mut field = |name: &str| match fields.get(name) {
        Some(value) => (*value).to_owned(),
        None => {
            issues.push(ImportIssue {
                code: "missing_field".into(),
                message: format!("CSV field {name} is missing"),
            });
            String::new()
        }
    };
    let text = field(&mapping.text_field);
    let label = field(&mapping.label_field);
    let dimensions = mapping
        .dimension_fields
        .iter()
        .map(|(name, source)| (name.clone(), field(source)))
        .collect();
    MappedRecord {
        source_row_number: row_number,
        text,
        label,
        dimensions,
        parse_issues: issues,
    }
}

fn reader_error(error: impl std::fmt::Display) -> ImportReaderError {
    ImportReaderError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, io::Cursor};

    use dataset_core::domain::ImportFieldMapping;

    use super::{CsvRecordReader, JsonlRecordReader};

    fn mapping() -> ImportFieldMapping {
        ImportFieldMapping::new(
            "message",
            "category",
            BTreeMap::from([("style".into(), "metadata.style".into())]),
        )
        .expect("mapping")
    }

    #[test]
    fn streams_jsonl_and_rejects_a_malformed_line() {
        let input = concat!(
            "{\"message\":\"hello\",\"category\":\"billing\",\"metadata\":{\"style\":\"clean\"}}\n",
            "not json\n"
        );
        let records = JsonlRecordReader::new(Cursor::new(input), mapping()).collect::<Vec<_>>();
        assert_eq!(records[0].dimensions["style"], "clean");
        assert_eq!(records[1].parse_issues[0].code, "parse_error");
    }

    #[test]
    fn streams_csv_with_header_mapping() {
        let mapping = ImportFieldMapping::new(
            "message",
            "category",
            BTreeMap::from([("style".into(), "style".into())]),
        )
        .expect("mapping");
        let input = "message,category,style\nhello,billing,messy\n";
        let records = CsvRecordReader::new(Cursor::new(input), mapping)
            .expect("reader")
            .collect::<Vec<_>>();
        assert_eq!(records[0].source_row_number, 2);
        assert_eq!(records[0].label, "billing");
        assert_eq!(records[0].dimensions["style"], "messy");
    }
}
