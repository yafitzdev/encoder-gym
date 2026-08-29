use std::collections::BTreeMap;

use artifact_core::{ArtifactKind, ProvenanceStore};
use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceProvenance, SplitConfiguration,
        SplitRatios,
    },
    ports::SnapshotStore,
};
use generation_core::ports::DatasetStore;
use project_config::{ProjectConfig, ProjectOverrides};
use project_preparation::{
    CohortEvidence, CohortManifest, ContaminationManifest, PreparationEvidence,
    PreparationManifest, PreparationStore, SuiteManifest, WorkflowManifest, compile_project,
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    allocation::InitialAllocationPolicy,
    benchmark::AcceptanceContract,
    governance::{CohortOrigin, CohortRole, DisclosureLevel},
    workflow::{IterationGovernance, WorkflowBudget, WorkflowPolicy},
};

const PROJECT: &str = r#"
version = 1

[dataset]
name = "support"
task = "Classify support requests"
labels = ["billing", "fraud"]

[[dataset.dimensions]]
name = "style"
values = ["clean", "messy"]

[generation]
target_per_cell = 10
batch_size = 20
"#;

#[tokio::test]
async fn preparation_bundle_is_atomic_queryable_and_idempotent_by_manifest() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("preparation.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = fixture(&store).await;

    let mut stale_evidence = evidence.clone();
    stale_evidence
        .cohorts
        .values_mut()
        .next()
        .expect("cohort")
        .snapshot
        .fingerprint = "sha256:changed-after-load".into();
    let stale_bundle = compile_project(&manifest, &stale_evidence).expect("stale bundle compiles");
    assert!(store.create_preparation(&stale_bundle).await.is_err());
    assert_eq!(table_count(&store, "dataset_definitions").await, 1);
    assert_eq!(table_count(&store, "project_configurations").await, 0);
    assert_eq!(table_count(&store, "workflow_evaluation_cohorts").await, 0);

    let first_bundle = compile_project(&manifest, &evidence).expect("bundle");
    let first = store
        .create_preparation(&first_bundle)
        .await
        .expect("preparation persists");
    let second_bundle = compile_project(&manifest, &evidence).expect("fresh bundle");
    let second = store
        .create_preparation(&second_bundle)
        .await
        .expect("repeat is idempotent");

    assert_eq!(second, first);
    assert_ne!(second_bundle.preparation.id, first.id);
    assert_eq!(
        store
            .get_preparation(first.id)
            .await
            .expect("query")
            .expect("preparation"),
        first
    );
    assert_eq!(
        store
            .get_preparation_by_manifest(&first.manifest_fingerprint)
            .await
            .expect("query by manifest")
            .expect("preparation"),
        first
    );
    assert_eq!(
        store
            .list_preparations(10, 0)
            .await
            .expect("list preparations"),
        vec![first.clone()]
    );
    assert_eq!(table_count(&store, "project_preparations").await, 1);
    assert_eq!(table_count(&store, "project_configurations").await, 1);
    assert_eq!(table_count(&store, "workflow_evaluation_cohorts").await, 1);
    assert_eq!(
        table_count(&store, "workflow_contamination_reports").await,
        1
    );
    assert_eq!(table_count(&store, "workflow_benchmark_suites").await, 1);
    assert_eq!(table_count(&store, "workflow_definitions").await, 1);
    assert!(
        store
            .get_dataset(first.dataset_id)
            .await
            .expect("dataset query")
            .is_some()
    );
    let provenance = store
        .trace_provenance(ArtifactKind::ProjectPreparation, first.id)
        .await
        .expect("provenance query")
        .expect("preparation provenance");
    assert_eq!(provenance.kind, ArtifactKind::ProjectPreparation);
    assert_eq!(provenance.parents[0].kind, ArtifactKind::WorkflowDefinition);
}

