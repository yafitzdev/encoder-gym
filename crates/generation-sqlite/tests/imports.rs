use std::{collections::BTreeMap, io::Cursor, sync::Arc, time::Duration};

use dataset_core::{
    domain::{
        DatasetImport, ImportFieldMapping, ImportFormat, ImportRowStatus, ImportState,
        SourceProvenance, SplitConfiguration, SplitRatios,
    },
    ports::{AcceptedRowSource, ImportStore, SnapshotStore},
    splitting::build_snapshot,
};
use dataset_import::{CsvRecordReader, ImportProcessor};
use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition, GenerationParameters},
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy},
    planning::equal_target_plan,
    ports::{DatasetStore, JobStore, PlanStore, RowStore},
    validation::ValidationPipeline,
};
use generation_test_support::FakeGenerationBackend;
use synthetic_data_sqlite::SqliteStore;

#[tokio::test]
async fn imported_and_generated_rows_share_snapshots_with_provenance() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("imports.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let dataset = DatasetDefinition::new(
        "support",
        "classify support messages",
        vec!["billing".into(), "fraud".into()],
        vec![
            DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                .expect("dimension"),
        ],
    )
    .expect("dataset");
    store.create_dataset(&dataset).await.expect("dataset");

    let mapping = ImportFieldMapping::new(
        "message",
        "category",
        BTreeMap::from([("style".into(), "tone".into())]),
    )
    .expect("mapping");
    let mut dataset_import = DatasetImport::queued(
        dataset.id,
        "fixtures/support.csv",
        ImportFormat::Csv,
        mapping.clone(),
    )
    .expect("import");
    store
        .create_import(&dataset_import)
        .await
        .expect("create import");
    dataset_import.state = ImportState::Running;
    store
        .save_import(&dataset_import)
        .await
        .expect("running import");
    let source = concat!(
        "message,category,tone\n",
        "charged twice,billing,clean\n",
        "unknown example,other,clean\n",
        "charged twice,billing,clean\n"
    );
    let reader = CsvRecordReader::new(Cursor::new(source), mapping).expect("CSV reader");
    let mut processor = ImportProcessor::new(
        dataset.clone(),
        dataset_import.id,
        ["style".into()],
        Vec::<String>::new(),
    )
    .expect("processor");
    let rows = reader
        .map(|record| processor.process(record))
        .collect::<Vec<_>>();
    assert_eq!(rows[0].status, ImportRowStatus::Accepted);
    assert_eq!(rows[1].status, ImportRowStatus::Rejected);
    assert_eq!(rows[2].status, ImportRowStatus::Rejected);
    store
        .insert_imported_rows(&dataset_import, &rows)
        .await
        .expect("rows persist");
    dataset_import.state = ImportState::Completed;
    dataset_import.processed_rows = 3;
    dataset_import.accepted_rows = 1;
    dataset_import.rejected_rows = 2;
    store
        .save_import(&dataset_import)
        .await
        .expect("complete import");

    let plan = equal_target_plan(&dataset, 1).expect("plan");
    store.create_plan(&plan).await.expect("plan");
    let job = GenerationJob::queued(
        dataset.id,
        plan.id,
        "fake",
        "deterministic-v1",
        plan.total_target_count(),
    );
    store.create_job(&job).await.expect("job");
    JobRunner::new(
        Arc::new(store.clone()),
        Arc::new(FakeGenerationBackend::default()),
        JobRunnerPolicy {
            batch_size: 2,
            max_request_retries: 0,
            max_attempt_multiplier: 2,
            retry_delay: Duration::ZERO,
        },
        ValidationPipeline::standard(None),
    )
    .run(job.id, GenerationParameters::default())
    .await
    .expect("generation");

    let source_rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("source rows");
    assert_eq!(source_rows.len(), 4);
    assert_eq!(
        source_rows
            .iter()
            .filter(|row| matches!(row.provenance, SourceProvenance::Imported { .. }))
            .count(),
        1
    );
    assert_eq!(
        source_rows
            .iter()
            .filter(|row| matches!(row.provenance, SourceProvenance::Generated { .. }))
            .count(),
        3
    );
    let billing_clean_key = plan
        .cells
        .iter()
        .find(|cell| cell.cell.label == "billing" && cell.cell.dimensions["style"] == "clean")
        .expect("billing clean")
        .cell
        .key();
    assert_eq!(
        store
            .dataset_cell_counts(dataset.id)
            .await
            .expect("coverage")[&billing_clean_key]
            .accepted,
        1
    );

    let (snapshot, members) = build_snapshot(
        dataset.id,
        "combined",
        None,
        SplitConfiguration::new(SplitRatios::new(0.75, 0.0, 0.25).expect("ratios"), 9),
        source_rows,
    )
    .expect("snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot persists");
    let persisted = store
        .list_snapshot_members(snapshot.id)
        .await
        .expect("snapshot members");
    assert!(persisted.iter().any(|member| matches!(
        member.source_provenance,
        SourceProvenance::Imported {
            source_row_number: 2,
            ..
        }
    )));
    let foreign_key_errors: Vec<(String, i64, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(store.pool())
            .await
            .expect("foreign key check");
    assert!(foreign_key_errors.is_empty());
}
