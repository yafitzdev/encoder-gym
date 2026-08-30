pub mod support;

use chrono::Utc;
use dataset_core::domain::{SplitConfiguration, SplitRatios};
use generation_core::{
    domain::{DatasetDefinition, DimensionDefinition},
    ports::DatasetStore,
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use support::run_json;

#[test]
fn manifest_previews_prepares_idempotently_and_yields_a_startable_workflow() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("project-preparation-cli.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let snapshot_id = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(create_snapshot_fixture(&database_url));
    let manifest_path = directory.path().join("project-preparation.toml");
    std::fs::write(
        &manifest_path,
        include_str!("../../../examples/project-preparation.toml").replace(
            "00000000-0000-0000-0000-000000000000",
            &snapshot_id.to_string(),
        ),
    )
    .expect("manifest written");
    let manifest = manifest_path.to_str().expect("UTF-8 path");

    let preview = run_json(&database_url, ["project", "preview", manifest]);
    assert_eq!(preview["eligible"], true);
    assert_eq!(preview["requested_total_rows"], 1_000);
    assert_eq!(preview["initial_target_rows"], 900);
    assert_eq!(preview["cells"].as_array().expect("cells").len(), 48);
    assert_eq!(
        preview["cells"]
            .as_array()
            .expect("cells")
            .iter()
            .map(|cell| cell["target"].as_u64().expect("target"))
            .sum::<u64>(),
        900
    );
    assert_eq!(
        run_json(&database_url, ["project", "list"]),
        serde_json::json!([]),
        "preview must not persist preparation artifacts"
    );

    let created = run_json(&database_url, ["project", "prepare", manifest]);
    assert_eq!(created["created"], true);
    let preparation_id = created["preparation"]["id"]
        .as_str()
        .expect("preparation ID");
    let definition_id = created["preparation"]["workflow_definition_id"]
        .as_str()
        .expect("definition ID");
    assert_eq!(
        created["next_command"],
        format!("synth workflow start {definition_id}")
    );
    let repeated = run_json(&database_url, ["project", "prepare", manifest]);
    assert_eq!(repeated["created"], false);
    assert_eq!(repeated["preparation"]["id"], preparation_id);
    assert_eq!(
        run_json(&database_url, ["project", "list"])
            .as_array()
            .expect("preparations")
            .len(),
        1
    );
    let shown = run_json(&database_url, ["project", "show", preparation_id]);
    assert_eq!(shown["workflow_definition"]["id"], definition_id);
    assert_eq!(
        run_json(&database_url, ["workflow", "list"]),
        serde_json::json!([]),
        "preparation must not start execution"
    );

    let started = run_json(
        &database_url,
        ["workflow", "start", definition_id, "--initialize-only"],
    );
    assert_eq!(started["run"]["definition_id"], definition_id);
    assert_eq!(started["attempt"]["stage"], "initial_allocation");
}

async fn create_snapshot_fixture(database_url: &str) -> Uuid {
    let store = SqliteStore::connect(database_url)
        .await
        .expect("database connects");
    let dataset = DatasetDefinition::new(
        "support benchmark",
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
    let snapshot_id = Uuid::new_v4();
    let split = SplitConfiguration::new(SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"), 42);
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, member_count, \
          fingerprint, created_at) VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset.id)
    .bind("project preparation CLI fixture")
    .bind(serde_json::to_string(&split).expect("split JSON"))
    .bind(1_i64)
    .bind("sha256:project-preparation-cli-fixture")
    .bind(Utc::now())
    .execute(store.pool())
    .await
    .expect("snapshot persisted");
    let source_row_id = Uuid::new_v4();
    let member_id = Uuid::new_v4();
    let created_at = Utc::now();
    let dimensions = r#"{"difficulty":"easy","writing_style":"clean","ambiguity":"obvious"}"#;
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
    .bind(dataset.id)
    .bind(source_row_id.to_string())
    .bind("billing/easy/clean/obvious")
    .bind("Why was I charged twice?")
    .bind("why was i charged twice?")
    .bind("billing")
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
    .bind(member_id)
    .bind(snapshot_id)
    .bind(source_row_id)
    .bind("Why was I charged twice?")
    .bind("billing")
    .bind(dimensions)
    .bind(provenance)
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("snapshot member persisted");
    snapshot_id
}