async fn fixture(store: &SqliteStore) -> (PreparationManifest, PreparationEvidence) {
    let project = ProjectConfig::parse(PROJECT).expect("project config");
    let source_dataset = project
        .clone()
        .resolve(ProjectOverrides::default())
        .expect("resolve")
        .dataset_definition()
        .expect("dataset");
    store
        .create_dataset(&source_dataset)
        .await
        .expect("source dataset");
    let now = Utc::now();
    let source_row_id = Uuid::new_v4();
    let provenance = SourceProvenance::Imported {
        import_id: Uuid::new_v4(),
        source_path: "benchmark.jsonl".into(),
        source_row_number: 1,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(source_row_id)
    .bind(source_dataset.id)
    .bind(source_row_id.to_string())
    .bind("billing/style=clean")
    .bind("Why was I charged twice?")
    .bind("why was i charged twice?")
    .bind("billing")
    .bind(r#"{"style":"clean"}"#)
    .bind(serde_json::to_string(&provenance).expect("provenance"))
    .bind(now)
    .execute(store.pool())
    .await
    .expect("source row");
    let snapshot_id = Uuid::new_v4();
    let snapshot = DatasetSnapshot {
        id: snapshot_id,
        source_dataset_id: source_dataset.id,
        name: "development snapshot".into(),
        description: None,
        split_configuration: SplitConfiguration::new(
            SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"),
            42,
        ),
        member_count: 1,
        fingerprint: "sha256:development".into(),
        created_at: now,
    };
    let member = SnapshotMember {
        id: Uuid::new_v4(),
        snapshot_id,
        source_row_id,
        split: SnapshotSplit::Test,
        text: "Why was I charged twice?".into(),
        label: "billing".into(),
        dimensions: BTreeMap::from([("style".into(), "clean".into())]),
        source_provenance: provenance,
        source_created_at: now,
    };
    store
        .create_snapshot(&snapshot, std::slice::from_ref(&member))
        .await
        .expect("snapshot");
    let manifest = PreparationManifest {
        version: 1,
        name: "support encoder".into(),
        project,
        contamination: ContaminationManifest::default(),
        development: SuiteManifest {
            name: "development".into(),
            required_model_formats: Vec::new(),
            cohorts: vec![CohortManifest {
                name: "development test".into(),
                snapshot_id,
                split: SnapshotSplit::Test,
                origin: CohortOrigin::InternalSnapshot,
                role: CohortRole::Development,
                protocol: None,
                disclosure: DisclosureLevel::Predictions,
                adaptation_eligible: true,
            }],
            contract: AcceptanceContract {
                metric_requirements: Vec::new(),
                regression: None,
            },
        },
        sealed: None,
        workflow: WorkflowManifest {
            name: "support workflow".into(),
            total_rows: 100,
            reserved_rows: 20,
            allocation_policy: InitialAllocationPolicy::Balanced,
            allocation_constraints: Vec::new(),
            analysis_protocol: None,
            optimization_protocol: None,
            advisor: None,
            training_iteration_policy: None,
            governance: IterationGovernance::ReviewEachIteration,
            budget: WorkflowBudget {
                maximum_iterations: 2,
                maximum_initial_rows: 100,
                maximum_cumulative_rows: 120,
                maximum_generation_attempts: 500,
                maximum_generation_requests: 100,
                maximum_advisor_calls: 0,
                maximum_advisor_tokens: None,
                maximum_stage_attempts: 3,
            },
            policy: WorkflowPolicy {
                minimum_improvement: 0.01,
                maximum_tolerated_regression: 0.01,
                stop_on_inconclusive: true,
                stop_on_invalid: true,
                enable_advisor: false,
                require_fresh_development_cohort_after_iterations: None,
            },
        },
    };
    (
        manifest,
        PreparationEvidence {
            cohorts: BTreeMap::from([(
                snapshot_id,
                CohortEvidence {
                    snapshot,
                    source_dataset,
                    members: vec![member],
                },
            )]),
        },
    )
}

async fn table_count(store: &SqliteStore, table: &str) -> i64 {
    let query = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar(&query)
        .fetch_one(store.pool())
        .await
        .expect("count")
}
