use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

use anyhow::{Context, bail};
use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetImport, ImportFieldMapping, ImportFormat, ImportRejection, ImportReport,
        ImportRowStatus, ImportState, ImportedRow,
    },
    ports::ImportStore,
};
use dataset_import::{CsvRecordReader, ImportProcessor, JsonlRecordReader, MappedRecord};
use generation_core::ports::DatasetStore;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::cli::{DatasetImportArgs, DatasetImportFormatArg};

pub async fn run(args: DatasetImportArgs, store: &SqliteStore) -> anyhow::Result<()> {
    anyhow::ensure!(args.batch_size > 0, "batch size must be greater than zero");
    let input = std::fs::canonicalize(&args.input)
        .with_context(|| format!("could not open import source {}", args.input.display()))?;
    let source_path = input.to_string_lossy().into_owned();
    let format = resolve_format(args.format, &input)?;
    let mapping = ImportFieldMapping::new(
        args.text_field,
        args.label_field,
        parse_dimension_mappings(args.dimensions)?,
    )?;
    let dataset = store
        .get_dataset(args.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", args.dataset_id))?;
    let existing = store.accepted_normalized_source_texts(dataset.id).await?;
    let dataset_id = dataset.id;
    let import_id = Uuid::new_v4();
    let mut processor = ImportProcessor::new(
        dataset,
        import_id,
        mapping.dimension_fields.keys().cloned(),
        existing,
    )?;
    let reader = open_reader(&input, format, mapping.clone())?;

    if args.dry_run {
        let report = process_dry_run(
            source_path,
            reader,
            &mut processor,
            args.max_rejection_samples,
        );
        return print_json(&report);
    }

    let mut dataset_import = DatasetImport::queued(dataset_id, source_path, format, mapping)?;
    dataset_import.id = import_id;
    store.create_import(&dataset_import).await?;
    dataset_import.state = ImportState::Running;
    dataset_import.updated_at = Utc::now();
    store.save_import(&dataset_import).await?;

    let result = process_persisted(
        reader,
        &mut processor,
        &mut dataset_import,
        args.batch_size,
        args.max_rejection_samples,
        store,
    )
    .await;
    match result {
        Ok(report) => {
            dataset_import.state = ImportState::Completed;
            dataset_import.updated_at = Utc::now();
            store.save_import(&dataset_import).await?;
            print_json(&report)
        }
        Err(error) => {
            dataset_import.state = ImportState::Failed;
            dataset_import.error_message = Some(error.to_string());
            dataset_import.updated_at = Utc::now();
            store.save_import(&dataset_import).await?;
            Err(error)
        }
    }
}

pub async fn list(
    dataset_id: Option<Uuid>,
    limit: u32,
    offset: u32,
    summary: bool,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    let imports = store.list_imports(dataset_id, limit, offset).await?;
    crate::presentation::print_page(
        &imports,
        imports.len(),
        crate::cli::PageArgs {
            limit,
            offset,
            summary,
        },
    )
}

pub async fn show(id: Uuid, store: &SqliteStore) -> anyhow::Result<()> {
    print_json(
        &store
            .get_import(id)
            .await?
            .with_context(|| format!("dataset import not found: {id}"))?,
    )
}

pub async fn rows(
    id: Uuid,
    limit: u32,
    offset: u32,
    summary: bool,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    store
        .get_import(id)
        .await?
        .with_context(|| format!("dataset import not found: {id}"))?;
    let rows = store.list_imported_rows(id, limit, offset).await?;
    crate::presentation::print_page(
        &rows,
        rows.len(),
        crate::cli::PageArgs {
            limit,
            offset,
            summary,
        },
    )
}

async fn process_persisted(
    reader: Box<dyn Iterator<Item = MappedRecord>>,
    processor: &mut ImportProcessor,
    dataset_import: &mut DatasetImport,
    batch_size: usize,
    max_rejection_samples: usize,
    store: &SqliteStore,
) -> anyhow::Result<ImportReport> {
    let mut batch = Vec::with_capacity(batch_size);
    let mut rejection_samples = Vec::new();
    for record in reader {
        let row = processor.process(record);
        record_counts(
            dataset_import,
            &row,
            &mut rejection_samples,
            max_rejection_samples,
        );
        batch.push(row);
        if batch.len() == batch_size {
            store.insert_imported_rows(dataset_import, &batch).await?;
            batch.clear();
            dataset_import.updated_at = Utc::now();
            store.save_import(dataset_import).await?;
        }
    }
    if !batch.is_empty() {
        store.insert_imported_rows(dataset_import, &batch).await?;
    }
    Ok(ImportReport {
        import_id: Some(dataset_import.id),
        source_path: dataset_import.source_path.clone(),
        dry_run: false,
        processed_rows: dataset_import.processed_rows,
        accepted_rows: dataset_import.accepted_rows,
        rejected_rows: dataset_import.rejected_rows,
        rejection_samples,
    })
}

fn process_dry_run(
    source_path: String,
    reader: Box<dyn Iterator<Item = MappedRecord>>,
    processor: &mut ImportProcessor,
    max_rejection_samples: usize,
) -> ImportReport {
    let mut processed_rows = 0_u64;
    let mut accepted_rows = 0_u64;
    let mut rejected_rows = 0_u64;
    let mut rejection_samples = Vec::new();
    for record in reader {
        let row = processor.process(record);
        processed_rows += 1;
        match row.status {
            ImportRowStatus::Accepted => accepted_rows += 1,
            ImportRowStatus::Rejected => {
                rejected_rows += 1;
                if rejection_samples.len() < max_rejection_samples {
                    rejection_samples.push(ImportRejection {
                        source_row_number: row.source_row_number,
                        issues: row.issues,
                    });
                }
            }
        }
    }
    ImportReport {
        import_id: None,
        source_path,
        dry_run: true,
        processed_rows,
        accepted_rows,
        rejected_rows,
        rejection_samples,
    }
}

fn record_counts(
    dataset_import: &mut DatasetImport,
    row: &ImportedRow,
    rejection_samples: &mut Vec<ImportRejection>,
    max_rejection_samples: usize,
) {
    dataset_import.processed_rows += 1;
    match row.status {
        ImportRowStatus::Accepted => dataset_import.accepted_rows += 1,
        ImportRowStatus::Rejected => {
            dataset_import.rejected_rows += 1;
            if rejection_samples.len() < max_rejection_samples {
                rejection_samples.push(ImportRejection {
                    source_row_number: row.source_row_number,
                    issues: row.issues.clone(),
                });
            }
        }
    }
}

pub(crate) fn open_reader(
    path: &Path,
    format: ImportFormat,
    mapping: ImportFieldMapping,
) -> anyhow::Result<Box<dyn Iterator<Item = MappedRecord>>> {
    let file = File::open(path)
        .with_context(|| format!("could not open import source {}", path.display()))?;
    match format {
        ImportFormat::Jsonl => Ok(Box::new(JsonlRecordReader::new(
            BufReader::new(file),
            mapping,
        ))),
        ImportFormat::Csv => Ok(Box::new(CsvRecordReader::new(file, mapping)?)),
    }
}

fn resolve_format(
    requested: Option<DatasetImportFormatArg>,
    path: &Path,
) -> anyhow::Result<ImportFormat> {
    if let Some(requested) = requested {
        return Ok(match requested {
            DatasetImportFormatArg::Jsonl => ImportFormat::Jsonl,
            DatasetImportFormatArg::Csv => ImportFormat::Csv,
        });
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("jsonl") => Ok(ImportFormat::Jsonl),
        Some(extension) if extension.eq_ignore_ascii_case("csv") => Ok(ImportFormat::Csv),
        _ => bail!("could not infer import format; provide --format jsonl or --format csv"),
    }
}

fn parse_dimension_mappings(values: Vec<String>) -> anyhow::Result<BTreeMap<String, String>> {
    values
        .into_iter()
        .map(|value| {
            let (name, field) = value.split_once('=').with_context(|| {
                format!("dimension mapping must use name=field syntax: {value}")
            })?;
            Ok((name.to_owned(), field.to_owned()))
        })
        .collect()
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}
