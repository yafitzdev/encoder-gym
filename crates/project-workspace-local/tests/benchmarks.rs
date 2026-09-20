use chrono::Utc;
use encoder_experiment_core::{
    benchmark::{BenchmarkDefinition, BenchmarkSuite},
    domain::{BackendIdentity, EncoderTaskKind, EvidenceRole},
    metrics::{MetricContract, MetricDefinition, MetricDirection, MetricGate, MetricGateCondition},
};
use project_workspace_core::{
    AdapterBinding, BenchmarkSource, BoundIdentity, RuntimeBinding, RuntimeKind, ScientificBinding,
    ScientificStoreBinding,
};
use project_workspace_local::{
    benchmarks, create_workspace, inspect_model, open_workspace, record_scientific_binding,
};
use sqlx::{Connection, SqliteConnection};
use std::path::Path;
use tempfile::TempDir;
use uuid::Uuid;

fn fp(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn reference(c: char) -> BoundIdentity {
    BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: fp(c),
    }
}
fn definition(c: char) -> BenchmarkDefinition {
    let mut value = BenchmarkDefinition {
        schema_version: 1,
        task: EncoderTaskKind::RetrievalRanking,
        backend: BackendIdentity::new("fixture", "v1", fp('1')).unwrap(),
        source_revision: "revision".into(),
        evaluation_configuration_fingerprint: fp(c),
        metric_contract: MetricContract::create(
            vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
            "mrr",
            [EvidenceRole::Development, EvidenceRole::SealedAcceptance]
                .into_iter()
                .map(|role| {
                    MetricGate::new(
                        "mrr",
                        role,
                        MetricGateCondition::MaximumRegression { value: 0.0 },
                    )
                    .unwrap()
                })
                .collect(),
        )
        .unwrap(),
        suites: vec![
            BenchmarkSuite {
                key: "development".into(),
                role: EvidenceRole::Development,
                fingerprint: fp('2'),
                support: 10,
            },
            BenchmarkSuite {
                key: "holdout".into(),
                role: EvidenceRole::SealedAcceptance,
                fingerprint: fp('3'),
                support: 10,
            },
        ],
        fingerprint: String::new(),
    };
    value.fingerprint = value.reproduce_fingerprint().unwrap();
    value.validate_integrity().unwrap();
    value
}
async fn fixture(root: &Path) -> (std::path::PathBuf, BenchmarkSource) {
    let model = root.join("checkpoint");
    training_transformer::fixture::write_tiny_bert_bundle(&model).unwrap();
    let preview = inspect_model(&model).unwrap();
    let folder = root.join("project");
    let workspace = create_workspace(
        &folder,
        "Benchmark fixture",
        &model,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let project_snapshot = reference('4');
    let binding = ScientificBinding::new(
        Uuid::new_v4(),
        workspace.manifest.id,
        workspace.model_catalog.unwrap().active_baseline_revision_id,
        None,
        AdapterBinding {
            key: "fixture".into(),
            protocol: "v1".into(),
            configuration_fingerprint: fp('1'),
        },
        RuntimeBinding {
            kind: RuntimeKind::Managed,
            location: "runs/runtime".into(),
            executable: Some("runs/runtime/python.exe".into()),
            package: Some(BoundIdentity {
                id: project_workspace_core::MANAGED_RUNTIME_PACKAGE_ID.into(),
                fingerprint: fp('9'),
            }),
            project_snapshot: project_snapshot.clone(),
        },
        ScientificStoreBinding {
            database_path: "runs/scientific.sqlite".into(),
            schema: reference('5'),
            snapshot_fingerprint: None,
            snapshot_bytes: None,
        },
        "fixture",
        "Offline metadata fixture",
        Utc::now(),
    )
    .unwrap();
    record_scientific_binding(&folder, binding.clone(), None)
        .await
        .unwrap();
    let source = BenchmarkSource {
        scientific_binding: BoundIdentity {
            id: binding.id.to_string(),
            fingerprint: binding.fingerprint,
        },
        project_snapshot,
        protocol: reference('6'),
    };
    (folder, source)
}

#[tokio::test]
async fn benchmark_versions_are_immutable_idempotent_and_survive_moving_the_project() {
    let temp = TempDir::new().unwrap();
    let (folder, source) = fixture(temp.path()).await;
    let before = open_workspace(&folder, false).await.unwrap();
    assert!(benchmarks::list(&folder).await.unwrap().is_empty());
    let id = Uuid::new_v4();
    let one = benchmarks::record(&folder, id, None, definition('7'), source.clone())
        .await
        .unwrap();
    assert_eq!(one.number, 1);
    let mut another_run = source.clone();
    another_run.protocol = reference('b');
    assert_eq!(
        one,
        benchmarks::record(&folder, id, None, definition('7'), another_run)
            .await
            .unwrap()
    );
    assert_eq!(
        one,
        benchmarks::record(&folder, id, None, definition('7'), source.clone())
            .await
            .unwrap()
    );
    assert_eq!(
        one,
        benchmarks::record(
            &folder,
            Uuid::new_v4(),
            Some(id),
            definition('7'),
            source.clone()
        )
        .await
        .unwrap()
    );
    let two_id = Uuid::new_v4();
    assert!(
        benchmarks::record(&folder, two_id, None, definition('8'), source.clone())
            .await
            .is_err()
    );
    let two = benchmarks::record(&folder, two_id, Some(id), definition('8'), source.clone())
        .await
        .unwrap();
    assert_eq!(two.number, 2);
    assert_eq!(two.parent.as_ref().unwrap().id, id.to_string());
    assert!(
        benchmarks::record(&folder, id, Some(two_id), definition('9'), source.clone())
            .await
            .is_err()
    );
    let mut foreign = source;
    foreign.project_snapshot = reference('a');
    assert!(
        benchmarks::record(
            &folder,
            Uuid::new_v4(),
            Some(two_id),
            definition('a'),
            foreign
        )
        .await
        .is_err()
    );
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE project_benchmark_versions SET number=9")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM project_benchmark_versions")
            .execute(&mut db)
            .await
            .is_err()
    );
    db.close().await.unwrap();
    let moved = temp.path().join("moved");
    std::fs::rename(&folder, &moved).unwrap();
    let after = open_workspace(&moved, true).await.unwrap();
    assert_eq!(after.benchmark_versions, vec![one, two]);
    assert_eq!(before.model_catalog, after.model_catalog);
    assert_eq!(before.scientific_binding, after.scientific_binding);
}

