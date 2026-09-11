use chrono::{TimeZone, Utc};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, AdapterBinding,
    BoundIdentity, DatasetPurpose, MANIFEST, ProviderAuthentication, ProviderCatalog,
    ProviderConfiguration, ProviderKind, ProviderLimits, ProviderRole, RuntimeBinding, RuntimeKind,
    ScientificBinding, ScientificStoreBinding, SecretReference,
};
use project_workspace_local::{
    AcceptedModelPromotion, AppendActivity, BaselineRestorationRequest, CompletedModelRegistration,
    append_activity, backfill_nomos, create_workspace, export_activity, import_dataset,
    initialize_activity, inspect_dataset, inspect_model, open_workspace, read_action,
    read_activity, record_accepted_model_promotion, record_baseline_restoration,
    record_provider_catalog, record_scientific_binding, register_completed_model,
    upgrade_workspace,
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
async fn project_activity_is_append_only_hash_chained_and_exportable_jsonl() {
    use sqlx::{Connection, SqliteConnection};

    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("activity-project");
    let workspace = create_workspace(
        &destination,
        "Activity fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let url = format!("sqlite://{}", destination.join("project.sqlite").display());
    let mut database = SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE project_activity_events")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 5")
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();
    let initialized = initialize_activity(&destination).await.unwrap();
    assert_eq!(initialized.project_id, workspace.manifest.id);
    assert!(initialized.created);
    assert!(!initialize_activity(&destination).await.unwrap().created);
    let action_id = uuid::Uuid::new_v4();
    let run_id = uuid::Uuid::new_v4();
    let at = Utc.with_ymd_and_hms(2026, 9, 10, 8, 0, 0).unwrap();
    let request = |state, stage, completed, total, references, failure| AppendActivity {
        action_id,
        operation: "optimization.resume".into(),
        source: ActivitySource::Desktop,
        state,
        stage,
        completed,
        total,
        references,
        failure,
        created_at: at,
    };
    let first = append_activity(
        &destination,
        request(
            ActivityEventState::Started,
            None,
            None,
            None,
            vec![ActivityReference::new("run", run_id.to_string()).unwrap()],
            None,
        ),
    )
    .await
    .unwrap();
    let progress = append_activity(
        &destination,
        request(
            ActivityEventState::Progress,
            Some("training".into()),
            Some(2),
            Some(5),
            vec![],
            None,
        ),
    )
    .await
    .unwrap();
    append_activity(
        &destination,
        request(
            ActivityEventState::Failed,
            None,
            None,
            None,
            vec![],
            Some(
                ActivityFailure::new("worker_exit", "Training worker stopped before completion.")
                    .unwrap(),
            ),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        progress.previous_event_fingerprint.as_deref(),
        Some(first.fingerprint.as_str())
    );
    assert!(
        append_activity(
            &destination,
            request(
                ActivityEventState::Succeeded,
                None,
                None,
                None,
                vec![],
                None
            )
        )
        .await
        .is_err()
    );
    let action = read_action(&destination, action_id).await.unwrap();
    assert_eq!(action.state, ActivityEventState::Failed);
    assert_eq!(action.project_id, workspace.manifest.id);
    assert_eq!(
        read_activity(&destination, 10).await.unwrap().actions.len(),
        1
    );
    let export = temp.path().join("activity.jsonl");
    assert_eq!(export_activity(&destination, &export).await.unwrap(), 3);
    let lines = fs::read_to_string(export)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<project_workspace_core::ProjectActivityEvent>(line).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|event| event.action_id == action_id));
}

