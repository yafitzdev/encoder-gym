use std::io::{BufWriter, Write};

use anyhow::Context;
use dataset_core::{domain::SourceProvenance, ports::AcceptedRowSource};
use generation_core::ports::DatasetStore;
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{ExportArgs, ExportFormat};

pub async fn execute(args: ExportArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let dataset = store
        .get_dataset(args.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", args.dataset_id))?;
    let file = std::fs::File::create(&args.output)
        .with_context(|| format!("could not create {}", args.output.display()))?;
    let mut json_writer = if matches!(args.format, ExportFormat::Jsonl) {
        Some(BufWriter::new(file.try_clone().with_context(|| {
            format!(
                "could not prepare {} for JSONL export",
                args.output.display()
            )
        })?))
    } else {
        None
    };
    let dimensions = dataset
        .dimensions
        .iter()
        .map(|dimension| dimension.name.clone())
        .collect::<Vec<_>>();
    let mut csv_writer = if matches!(args.format, ExportFormat::Csv) {
        let mut writer = csv::Writer::from_writer(BufWriter::new(file));
        let mut header = vec!["id".to_owned(), "text".to_owned(), "label".to_owned()];
        header.extend(dimensions.iter().map(|name| format!("dimension.{name}")));
        header.extend([
            "created_at".to_owned(),
            "source_kind".to_owned(),
            "source_provenance".to_owned(),
        ]);
        writer.write_record(header)?;
        Some(writer)
    } else {
        None
    };
    let mut offset = 0_u32;
    let mut row_count = 0_usize;
    loop {
        let page = store
            .query_accepted_source_rows(args.dataset_id, 10_000, offset)
            .await?;
        let page_size = page.len();
        for row in page {
            match args.format {
                ExportFormat::Jsonl => {
                    let writer = json_writer.as_mut().context("JSONL writer is missing")?;
                    serde_json::to_writer(&mut *writer, &row)?;
                    writer.write_all(b"\n")?;
                }
                ExportFormat::Csv => {
                    let writer = csv_writer.as_mut().context("CSV writer is missing")?;
                    let mut record = vec![row.id.to_string(), row.text, row.label];
                    record.extend(
                        dimensions
                            .iter()
                            .map(|name| row.dimensions.get(name).cloned().unwrap_or_default()),
                    );
                    record.extend([
                        row.created_at.to_rfc3339(),
                        match &row.provenance {
                            SourceProvenance::Generated { .. } => "generated".into(),
                            SourceProvenance::Imported { .. } => "imported".into(),
                        },
                        serde_json::to_string(&row.provenance)?,
                    ]);
                    writer.write_record(record)?;
                }
            }
            row_count += 1;
        }
        if page_size < 10_000 {
            break;
        }
        offset = offset.saturating_add(10_000);
    }
    if let Some(mut writer) = json_writer {
        writer.flush()?;
    }
    if let Some(mut writer) = csv_writer {
        writer.flush()?;
    }
    crate::presentation::print(&serde_json::json!({
        "exported": true,
        "artifact": "accepted_rows",
        "dataset_id": args.dataset_id,
        "row_count": row_count,
        "output": args.output,
    }))?;
    Ok(())
}
