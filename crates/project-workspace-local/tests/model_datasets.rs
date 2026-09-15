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

async fn completed_request(
    root: &Path,
    folder: &Path,
    extra: bool,
) -> model_datasets::CompletedTrainingData {
    use project_workspace_core::{BoundIdentity, DatasetPurpose};
    use project_workspace_local::{
        CompletedModelRegistration, inspect_dataset, register_completed_model,
    };
    let workspace = open_workspace(folder, false).await.unwrap();
    let checkpoint = root.join(format!("candidate-{}", Uuid::new_v4()));
    training_transformer::fixture::write_tiny_bert_bundle(&checkpoint).unwrap();
    fs::write(
        root.join("extra.jsonl"),
        "{\"question\":\"Added training row\",\"split\":\"train\"}\n",
    )
    .unwrap();
    let mut keys = vec!["b.jsonl", "a.jsonl"];
    if extra {
        keys.push("extra.jsonl");
    }
    let inputs = keys
        .iter()
        .map(|key| {
            let path = root.join(key);
            let preview = inspect_dataset(&path, DatasetPurpose::Training).unwrap();
            model_datasets::RecordedTrainingInput {
                key: key.to_string(),
                path,
                bytes: preview.artifact.bytes,
                fingerprint: preview.artifact.fingerprint,
                rows: preview.rows,
            }
        })
        .collect::<Vec<_>>();
    let counts = inputs
        .iter()
        .map(|input| (input.key.clone(), input.rows))
        .collect::<std::collections::BTreeMap<_, _>>();
    fs::write(
        checkpoint.join("nomos_training_manifest.json"),
        serde_json::to_vec(
            &json!({"inputs":keys, "input_row_counts":counts,"fixture_run":Uuid::new_v4()}),
        )
        .unwrap(),
    )
    .unwrap();
    let model = inspect_model(&checkpoint).unwrap();
    let identity = || BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: "sha256:".to_string() + &"3".repeat(64),
    };
    let snapshot = identity();
    let run = identity();
    let source_model = identity();
    let registered = register_completed_model(
        folder,
        &checkpoint,
        CompletedModelRegistration {
            parent_model_id: workspace.model_catalog.as_ref().unwrap().active_model().id,
            name: "Candidate 01".into(),
            source_model: source_model.clone(),
            source_model_format: model.format,
            source_model_bytes: model.bytes,
            producing_run: run.clone(),
            training_snapshot: snapshot.clone(),
            trainer: identity(),
            effective_configuration_fingerprint: "sha256:".to_string() + &"4".repeat(64),
            source_revision: "fixture-revision".into(),
        },
    )
    .await
    .unwrap();
    let model_id = registered
        .model_catalog
        .unwrap()
        .artifacts
        .iter()
        .find(|model| model.source_model.as_ref() == Some(&source_model))
        .unwrap()
        .id;
    model_datasets::CompletedTrainingData {
        model_id,
        parent_version_id: None,
        snapshot,
        run,
        manifest: model
            .files
            .into_iter()
            .find(|file| file.path == "nomos_training_manifest.json")
            .unwrap(),
        inputs,
    }
}

#[tokio::test]
async fn completed_candidate_keeps_native_order_and_inspectable_base_plus_delta_history() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let base = model_datasets::adopt_baseline(&folder).await.unwrap();
    let request = completed_request(temp.path(), &folder, true).await;
    let before = open_workspace(&folder, true).await.unwrap();
    let link = model_datasets::adopt_completed(&folder, request.clone())
        .await
        .unwrap();
    assert_eq!(
        model_datasets::adopt_completed(&folder, request)
            .await
            .unwrap(),
        link
    );
    assert_eq!(
        link.inputs
            .iter()
            .map(|input| input.key.as_str())
            .collect::<Vec<_>>(),
        ["b.jsonl", "a.jsonl", "extra.jsonl"]
    );
    let entries = dataset_versions::list(&folder).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].dataset.origin.as_ref(), Some(&base.version));
    assert_eq!(entries[1].versions.len(), 2);
    assert_eq!(entries[1].versions[0].rows, 4);
    let version = dataset_versions::inspect(&folder, link.version.id)
        .await
        .unwrap();
    assert_eq!(version.changes.added.len(), 1);
    assert!(version.changes.removed.is_empty());
    assert_eq!(
        version.members[0].source.import_id,
        base.inputs[0].import_id
    );
    let changes = dataset_versions::read_changes(&folder, version.id, 0, 25)
        .await
        .unwrap();
    assert_eq!(
        changes.changes[0].after.as_ref().unwrap().value["question"],
        "Added training row"
    );
    assert_eq!(
        open_workspace(&folder, true).await.unwrap().model_catalog,
        before.model_catalog
    );
    let moved = temp.path().join("moved-candidate");
    fs::rename(&folder, &moved).unwrap();
    assert_eq!(
        open_workspace(&moved, true)
            .await
            .unwrap()
            .model_dataset_links[1],
        link
    );
}