#[test]
fn native_trainer_module_paths_are_supported_without_accepting_custom_modules() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    fs::create_dir(source.join("1_Pooling")).unwrap();
    fs::create_dir(source.join("2_Normalize")).unwrap();
    fs::write(source.join("1_Pooling/config.json"), b"{}").unwrap();
    for kinds in [
        [
            "sentence_transformers.models.Transformer",
            "sentence_transformers.models.Pooling",
            "sentence_transformers.models.Normalize",
        ],
        [
            "sentence_transformers.base.modules.transformer.Transformer",
            "sentence_transformers.sentence_transformer.modules.pooling.Pooling",
            "sentence_transformers.base.modules.normalize.Normalize",
        ],
    ] {
        let modules = json!([
            {"idx":0,"path":"","type":kinds[0]},
            {"idx":1,"path":"1_Pooling","type":kinds[1]},
            {"idx":2,"path":"2_Normalize","type":kinds[2]}
        ]);
        fs::write(
            source.join("modules.json"),
            serde_json::to_vec(&modules).unwrap(),
        )
        .unwrap();
        assert!(inspect_model(&source).is_ok());
        let mut unsafe_module = modules;
        unsafe_module[0]["type"] = json!("custom.Transformer");
        fs::write(
            source.join("modules.json"),
            serde_json::to_vec(&unsafe_module).unwrap(),
        )
        .unwrap();
        assert!(inspect_model(&source).is_err());
    }
}

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn scientific_binding(
    workspace: &project_workspace_local::ManagedWorkspace,
    id: uuid::Uuid,
    previous_binding_id: Option<uuid::Uuid>,
) -> ScientificBinding {
    ScientificBinding::new(
        id,
        workspace.manifest.id,
        workspace
            .model_catalog
            .as_ref()
            .unwrap()
            .active_baseline_revision_id,
        previous_binding_id,
        AdapterBinding {
            key: "nomos".into(),
            protocol: "nomos-production-v1".into(),
            configuration_fingerprint: digest('a'),
        },
        RuntimeBinding {
            kind: RuntimeKind::ExternalIsolated,
            location: "C:/isolated/nomos".into(),
            executable: Some("python".into()),
            project_snapshot: BoundIdentity {
                id: "revision-1".into(),
                fingerprint: digest('b'),
            },
        },
        ScientificStoreBinding {
            database_path: "runs/scientific.sqlite".into(),
            schema: BoundIdentity {
                id: "production-schema-v1".into(),
                fingerprint: digest('c'),
            },
            snapshot_fingerprint: None,
            snapshot_bytes: None,
        },
        "operator",
        "configure verified runtime",
        Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap(),
    )
    .unwrap()
}

