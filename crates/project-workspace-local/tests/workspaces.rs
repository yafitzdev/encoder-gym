use project_workspace_core::{DatasetPurpose, MANIFEST};
use project_workspace_local::{
    backfill_nomos, create_workspace, import_dataset, inspect_dataset, inspect_model,
    open_workspace, upgrade_workspace,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn model(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("source-model");
    training_transformer::fixture::write_tiny_bert_bundle(&path).unwrap();
    path
}

#[tokio::test]
async fn create_import_move_and_verify_without_original_sources() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("Managed project");
    let created = create_workspace(
        &destination,
        "My encoder",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    assert_eq!(created.manifest.baseline, preview);
    let catalog = created.model_catalog.as_ref().unwrap();
    assert_eq!(catalog.active_model().fingerprint, preview.fingerprint);
    assert_eq!(catalog.baseline_revisions.len(), 1);
    assert!(created.datasets.is_empty());
    assert!(created.verified);
    assert!(
        create_workspace(&destination, "Other", &source, &preview.fingerprint, None)
            .await
            .is_err()
    );
    let data = temp.path().join("input.jsonl");
    fs::write(&data, "{\"text\":\"first\"}\n{\"text\":\"second\"}\n").unwrap();
    let data_preview = inspect_dataset(&data, DatasetPurpose::Training).unwrap();
    let imported = import_dataset(
        &destination,
        &data,
        "Local input",
        DatasetPurpose::Training,
        &data_preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    assert_eq!(imported.datasets[0].rows, 2);
    let again = import_dataset(
        &destination,
        &data,
        "Different display name",
        DatasetPurpose::Training,
        &data_preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    assert_eq!(again.datasets, imported.datasets);
    assert!(
        import_dataset(
            &destination,
            &data,
            "Conflict",
            DatasetPurpose::Development,
            &data_preview.artifact.fingerprint
        )
        .await
        .is_err()
    );
    let moved = temp.path().join("moved");
    fs::rename(&destination, &moved).unwrap();
    fs::remove_dir_all(&source).unwrap();
    fs::remove_file(&data).unwrap();
    let reopened = open_workspace(&moved, true).await.unwrap();
    assert_eq!(reopened.manifest.id, created.manifest.id);
    assert_eq!(reopened.datasets, imported.datasets);
    let artifact = moved.join(&reopened.datasets[0].artifact.path);
    fs::write(&artifact, "{\"text\":\"third\"}\n{\"text\":\"second\"}\n").unwrap();
    assert!(open_workspace(&moved, true).await.is_err());
}

#[tokio::test]
async fn old_workspace_requires_and_survives_an_explicit_idempotent_catalog_upgrade() {
    use sqlx::{Connection, SqliteConnection};

    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    let created = create_workspace(
        &destination,
        "Upgrade fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let first_catalog = created.model_catalog.unwrap();
    let url = format!("sqlite://{}", destination.join("project.sqlite").display());
    let mut database = SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut database)
        .await
        .unwrap();
    for table in [
        "model_catalog_state",
        "baseline_revisions",
        "model_artifacts",
    ] {
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut database)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 2")
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();

    assert!(
        open_workspace(&destination, false)
            .await
            .unwrap()
            .model_catalog
            .is_none()
    );
    let upgraded = upgrade_workspace(&destination).await.unwrap();
    let catalog = upgraded.model_catalog.unwrap();
    assert_eq!(catalog.active_model().fingerprint, preview.fingerprint);
    assert_ne!(
        catalog.active_baseline_revision_id,
        first_catalog.active_baseline_revision_id
    );
    assert_eq!(
        upgrade_workspace(&destination)
            .await
            .unwrap()
            .model_catalog
            .unwrap(),
        catalog
    );
}

#[tokio::test]
async fn rejects_invalid_bundles_changed_preview_and_foreign_manifests() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    fs::write(source.join("README.md"), "new metadata").unwrap();
    let destination = temp.path().join("must-not-exist");
    assert!(
        create_workspace(
            &destination,
            "My encoder",
            &source,
            &preview.fingerprint,
            None
        )
        .await
        .is_err()
    );
    assert!(!destination.exists());
    fs::write(source.join("custom.py"), "do not execute").unwrap();
    assert!(inspect_model(&source).is_err());
    fs::remove_file(source.join("custom.py")).unwrap();
    let preview = inspect_model(&source).unwrap();
    create_workspace(&destination, "One", &source, &preview.fingerprint, None)
        .await
        .unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join(MANIFEST)).unwrap()).unwrap();
    manifest["id"] = json!(uuid::Uuid::new_v4());
    fs::write(
        destination.join(MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(open_workspace(&destination, false).await.is_err());
    assert!(open_workspace(temp.path(), false).await.is_err());
}

#[test]
fn training_imports_reject_held_out_rows_and_invalid_json() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("data.jsonl");
    for row in [
        "{\"evaluation_partition\":\"sealed\"}\n",
        "{\"split\":\"test\"}\n",
        "{\"sealed\":true}\n",
        "[]\n",
        "not json\n",
        "  \n",
    ] {
        fs::write(&source, row).unwrap();
        assert!(
            inspect_dataset(&source, DatasetPurpose::Training).is_err(),
            "{row}"
        );
    }
    fs::write(&source, "{\"evaluation_partition\":\"sealed\"}\n").unwrap();
    assert_eq!(
        inspect_dataset(&source, DatasetPurpose::Sealed)
            .unwrap()
            .rows,
        1
    );
}

#[tokio::test]
async fn backfill_links_exact_final_stage_inputs_without_flattening_native_rows() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    fs::write(
        source.join("nomos_training_manifest.json"),
        serde_json::to_vec(&json!({
            "inputs": ["data\\training.jsonl"], "input_state_counts": {"data\\training.jsonl": 1}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::create_dir(temp.path().join("data")).unwrap();
    let row = "{\"evaluation_partition\":\"train\",\"accepted\":true,\"decision_state_id\":\"offline-state\",\"tool_registry\":[],\"legal_candidate_ids\":[],\"nested\":{\"native\":true}}\n";
    fs::write(temp.path().join("data/training.jsonl"), row).unwrap();
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    create_workspace(
        &destination,
        "Native fixture",
        &source,
        &preview.fingerprint,
        Some("Tool routing".into()),
    )
    .await
    .unwrap();
    let result = backfill_nomos(&destination, temp.path()).await.unwrap();
    assert_eq!(result.datasets.len(), 1);
    let dataset = &result.datasets[0];
    assert_eq!(
        dataset
            .training_source
            .as_ref()
            .unwrap()
            .baseline_fingerprint,
        preview.fingerprint
    );
    assert_eq!(
        fs::read_to_string(destination.join(&dataset.artifact.path)).unwrap(),
        row
    );
    assert_eq!(
        backfill_nomos(&destination, temp.path())
            .await
            .unwrap()
            .datasets,
        result.datasets
    );
}

#[cfg(unix)]
#[test]
fn rejects_linked_model_files_and_parent_directories() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    symlink(&source, temp.path().join("link")).unwrap();
    assert!(inspect_model(&temp.path().join("link")).is_err());
    symlink(source.join("config.json"), source.join("linked.json")).unwrap();
    assert!(inspect_model(&source).is_err());
}

#[cfg(windows)]
#[test]
fn rejects_windows_junctions_in_source_ancestors() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let link = temp.path().join("junction");
    let result = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(inspect_model(&link).is_err());
}

#[tokio::test]
async fn a_published_copy_without_a_database_row_is_recoverable_not_overwritten() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    create_workspace(
        &destination,
        "Recovery fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let data = temp.path().join("input.jsonl");
    fs::write(&data, "{\"text\":\"recovery fixture\"}\n").unwrap();
    let preview = inspect_dataset(&data, DatasetPurpose::Unassigned).unwrap();
    let artifact = destination
        .join("datasets/imports")
        .join(&preview.artifact.fingerprint[7..]);
    fs::create_dir(&artifact).unwrap();
    fs::copy(&data, artifact.join("data.jsonl")).unwrap();
    let imported = import_dataset(
        &destination,
        &data,
        "Recovered copy",
        DatasetPurpose::Unassigned,
        &preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    assert_eq!(imported.datasets.len(), 1);
    assert_eq!(imported.datasets[0].rows, 1);
}
