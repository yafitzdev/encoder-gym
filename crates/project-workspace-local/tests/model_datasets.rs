use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use project_workspace_core::{ModelDatasetLink, ModelTrainingEvidence};
use project_workspace_local::{
    backfill_nomos, create_workspace, dataset_versions, inspect_model, model_datasets,
    open_workspace,
};
use serde_json::json;
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;
use uuid::Uuid;

async fn fixture(root: &Path) -> PathBuf {
    let checkpoint = root.join("checkpoint");
    training_transformer::fixture::write_tiny_bert_bundle(&checkpoint).unwrap();
    fs::write(checkpoint.join("nomos_training_manifest.json"), serde_json::to_vec(&json!({"inputs": ["a.jsonl", "b.jsonl"], "input_state_counts": {"a.jsonl": 2, "b.jsonl": 1}})).unwrap()).unwrap();
    let row = |id| {
        format!(
            "{}\n",
            json!({"evaluation_partition":"train","accepted":true,"decision_state_id":id,"tool_registry":[],"legal_candidate_ids":[]})
        )
    };
    fs::write(root.join("a.jsonl"), row("one") + &row("two")).unwrap();
    fs::write(root.join("b.jsonl"), row("three")).unwrap();
    let model = inspect_model(&checkpoint).unwrap();
    let folder = root.join("project");
    create_workspace(
        &folder,
        "Training lineage",
        &checkpoint,
        &model.fingerprint,
        None,
    )
    .await
    .unwrap();
    backfill_nomos(&folder, root).await.unwrap();
    folder
}

#[tokio::test]
async fn adoption_links_exact_recorded_inputs_and_preserves_model_and_version_history() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let before = open_workspace(&folder, true).await.unwrap();
    let link = model_datasets::adopt_baseline(&folder).await.unwrap();
    assert_eq!(link.inputs.iter().map(|input| input.rows).sum::<u64>(), 3);
    assert_eq!(
        link.inputs
            .iter()
            .map(|input| input.key.as_str())
            .collect::<Vec<_>>(),
        ["a.jsonl", "b.jsonl"]
    );
    assert_eq!(model_datasets::adopt_baseline(&folder).await.unwrap(), link);
    let after = open_workspace(&folder, true).await.unwrap();
    assert_eq!(before.model_catalog, after.model_catalog);
    assert_eq!(after.model_dataset_links, vec![link.clone()]);
    let initial = dataset_versions::inspect(&folder, link.version.id)
        .await
        .unwrap();
    let changed = dataset_versions::revise(
        &folder,
        dataset_versions::DatasetVersionUpdate {
            version_id: Uuid::new_v4(),
            dataset_id: initial.dataset_id,
            parent_id: initial.id,
            added: vec![],
            removed: vec![initial.members[0].id.clone()],
            replaced: vec![],
        },
    )
    .await
    .unwrap();
    assert_ne!(changed.id, initial.id);
    assert_eq!(model_datasets::adopt_baseline(&folder).await.unwrap(), link);
    assert_eq!(
        dataset_versions::list(&folder).await.unwrap()[0]
            .versions
            .len(),
        2
    );
    let moved = temp.path().join("moved");
    fs::rename(&folder, &moved).unwrap();
    assert_eq!(
        open_workspace(&moved, true)
            .await
            .unwrap()
            .model_dataset_links,
        vec![link]
    );
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        moved.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    assert!(
        sqlx::query("DELETE FROM model_dataset_links")
            .execute(&mut database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE model_dataset_links SET version_id='changed'")
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
}

#[tokio::test]
async fn domain_binding_rejects_foreign_versions_missing_rows_and_changed_receipts() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let link = model_datasets::adopt_baseline(&folder).await.unwrap();
    let workspace = open_workspace(&folder, true).await.unwrap();
    let model = workspace.model_catalog.as_ref().unwrap().active_model();
    let version = dataset_versions::inspect(&folder, link.version.id)
        .await
        .unwrap();
    let mut inputs = link.inputs.clone();
    inputs[0].rows += 1;
    assert!(
        ModelDatasetLink::new(model, &version, inputs, link.evidence.clone(), Utc::now()).is_err()
    );
    let mut other_model = model.clone();
    other_model.project_id = Uuid::new_v4();
    assert!(
        ModelDatasetLink::new(
            &other_model,
            &version,
            link.inputs.clone(),
            link.evidence.clone(),
            Utc::now()
        )
        .is_err()
    );
    let mut tampered = link.clone();
    tampered.version.fingerprint = "sha256:".to_string() + &"e".repeat(64);
    assert!(tampered.validate_for(model, &version).is_err());
    let receipt = project_workspace_core::BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: "sha256:".to_string() + &"f".repeat(64),
    };
    assert!(
        ModelDatasetLink::new(
            model,
            &version,
            link.inputs,
            ModelTrainingEvidence::CompletedTraining {
                manifest: link.evidence.manifest().clone(),
                snapshot: receipt.clone(),
                run: receipt
            },
            Utc::now()
        )
        .is_err()
    );
}

#[tokio::test]
async fn retry_after_link_write_failure_reuses_the_existing_dataset() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_link BEFORE INSERT ON model_dataset_links BEGIN SELECT RAISE(ABORT, 'injected'); END").execute(&mut database).await.unwrap();
    assert!(model_datasets::adopt_baseline(&folder).await.is_err());
    let reserved = dataset_versions::list(&folder).await.unwrap();
    assert_eq!(reserved.len(), 1);
    sqlx::query("DROP TRIGGER fail_link")
        .execute(&mut database)
        .await
        .unwrap();
    let link = model_datasets::adopt_baseline(&folder).await.unwrap();
    assert_eq!(link.version, reserved[0].versions[0].version);
    assert_eq!(dataset_versions::list(&folder).await.unwrap().len(), 1);
    // Even a re-fingerprinted link cannot reorder the pinned manifest's inputs.
    let mut changed = link;
    changed.inputs.reverse();
    changed.fingerprint.clear();
    changed.fingerprint = artifact_core::fingerprint(&changed).unwrap();
    sqlx::query("DROP TRIGGER immutable_model_dataset_links_update")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE model_dataset_links SET fingerprint=?, metadata_json=?")
        .bind(&changed.fingerprint)
        .bind(serde_json::to_string(&changed).unwrap())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(
        open_workspace(&folder, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("input order changed")
    );
}