fn provider_catalog(
    workspace: &project_workspace_local::ManagedWorkspace,
    id: uuid::Uuid,
    previous_revision_id: Option<uuid::Uuid>,
    sequence: u64,
) -> ProviderCatalog {
    let provider = |role, environment: &str| ProviderConfiguration {
        role,
        kind: ProviderKind::OpenaiCompatible,
        endpoint: Some("https://api.example.test/v1".into()),
        model: "bounded-model".into(),
        authentication: ProviderAuthentication::Bearer,
        secret: Some(
            SecretReference::for_role(workspace.manifest.id, role, Some(environment.into()))
                .unwrap(),
        ),
        limits: ProviderLimits {
            maximum_requests: 10,
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 2_000,
            maximum_cost_microusd: 50_000,
        },
    };
    ProviderCatalog::create(
        id,
        workspace.manifest.id,
        sequence,
        previous_revision_id,
        vec![
            provider(ProviderRole::Generation, "GENERATION_KEY"),
            provider(ProviderRole::Advisor, "ADVISOR_KEY"),
        ],
        "operator",
        "configure providers",
        Utc::now(),
    )
    .unwrap()
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
async fn accepted_model_promotion_copies_once_and_advances_the_baseline_atomically() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    let created = create_workspace(
        &destination,
        "Promotion fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let candidate = temp.path().join("candidate");
    training_transformer::fixture::write_tiny_bert_bundle(&candidate).unwrap();
    fs::write(candidate.join("candidate.json"), b"{\"accepted\":true}\n").unwrap();
    let candidate_model = inspect_model(&candidate).unwrap();
    let expected = created
        .model_catalog
        .as_ref()
        .unwrap()
        .active_baseline_revision_id;
    let request = AcceptedModelPromotion {
        expected_baseline_revision_id: expected,
        name: "Accepted candidate".into(),
        source_model: BoundIdentity {
            id: uuid::Uuid::new_v4().to_string(),
            fingerprint: digest('d'),
        },
        source_model_format: candidate_model.format.clone(),
        source_model_bytes: candidate_model.bytes,
        producing_run: BoundIdentity {
            id: uuid::Uuid::new_v4().to_string(),
            fingerprint: digest('e'),
        },
        training_snapshot: BoundIdentity {
            id: uuid::Uuid::new_v4().to_string(),
            fingerprint: digest('f'),
        },
        trainer: BoundIdentity {
            id: "fixture-trainer:v1".into(),
            fingerprint: digest('1'),
        },
        effective_configuration_fingerprint: digest('2'),
        source_revision: "fixture-source-revision".into(),
        decision_id: "experiment-final:fixture:9".into(),
        decision_fingerprint: digest('3'),
        actor: "fixture-operator".into(),
        reason: "Passed explicit sealed acceptance".into(),
    };
    let registration = CompletedModelRegistration {
        parent_model_id: created.model_catalog.as_ref().unwrap().active_model().id,
        name: request.name.clone(),
        source_model: request.source_model.clone(),
        source_model_format: request.source_model_format.clone(),
        source_model_bytes: request.source_model_bytes,
        producing_run: request.producing_run.clone(),
        training_snapshot: request.training_snapshot.clone(),
        trainer: request.trainer.clone(),
        effective_configuration_fingerprint: request.effective_configuration_fingerprint.clone(),
        source_revision: request.source_revision.clone(),
    };
    let registered = register_completed_model(&destination, &candidate, registration.clone())
        .await
        .unwrap();
    let registered_catalog = registered.model_catalog.as_ref().unwrap();
    assert_eq!(registered_catalog.artifacts.len(), 2);
    assert_eq!(registered_catalog.active_baseline_revision_id, expected);
    assert_eq!(registered_catalog.baseline_revisions.len(), 1);
    let again = register_completed_model(&destination, &candidate, registration.clone())
        .await
        .unwrap();
    assert_eq!(again.model_catalog.as_ref().unwrap(), registered_catalog);
    let mut invalid = registration;
    invalid.training_snapshot.fingerprint = digest('9');
    assert!(
        register_completed_model(&destination, &candidate, invalid)
            .await
            .is_err()
    );
    let promoted = record_accepted_model_promotion(&destination, &candidate, request.clone())
        .await
        .unwrap();
    let catalog = promoted.model_catalog.as_ref().unwrap();
    assert_eq!(
        catalog.artifacts.len(),
        2,
        "promotion reuses the registered model"
    );
    assert_eq!(
        catalog.active_model().id,
        registered_catalog.artifacts[1].id
    );
    assert_eq!(catalog.baseline_revisions.len(), 2);
    assert_eq!(catalog.active_model().name, "Accepted candidate");
    assert_eq!(
        catalog.active_model().fingerprint,
        candidate_model.fingerprint
    );
    assert_eq!(
        catalog.active_model().source_model.as_ref().unwrap(),
        &request.source_model
    );
    assert!(destination.join(&catalog.active_model().path).is_dir());
    let again = record_accepted_model_promotion(&destination, &candidate, request)
        .await
        .unwrap();
    assert_eq!(again.model_catalog.unwrap(), *catalog);
    fs::write(
        destination
            .join(&catalog.active_model().path)
            .join("candidate.json"),
        b"{\"accepted\":false}\n",
    )
    .unwrap();
    assert!(open_workspace(&destination, true).await.is_err());
}

#[tokio::test]
async fn baseline_restoration_appends_once_and_preserves_every_artifact() {
    use sqlx::{Connection, SqliteConnection};

    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("restoration-project");
    let created = create_workspace(
        &destination,
        "Restoration fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let initial_revision = created
        .model_catalog
        .as_ref()
        .unwrap()
        .active_baseline_revision_id;
    let initial_model = created.model_catalog.as_ref().unwrap().active_model().id;
    let candidate = temp.path().join("restoration-candidate");
    training_transformer::fixture::write_tiny_bert_bundle(&candidate).unwrap();
    fs::write(candidate.join("candidate.json"), b"{\"accepted\":true}\n").unwrap();
    let candidate_model = inspect_model(&candidate).unwrap();
    let promoted = record_accepted_model_promotion(
        &destination,
        &candidate,
        AcceptedModelPromotion {
            expected_baseline_revision_id: initial_revision,
            name: "Accepted candidate".into(),
            source_model: BoundIdentity {
                id: uuid::Uuid::new_v4().to_string(),
                fingerprint: digest('d'),
            },
            source_model_format: candidate_model.format,
            source_model_bytes: candidate_model.bytes,
            producing_run: BoundIdentity {
                id: uuid::Uuid::new_v4().to_string(),
                fingerprint: digest('e'),
            },
            training_snapshot: BoundIdentity {
                id: uuid::Uuid::new_v4().to_string(),
                fingerprint: digest('f'),
            },
            trainer: BoundIdentity {
                id: "fixture-trainer:v1".into(),
                fingerprint: digest('1'),
            },
            effective_configuration_fingerprint: digest('2'),
            source_revision: "fixture-source-revision".into(),
            decision_id: "experiment-final:restoration:9".into(),
            decision_fingerprint: digest('3'),
            actor: "fixture-operator".into(),
            reason: "Passed final evaluation".into(),
        },
    )
    .await
    .unwrap();
    let promoted_catalog = promoted.model_catalog.as_ref().unwrap();
    let promoted_revision = promoted_catalog.active_baseline_revision_id;
    let request = BaselineRestorationRequest {
        revision_id: uuid::Uuid::new_v4(),
        expected_baseline_revision_id: promoted_revision,
        target_revision_id: initial_revision,
        actor: "fixture-operator".into(),
        reason: "Restore previous baseline".into(),
    };
    let restored = record_baseline_restoration(&destination, request.clone())
        .await
        .unwrap();
    let catalog = restored.model_catalog.as_ref().unwrap();
    assert_eq!(catalog.active_baseline_revision_id, request.revision_id);
    assert_eq!(catalog.active_model().id, initial_model);
    assert_eq!(catalog.artifacts, promoted_catalog.artifacts);
    assert_eq!(catalog.baseline_revisions.len(), 3);
    assert_eq!(
        record_baseline_restoration(&destination, request)
            .await
            .unwrap()
            .model_catalog
            .unwrap(),
        *catalog
    );

    let stale = BaselineRestorationRequest {
        revision_id: uuid::Uuid::new_v4(),
        expected_baseline_revision_id: promoted_revision,
        target_revision_id: initial_revision,
        actor: "fixture-operator".into(),
        reason: "Stale request".into(),
    };
    assert!(
        record_baseline_restoration(&destination, stale)
            .await
            .is_err()
    );
    let url = format!("sqlite://{}", destination.join("project.sqlite").display());
    let mut database = SqliteConnection::connect(&url).await.unwrap();
    assert!(
        sqlx::query("UPDATE baseline_revisions SET change_kind='changed' WHERE id=?")
            .bind(initial_revision.to_string())
            .execute(&mut database)
            .await
            .unwrap_err()
            .to_string()
            .contains("Baseline revisions are immutable")
    );
    assert!(
        sqlx::query("DELETE FROM model_artifacts WHERE id=?")
            .bind(initial_model.to_string())
            .execute(&mut database)
            .await
            .unwrap_err()
            .to_string()
            .contains("Model artifacts are immutable")
    );
}

#[tokio::test]
async fn stale_promotion_does_not_copy_or_change_catalog_state() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    let created = create_workspace(
        &destination,
        "Stale promotion fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    let request = AcceptedModelPromotion {
        expected_baseline_revision_id: uuid::Uuid::new_v4(),
        name: "Stale candidate".into(),
        source_model: BoundIdentity {
            id: "source-model".into(),
            fingerprint: digest('d'),
        },
        source_model_format: preview.format.clone(),
        source_model_bytes: preview.bytes,
        producing_run: BoundIdentity {
            id: "run".into(),
            fingerprint: digest('e'),
        },
        training_snapshot: BoundIdentity {
            id: "snapshot".into(),
            fingerprint: digest('f'),
        },
        trainer: BoundIdentity {
            id: "trainer".into(),
            fingerprint: digest('1'),
        },
        effective_configuration_fingerprint: digest('2'),
        source_revision: "revision".into(),
        decision_id: "decision".into(),
        decision_fingerprint: digest('3'),
        actor: "operator".into(),
        reason: "stale".into(),
    };
    assert!(
        record_accepted_model_promotion(&destination, &source, request)
            .await
            .is_err()
    );
    let reopened = open_workspace(&destination, true).await.unwrap();
    assert_eq!(reopened.model_catalog, created.model_catalog);
    assert_eq!(
        fs::read_dir(destination.join("models/candidates"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn scientific_bindings_are_project_and_baseline_scoped_compare_and_append_records() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    let workspace = create_workspace(
        &destination,
        "Binding fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    assert!(workspace.scientific_binding.is_none());
    let first_id = uuid::Uuid::new_v4();
    let first = scientific_binding(&workspace, first_id, None);
    let bound = record_scientific_binding(&destination, first.clone(), None)
        .await
        .unwrap();
    assert_eq!(bound.scientific_binding, Some(first.clone()));
    assert_eq!(
        record_scientific_binding(&destination, first, None)
            .await
            .unwrap()
            .scientific_binding
            .unwrap()
            .id,
        first_id
    );

    let second_id = uuid::Uuid::new_v4();
    let second = scientific_binding(&bound, second_id, Some(first_id));
    assert!(
        record_scientific_binding(&destination, second.clone(), None)
            .await
            .is_err()
    );
    let rebound = record_scientific_binding(&destination, second, Some(first_id))
        .await
        .unwrap();
    assert_eq!(rebound.scientific_binding.as_ref().unwrap().id, second_id);

    let mut foreign = scientific_binding(&rebound, uuid::Uuid::new_v4(), Some(second_id));
    foreign.project_id = uuid::Uuid::new_v4();
    foreign.specification_fingerprint = foreign.reproduce_specification_fingerprint().unwrap();
    foreign.fingerprint = foreign.reproduce_fingerprint().unwrap();
    assert!(
        record_scientific_binding(&destination, foreign, Some(second_id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn provider_settings_are_non_secret_append_only_compare_and_append_records() {
    let temp = TempDir::new().unwrap();
    let source = model(temp.path());
    let preview = inspect_model(&source).unwrap();
    let destination = temp.path().join("managed");
    let workspace = create_workspace(
        &destination,
        "Provider fixture",
        &source,
        &preview.fingerprint,
        None,
    )
    .await
    .unwrap();
    assert!(workspace.provider_catalog.is_none());
    let first_id = uuid::Uuid::new_v4();
    let first = provider_catalog(&workspace, first_id, None, 1);
    let configured = record_provider_catalog(&destination, first.clone(), None)
        .await
        .unwrap();
    assert!(!configured.verified);
    assert_eq!(configured.provider_catalog, Some(first.clone()));
    assert!(!serde_json::to_string(&first).unwrap().contains("sk-"));
    assert_eq!(
        record_provider_catalog(&destination, first, None)
            .await
            .unwrap()
            .provider_catalog
            .unwrap()
            .id,
        first_id
    );

    let second_id = uuid::Uuid::new_v4();
    let second = provider_catalog(&configured, second_id, Some(first_id), 2);
    assert!(
        record_provider_catalog(&destination, second.clone(), None)
            .await
            .is_err()
    );
    let updated = record_provider_catalog(&destination, second, Some(first_id))
        .await
        .unwrap();
    assert!(!updated.verified);
    assert_eq!(updated.provider_catalog.unwrap().id, second_id);
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
