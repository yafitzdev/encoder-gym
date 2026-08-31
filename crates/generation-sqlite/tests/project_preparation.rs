use std::collections::BTreeMap;

use artifact_core::{ArtifactKind, ProvenanceStore};
use chrono::Utc;
use dataset_core::{
    domain::{
        DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceProvenance, SourceRow,
        SplitConfiguration, SplitRatios,
    },
    ports::SnapshotStore,
    splitting::{build_snapshot, reproduce_snapshot_fingerprint},
};
use generation_core::ports::DatasetStore;
use project_config::{ProjectConfig, ProjectOverrides};
use project_preparation::{
    BenchmarkQualificationManifest, CohortEvidence, CohortManifest, ContaminationManifest,
    PreparationEvidence, PreparationManifest, PreparationStore, SuiteManifest, WorkflowManifest,
    compile_project,
};
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{
        TrainingConfiguration, TrainingExample, TrainingInputBinding, TrainingRequest, TrainingRun,
    },
    ports::TrainingStore,
};
use uuid::Uuid;
use workflow_core::{
    allocation::InitialAllocationPolicy,
    benchmark::{AcceptanceContract, BenchmarkMetric, MetricRequirement, MetricTarget},
    benchmark_qualification::{
        BenchmarkQualificationPolicy, BenchmarkQualificationReviewDecision,
        BenchmarkQualificationReviewRequest, BenchmarkReadiness, QualificationConfidence,
        QualificationPopulation, qualify_benchmark_bundle, review_benchmark_qualification,
    },
    contamination::{
        CohortContaminationInput, ContaminationKind, ContaminationMember, ContaminationPolicy,
        ContaminationStatus, check_contamination,
    },
    governance::{CohortOrigin, CohortRole, CohortRoleDecision, DisclosureLevel, EvaluationCohort},
    ports::{
        BenchmarkBundleQuery, BenchmarkBundleStore, BenchmarkQualificationQuery,
        BenchmarkQualificationStore, ContaminationStore, GovernanceStore,
        TrainingBenchmarkCheckQuery, TrainingBenchmarkCheckStore,
    },
    training_benchmark::{
        TrainingCohortEvidence, TrainingInputProtocol, build_training_benchmark_check,
    },
    workflow::{IterationGovernance, WorkflowBudget, WorkflowPolicy},
};

const PROJECT: &str = r#"
version = 1

[dataset]
name = "support"
task = "Classify support requests"
labels = ["billing", "fraud"]

[[dataset.dimensions]]
name = "style"
values = ["clean", "messy"]

[generation]
target_per_cell = 10
batch_size = 20
"#;

#[tokio::test]
async fn benchmark_qualification_is_recomputed_from_snapshot_evidence() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("benchmark-qualification.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = fixture(&store).await;
    let bundle = compile_project(&manifest, &evidence).expect("bundle");
    store
        .create_preparation(&bundle)
        .await
        .expect("preparation persists");
    let cohort = &bundle.development_suite.cohorts[0];
    let members = store
        .list_snapshot_members(cohort.snapshot_id)
        .await
        .expect("snapshot members")
        .into_iter()
        .filter(|member| member.split == cohort.split)
        .collect::<Vec<_>>();
    let policy = BenchmarkQualificationPolicy {
        minimum_overall_support: 1,
        minimum_label_support: 1,
        maximum_proportion_margin_of_error: 0.49,
        maximum_normalized_duplicate_rate: 1.0,
        ..BenchmarkQualificationPolicy::default()
    };
    let qualification = qualify_benchmark_bundle(
        &bundle.benchmark_bundle,
        &bundle.development_suite,
        None,
        vec![QualificationPopulation {
            cohort_id: cohort.cohort_id,
            members: &members,
        }],
        policy,
    )
    .expect("qualification");
    assert_eq!(qualification.readiness, BenchmarkReadiness::Blocked);
    store
        .create_benchmark_qualification(&qualification)
        .await
        .expect("qualification persists");
    assert_eq!(
        store
            .get_benchmark_qualification(qualification.id)
            .await
            .expect("qualification read"),
        Some(qualification.clone())
    );
    assert_eq!(
        store
            .get_executable_benchmark_qualification(qualification.id)
            .await
            .expect("executable qualification read"),
        Some(qualification.clone())
    );
    assert_eq!(
        store
            .query_benchmark_qualifications(BenchmarkQualificationQuery {
                benchmark_bundle_id: Some(bundle.benchmark_bundle.id),
                readiness: Some(BenchmarkReadiness::Blocked),
                ..BenchmarkQualificationQuery::default()
            })
            .await
            .expect("qualification list"),
        vec![qualification.clone()]
    );
    let provenance = store
        .trace_provenance(ArtifactKind::BenchmarkQualification, qualification.id)
        .await
        .expect("qualification provenance")
        .expect("qualification node");
    assert_eq!(provenance.kind, ArtifactKind::BenchmarkQualification);
    assert_eq!(provenance.parents.len(), 1);
    assert_eq!(provenance.parents[0].kind, ArtifactKind::BenchmarkBundle);

    let review = review_benchmark_qualification(
        &qualification,
        BenchmarkQualificationReviewRequest {
            decision: BenchmarkQualificationReviewDecision::Reject,
            reviewed_by: "test operator".into(),
            rationale: "missing label support".into(),
        },
    )
    .expect("review");
    store
        .create_benchmark_qualification_review(&review)
        .await
        .expect("review persists");
    assert_eq!(
        store
            .get_benchmark_qualification_review(review.id)
            .await
            .expect("review read"),
        Some(review.clone())
    );
    assert_eq!(
        store
            .get_benchmark_qualification_review_for_qualification(qualification.id)
            .await
            .expect("review by qualification"),
        Some(review.clone())
    );
    let review_provenance = store
        .trace_provenance(ArtifactKind::BenchmarkQualificationReview, review.id)
        .await
        .expect("review provenance")
        .expect("review node");
    assert_eq!(
        review_provenance.parents[0].kind,
        ArtifactKind::BenchmarkQualification
    );

    sqlx::query("UPDATE dataset_snapshot_members SET label = 'fraud' WHERE id = ?")
        .bind(
            members
                .iter()
                .find(|member| member.label == "billing")
                .expect("billing member")
                .id,
        )
        .execute(store.pool())
        .await
        .expect("tamper population");
    assert!(
        store
            .get_benchmark_qualification(qualification.id)
            .await
            .is_err(),
        "population tampering must invalidate readiness evidence"
    );
}

