use super::*;
use crate::{create_workspace, import_dataset, inspect_dataset, inspect_model};
use serde_json::json;

async fn fixture(root: &Path) -> (std::path::PathBuf, AgentAnalysisScope) {
    let model = root.join("model");
    training_transformer::fixture::write_tiny_bert_bundle(&model).unwrap();
    let checkpoint = inspect_model(&model).unwrap();
    let folder = root.join("workspace");
    create_workspace(
        &folder,
        "Generated dataset test",
        &model,
        &checkpoint.fingerprint,
        None,
    )
    .await
    .unwrap();
    let source = root.join("source.jsonl");
    std::fs::write(
        &source,
        b"{\"text\":\"remove this\"}\n{\"text\":\"retain this\"}\n",
    )
    .unwrap();
    let preview = inspect_dataset(&source, DatasetPurpose::Training).unwrap();
    let workspace = import_dataset(
        &folder,
        &source,
        "Base",
        DatasetPurpose::Training,
        &preview.artifact.fingerprint,
    )
    .await
    .unwrap();
    let parent = dataset_versions::create_base(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Base dataset",
        &[workspace.datasets[0].id],
    )
    .await
    .unwrap();
    let scope = AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: parent.id,
        dataset_fingerprint: parent.fingerprint,
        development_evidence_fingerprint: fingerprint(&"development").unwrap(),
        objective: String::new(),
        analysis_protocol: 1,
        maximum_turns: 4,
        maximum_row_changes: 8,
    };
    (folder, scope)
}

#[tokio::test]
async fn generated_import_and_diff_recover_without_duplicate_versions_or_source_changes() {
    let temp = tempfile::tempdir().unwrap();
    let (folder, scope) = fixture(temp.path()).await;
    let original = dataset_versions::inspect(&folder, scope.dataset_version_id)
        .await
        .unwrap();
    let rows = vec![
        json!({"text":"new targeted example","generation":{"call":"fixture-call","template":original.members[1].id}}),
    ];
    let removed = vec![original.members[0].id.clone()];
    let proposal = fingerprint(&(&rows, &removed)).unwrap();
    let mut db = connect(&folder, false, false).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_derived_version BEFORE INSERT ON dataset_versions WHEN NEW.number=2 BEGIN SELECT RAISE(ABORT,'injected after import and fork'); END").execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    assert!(
        publish_version(&folder, &scope, &proposal, &rows, &removed)
            .await
            .is_err()
    );
    assert_eq!(
        open_workspace(&folder, false).await.unwrap().datasets.len(),
        2
    );
    assert_eq!(dataset_versions::list(&folder).await.unwrap().len(), 2);
    let mut db = connect(&folder, false, false).await.unwrap();
    sqlx::query("DROP TRIGGER fail_derived_version")
        .execute(&mut db)
        .await
        .unwrap();
    db.close().await.unwrap();
    let result = publish_version(&folder, &scope, &proposal, &rows, &removed)
        .await
        .unwrap();
    assert_eq!(
        result,
        publish_version(&folder, &scope, &proposal, &rows, &removed)
            .await
            .unwrap()
    );
    assert_eq!(
        open_workspace(&folder, false).await.unwrap().datasets.len(),
        2
    );
    let entries = dataset_versions::list(&folder).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].versions.len(), 2);
    let version = dataset_versions::inspect(&folder, result.0.id)
        .await
        .unwrap();
    assert_eq!(version.changes.removed, removed);
    assert_eq!(version.changes.added.len(), 1);
    assert_eq!(version.members.len(), 2);
    assert_eq!(
        dataset_versions::inspect(&folder, original.id)
            .await
            .unwrap(),
        original
    );
    let inspected = dataset_versions::read_rows(&folder, result.0.id, 0, 50)
        .await
        .unwrap();
    assert!(inspected.rows.iter().any(|row| row.value == rows[0]));
    assert_eq!(
        dataset_versions::read_changes(&folder, result.0.id, 0, 50)
            .await
            .unwrap()
            .total,
        2
    );
    let mut db = connect(&folder, true, false).await.unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn rejected_or_changed_sources_cannot_be_published_as_training_edits() {
    let temp = tempfile::tempdir().unwrap();
    let (folder, scope) = fixture(temp.path()).await;
    let proposal = fingerprint(&"proposal").unwrap();
    assert!(
        publish_version(
            &folder,
            &scope,
            &proposal,
            &[json!({"text":"protected","evaluation_partition":"sealed"})],
            &[]
        )
        .await
        .is_err()
    );
    assert_eq!(
        open_workspace(&folder, false).await.unwrap().datasets.len(),
        1
    );
    let mut changed = scope;
    changed.dataset_fingerprint = fingerprint(&"changed").unwrap();
    assert!(
        publish_version(&folder, &changed, &proposal, &[json!({"text":"new"})], &[])
            .await
            .is_err()
    );
    assert_eq!(dataset_versions::list(&folder).await.unwrap().len(), 1);
}
