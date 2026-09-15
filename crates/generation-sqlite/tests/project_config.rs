use generation_core::{
    domain::{GenerationPlan, PlannedCell},
    ports::{BackendConfigurationStore, DatasetStore, PlanStore},
};
use project_config::{ProjectConfig, ProjectInitializer, ProjectOverrides};
use synthetic_data_sqlite::SqliteStore;

const CONFIG: &str = include_str!("fixtures/project.toml");

#[tokio::test]
async fn project_initialization_is_atomic_and_persists_resolved_settings() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("project.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let resolved = ProjectConfig::parse(CONFIG)
        .expect("example parses")
        .resolve(ProjectOverrides {
            target_per_cell: Some(7),
            ..ProjectOverrides::default()
        })
        .expect("example resolves");
    let dataset = resolved.dataset_definition().expect("dataset");
    let plan = resolved.generation_plan(&dataset).expect("plan");
    let configuration = resolved
        .persisted(dataset.id, plan.id)
        .expect("persisted configuration");

    store
        .initialize_project(
            &dataset,
            &plan,
            resolved.backend_configuration().as_ref(),
            &configuration,
        )
        .await
        .expect("project initializes");

    assert_eq!(
        store
            .get_dataset(dataset.id)
            .await
            .expect("dataset query")
            .expect("dataset persists"),
        dataset
    );
    assert_eq!(
        store
            .get_plan(plan.id)
            .await
            .expect("plan query")
            .expect("plan persists")
            .total_target_count(),
        48 * 7
    );
    assert!(
        store
            .get_backend_configuration("openai-compatible")
            .await
            .expect("backend query")
            .is_none(),
        "fake backend configuration must not be persisted"
    );

    let second_dataset = resolved.dataset_definition().expect("second dataset");
    let second_plan = resolved
        .generation_plan(&second_dataset)
        .expect("second plan");
    let conflicting_plan = GenerationPlan::with_identity(
        plan.id,
        second_dataset.id,
        second_plan
            .cells
            .into_iter()
            .map(|cell| PlannedCell { ..cell })
            .collect(),
        second_plan.created_at,
    )
    .expect("conflicting plan");
    let second_configuration = resolved
        .persisted(second_dataset.id, conflicting_plan.id)
        .expect("second configuration");
    assert!(
        store
            .initialize_project(
                &second_dataset,
                &conflicting_plan,
                None,
                &second_configuration,
            )
            .await
            .is_err()
    );
    assert!(
        store
            .get_dataset(second_dataset.id)
            .await
            .expect("dataset query")
            .is_none(),
        "failed plan insert must roll back the preceding dataset insert"
    );
}
