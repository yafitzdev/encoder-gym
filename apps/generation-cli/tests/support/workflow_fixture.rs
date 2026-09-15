use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetImport, ImportFieldMapping, ImportFormat, ImportRowStatus, ImportState, ImportedRow,
        SourceProvenance, SourceRow, SplitConfiguration, SplitRatios,
    },
    ports::{ImportStore, SnapshotStore},
    splitting::build_snapshot,
};
use generation_core::{
    deduplication::normalize_text,
    domain::{DatasetDefinition, DimensionDefinition, GenerationCell},
    ports::DatasetStore,
};
use project_config::GenerationBackendKind;
use project_preparation::PreparationManifest;
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use tempfile::TempDir;
use uuid::Uuid;
use workflow_core::benchmark_qualification::QualificationConfidence;

use super::run_json;

pub enum GenerationMode {
    Fake,
    OpenAiCompatible { base_url: String, model: String },
}

pub struct WorkflowFixture {
    _directory: TempDir,
    database_url: String,
    manifest_path: PathBuf,
    pub development_snapshot_id: Uuid,
    pub sealed_snapshot_id: Uuid,
}

impl WorkflowFixture {
    pub fn new(mode: GenerationMode) -> Self {
        let directory = tempfile::tempdir().expect("temporary workflow directory");
        let database = directory.path().join("workflow-acceptance.db");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            database.to_string_lossy().replace('\\', "/")
        );
        let (development_snapshot_id, sealed_snapshot_id) = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(create_snapshot_fixtures(&database_url));
        let source = include_str!("../fixtures/project-preparation.toml").replace(
            "00000000-0000-0000-0000-000000000000",
            &development_snapshot_id.to_string(),
        );
        let mut manifest = PreparationManifest::parse_toml(&source).expect("example manifest");
        // The shared workflow fixture intentionally uses a small benchmark
        // population. Keep the readiness policy explicit while lowering its
        // statistical floor so governance tests exercise their intended
        // behavior rather than failing during project preparation.
        manifest
            .benchmark_qualification
            .policy
            .minimum_overall_support = 1;
        manifest
            .benchmark_qualification
            .policy
            .minimum_label_support = 1;
        manifest.benchmark_qualification.policy.confidence = QualificationConfidence::Eighty;
        manifest
            .benchmark_qualification
            .policy
            .maximum_proportion_margin_of_error = 0.49;
        manifest.project.generation.target_per_cell = 1;
        manifest.project.generation.batch_size = 8;
        manifest.project.generation.max_retries = 0;
        manifest.project.generation.max_attempt_multiplier = 1;
        match mode {
            GenerationMode::Fake => {
                manifest.project.generation.backend = GenerationBackendKind::Fake;
                manifest.project.generation.base_url = None;
                manifest.project.generation.model = "deterministic-v1".into();
            }
            GenerationMode::OpenAiCompatible { base_url, model } => {
                manifest.project.generation.backend = GenerationBackendKind::OpenaiCompatible;
                manifest.project.generation.base_url = Some(base_url);
                manifest.project.generation.model = model;
            }
        }
        manifest.project.snapshot.train_ratio = 0.8;
        manifest.project.snapshot.validation_ratio = 0.0;
        manifest.project.snapshot.test_ratio = 0.2;
        manifest.project.training.feature_dimension = 64;
        manifest.project.training.epochs = 1;
        manifest.project.training.checkpoint_every = 1;
        manifest.project.training.artifact_root = directory.path().join("artifacts");
        manifest.workflow.total_rows = 8;
        manifest.workflow.reserved_rows = 2;
        manifest.workflow.budget.maximum_iterations = 1;
        manifest.workflow.budget.maximum_initial_rows = 8;
        manifest.workflow.budget.maximum_cumulative_rows = 10;
        manifest.workflow.budget.maximum_generation_attempts = 32;
        manifest.workflow.budget.maximum_generation_requests = 16;
        manifest.workflow.budget.maximum_stage_attempts = 2;

        let manifest_path = directory.path().join("workflow.toml");
        write_manifest(&manifest_path, &manifest);
        Self {
            _directory: directory,
            database_url,
            manifest_path,
            development_snapshot_id,
            sealed_snapshot_id,
        }
    }

    pub fn database_url(&self) -> &str {
        &self.database_url
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn prepare(&self) -> Value {
        run_json(
            &self.database_url,
            [
                "project",
                "prepare",
                self.manifest_path.to_str().expect("UTF-8 manifest path"),
            ],
        )
    }

    pub fn write_variant(
        &self,
        name: &str,
        mutate: impl FnOnce(&mut PreparationManifest),
    ) -> PathBuf {
        let source = std::fs::read_to_string(&self.manifest_path).expect("manifest source");
        let mut manifest = PreparationManifest::parse_toml(&source).expect("manifest parses");
        mutate(&mut manifest);
        let path = self
            .manifest_path
            .parent()
            .expect("manifest directory")
            .join(name);
        write_manifest(&path, &manifest);
        path
    }
}

fn write_manifest(path: &Path, manifest: &PreparationManifest) {
    std::fs::write(
        path,
        toml::to_string_pretty(manifest).expect("manifest serializes"),
    )
    .expect("manifest writes");
}

