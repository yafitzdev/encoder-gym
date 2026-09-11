use std::{fs, path::Path};

use project_workspace_core::DatasetPurpose;
use project_workspace_local::dataset_versions::{
    self as datasets, DatasetVersionUpdate, RecordSelection,
};
use project_workspace_local::{create_workspace, import_dataset, inspect_dataset, inspect_model};
use serde_json::json;
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;
use uuid::Uuid;

async fn fixture(root: &Path) -> std::path::PathBuf {
    let source = root.join("model");
    training_transformer::fixture::write_tiny_bert_bundle(&source).unwrap();
    let checkpoint = inspect_model(&source).unwrap();
    let destination = root.join("workspace");
    create_workspace(
        &destination,
        "Versions",
        &source,
        &checkpoint.fingerprint,
        None,
    )
    .await
    .unwrap();
    destination
}

async fn import(
    folder: &Path,
    root: &Path,
    name: &str,
    rows: &[serde_json::Value],
    purpose: DatasetPurpose,
) -> Uuid {
    let source = root.join(format!("{name}.jsonl"));
    fs::write(
        &source,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let preview = inspect_dataset(&source, purpose).unwrap();
    let workspace = import_dataset(
        folder,
        &source,
        name,
        purpose,
        &preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    workspace
        .datasets
        .iter()
        .find(|d| d.name == name)
        .unwrap()
        .id
}

#[tokio::test]
async fn base_variant_edit_rows_and_immutable_history_survive_reopen() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let input = import(
        &folder,
        temp.path(),
        "base",
        &[json!({"text":"first"}), json!({"text":"second"})],
        DatasetPurpose::Training,
    )
    .await;
    let added = import(
        &folder,
        temp.path(),
        "extra",
        &[json!({"text":"third"})],
        DatasetPurpose::Training,
    )
    .await;
    let base_id = Uuid::new_v4();
    let initial_id = Uuid::new_v4();
    let base = datasets::create_base(&folder, base_id, initial_id, "Base dataset", &[input])
        .await
        .unwrap();
    assert_eq!(
        datasets::create_base(&folder, base_id, initial_id, "Base dataset", &[input])
            .await
            .unwrap(),
        base
    );
    assert!(
        datasets::create_base(
            &folder,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Another base",
            &[input]
        )
        .await
        .is_err()
    );
    let variant_id = Uuid::new_v4();
    let variant = datasets::fork(&folder, variant_id, Uuid::new_v4(), "Variant 1", base.id)
        .await
        .unwrap();
    let update = DatasetVersionUpdate {
        version_id: Uuid::new_v4(),
        dataset_id: variant_id,
        parent_id: variant.id,
        added: vec![RecordSelection {
            import_id: added,
            record: 1,
        }],
        removed: vec![base.members[0].id.clone()],
        replaced: vec![],
    };
    let changed = datasets::revise(&folder, update.clone()).await.unwrap();
    assert_eq!(
        datasets::revise(&folder, update.clone()).await.unwrap(),
        changed
    );
    assert_eq!(
        datasets::inspect(&folder, changed.id).await.unwrap(),
        changed
    );
    assert_eq!(datasets::inspect(&folder, base.id).await.unwrap(), base);
    assert_eq!(changed.members[0].id, base.members[1].id);
    let page = datasets::read_rows(&folder, changed.id, 0, 1)
        .await
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows[0].value, json!({"text":"second"}));
    assert_eq!(
        datasets::read_rows(&folder, changed.id, 1, 1)
            .await
            .unwrap()
            .rows[0]
            .value,
        json!({"text":"third"})
    );
    assert!(
        datasets::read_rows(&folder, changed.id, 0, 101)
            .await
            .is_err()
    );
    let mut stale = update.clone();
    stale.version_id = Uuid::new_v4();
    assert!(datasets::revise(&folder, stale).await.is_err());
    let mut replay = update;
    replay.removed.clear();
    assert!(datasets::revise(&folder, replay).await.is_err());
    let catalog = datasets::list(&folder).await.unwrap();
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog[1].versions[0].added, 1);
    assert_eq!(catalog[1].versions[0].removed, 1);
    assert_eq!(catalog[1].versions[0].rows, 2);
    let changes = datasets::read_changes(&folder, changed.id, 0, 50)
        .await
        .unwrap();
    assert_eq!(changes.total, 2);
    assert_eq!(changes.changes[0].kind, "added");
    assert_eq!(
        changes.changes[0].after.as_ref().unwrap().value,
        json!({"text":"third"})
    );
    assert_eq!(changes.changes[1].kind, "removed");
    assert_eq!(
        changes.changes[1].before.as_ref().unwrap().value,
        json!({"text":"first"})
    );
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE dataset_versions SET number=99")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM dataset_branches")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut database)
            .await
            .unwrap()
            .is_empty()
    );
    database.close().await.unwrap();
}