#[tokio::test]
async fn preparation_bundle_is_atomic_queryable_and_idempotent_by_manifest() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("preparation.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = fixture(&store).await;

    let mut stale_evidence = evidence.clone();
    stale_evidence
        .cohorts
        .values_mut()
        .next()
        .expect("cohort")
        .snapshot
        .fingerprint = "sha256:changed-after-load".into();
    let stale_bundle = compile_project(&manifest, &stale_evidence).expect("stale bundle compiles");
    assert!(store.create_preparation(&stale_bundle).await.is_err());
    assert_eq!(table_count(&store, "dataset_definitions").await, 1);
    assert_eq!(table_count(&store, "project_configurations").await, 0);
    assert_eq!(table_count(&store, "workflow_evaluation_cohorts").await, 0);
    assert_eq!(table_count(&store, "workflow_benchmark_bundles").await, 0);

    let first_bundle = compile_project(&manifest, &evidence).expect("bundle");
    let first = store
        .create_preparation(&first_bundle)
        .await
        .expect("preparation persists");
    let second_bundle = compile_project(&manifest, &evidence).expect("fresh bundle");
    let second = store
        .create_preparation(&second_bundle)
        .await
        .expect("repeat is idempotent");

    assert_eq!(second, first);
    assert_ne!(second_bundle.preparation.id, first.id);
    assert_eq!(
        store
            .get_preparation(first.id)
            .await
            .expect("query")
            .expect("preparation"),
        first
    );
    assert_eq!(
        store
            .get_preparation_by_manifest(&first.manifest_fingerprint)
            .await
            .expect("query by manifest")
            .expect("preparation"),
        first
    );
    assert_eq!(
        store
            .list_preparations(10, 0)
            .await
            .expect("list preparations"),
        vec![first.clone()]
    );
    assert_eq!(table_count(&store, "project_preparations").await, 1);
    assert_eq!(table_count(&store, "project_configurations").await, 1);
    assert_eq!(table_count(&store, "workflow_evaluation_cohorts").await, 1);
    assert_eq!(
        table_count(&store, "workflow_contamination_reports").await,
        1
    );
    assert_eq!(table_count(&store, "workflow_benchmark_suites").await, 1);
    assert_eq!(table_count(&store, "workflow_benchmark_bundles").await, 1);
    assert_eq!(table_count(&store, "workflow_definitions").await, 1);
    assert_eq!(
        store
            .get_benchmark_bundle(first_bundle.benchmark_bundle.id)
            .await
            .expect("bundle query")
            .expect("benchmark bundle"),
        first_bundle.benchmark_bundle
    );
    assert_eq!(
        store
            .query_benchmark_bundles(BenchmarkBundleQuery {
                development_suite_id: Some(first_bundle.development_suite.id),
                sealed_suite_id: None,
                contamination_report_id: Some(
                    first_bundle.benchmark_bundle.contamination_report_id,
                ),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("bundle list"),
        vec![first_bundle.benchmark_bundle.clone()]
    );
    assert!(
        store
            .get_dataset(first.dataset_id)
            .await
            .expect("dataset query")
            .is_some()
    );
    let provenance = store
        .trace_provenance(ArtifactKind::ProjectPreparation, first.id)
        .await
        .expect("provenance query")
        .expect("preparation provenance");
    assert_eq!(provenance.kind, ArtifactKind::ProjectPreparation);
    assert_eq!(provenance.parents[0].kind, ArtifactKind::WorkflowDefinition);
    let definition = &provenance.parents[0];
    let benchmark_bundle = definition
        .parents
        .iter()
        .find(|parent| parent.kind == ArtifactKind::BenchmarkBundle)
        .expect("workflow definition benchmark bundle parent");
    assert_eq!(benchmark_bundle.id, first_bundle.benchmark_bundle.id);
    assert_eq!(
        benchmark_bundle.fingerprint.as_deref(),
        Some(first_bundle.benchmark_bundle.fingerprint.as_str())
    );
    assert_eq!(
        benchmark_bundle
            .parents
            .iter()
            .map(|parent| parent.kind)
            .collect::<Vec<_>>(),
        vec![
            ArtifactKind::BenchmarkSuite,
            ArtifactKind::ContaminationReport,
        ]
    );
    assert_eq!(
        benchmark_bundle.parents[0].id,
        first_bundle.development_suite.id
    );
    assert_eq!(
        benchmark_bundle.parents[1].id,
        first_bundle.benchmark_bundle.contamination_report_id
    );
    assert!(
        benchmark_bundle
            .parents
            .iter()
            .all(|parent| parent.parents.is_empty()),
        "suite and contamination evidence stay bounded leaves"
    );

    let direct_bundle = store
        .trace_provenance(
            ArtifactKind::BenchmarkBundle,
            first_bundle.benchmark_bundle.id,
        )
        .await
        .expect("bundle provenance query")
        .expect("bundle provenance");
    assert_eq!(direct_bundle, *benchmark_bundle);
    assert!(
        store
            .trace_provenance(
                ArtifactKind::BenchmarkSuite,
                first_bundle.development_suite.id,
            )
            .await
            .expect("suite provenance query")
            .is_some()
    );
    assert!(
        store
            .trace_provenance(
                ArtifactKind::ContaminationReport,
                first_bundle.benchmark_bundle.contamination_report_id,
            )
            .await
            .expect("contamination provenance query")
            .is_some()
    );

    let mut forged_bundle = first_bundle.benchmark_bundle.clone();
    forged_bundle.id = Uuid::new_v4();
    forged_bundle.development_suite_fingerprint = "sha256:self-consistent-forgery".into();
    forged_bundle.fingerprint = forged_bundle
        .reproduce_fingerprint()
        .expect("forged bundle fingerprint");
    forged_bundle
        .validate_fingerprint()
        .expect("forgery is internally self-consistent");
    assert!(
        store.create_benchmark_bundle(&forged_bundle).await.is_err(),
        "persistence must reconstruct authority from persisted evidence, not trust a rehash"
    );
    assert_eq!(table_count(&store, "workflow_benchmark_bundles").await, 1);

    let suite_cohort = &first_bundle.development_suite.cohorts[0];
    sqlx::query(
        "DELETE FROM workflow_benchmark_suite_cohorts WHERE suite_id = ? AND cohort_id = ?",
    )
    .bind(first_bundle.development_suite.id)
    .bind(suite_cohort.cohort_id)
    .execute(store.pool())
    .await
    .expect("tamper normalized suite index");
    assert!(
        store
            .get_benchmark_bundle(first_bundle.benchmark_bundle.id)
            .await
            .is_err(),
        "bundle reads must fail closed when a normalized child index diverges"
    );
    assert!(
        store
            .trace_provenance(
                ArtifactKind::WorkflowDefinition,
                first_bundle.workflow_definition.id,
            )
            .await
            .is_err(),
        "workflow provenance must fail closed when its exact bundle is corrupt"
    );
    sqlx::query(
        "INSERT INTO workflow_benchmark_suite_cohorts \
         (suite_id, cohort_id, role_decision_id, protocol_fingerprint) VALUES (?, ?, ?, ?)",
    )
    .bind(first_bundle.development_suite.id)
    .bind(suite_cohort.cohort_id)
    .bind(suite_cohort.role_decision_id)
    .bind(&suite_cohort.protocol_fingerprint)
    .execute(store.pool())
    .await
    .expect("restore normalized suite index");
}

#[tokio::test]
async fn legacy_workflow_definition_provenance_remains_readable_without_a_bundle() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("legacy-provenance.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = fixture(&store).await;
    let bundle = compile_project(&manifest, &evidence).expect("bundle");
    store
        .create_preparation(&bundle)
        .await
        .expect("preparation persists");

    let mut legacy = bundle.workflow_definition.clone();
    legacy.id = Uuid::new_v4();
    legacy.benchmark_bundle = None;
    legacy.benchmark_qualification = None;
    legacy.fingerprint = legacy
        .reproduce_fingerprint()
        .expect("legacy fingerprint reproduces");
    sqlx::query(
        "INSERT INTO workflow_definitions \
         (id, name, dataset_id, project_configuration_id, development_suite_id, \
          sealed_suite_id, benchmark_bundle_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
    )
    .bind(legacy.id)
    .bind(&legacy.name)
    .bind(legacy.dataset_id)
    .bind(legacy.project_configuration_id)
    .bind(legacy.development_suite_id)
    .bind(legacy.sealed_suite_id)
    .bind(serde_json::to_string(&legacy).expect("legacy definition JSON"))
    .bind(&legacy.fingerprint)
    .bind(legacy.created_at)
    .execute(store.pool())
    .await
    .expect("install legacy definition shape");

    let mut legacy_preparation = bundle.preparation.clone();
    legacy_preparation.id = Uuid::new_v4();
    legacy_preparation.manifest_fingerprint = "sha256:legacy-manifest".into();
    legacy_preparation.workflow_definition_id = legacy.id;
    legacy_preparation.benchmark_bundle_id = None;
    legacy_preparation.benchmark_bundle_fingerprint = None;
    legacy_preparation.benchmark_qualification_id = None;
    legacy_preparation.benchmark_qualification_fingerprint = None;
    legacy_preparation.benchmark_qualification_review_id = None;
    legacy_preparation.benchmark_qualification_review_fingerprint = None;
    legacy_preparation.fingerprint = legacy_preparation
        .reproduce_fingerprint()
        .expect("legacy preparation fingerprint reproduces");
    sqlx::query(
        "INSERT INTO project_preparations \
         (id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
          development_suite_id, sealed_suite_id, benchmark_bundle_id, workflow_definition_id, \
          artifact_json, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)",
    )
    .bind(legacy_preparation.id)
    .bind(&legacy_preparation.name)
    .bind(&legacy_preparation.manifest_fingerprint)
    .bind(legacy_preparation.dataset_id)
    .bind(legacy_preparation.project_configuration_id)
    .bind(legacy_preparation.development_suite_id)
    .bind(legacy_preparation.workflow_definition_id)
    .bind(serde_json::to_string(&legacy_preparation).expect("legacy preparation JSON"))
    .bind(&legacy_preparation.fingerprint)
    .bind(legacy_preparation.created_at)
    .execute(store.pool())
    .await
    .expect("install legacy preparation shape");

    let provenance = store
        .trace_provenance(ArtifactKind::ProjectPreparation, legacy_preparation.id)
        .await
        .expect("legacy provenance query")
        .expect("legacy preparation provenance");
    assert_eq!(provenance.kind, ArtifactKind::ProjectPreparation);
    let definition = provenance
        .parents
        .first()
        .expect("legacy workflow definition parent");
    assert_eq!(definition.kind, ArtifactKind::WorkflowDefinition);
    assert!(
        definition
            .parents
            .iter()
            .all(|parent| parent.kind != ArtifactKind::BenchmarkBundle)
    );
}

#[tokio::test]
async fn benchmark_bundle_provenance_includes_the_optional_sealed_suite() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("sealed-provenance.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = sealed_fixture(&store).await;
    let bundle = compile_project(&manifest, &evidence).expect("sealed bundle");
    store
        .create_preparation(&bundle)
        .await
        .expect("sealed preparation persists");

    let sealed_suite = bundle.sealed_suite.as_ref().expect("sealed suite");
    let provenance = store
        .trace_provenance(ArtifactKind::BenchmarkBundle, bundle.benchmark_bundle.id)
        .await
        .expect("bundle provenance query")
        .expect("bundle provenance");
    assert_eq!(
        provenance
            .parents
            .iter()
            .map(|parent| (parent.kind, parent.id))
            .collect::<Vec<_>>(),
        vec![
            (ArtifactKind::BenchmarkSuite, bundle.development_suite.id,),
            (ArtifactKind::BenchmarkSuite, sealed_suite.id),
            (
                ArtifactKind::ContaminationReport,
                bundle.benchmark_bundle.contamination_report_id,
            ),
        ]
    );
    assert!(
        provenance
            .parents
            .iter()
            .all(|parent| parent.parents.is_empty())
    );
}

#[tokio::test]
async fn contamination_persistence_rejects_a_self_consistent_fake_clean_report_from_members() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("forged-clean-bundle.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, mut evidence) = sealed_fixture(&store).await;
    let development_snapshot_id = manifest.development.cohorts[0].snapshot_id;
    let sealed_snapshot_id =
        manifest.sealed.as_ref().expect("sealed manifest").cohorts[0].snapshot_id;

    let sealed_evidence = evidence
        .cohorts
        .get_mut(&sealed_snapshot_id)
        .expect("sealed evidence");
    sealed_evidence.members[0]
        .dimensions
        .insert("style".into(), "clean".into());
    sealed_evidence.snapshot.fingerprint =
        reproduce_snapshot_fingerprint(&sealed_evidence.snapshot, &sealed_evidence.members)
            .expect("updated sealed snapshot fingerprint");
    sqlx::query(
        "UPDATE dataset_snapshot_members SET dimensions_json = ? \
         WHERE id = ?",
    )
    .bind(r#"{"style":"clean"}"#)
    .bind(sealed_evidence.members[0].id)
    .execute(store.pool())
    .await
    .expect("persist group overlap");
    sqlx::query("UPDATE dataset_snapshots SET fingerprint = ? WHERE id = ?")
        .bind(&sealed_evidence.snapshot.fingerprint)
        .bind(sealed_snapshot_id)
        .execute(store.pool())
        .await
        .expect("persist recomputed sealed snapshot fingerprint");
    let sealed_evidence = sealed_evidence.clone();

    let development_evidence = evidence
        .cohorts
        .get(&development_snapshot_id)
        .expect("development evidence")
        .clone();
    let development_cohort = EvaluationCohort::new(
        "development test",
        development_snapshot_id,
        development_evidence.snapshot.fingerprint.clone(),
        SnapshotSplit::Test,
        CohortOrigin::InternalSnapshot,
    )
    .expect("development cohort");
    let development_role = CohortRoleDecision::initial(
        &development_cohort,
        CohortRole::Development,
        "adversarial fixture",
    )
    .expect("development role");
    let sealed_cohort = EvaluationCohort::new(
        "sealed test",
        sealed_snapshot_id,
        sealed_evidence.snapshot.fingerprint.clone(),
        SnapshotSplit::Test,
        CohortOrigin::InternalSnapshot,
    )
    .expect("sealed cohort");
    let sealed_role = CohortRoleDecision::initial(
        &sealed_cohort,
        CohortRole::SealedAcceptance,
        "adversarial fixture",
    )
    .expect("sealed role");
    store
        .create_cohort(&development_cohort, &development_role)
        .await
        .expect("development governance persists");
    store
        .create_cohort(&sealed_cohort, &sealed_role)
        .await
        .expect("sealed governance persists");

    let group_dimension = Some("style".to_owned());
    let development_input = CohortContaminationInput {
        cohort: development_cohort.clone(),
        role: development_role.clone(),
        members: development_evidence
            .members
            .iter()
            .map(|member| ContaminationMember::from_snapshot_member(member, Some("style")))
            .collect(),
    };
    let sealed_input = CohortContaminationInput {
        cohort: sealed_cohort.clone(),
        role: sealed_role.clone(),
        members: sealed_evidence
            .members
            .iter()
            .map(|member| ContaminationMember::from_snapshot_member(member, Some("style")))
            .collect(),
    };
    let mut fake_clean_report = check_contamination(
        vec![development_input, sealed_input],
        group_dimension,
        ContaminationPolicy::default(),
    )
    .expect("actual global contamination");
    assert_eq!(fake_clean_report.status, ContaminationStatus::Blocked);
    assert_eq!(fake_clean_report.counts[&ContaminationKind::Group], 1);
    assert_eq!(fake_clean_report.counts[&ContaminationKind::ExactText], 0);
    assert_eq!(
        fake_clean_report.counts[&ContaminationKind::NormalizedText],
        0
    );
    fake_clean_report
        .counts
        .values_mut()
        .for_each(|count| *count = 0);
    fake_clean_report.findings.clear();
    fake_clean_report.reasons.clear();
    fake_clean_report.status = ContaminationStatus::Clean;
    fake_clean_report.fingerprint = fake_clean_report
        .reproduce_fingerprint()
        .expect("fake-clean fingerprint");
    assert_eq!(
        fake_clean_report
            .reproduce_fingerprint()
            .expect("report fingerprint"),
        fake_clean_report.fingerprint,
        "the adversarial report is internally self-consistent"
    );
    let error = store
        .create_contamination_report(&fake_clean_report)
        .await
        .expect_err("persisted-member recomputation must reject the fake-clean report");
    assert!(
        error
            .to_string()
            .contains("differs from recomputed persisted-member evidence"),
        "unexpected error: {error}"
    );
    assert_eq!(
        table_count(&store, "workflow_contamination_reports").await,
        0
    );
    assert_eq!(table_count(&store, "workflow_benchmark_bundles").await, 0);
}

#[tokio::test]
async fn benchmark_bundle_read_rejects_persisted_member_tampering_before_report_reuse() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("group-tamper-bundle.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (mut manifest, evidence) = sealed_fixture(&store).await;
    manifest.contamination.group_dimension = Some("style".into());
    let bundle = compile_project(&manifest, &evidence).expect("group-aware sealed bundle");
    store
        .create_preparation(&bundle)
        .await
        .expect("valid preparation persists");

    let sealed_snapshot_id =
        manifest.sealed.as_ref().expect("sealed manifest").cohorts[0].snapshot_id;
    sqlx::query(
        "UPDATE dataset_snapshot_members SET dimensions_json = ? \
         WHERE snapshot_id = ? AND split = 'test'",
    )
    .bind(r#"{"style":"clean"}"#)
    .bind(sealed_snapshot_id)
    .execute(store.pool())
    .await
    .expect("introduce group overlap through persisted dimensions");

    let error = store
        .get_benchmark_bundle(bundle.benchmark_bundle.id)
        .await
        .expect_err("bundle read must reject group overlap hidden by the clean report");
    assert!(
        error
            .to_string()
            .contains("snapshot fingerprint does not reproduce"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn training_benchmark_check_is_atomic_queryable_and_deeply_verified() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("training-benchmark.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    let (manifest, evidence) = fixture(&store).await;
    let bundle = compile_project(&manifest, &evidence).expect("bundle");
    store
        .create_preparation(&bundle)
        .await
        .expect("preparation persists");

    let dataset_id = bundle.dataset.id;
    let now = Utc::now();
    let rows = (0..10)
        .map(|index| SourceRow {
            id: Uuid::new_v4(),
            dataset_id,
            text: format!("new training example {index}"),
            label: if index % 2 == 0 {
                "billing".into()
            } else {
                "fraud".into()
            },
            dimensions: BTreeMap::from([(
                "style".into(),
                if index % 2 == 0 {
                    "clean".into()
                } else {
                    "messy".into()
                },
            )]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Generated {
                generation_job_id: Uuid::new_v4(),
                backend: "fake".into(),
                model: "fake-v1".into(),
                construction_plan_fingerprint: None,
            },
            created_at: now,
        })
        .collect::<Vec<_>>();
    for row in &rows {
        sqlx::query(
            "INSERT INTO dataset_source_rows \
             (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
              dimensions_json, fields_json, provenance_json, created_at) \
             VALUES (?, ?, 'generated', ?, ?, ?, ?, ?, ?, '{}', ?, ?)",
        )
        .bind(row.id)
        .bind(row.dataset_id)
        .bind(row.id.to_string())
        .bind(format!("{}/style={}", row.label, row.dimensions["style"]))
        .bind(&row.text)
        .bind(row.text.to_lowercase())
        .bind(&row.label)
        .bind(serde_json::to_string(&row.dimensions).expect("dimensions"))
        .bind(serde_json::to_string(&row.provenance).expect("provenance"))
        .bind(row.created_at)
        .execute(store.pool())
        .await
        .expect("source row");
    }
    let (snapshot, members) = build_snapshot(
        dataset_id,
        "training snapshot",
        None,
        SplitConfiguration::new(SplitRatios::new(0.8, 0.2, 0.0).expect("ratios"), 7),
        rows,
    )
    .expect("training snapshot");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("training snapshot persists");

    let training = [SnapshotSplit::Train, SnapshotSplit::Validation]
        .into_iter()
        .filter(|split| members.iter().any(|member| member.split == *split))
        .map(|split| {
            let cohort = EvaluationCohort::new(
                format!("training {split:?}"),
                snapshot.id,
                snapshot.fingerprint.clone(),
                split,
                CohortOrigin::InternalSnapshot,
            )
            .expect("training cohort");
            let role = CohortRoleDecision::initial(&cohort, CohortRole::Training, "workflow")
                .expect("training role");
            TrainingCohortEvidence { cohort, role }
        })
        .collect::<Vec<_>>();
    let mut inputs = training
        .iter()
        .map(|evidence| CohortContaminationInput {
            cohort: evidence.cohort.clone(),
            role: evidence.role.clone(),
            members: members
                .iter()
                .filter(|member| member.split == evidence.cohort.split)
                .map(|member| ContaminationMember::from_snapshot_member(member, None))
                .collect(),
        })
        .collect::<Vec<_>>();
    let benchmark_evidence = evidence
        .cohorts
        .values()
        .next()
        .expect("benchmark evidence");
    let benchmark_cohort = bundle.cohorts.first().expect("benchmark cohort").clone();
    let benchmark_role = bundle
        .role_decisions
        .first()
        .expect("benchmark role")
        .clone();
    inputs.push(CohortContaminationInput {
        cohort: benchmark_cohort,
        role: benchmark_role,
        members: benchmark_evidence
            .members
            .iter()
            .map(|member| ContaminationMember::from_snapshot_member(member, None))
            .collect(),
    });
    let report = check_contamination(inputs, None, ContaminationPolicy::default())
        .expect("training contamination report");
    assert_eq!(report.status, ContaminationStatus::Clean);
    let benchmark_report = bundle
        .contamination_reports
        .iter()
        .find(|report| report.id == bundle.benchmark_bundle.contamination_report_id)
        .expect("benchmark report");
    let check = build_training_benchmark_check(
        &snapshot,
        &members,
        TrainingInputProtocol::TrainAndValidationV1,
        training.clone(),
        &bundle.benchmark_bundle,
        benchmark_report,
        &report,
    )
    .expect("check");
    let extra_cohort = EvaluationCohort::new(
        "unreferenced training cohort",
        snapshot.id,
        snapshot.fingerprint.clone(),
        SnapshotSplit::Train,
        CohortOrigin::InternalSnapshot,
    )
    .expect("extra cohort");
    let extra_role = CohortRoleDecision::initial(
        &extra_cohort,
        CohortRole::Training,
        "must not leak through composite persistence",
    )
    .expect("extra role");
    let mut excessive = training.clone();
    excessive.push(TrainingCohortEvidence {
        cohort: extra_cohort.clone(),
        role: extra_role,
    });
    assert!(
        store
            .create_training_benchmark_check(&excessive, &report, &check)
            .await
            .is_err(),
        "unreferenced composite cohorts must be rejected"
    );
    assert!(
        store
            .get_cohort(extra_cohort.id)
            .await
            .expect("extra cohort lookup")
            .is_none(),
        "rejected composite persistence must roll back every cohort"
    );
    store
        .create_training_benchmark_check(&training, &report, &check)
        .await
        .expect("atomic check persistence");
    assert_eq!(
        store
            .get_executable_training_benchmark_check(check.id)
            .await
            .expect("deep read"),
        Some(check.clone())
    );
    assert_eq!(
        store
            .query_training_benchmark_checks(TrainingBenchmarkCheckQuery {
                training_snapshot_id: Some(snapshot.id),
                benchmark_bundle_id: Some(bundle.benchmark_bundle.id),
                status: Some(ContaminationStatus::Clean),
                protocol: Some(TrainingInputProtocol::TrainAndValidationV1),
                check_protocol_version: Some(
                    workflow_core::training_benchmark::TRAINING_BENCHMARK_CHECK_PROTOCOL.into(),
                ),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("query"),
        vec![check.clone()]
    );

    let dataset = store
        .get_dataset(snapshot.source_dataset_id)
        .await
        .expect("dataset read")
        .expect("dataset exists");
    let training_examples = members
        .iter()
        .filter(|member| member.split == SnapshotSplit::Train)
        .map(|member| TrainingExample {
            snapshot_member_id: member.id,
            text: member.text.clone(),
            label: member.label.clone(),
        })
        .collect();
    let validation_examples = members
        .iter()
        .filter(|member| member.split == SnapshotSplit::Validation)
        .map(|member| TrainingExample {
            snapshot_member_id: member.id,
            text: member.text.clone(),
            label: member.label.clone(),
        })
        .collect();
    let input_fingerprint = TrainingRequest::in_memory(
        Uuid::nil(),
        snapshot.id,
        dataset.labels,
        training_examples,
        validation_examples,
        TrainingConfiguration::default(),
    )
    .reproduce_input_fingerprint()
    .expect("backend-facing input fingerprint");
    let binding = TrainingInputBinding {
        protocol: TrainingInputProtocol::TrainAndValidationV1
            .stable_name()
            .into(),
        population_fingerprint: check.training_population_fingerprint.clone(),
        member_count: check.training_member_count,
        input_fingerprint,
        authority_kind: "training_benchmark_check".into(),
        authority_id: check.id,
        authority_fingerprint: check.fingerprint.clone(),
    };
    let run = TrainingRun::queued(
        snapshot.id,
        "hashing-linear",
        "hashing-linear-v1",
        TrainingConfiguration::default(),
    )
    .expect("queued run")
    .with_input_binding(binding.clone())
    .expect("bound run");
    store
        .create_training_run(&run)
        .await
        .expect("clean authority permits run persistence");
    assert_eq!(
        store
            .get_training_run(run.id)
            .await
            .expect("run read")
            .expect("run exists")
            .input_binding,
        Some(binding.clone())
    );
    let mut wrong_binding = binding.clone();
    wrong_binding.population_fingerprint = "sha256:wrong-population".into();
    let wrong_run = TrainingRun::queued(
        snapshot.id,
        "hashing-linear",
        "hashing-linear-v1",
        TrainingConfiguration::default(),
    )
    .expect("queued run")
    .with_input_binding(wrong_binding)
    .expect("shaped binding");
    assert!(
        store.create_training_run(&wrong_run).await.is_err(),
        "a copied claim that differs from its check must not persist"
    );
    assert!(
        sqlx::query("UPDATE training_runs SET input_fingerprint = ? WHERE id = ?")
            .bind("sha256:tampered-input")
            .bind(run.id)
            .execute(store.pool())
            .await
            .is_err(),
        "training input authority must remain immutable"
    );
    assert!(
        sqlx::query("UPDATE workflow_training_benchmark_checks SET created_at = ? WHERE id = ?",)
            .bind(Utc::now())
            .bind(check.id)
            .execute(store.pool())
            .await
            .is_err(),
        "training-benchmark check rows must be immutable even outside their fingerprint"
    );
    assert!(
        sqlx::query("DELETE FROM workflow_training_benchmark_check_cohorts WHERE check_id = ?",)
            .bind(check.id)
            .execute(store.pool())
            .await
            .is_err(),
        "training-benchmark cohort bindings must be append-only"
    );

    let member = members
        .iter()
        .find(|member| member.split == SnapshotSplit::Train)
        .expect("train member");
    sqlx::query(
        "UPDATE dataset_snapshot_members SET text = 'tampered' \
         WHERE snapshot_id = ? AND id = ?",
    )
    .bind(snapshot.id)
    .bind(member.id)
    .execute(store.pool())
    .await
    .expect("tamper");
    assert!(
        store.get_training_benchmark_check(check.id).await.is_err(),
        "snapshot member tampering must invalidate the check"
    );
}

async fn fixture(store: &SqliteStore) -> (PreparationManifest, PreparationEvidence) {
    let project = ProjectConfig::parse(PROJECT).expect("project config");
    let source_dataset = project
        .clone()
        .resolve(ProjectOverrides::default())
        .expect("resolve")
        .dataset_definition()
        .expect("dataset");
    store
        .create_dataset(&source_dataset)
        .await
        .expect("source dataset");
    let now = Utc::now();
    let source_row_id = Uuid::new_v4();
    let provenance = SourceProvenance::Imported {
        import_id: Uuid::new_v4(),
        source_path: "benchmark.jsonl".into(),
        source_row_number: 1,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(source_row_id)
    .bind(source_dataset.id)
    .bind(source_row_id.to_string())
    .bind("billing/style=clean")
    .bind("Why was I charged twice?")
    .bind("why was i charged twice?")
    .bind("billing")
    .bind(r#"{"style":"clean"}"#)
    .bind(serde_json::to_string(&provenance).expect("provenance"))
    .bind(now)
    .execute(store.pool())
    .await
    .expect("source row");
    let second_source_row_id = Uuid::new_v4();
    let second_provenance = SourceProvenance::Imported {
        import_id: Uuid::new_v4(),
        source_path: "benchmark.jsonl".into(),
        source_row_number: 2,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(second_source_row_id)
    .bind(source_dataset.id)
    .bind(second_source_row_id.to_string())
    .bind("fraud/style=messy")
    .bind("I do not recognize this transfer.")
    .bind("i do not recognize this transfer.")
    .bind("fraud")
    .bind(r#"{"style":"messy"}"#)
    .bind(serde_json::to_string(&second_provenance).expect("provenance"))
    .bind(now)
    .execute(store.pool())
    .await
    .expect("second source row");
    let snapshot_id = Uuid::new_v4();
    let mut snapshot = DatasetSnapshot {
        id: snapshot_id,
        source_dataset_id: source_dataset.id,
        name: "development snapshot".into(),
        description: None,
        split_configuration: SplitConfiguration::new(
            SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"),
            42,
        ),
        member_count: 2,
        fingerprint: "sha256:development".into(),
        created_at: now,
    };
    let member = SnapshotMember {
        id: Uuid::new_v4(),
        snapshot_id,
        source_row_id,
        split: SnapshotSplit::Test,
        text: "Why was I charged twice?".into(),
        label: "billing".into(),
        dimensions: BTreeMap::from([("style".into(), "clean".into())]),
        fields: BTreeMap::new(),
        source_provenance: provenance,
        source_created_at: now,
    };
    let second_member = SnapshotMember {
        id: Uuid::new_v4(),
        snapshot_id,
        source_row_id: second_source_row_id,
        split: SnapshotSplit::Test,
        text: "I do not recognize this transfer.".into(),
        label: "fraud".into(),
        dimensions: BTreeMap::from([("style".into(), "messy".into())]),
        fields: BTreeMap::new(),
        source_provenance: second_provenance,
        source_created_at: now,
    };
    let members = vec![member, second_member];
    let mut members = members;
    members.sort_by_key(|member| member.source_row_id);
    snapshot.fingerprint = reproduce_snapshot_fingerprint(&snapshot, &members)
        .expect("development snapshot fingerprint");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("snapshot");
    let manifest = PreparationManifest {
        version: 1,
        name: "support encoder".into(),
        project,
        contamination: ContaminationManifest::default(),
        benchmark_qualification: BenchmarkQualificationManifest {
            policy: BenchmarkQualificationPolicy {
                minimum_overall_support: 2,
                minimum_label_support: 1,
                confidence: QualificationConfidence::Eighty,
                maximum_proportion_margin_of_error: 0.49,
                maximum_normalized_duplicate_rate: 0.0,
                minimum_distinct_producers: 1,
                maximum_single_producer_share: 1.0,
                maximum_label_imbalance_ratio: 1.0,
            },
            reviewed_by: "test operator".into(),
            approval_rationale: "fixture satisfies its explicit readiness policy".into(),
        },
        development: SuiteManifest {
            name: "development".into(),
            required_model_formats: Vec::new(),
            cohorts: vec![CohortManifest {
                name: "development test".into(),
                snapshot_id,
                split: SnapshotSplit::Test,
                origin: CohortOrigin::InternalSnapshot,
                role: CohortRole::Development,
                protocol: None,
                disclosure: DisclosureLevel::RowContent,
                adaptation_eligible: true,
            }],
            contract: AcceptanceContract {
                metric_requirements: vec![MetricRequirement {
                    target: MetricTarget::Overall,
                    metric: BenchmarkMetric::MacroF1,
                    minimum: Some(0.01),
                    maximum: None,
                    minimum_support: 1,
                }],
                regression: None,
            },
        },
        sealed: None,
        workflow: WorkflowManifest {
            name: "support workflow".into(),
            total_rows: 100,
            reserved_rows: 20,
            allocation_policy: InitialAllocationPolicy::Balanced,
            allocation_constraints: Vec::new(),
            analysis_protocol: None,
            optimization_protocol: None,
            advisor: None,
            training_iteration_policy: None,
            quality_gate: None,
            governance: IterationGovernance::ReviewEachIteration,
            budget: WorkflowBudget {
                maximum_iterations: 2,
                maximum_initial_rows: 100,
                maximum_cumulative_rows: 120,
                maximum_generation_attempts: 500,
                maximum_generation_requests: 100,
                maximum_advisor_calls: 0,
                maximum_advisor_tokens: None,
                maximum_stage_attempts: 3,
            },
            policy: WorkflowPolicy {
                minimum_improvement: 0.01,
                maximum_tolerated_regression: 0.01,
                stop_on_inconclusive: true,
                stop_on_invalid: true,
                enable_advisor: false,
                require_fresh_development_cohort_after_iterations: None,
            },
        },
    };
    (
        manifest,
        PreparationEvidence {
            cohorts: BTreeMap::from([(
                snapshot_id,
                CohortEvidence {
                    snapshot,
                    source_dataset,
                    members,
                },
            )]),
        },
    )
}

async fn sealed_fixture(store: &SqliteStore) -> (PreparationManifest, PreparationEvidence) {
    let (mut manifest, mut evidence) = fixture(store).await;
    let development = evidence
        .cohorts
        .values()
        .next()
        .expect("development evidence")
        .clone();
    let now = Utc::now();
    let source_row_id = Uuid::new_v4();
    let source_provenance = SourceProvenance::Imported {
        import_id: Uuid::new_v4(),
        source_path: "sealed.jsonl".into(),
        source_row_number: 1,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(source_row_id)
    .bind(development.source_dataset.id)
    .bind(source_row_id.to_string())
    .bind("fraud/style=other")
    .bind("A transfer I do not recognize appeared today.")
    .bind("A transfer I do not recognize appeared today.")
    .bind("fraud")
    .bind(r#"{"style":"other"}"#)
    .bind(serde_json::to_string(&source_provenance).expect("provenance JSON"))
    .bind(now)
    .execute(store.pool())
    .await
    .expect("sealed source row");
    let second_source_row_id = Uuid::new_v4();
    let second_source_provenance = SourceProvenance::Imported {
        import_id: Uuid::new_v4(),
        source_path: "sealed.jsonl".into(),
        source_row_number: 2,
    };
    sqlx::query(
        "INSERT INTO dataset_source_rows \
         (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, label, \
          dimensions_json, provenance_json, created_at) VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(second_source_row_id)
    .bind(development.source_dataset.id)
    .bind(second_source_row_id.to_string())
    .bind("billing/style=other")
    .bind("Please explain the fee on my invoice.")
    .bind("please explain the fee on my invoice.")
    .bind("billing")
    .bind(r#"{"style":"other"}"#)
    .bind(serde_json::to_string(&second_source_provenance).expect("provenance JSON"))
    .bind(now)
    .execute(store.pool())
    .await
    .expect("second sealed source row");

    let snapshot_id = Uuid::new_v4();
    let mut snapshot = DatasetSnapshot {
        id: snapshot_id,
        source_dataset_id: development.source_dataset.id,
        name: "sealed snapshot".into(),
        description: None,
        split_configuration: SplitConfiguration::new(
            SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"),
            43,
        ),
        member_count: 2,
        fingerprint: "sha256:sealed".into(),
        created_at: now,
    };
    let member = SnapshotMember {
        id: Uuid::new_v4(),
        snapshot_id,
        source_row_id,
        split: SnapshotSplit::Test,
        text: "A transfer I do not recognize appeared today.".into(),
        label: "fraud".into(),
        dimensions: BTreeMap::from([("style".into(), "other".into())]),
        fields: BTreeMap::new(),
        source_provenance,
        source_created_at: now,
    };
    let second_member = SnapshotMember {
        id: Uuid::new_v4(),
        snapshot_id,
        source_row_id: second_source_row_id,
        split: SnapshotSplit::Test,
        text: "Please explain the fee on my invoice.".into(),
        label: "billing".into(),
        dimensions: BTreeMap::from([("style".into(), "other".into())]),
        fields: BTreeMap::new(),
        source_provenance: second_source_provenance,
        source_created_at: now,
    };
    let members = vec![member, second_member];
    let mut members = members;
    members.sort_by_key(|member| member.source_row_id);
    snapshot.fingerprint =
        reproduce_snapshot_fingerprint(&snapshot, &members).expect("sealed snapshot fingerprint");
    store
        .create_snapshot(&snapshot, &members)
        .await
        .expect("sealed snapshot");
    evidence.cohorts.insert(
        snapshot_id,
        CohortEvidence {
            snapshot,
            source_dataset: development.source_dataset,
            members,
        },
    );
    manifest.sealed = Some(SuiteManifest {
        name: "sealed".into(),
        required_model_formats: Vec::new(),
        cohorts: vec![CohortManifest {
            name: "sealed test".into(),
            snapshot_id,
            split: SnapshotSplit::Test,
            origin: CohortOrigin::InternalSnapshot,
            role: CohortRole::SealedAcceptance,
            protocol: None,
            disclosure: DisclosureLevel::Aggregate,
            adaptation_eligible: false,
        }],
        contract: manifest.development.contract.clone(),
    });
    (manifest, evidence)
}

async fn table_count(store: &SqliteStore, table: &str) -> i64 {
    let query = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar(&query)
        .fetch_one(store.pool())
        .await
        .expect("count")
}
