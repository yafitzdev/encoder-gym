use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition},
    ports::{DatasetStore, PlanStore},
};
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    allocation::{
        InitialAllocationPolicy, InitialAllocationRecord, InitialAllocationRequest,
        allocate_initial_budget,
    },
    ports::{InitialAllocationQuery, InitialAllocationStore},
};

#[tokio::test]
async fn allocation_and_ordinary_plan_are_persisted_atomically() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("allocation.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let dataset = DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into(), "fraud".into()],
        vec![
            DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                .expect("dimension"),
        ],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let result = allocate_initial_budget(
        &dataset,
        InitialAllocationRequest {
            total_rows: 20_000,
            reserved_rows: 2_000,
            policy: InitialAllocationPolicy::Balanced,
            current_coverage: Vec::new(),
            constraints: Vec::new(),
        },
    )
    .expect("allocation");
    let plan = result.to_generation_plan(&dataset).expect("plan");
    let record = InitialAllocationRecord::new(result, plan.id).expect("record");

    store
        .create_initial_allocation(&record, &plan)
        .await
        .expect("allocation persisted");

    assert_eq!(
        store
            .get_initial_allocation(record.id)
            .await
            .expect("allocation read"),
        Some(record.clone())
    );
    assert_eq!(
        store
            .get_plan(plan.id)
            .await
            .expect("plan read")
            .expect("plan persists"),
        plan
    );
    assert_eq!(
        store
            .query_initial_allocations(InitialAllocationQuery {
                dataset_id: Some(dataset.id),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("allocations queried"),
        vec![record.clone()]
    );

    let mut tampered = record;
    tampered.result.cells[0].target += 1;
    let tampered_plan = tampered
        .result
        .to_generation_plan(&dataset)
        .expect("still structurally feasible");
    assert!(
        store
            .create_initial_allocation(&tampered, &tampered_plan)
            .await
            .is_err()
    );
    assert!(
        store
            .get_plan(tampered_plan.id)
            .await
            .expect("plan read")
            .is_none(),
        "failed allocation validation must not leave a plan"
    );
}