#[tokio::test]
async fn failed_writes_retry_once_and_rehashed_foreign_source_or_broken_history_is_rejected() {
    let temp = TempDir::new().unwrap();
    let (folder, source) = fixture(temp.path()).await;
    let id = Uuid::new_v4();
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER injected BEFORE INSERT ON project_benchmark_versions BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&mut db).await.unwrap();
    assert!(
        benchmarks::record(&folder, id, None, definition('7'), source.clone())
            .await
            .is_err()
    );
    assert!(benchmarks::list(&folder).await.unwrap().is_empty());
    sqlx::query("DROP TRIGGER injected")
        .execute(&mut db)
        .await
        .unwrap();
    let one = benchmarks::record(&folder, id, None, definition('7'), source)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER immutable_project_benchmarks_update")
        .execute(&mut db)
        .await
        .unwrap();
    let mut changed = one.clone();
    changed.source.project_snapshot = reference('a');
    changed.fingerprint = changed.reproduce().unwrap();
    sqlx::query("UPDATE project_benchmark_versions SET metadata_json=?, fingerprint=? WHERE id=?")
        .bind(serde_json::to_string(&changed).unwrap())
        .bind(&changed.fingerprint)
        .bind(id.to_string())
        .execute(&mut db)
        .await
        .unwrap();
    assert!(benchmarks::list(&folder).await.is_err());
    assert!(open_workspace(&folder, true).await.is_err());
    sqlx::query(
        "UPDATE project_benchmark_versions SET metadata_json=?, fingerprint=?, number=3 WHERE id=?",
    )
    .bind(serde_json::to_string(&one).unwrap())
    .bind(&one.fingerprint)
    .bind(id.to_string())
    .execute(&mut db)
    .await
    .unwrap();
    assert!(benchmarks::list(&folder).await.is_err());
}