#[tokio::test]
async fn changed_source_bytes_are_not_silently_reinterpreted_as_version_rows() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let input = import(
        &folder,
        temp.path(),
        "source",
        &[json!({"text":"first"}), json!({"text":"second"})],
        DatasetPurpose::Training,
    )
    .await;
    let version = datasets::create_base(&folder, Uuid::new_v4(), Uuid::new_v4(), "Base", &[input])
        .await
        .unwrap();
    let workspace = project_workspace_local::open_workspace(&folder, false)
        .await
        .unwrap();
    let source = &workspace.datasets[0].artifact;
    let original = fs::read(folder.join(&source.path)).unwrap();
    let changed = String::from_utf8(original)
        .unwrap()
        .replace("first", "other");
    fs::write(folder.join(&source.path), changed).unwrap();
    assert!(
        datasets::read_rows(&folder, version.id, 0, 1)
            .await
            .is_err()
    );
    // An unrequested row changing still invalidates the complete source.
    assert!(
        datasets::read_rows(&folder, version.id, 1, 1)
            .await
            .is_err()
    );
    assert!(
        datasets::fork(
            &folder,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Variant",
            version.id
        )
        .await
        .is_err()
    );
    assert_eq!(datasets::list(&folder).await.unwrap().len(), 1);
}

#[tokio::test]
async fn foreign_and_protected_sources_never_enter_dataset_versions() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let sealed = import(
        &folder,
        temp.path(),
        "sealed",
        &[json!({"text":"protected", "sealed":true})],
        DatasetPurpose::Sealed,
    )
    .await;
    assert!(
        datasets::create_base(&folder, Uuid::new_v4(), Uuid::new_v4(), "Bad", &[sealed])
            .await
            .is_err()
    );
    assert!(
        datasets::create_base(
            &folder,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Missing",
            &[Uuid::new_v4()]
        )
        .await
        .is_err()
    );
    assert!(datasets::list(&folder).await.unwrap().is_empty());
    let training = import(
        &folder,
        temp.path(),
        "train",
        &[json!({"text":"allowed"})],
        DatasetPurpose::Training,
    )
    .await;
    let version =
        datasets::create_base(&folder, Uuid::new_v4(), Uuid::new_v4(), "Good", &[training])
            .await
            .unwrap();
    let other_temp = TempDir::new().unwrap();
    let other = fixture(other_temp.path()).await;
    assert!(datasets::inspect(&other, version.id).await.is_err());
    assert!(
        datasets::fork(
            &other,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Foreign",
            version.id
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn sparse_pages_preserve_source_positions_and_exact_content_fingerprints() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let values = (0..8).map(|index| json!({"text": format!("Row {index}"), "native": {"registry": [index, index + 1]}})).collect::<Vec<_>>();
    let input = import(
        &folder,
        temp.path(),
        "paged",
        &values,
        DatasetPurpose::Training,
    )
    .await;
    let version = datasets::create_base(&folder, Uuid::new_v4(), Uuid::new_v4(), "Base", &[input])
        .await
        .unwrap();
    let page = datasets::read_rows(&folder, version.id, 3, 2)
        .await
        .unwrap();
    assert_eq!(page.total, 8);
    assert_eq!(page.rows.len(), 2);
    for (offset, row) in page.rows.iter().enumerate() {
        assert_eq!(row.member, version.members[offset + 3]);
        assert_eq!(row.value, values[offset + 3]);
        assert_eq!(row.member.source.record, offset as u64 + 4);
        assert_eq!(
            row.member.content_fingerprint,
            artifact_core::fingerprint(&row.value).unwrap()
        );
    }
    assert!(
        datasets::read_rows(&folder, version.id, 8, 2)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
}

#[tokio::test]
async fn training_materialization_is_not_limited_to_one_ui_page() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let payload = "x".repeat(6 * 1_048_576);
    let values = (0..3)
        .map(|index| json!({"text": payload, "index": index}))
        .collect::<Vec<_>>();
    let input = import(
        &folder,
        temp.path(),
        "large-training-input",
        &values,
        DatasetPurpose::Training,
    )
    .await;
    let version = datasets::create_base(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Large training dataset",
        &[input],
    )
    .await
    .unwrap();

    assert!(
        datasets::read_rows(&folder, version.id, 0, 3)
            .await
            .is_err()
    );
    let materialized = datasets::materialization_rows(&folder, version.id)
        .await
        .unwrap();
    assert_eq!(materialized.len(), 3);
    assert_eq!(materialized[2].value, values[2]);
}