#[tokio::test]
async fn completed_candidate_branches_from_the_dataset_selected_for_its_run() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let base = model_datasets::adopt_baseline(&folder).await.unwrap();
    let selected = dataset_versions::fork(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Selected training data",
        base.version.id,
    )
    .await
    .unwrap();
    let mut request = completed_request(temp.path(), &folder, true).await;
    request.parent_version_id = Some(selected.id);

    let link = model_datasets::adopt_completed(&folder, request)
        .await
        .unwrap();
    let entries = dataset_versions::list(&folder).await.unwrap();
    let candidate = entries
        .iter()
        .find(|entry| {
            entry
                .versions
                .iter()
                .any(|version| version.version == link.version)
        })
        .unwrap();

    assert_eq!(
        candidate.dataset.origin.as_ref(),
        Some(&selected.reference())
    );
    assert_eq!(candidate.versions.len(), 2);
    assert_eq!(candidate.versions[0].added, 1);
}

#[tokio::test]
async fn completed_model_reuses_unchanged_data_and_failed_link_retry_reuses_its_variant() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let base = model_datasets::adopt_baseline(&folder).await.unwrap();
    let unchanged = completed_request(temp.path(), &folder, false).await;
    let link = model_datasets::adopt_completed(&folder, unchanged)
        .await
        .unwrap();
    assert_eq!(link.version, base.version);
    assert_eq!(dataset_versions::list(&folder).await.unwrap().len(), 1);
    let request = completed_request(temp.path(), &folder, true).await;
    let mut invalid = request.clone();
    invalid.snapshot.id = Uuid::new_v4().to_string();
    assert!(
        model_datasets::adopt_completed(&folder, invalid)
            .await
            .is_err()
    );
    assert_eq!(
        open_workspace(&folder, false).await.unwrap().datasets.len(),
        2
    );
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_completed_link BEFORE INSERT ON model_dataset_links BEGIN SELECT RAISE(ABORT, 'injected'); END").execute(&mut database).await.unwrap();
    assert!(
        model_datasets::adopt_completed(&folder, request.clone())
            .await
            .is_err()
    );
    let entries = dataset_versions::list(&folder).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].versions.len(), 2);
    sqlx::query("DROP TRIGGER fail_completed_link")
        .execute(&mut database)
        .await
        .unwrap();
    let linked = model_datasets::adopt_completed(&folder, request)
        .await
        .unwrap();
    assert_eq!(linked.version, entries[1].versions[0].version);
    assert_eq!(dataset_versions::list(&folder).await.unwrap().len(), 2);
}