async fn create_snapshot_fixtures(database_url: &str) -> (Uuid, Uuid) {
    let store = SqliteStore::connect(database_url)
        .await
        .expect("database connects");
    let dataset = DatasetDefinition::new(
        "workflow benchmark source",
        "Classify customer support requests",
        vec![
            "billing".into(),
            "fraud".into(),
            "account".into(),
            "technical".into(),
        ],
        vec![
            DimensionDefinition::new(
                "difficulty",
                vec!["easy".into(), "medium".into(), "hard".into()],
            )
            .expect("dimension"),
            DimensionDefinition::new("writing_style", vec!["clean".into(), "messy".into()])
                .expect("dimension"),
            DimensionDefinition::new("ambiguity", vec!["obvious".into(), "ambiguous".into()])
                .expect("dimension"),
        ],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let mut development_rows = Vec::new();
    for (label, base_text) in [
        ("billing", "Why was I charged twice?"),
        ("fraud", "Someone used my card without permission"),
        ("account", "I cannot sign in to my account"),
        ("technical", "The application is not responding"),
    ] {
        for index in 0..7 {
            development_rows.push((
                if label == "billing" && index == 0 {
                    base_text.to_owned()
                } else {
                    format!("{base_text} (development example {index})")
                },
                label.to_owned(),
                BTreeMap::from([
                    (
                        "difficulty".to_owned(),
                        ["easy", "medium", "hard"][index % 3].to_owned(),
                    ),
                    (
                        "writing_style".to_owned(),
                        ["clean", "messy"][index % 2].to_owned(),
                    ),
                    (
                        "ambiguity".to_owned(),
                        ["obvious", "ambiguous"][index % 2].to_owned(),
                    ),
                ]),
            ));
        }
    }
    let development =
        insert_snapshot_rows(&store, dataset.id, "development fixture", development_rows).await;
    let sealed = insert_snapshot(
        &store,
        dataset.id,
        "sealed fixture",
        "Someone used my card without permission",
        "fraud",
        r#"{"difficulty":"hard","writing_style":"messy","ambiguity":"ambiguous"}"#,
    )
    .await;
    (development, sealed)
}

async fn insert_snapshot(
    store: &SqliteStore,
    dataset_id: Uuid,
    name: &str,
    text: &str,
    label: &str,
    dimensions: &str,
) -> Uuid {
    let dimensions =
        serde_json::from_str::<BTreeMap<String, String>>(dimensions).expect("fixture dimensions");
    insert_snapshot_rows(
        store,
        dataset_id,
        name,
        vec![(text.to_owned(), label.to_owned(), dimensions)],
    )
    .await
}

async fn insert_snapshot_rows(
    store: &SqliteStore,
    dataset_id: Uuid,
    name: &str,
    rows: Vec<(String, String, BTreeMap<String, String>)>,
) -> Uuid {
    assert!(!rows.is_empty(), "fixture snapshot needs rows");
    let split = SplitConfiguration::new(SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"), 42);
    let source_path = format!("fixture://{name}.jsonl");
    let mapping = ImportFieldMapping::new(
        "text",
        "label",
        rows[0]
            .2
            .keys()
            .map(|name| (name.clone(), name.clone()))
            .collect(),
    )
    .expect("fixture import mapping");
    let mut dataset_import = DatasetImport::queued(
        dataset_id,
        source_path.clone(),
        ImportFormat::Jsonl,
        mapping,
    )
    .expect("fixture import");
    dataset_import.state = ImportState::Completed;
    dataset_import.processed_rows = rows.len() as u64;
    dataset_import.accepted_rows = rows.len() as u64;
    store
        .create_import(&dataset_import)
        .await
        .expect("fixture import persisted");
    let mut imported_rows = Vec::with_capacity(rows.len());
    let mut source_rows = Vec::with_capacity(rows.len());
    for (index, (text, label, dimensions)) in rows.into_iter().enumerate() {
        let source_row_id = Uuid::new_v4();
        let created_at = Utc::now();
        let cell_key = GenerationCell {
            label: label.clone(),
            dimensions: dimensions.clone(),
        }
        .key();
        imported_rows.push(ImportedRow {
            id: source_row_id,
            import_id: dataset_import.id,
            dataset_id,
            source_row_number: index as u64 + 1,
            text: text.clone(),
            normalized_text: normalize_text(&text),
            label: label.clone(),
            dimensions: dimensions.clone(),
            cell_key: Some(cell_key),
            status: ImportRowStatus::Accepted,
            issues: Vec::new(),
            created_at,
        });
        let provenance = SourceProvenance::Imported {
            import_id: dataset_import.id,
            source_path: source_path.clone(),
            source_row_number: index as u64 + 1,
        };
        source_rows.push(SourceRow {
            id: source_row_id,
            dataset_id,
            text,
            label,
            dimensions,
            fields: BTreeMap::new(),
            provenance,
            created_at,
        });
    }
    store
        .insert_imported_rows(&dataset_import, &imported_rows)
        .await
        .expect("fixture source row persisted");
    let (snapshot, members) =
        build_snapshot(dataset_id, name, None, split, source_rows).expect("fixture snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot persisted");
    snapshot.id
}
