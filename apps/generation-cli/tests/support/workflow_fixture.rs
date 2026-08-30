use std::path::{Path, PathBuf};

use chrono::Utc;
use dataset_core::domain::{SplitConfiguration, SplitRatios};
use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition},
    ports::DatasetStore,
};
use project_config::GenerationBackendKind;
use project_preparation::PreparationManifest;
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use tempfile::TempDir;
use uuid::Uuid;

use super::run_json;

pub enum GenerationMode {
    Fake,
    OpenAiCompatible { base_url: String, model: String },
}

pub struct WorkflowFixture {
    _directory: TempDir,
    database_url: String,
    manifest_path: PathBuf,
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
        let source = include_str!("../../../../examples/project-preparation.toml").replace(
            "00000000-0000-0000-0000-000000000000",
            &development_snapshot_id.to_string(),
        );
        let mut manifest = PreparationManifest::parse_toml(&source).expect("example manifest");
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
    let development = insert_snapshot(
        &store,
        dataset.id,
        "development fixture",
        "Why was I charged twice?",
        "billing",
        r#"{"difficulty":"easy","writing_style":"clean","ambiguity":"obvious"}"#,
    )
    .await;
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
    let snapshot_id = Uuid::new_v4();
    let split = SplitConfiguration::new(SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"), 42);
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, member_count, \
          fingerprint, created_at) VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset_id)
    .bind(name)
    .bind(serde_json::to_string(&split).expect("split JSON"))
    .bind(1_i64)
    .bind(format!("sha256:{snapshot_id}"))
    .bind(Utc::now())
    .execute(store.pool())
    .await
    .expect("snapshot persisted");
    let source_row_id = Uuid::new_v4();
    let created_at = Utc::now();
    let provenance = format!(
        r#"{{"kind":"generated","generation_job_id":"{}","backend":"fake","model":"deterministic-v1"}}"#,
        Uuid::nil()
    );
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) \
         VALUES (?, ?, 'generated', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(source_row_id)
    .bind(dataset_id)
    .bind(source_row_id.to_string())
    .bind(format!("{label}/{source_row_id}"))
    .bind(text)
    .bind(text.to_ascii_lowercase())
    .bind(label)
    .bind(dimensions)
    .bind(&provenance)
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("source row persisted");
    sqlx::query(
        "INSERT INTO dataset_snapshot_members \
         (id, snapshot_id, source_row_id, split, text, label, dimensions_json, \
          source_provenance_json, source_created_at) VALUES (?, ?, ?, 'test', ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4())
    .bind(snapshot_id)
    .bind(source_row_id)
    .bind(text)
    .bind(label)
    .bind(dimensions)
    .bind(provenance)
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("snapshot member persisted");
    snapshot_id
}
