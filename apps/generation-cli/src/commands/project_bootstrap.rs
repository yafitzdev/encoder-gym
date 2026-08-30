use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetImport, ImportFieldMapping, ImportFormat, ImportRowStatus, ImportState,
        SourceProvenance, SourceRow, SplitConfiguration, SplitRatios,
    },
    splitting::build_snapshot,
};
use dataset_import::ImportProcessor;
use generation_core::domain::{DatasetDefinition, DimensionDefinition};
use project_preparation::{
    BootstrapCohortManifest, BootstrapManifest, BootstrapSourceBundle, BootstrapStore,
    PreparationStore, compile_bootstrap, preview_bootstrap,
};
use sha2::{Digest, Sha256};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::cli::PageArgs;

use super::ingestion::open_reader;

pub async fn preview(path: &Path, _store: &SqliteStore) -> anyhow::Result<()> {
    let (manifest, root) = load_manifest(path)?;
    let sources = load_sources(&manifest, &root)?;
    crate::presentation::print(&preview_bootstrap(&manifest, &sources)?)
}

pub async fn create(path: &Path, store: &SqliteStore) -> anyhow::Result<()> {
    let (manifest, root) = load_manifest(path)?;
    let sources = load_sources(&manifest, &root)?;
    let preview = preview_bootstrap(&manifest, &sources)?;
    if !preview.eligible {
        bail!(
            "project bootstrap is blocked: {}",
            preview
                .issues
                .iter()
                .map(|issue| issue.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    if let Some(existing) = store
        .get_bootstrap_by_fingerprint(&preview.bootstrap_fingerprint)
        .await?
    {
        return print_result(false, existing, store).await;
    }
    let bundle = compile_bootstrap(&manifest, sources)?;
    let bootstrap = store.create_bootstrap(&bundle).await?;
    print_result(bootstrap.id == bundle.bootstrap.id, bootstrap, store).await
}

pub async fn show(id: Uuid, store: &SqliteStore) -> anyhow::Result<()> {
    let bootstrap = store
        .get_bootstrap(id)
        .await?
        .with_context(|| format!("project bootstrap not found: {id}"))?;
    print_result(false, bootstrap, store).await
}

pub async fn list(page: PageArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let values = store.list_bootstraps(page.limit, page.offset).await?;
    crate::presentation::print_page(&values, values.len(), page)
}

async fn print_result(
    created: bool,
    bootstrap: project_preparation::ProjectBootstrap,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    let preparation = store
        .get_preparation(bootstrap.preparation_id)
        .await?
        .with_context(|| {
            format!(
                "prepared project not found for bootstrap {}: {}",
                bootstrap.id, bootstrap.preparation_id
            )
        })?;
    crate::presentation::print(&serde_json::json!({
        "created": created,
        "bootstrap": bootstrap,
        "preparation": preparation,
        "next_command": format!(
            "synth workflow start {}",
            preparation.workflow_definition_id
        ),
    }))
}

fn load_manifest(path: &Path) -> anyhow::Result<(BootstrapManifest, PathBuf)> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("could not read bootstrap manifest {}", path.display()))?;
    let source = std::fs::read_to_string(&canonical)
        .with_context(|| format!("could not read bootstrap manifest {}", path.display()))?;
    let manifest = if canonical
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        BootstrapManifest::parse_json(&source)?
    } else {
        BootstrapManifest::parse_toml(&source)?
    };
    let root = canonical
        .parent()
        .context("bootstrap manifest has no parent directory")?
        .to_owned();
    Ok((manifest, root))
}

fn load_sources(
    manifest: &BootstrapManifest,
    root: &Path,
) -> anyhow::Result<Vec<BootstrapSourceBundle>> {
    declarations(manifest)
        .into_iter()
        .map(|(key, cohort)| load_source(manifest, root, key, cohort))
        .collect()
}

fn declarations(manifest: &BootstrapManifest) -> Vec<(String, &BootstrapCohortManifest)> {
    manifest
        .development
        .cohorts
        .iter()
        .enumerate()
        .map(|(index, cohort)| (format!("development:{index}"), cohort))
        .chain(
            manifest
                .sealed
                .iter()
                .flat_map(|suite| suite.cohorts.iter().enumerate())
                .map(|(index, cohort)| (format!("sealed:{index}"), cohort)),
        )
        .collect()
}