#[tokio::test]
async fn metadata_navigation_is_not_a_substitute_for_full_membership_verification() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let link = model_datasets::adopt_baseline(&folder).await.unwrap();
    let version = dataset_versions::inspect(&folder, link.version.id)
        .await
        .unwrap();
    let mut changed = serde_json::to_value(&version).unwrap();
    changed["members"][0]["contentFingerprint"] = json!("sha256:".to_string() + &"a".repeat(64));
    let mut database = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER immutable_dataset_versions_update")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("UPDATE dataset_versions SET metadata_json=? WHERE id=?")
        .bind(serde_json::to_string(&changed).unwrap())
        .bind(version.id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(!open_workspace(&folder, false).await.unwrap().verified);
    assert!(open_workspace(&folder, true).await.is_err());
    assert!(
        dataset_versions::read_rows(&folder, version.id, 0, 1)
            .await
            .is_err()
    );
    assert!(
        dataset_versions::fork(
            &folder,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Invalid fork",
            version.id
        )
        .await
        .is_err()
    );
    // Mismatched or missing metadata also fails on the lightweight path.
    changed["projectId"] = json!(Uuid::new_v4());
    sqlx::query("UPDATE dataset_versions SET metadata_json=? WHERE id=?")
        .bind(serde_json::to_string(&changed).unwrap())
        .bind(version.id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(open_workspace(&folder, false).await.is_err());
    changed["projectId"] = serde_json::Value::Null;
    sqlx::query("UPDATE dataset_versions SET metadata_json=? WHERE id=?")
        .bind(serde_json::to_string(&changed).unwrap())
        .bind(version.id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    assert!(open_workspace(&folder, false).await.is_err());
}

async fn materialized_request(
    root: &Path,
    folder: &Path,
    version_id: Uuid,
    reverse: bool,
) -> model_datasets::CompletedTrainingData {
    use project_workspace_core::{BoundIdentity, DatasetPurpose};
    use project_workspace_local::{
        CompletedModelRegistration, inspect_dataset, register_completed_model,
    };
    let version = dataset_versions::inspect(folder, version_id).await.unwrap();
    let mut rows = dataset_versions::materialization_rows(folder, version_id)
        .await
        .unwrap();
    if reverse {
        rows.reverse();
    }
    let source = root.join(format!("rendered-{}.jsonl", Uuid::new_v4()));
    fs::write(
        &source,
        rows.iter()
            .map(|row| format!("{}\n", row.value))
            .collect::<String>(),
    )
    .unwrap();
    let preview = inspect_dataset(&source, DatasetPurpose::Training).unwrap();
    let checkpoint = root.join(format!("trained-{}", Uuid::new_v4()));
    training_transformer::fixture::write_tiny_bert_bundle(&checkpoint).unwrap();
    fs::write(
        checkpoint.join("nomos_training_manifest.json"),
        serde_json::to_vec(&json!({
            "inputs":["rendered.jsonl"], "input_row_counts":{"rendered.jsonl":rows.len()},
            "fixture_run":Uuid::new_v4()
        }))
        .unwrap(),
    )
    .unwrap();
    let model = inspect_model(&checkpoint).unwrap();
    let workspace = open_workspace(folder, false).await.unwrap();
    let identity = || BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: format!("sha256:{}", "7".repeat(64)),
    };
    let source_model = identity();
    let snapshot = BoundIdentity {
        id: version.id.to_string(),
        fingerprint: version.fingerprint,
    };
    let run = identity();
    let registered = register_completed_model(
        folder,
        &checkpoint,
        CompletedModelRegistration {
            parent_model_id: workspace.model_catalog.as_ref().unwrap().active_model().id,
            name: "Materialized candidate".into(),
            source_model: source_model.clone(),
            source_model_format: model.format,
            source_model_bytes: model.bytes,
            producing_run: run.clone(),
            training_snapshot: snapshot.clone(),
            trainer: identity(),
            effective_configuration_fingerprint: format!("sha256:{}", "8".repeat(64)),
            source_revision: "fixture".into(),
        },
    )
    .await
    .unwrap();
    let model_id = registered
        .model_catalog
        .unwrap()
        .artifacts
        .iter()
        .find(|model| model.source_model.as_ref() == Some(&source_model))
        .unwrap()
        .id;
    model_datasets::CompletedTrainingData {
        model_id,
        parent_version_id: Some(version_id),
        snapshot,
        run,
        manifest: model
            .files
            .into_iter()
            .find(|file| file.path == "nomos_training_manifest.json")
            .unwrap(),
        inputs: vec![model_datasets::RecordedTrainingInput {
            key: "rendered.jsonl".into(),
            path: source,
            bytes: preview.artifact.bytes,
            fingerprint: preview.artifact.fingerprint,
            rows: preview.rows,
        }],
    }
}

#[tokio::test]
async fn materialized_subset_preserves_original_rows_and_retries_without_new_versions() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let baseline = model_datasets::adopt_baseline(&folder).await.unwrap();
    let original = dataset_versions::inspect(&folder, baseline.version.id)
        .await
        .unwrap();
    let selected = dataset_versions::revise(
        &folder,
        dataset_versions::DatasetVersionUpdate {
            dataset_id: original.dataset_id,
            version_id: Uuid::new_v4(),
            parent_id: original.id,
            removed: vec![original.members[0].id.clone()],
            added: vec![],
            replaced: vec![],
        },
    )
    .await
    .unwrap();
    let request = materialized_request(temp.path(), &folder, selected.id, false).await;
    let before = open_workspace(&folder, true).await.unwrap();
    let versions = dataset_versions::list(&folder).await.unwrap();
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_rendered_link BEFORE INSERT ON model_dataset_links BEGIN SELECT RAISE(ABORT, 'injected'); END").execute(&mut db).await.unwrap();
    assert!(
        model_datasets::adopt_materialized(&folder, request.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("injected")
    );
    sqlx::query("DROP TRIGGER fail_rendered_link")
        .execute(&mut db)
        .await
        .unwrap();
    db.close().await.unwrap();
    let link = model_datasets::adopt_materialized(&folder, request.clone())
        .await
        .unwrap();
    assert_eq!(
        link,
        model_datasets::adopt_materialized(&folder, request)
            .await
            .unwrap()
    );
    assert_eq!(link.version, selected.reference());
    assert_eq!(
        dataset_versions::inspect(&folder, selected.id)
            .await
            .unwrap(),
        selected
    );
    assert_eq!(
        serde_json::to_value(dataset_versions::list(&folder).await.unwrap()).unwrap(),
        serde_json::to_value(versions).unwrap()
    );
    assert!(
        selected
            .members
            .iter()
            .all(|row| original.members.contains(row))
    );
    assert!(
        selected
            .members
            .iter()
            .all(|row| row.source.import_id != link.inputs[0].import_id)
    );
    let after = open_workspace(&folder, true).await.unwrap();
    assert_eq!(after.model_catalog, before.model_catalog);
    assert!(after.model_dataset_links.contains(&baseline));
    let model = after
        .model_catalog
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .find(|model| model.id == link.model_id)
        .unwrap();
    let mut wrong_count = link.inputs.clone();
    wrong_count[0].rows += 1;
    assert!(
        ModelDatasetLink::new(
            model,
            &selected,
            wrong_count,
            link.evidence.clone(),
            Utc::now()
        )
        .is_err()
    );
    let mut wrong_order = selected.members.clone();
    wrong_order.reverse();
    let mut evidence = link.evidence.clone();
    if let ModelTrainingEvidence::MaterializedTraining {
        ordered_content_fingerprint,
        ..
    } = &mut evidence
    {
        *ordered_content_fingerprint = ModelDatasetLink::ordered_content_fingerprint(
            wrong_order
                .iter()
                .map(|row| row.content_fingerprint.as_str()),
        )
        .unwrap();
    } else {
        panic!("Missing materialized evidence");
    }
    assert!(
        ModelDatasetLink::new(model, &selected, link.inputs.clone(), evidence, Utc::now()).is_err()
    );
    let moved = temp.path().join("moved-rendered");
    fs::rename(&folder, &moved).unwrap();
    assert!(
        open_workspace(&moved, true)
            .await
            .unwrap()
            .model_dataset_links
            .contains(&link)
    );
}

#[tokio::test]
async fn materialized_link_rejects_reordered_native_rows_and_foreign_version() {
    let temp = TempDir::new().unwrap();
    let folder = fixture(temp.path()).await;
    let baseline = model_datasets::adopt_baseline(&folder).await.unwrap();
    let request = materialized_request(temp.path(), &folder, baseline.version.id, true).await;
    let error = model_datasets::adopt_materialized(&folder, request.clone())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("rows or their order"),
        "{error:#}"
    );
    assert_eq!(
        open_workspace(&folder, true)
            .await
            .unwrap()
            .model_dataset_links,
        vec![baseline.clone()]
    );
    let other = dataset_versions::fork(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Same contents, another version",
        baseline.version.id,
    )
    .await
    .unwrap();
    let mut wrong_version = request;
    wrong_version.parent_version_id = Some(other.id);
    assert!(
        model_datasets::adopt_materialized(&folder, wrong_version)
            .await
            .unwrap_err()
            .to_string()
            .contains("snapshot differs")
    );
    assert_eq!(
        open_workspace(&folder, true)
            .await
            .unwrap()
            .model_dataset_links,
        vec![baseline]
    );
}
