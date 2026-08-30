use std::collections::BTreeMap;

use chrono::Utc;
use dataset_core::{
    domain::{SnapshotSplit, SplitConfiguration, SplitRatios},
    ports::{AcceptedRowSource, SnapshotStore},
    splitting::build_snapshot,
};
use generation_core::{
    coverage::calculate_coverage,
    deduplication::normalize_text,
    domain::{
        BackendConfiguration, DatasetDefinition, DimensionDefinition, GeneratedRow,
        GenerationParameters, GenerationPlan, ValidationStatus,
    },
    jobs::{GenerationJob, JobState},
    planning::equal_target_plan,
    ports::{BackendConfigurationStore, DatasetStore, JobStore, PlanStore, RowQuery, RowStore},
};
use synthetic_data_sqlite::SqliteStore;
use tempfile::TempDir;
use uuid::Uuid;

async fn store() -> (TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("test.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

fn dataset() -> DatasetDefinition {
    DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into(), "fraud".into()],
        vec![
            DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                .expect("valid dimension"),
        ],
    )
    .expect("valid dataset")
}

#[tokio::test]
async fn round_trips_all_slice_one_persistence_shapes() {
    let (_directory, store) = store().await;
    let dataset = dataset();
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    assert_eq!(
        store.get_dataset(dataset.id).await.expect("dataset read"),
        Some(dataset.clone())
    );

    let plan = equal_target_plan(&dataset, 2).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    assert_eq!(
        store.get_plan(plan.id).await.expect("plan read"),
        Some(plan.clone())
    );

    let mut job = GenerationJob::queued(dataset.id, plan.id, "fake", "fake-v1", 4);
    store.create_job(&job).await.expect("job persisted");
    job.transition(JobState::Running).expect("valid transition");
    job.generated_rows = 2;
    job.accepted_rows = 1;
    job.rejected_rows = 1;
    store.save_job(&job).await.expect("job updated");
    assert_eq!(
        store.get_job(job.id).await.expect("job read"),
        Some(job.clone())
    );

    let accepted = row(&dataset, &plan, &job, 0, ValidationStatus::Accepted);
    let rejected = row(&dataset, &plan, &job, 1, ValidationStatus::Rejected);
    store
        .insert_rows(&[accepted.clone(), rejected.clone()])
        .await
        .expect("rows persisted");
    let rows = store
        .list_rows(RowQuery {
            dataset_id: Some(dataset.id),
            limit: 100,
            ..RowQuery::default()
        })
        .await
        .expect("rows read");
    assert_eq!(rows, vec![accepted.clone(), rejected]);
    assert_eq!(
        store
            .accepted_normalized_texts(dataset.id)
            .await
            .expect("text read"),
        vec![accepted.normalized_text]
    );

    let counts = store.cell_counts(plan.id).await.expect("counts read");
    let coverage = calculate_coverage(&plan, &counts);
    assert_eq!(coverage[0].attempted, 2);
    assert_eq!(coverage[0].accepted, 1);
    assert_eq!(coverage[0].remaining, 1);

    let configuration = BackendConfiguration {
        name: "openai-compatible".into(),
        base_url: Some("http://localhost:1234/v1".into()),
        model: "local-model".into(),
        parameters: GenerationParameters::default(),
        updated_at: Utc::now(),
    };
    store
        .save_backend_configuration(&configuration)
        .await
        .expect("configuration persisted");
    assert_eq!(
        store
            .get_backend_configuration("openai-compatible")
            .await
            .expect("configuration read"),
        Some(configuration)
    );
}

#[tokio::test]
async fn cancellation_is_durable() {
    let (_directory, store) = store().await;
    let dataset = dataset();
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let plan = equal_target_plan(&dataset, 1).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "fake-v1", 4);
    store.create_job(&job).await.expect("job persisted");

    assert!(
        store
            .request_job_cancellation(job.id)
            .await
            .expect("cancellation update")
    );
    assert!(
        store
            .get_job(job.id)
            .await
            .expect("job read")
            .expect("job exists")
            .cancel_requested
    );
}

#[tokio::test]
async fn accepted_rows_become_an_immutable_snapshot_with_provenance() {
    let (_directory, store) = store().await;
    let dataset = dataset();
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let plan = equal_target_plan(&dataset, 2).expect("valid plan");
    store.create_plan(&plan).await.expect("plan persisted");
    let job = GenerationJob::queued(dataset.id, plan.id, "fake", "fake-v1", 2);
    store.create_job(&job).await.expect("job persisted");
    let accepted = row(&dataset, &plan, &job, 0, ValidationStatus::Accepted);
    let rejected = row(&dataset, &plan, &job, 1, ValidationStatus::Rejected);
    store
        .insert_rows(&[accepted.clone(), rejected])
        .await
        .expect("rows persisted");

    let source_rows = store
        .list_accepted_source_rows(dataset.id)
        .await
        .expect("accepted source rows");
    assert_eq!(source_rows.len(), 1);
    assert_eq!(source_rows[0].id, accepted.id);
    let (snapshot, members) = build_snapshot(
        dataset.id,
        "first snapshot",
        Some("training baseline".into()),
        SplitConfiguration::new(SplitRatios::new(1.0, 0.0, 0.0).expect("ratios"), 42),
        source_rows,
    )
    .expect("snapshot builds");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot persisted");

    assert_eq!(
        store
            .get_snapshot(snapshot.id)
            .await
            .expect("snapshot read"),
        Some(snapshot.clone())
    );
    assert_eq!(
        store.list_snapshots().await.expect("snapshots"),
        vec![snapshot]
    );
    let persisted_members = store
        .list_snapshot_members(members[0].snapshot_id)
        .await
        .expect("members read");
    assert_eq!(persisted_members, members);
    assert_eq!(persisted_members[0].source_row_id, accepted.id);
    assert_eq!(persisted_members[0].split, SnapshotSplit::Train);
}

fn row(
    dataset: &DatasetDefinition,
    plan: &GenerationPlan,
    job: &GenerationJob,
    sequence: usize,
    status: ValidationStatus,
) -> GeneratedRow {
    let text = format!("example {sequence}");
    GeneratedRow {
        id: Uuid::new_v4(),
        dataset_id: dataset.id,
        plan_id: plan.id,
        generation_job_id: job.id,
        cell_key: plan.cells[0].cell.key(),
        text: text.clone(),
        normalized_text: normalize_text(&text),
        label: plan.cells[0].cell.label.clone(),
        dimensions: BTreeMap::from([("style".into(), "clean".into())]),
        fields: BTreeMap::from([("ticket_id".into(), serde_json::json!(sequence))]),
        construction: None,
        generator_backend: "fake".into(),
        generator_model: "fake-v1".into(),
        created_at: Utc::now(),
        validation_status: status,
        validation_errors: if status == ValidationStatus::Rejected {
            vec!["duplicate".into()]
        } else {
            vec![]
        },
        generation_metadata: serde_json::json!({"batch": 1}),
    }
}