fn load_source(
    manifest: &BootstrapManifest,
    root: &Path,
    key: String,
    cohort: &BootstrapCohortManifest,
) -> anyhow::Result<BootstrapSourceBundle> {
    let source_path = if cohort.source.path.is_absolute() {
        cohort.source.path.clone()
    } else {
        root.join(&cohort.source.path)
    };
    let canonical = std::fs::canonicalize(&source_path).with_context(|| {
        format!(
            "could not open bootstrap source {} for {key}",
            source_path.display()
        )
    })?;
    let content_fingerprint = file_fingerprint(&canonical)?;
    let format = resolve_format(cohort.source.format, &canonical)?;
    let mapping = ImportFieldMapping::new(
        cohort.source.text_field.clone(),
        cohort.source.label_field.clone(),
        cohort.source.dimensions.clone(),
    )?;
    let dataset = source_dataset(manifest, &key, cohort)?;
    let mut dataset_import = DatasetImport::queued(
        dataset.id,
        canonical.to_string_lossy().into_owned(),
        format,
        mapping.clone(),
    )?;
    let mut processor = ImportProcessor::new(
        dataset.clone(),
        dataset_import.id,
        mapping.dimension_fields.keys().cloned(),
        Vec::<String>::new(),
    )?;
    let rows = open_reader(&canonical, format, mapping)?
        .map(|record| processor.process(record))
        .collect::<Vec<_>>();
    dataset_import.processed_rows = rows.len() as u64;
    dataset_import.accepted_rows = rows
        .iter()
        .filter(|row| row.status == ImportRowStatus::Accepted)
        .count() as u64;
    dataset_import.rejected_rows = dataset_import
        .processed_rows
        .saturating_sub(dataset_import.accepted_rows);
    dataset_import.state = ImportState::Completed;
    dataset_import.updated_at = Utc::now();
    let accepted = rows
        .iter()
        .filter(|row| row.status == ImportRowStatus::Accepted)
        .map(|row| SourceRow {
            id: row.id,
            dataset_id: row.dataset_id,
            text: row.text.clone(),
            label: row.label.clone(),
            dimensions: row.dimensions.clone(),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Imported {
                import_id: dataset_import.id,
                source_path: dataset_import.source_path.clone(),
                source_row_number: row.source_row_number,
            },
            created_at: row.created_at,
        })
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        bail!("bootstrap source {key} contains no valid rows");
    }
    let (snapshot, members) = build_snapshot(
        dataset.id,
        format!("{} immutable cohort", cohort.name),
        Some(format!(
            "Bootstrapped from {} ({content_fingerprint})",
            cohort.source.path.display()
        )),
        SplitConfiguration::new(SplitRatios::new(0.0, 0.0, 1.0)?, 0),
        accepted,
    )?;
    Ok(BootstrapSourceBundle {
        key,
        content_fingerprint,
        dataset,
        dataset_import,
        imported_rows: rows,
        snapshot,
        members,
    })
}

fn source_dataset(
    manifest: &BootstrapManifest,
    key: &str,
    cohort: &BootstrapCohortManifest,
) -> anyhow::Result<DatasetDefinition> {
    DatasetDefinition::new(
        format!("{} / {} / {key}", manifest.name, cohort.name),
        manifest.project.dataset.task.clone(),
        manifest.project.dataset.labels.clone(),
        manifest
            .project
            .dataset
            .dimensions
            .iter()
            .map(|dimension| {
                DimensionDefinition::new(dimension.name.clone(), dimension.values.clone())
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(Into::into)
}

fn resolve_format(requested: Option<ImportFormat>, path: &Path) -> anyhow::Result<ImportFormat> {
    if let Some(format) = requested {
        return Ok(format);
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("jsonl") => Ok(ImportFormat::Jsonl),
        Some(extension) if extension.eq_ignore_ascii_case("csv") => Ok(ImportFormat::Csv),
        _ => bail!(
            "could not infer bootstrap source format for {}; set source.format",
            path.display()
        ),
    }
}

fn file_fingerprint(path: &Path) -> anyhow::Result<String> {
    let file = File::open(path)
        .with_context(|| format!("could not fingerprint bootstrap source {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}
