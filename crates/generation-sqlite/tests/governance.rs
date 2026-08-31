use chrono::Utc;
use dataset_core::domain::{SnapshotSplit, SplitConfiguration, SplitRatios};
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    governance::{
        AdaptiveRiskLevel, CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision,
        DisclosureLevel, EvaluationCohort, EvidenceExposure, EvidenceExposureRequest,
        ExposurePurpose, summarize_exposure_risk,
    },
    ports::{CohortQuery, ExposureQuery, GovernanceStore},
};

#[tokio::test]
async fn roles_and_exposures_are_append_only_and_sealed_resolution_is_atomic() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("governance.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let dataset = DatasetDefinition::new(
        "support",
        "classify support requests",
        vec!["billing".into(), "fraud".into()],
        Vec::new(),
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("dataset persisted");
    let snapshot_id = Uuid::new_v4();
    let snapshot_fingerprint = "sha256:governance-fixture";
    let split = SplitConfiguration::new(SplitRatios::new(0.8, 0.1, 0.1).expect("ratios"), 42);
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, member_count, \
          fingerprint, created_at) VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
    )
    .bind(snapshot_id)
    .bind(dataset.id)
    .bind("governance fixture")
    .bind(serde_json::to_string(&split).expect("split JSON"))
    .bind(1_i64)
    .bind(snapshot_fingerprint)
    .bind(Utc::now())
    .execute(store.pool())
    .await
    .expect("snapshot fixture persisted");

    let sealed = EvaluationCohort::new(
        "release holdout",
        snapshot_id,
        snapshot_fingerprint,
        SnapshotSplit::Test,
        CohortOrigin::InternalSnapshot,
    )
    .expect("cohort");
    let sealed_role = CohortRoleDecision::initial(
        &sealed,
        CohortRole::SealedAcceptance,
        "fresh untouched test cohort",
    )
    .expect("role");
    store
        .create_cohort(&sealed, &sealed_role)
        .await
        .expect("cohort persisted");

    let exposure = EvidenceExposure::new(
        &sealed,
        &sealed_role,
        EvidenceExposureRequest {
            evaluation_run_id: None,
            workflow_run_id: None,
            workflow_iteration: None,
            purpose: ExposurePurpose::ManualInspection,
            disclosure: DisclosureLevel::RowContent,
            adaptation_eligible: false,
            note: Some("inspect failed release".into()),
        },
    )
    .expect("exposure");
    assert!(
        store.append_exposure(&exposure, None).await.is_err(),
        "row disclosure must not persist without an append-only resolution"
    );
    assert!(
        store
            .query_exposures(ExposureQuery {
                cohort_id: sealed.id,
                purpose: None,
                limit: 10,
                offset: 0,
            })
            .await
            .expect("exposure query")
            .is_empty()
    );

    let retirement = CohortRoleDecision::transition(
        &sealed_role,
        CohortRole::SealedAcceptance,
        CohortDisposition::Retired,
        "row contents were disclosed",
    )
    .expect("retirement");
    store
        .append_exposure(&exposure, Some(&retirement))
        .await
        .expect("exposure and retirement persisted");

    assert_eq!(
        store.get_cohort(sealed.id).await.expect("cohort query"),
        Some(sealed.clone())
    );
    assert_eq!(
        store
            .query_cohorts(CohortQuery {
                snapshot_id: Some(snapshot_id),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("cohort list"),
        vec![sealed.clone()]
    );
    assert_eq!(
        store
            .list_cohort_role_history(sealed.id)
            .await
            .expect("role history"),
        vec![sealed_role, retirement.clone()]
    );
    assert_eq!(
        store
            .get_current_cohort_role(sealed.id)
            .await
            .expect("current role"),
        Some(retirement)
    );
    let exposures = store
        .query_exposures(ExposureQuery {
            cohort_id: sealed.id,
            purpose: Some(ExposurePurpose::ManualInspection),
            limit: 10,
            offset: 0,
        })
        .await
        .expect("exposures");
    assert_eq!(exposures, vec![exposure]);
    assert_eq!(
        summarize_exposure_risk(sealed.id, &exposures)
            .expect("risk")
            .risk,
        AdaptiveRiskLevel::Compromised
    );

    let development = EvaluationCohort::new(
        "adaptive development",
        snapshot_id,
        snapshot_fingerprint,
        SnapshotSplit::Test,
        CohortOrigin::InternalSnapshot,
    )
    .expect("development cohort");
    let development_role = CohortRoleDecision::initial(
        &development,
        CohortRole::Development,
        "adaptive diagnostics are explicitly permitted",
    )
    .expect("development role");
    store
        .create_cohort(&development, &development_role)
        .await
        .expect("development cohort persisted");
    let architecture_exposure = EvidenceExposure::new(
        &development,
        &development_role,
        EvidenceExposureRequest {
            evaluation_run_id: None,
            workflow_run_id: None,
            workflow_iteration: None,
            purpose: ExposurePurpose::DatasetArchitecture,
            disclosure: DisclosureLevel::Slices,
            adaptation_eligible: true,
            note: Some("aggregate diagnostics informed dataset architecture".into()),
        },
    )
    .expect("dataset architecture exposure");
    store
        .append_exposure(&architecture_exposure, None)
        .await
        .expect("dataset architecture exposure persisted");
    assert_eq!(
        store
            .query_exposures(ExposureQuery {
                cohort_id: development.id,
                purpose: Some(ExposurePurpose::DatasetArchitecture),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("architecture exposure query"),
        vec![architecture_exposure]
    );
}
