use std::collections::BTreeMap;

use project_preparation::{
    BoxFuture, PreparationBundle, PreparationStore, PreparationStoreError, PreparedProject,
};
use sqlx::{FromRow, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::benchmark_bundle::build_benchmark_bundle;

use crate::SqliteStore;

impl PreparationStore for SqliteStore {
    fn create_preparation(
        &self,
        bundle: &PreparationBundle,
    ) -> BoxFuture<'_, Result<PreparedProject, PreparationStoreError>> {
        let bundle = bundle.clone();
        Box::pin(async move {
            validate_bundle(&bundle)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            if let Some(existing) =
                get_by_manifest(&mut *transaction, &bundle.preparation.manifest_fingerprint).await?
            {
                transaction.rollback().await.map_err(store_error)?;
                return Ok(existing);
            }
            insert_preparation_bundle(&mut transaction, &bundle).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(bundle.preparation)
        })
    }

    fn get_preparation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PreparedProject>, PreparationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, PreparationRow>(PREPARATION_SELECT_BY_ID)
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?
                .map(PreparationRow::into_domain)
                .transpose()
        })
    }

    fn get_preparation_by_manifest(
        &self,
        manifest_fingerprint: &str,
    ) -> BoxFuture<'_, Result<Option<PreparedProject>, PreparationStoreError>> {
        let manifest_fingerprint = manifest_fingerprint.to_owned();
        Box::pin(async move { get_by_manifest(self.pool(), &manifest_fingerprint).await })
    }

    fn list_preparations(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<PreparedProject>, PreparationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, PreparationRow>(
                "SELECT id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
                 development_suite_id, sealed_suite_id, benchmark_bundle_id, workflow_definition_id, artifact_json, \
                 fingerprint, created_at FROM project_preparations \
                 ORDER BY created_at DESC, id ASC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(PreparationRow::into_domain)
            .collect()
        })
    }
}

pub(crate) async fn insert_preparation_bundle(
    connection: &mut SqliteConnection,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    validate_bundle(bundle)?;
    crate::insert_dataset(connection, &bundle.dataset)
        .await
        .map_err(store_error)?;
    crate::insert_plan(connection, &bundle.default_generation_plan)
        .await
        .map_err(store_error)?;
    if let Some(backend) = &bundle.backend_configuration {
        crate::upsert_backend(connection, backend)
            .await
            .map_err(store_error)?;
    }
    crate::project::insert_project_configuration(connection, &bundle.project_configuration)
        .await
        .map_err(store_error)?;
    let roles = bundle
        .role_decisions
        .iter()
        .map(|role| (role.cohort_id, role))
        .collect::<BTreeMap<_, _>>();
    for cohort in &bundle.cohorts {
        let role = roles.get(&cohort.id).ok_or_else(|| {
            PreparationStoreError(format!("cohort has no initial role: {}", cohort.id))
        })?;
        crate::governance::insert_cohort_with_initial_role(connection, cohort, role)
            .await
            .map_err(store_error)?;
    }
    for report in &bundle.contamination_reports {
        crate::contamination::insert_contamination_report(connection, report)
            .await
            .map_err(store_error)?;
    }
    crate::benchmark::insert_benchmark_suite(connection, &bundle.development_suite)
        .await
        .map_err(store_error)?;
    if let Some(suite) = &bundle.sealed_suite {
        crate::benchmark::insert_benchmark_suite(connection, suite)
            .await
            .map_err(store_error)?;
    }
    crate::benchmark_bundle::insert_benchmark_bundle(connection, &bundle.benchmark_bundle)
        .await
        .map_err(store_error)?;
    crate::workflow_run::insert_workflow_definition(connection, &bundle.workflow_definition)
        .await
        .map_err(store_error)?;
    insert_preparation(connection, &bundle.preparation).await
}

async fn insert_preparation(
    connection: &mut SqliteConnection,
    value: &PreparedProject,
) -> Result<(), PreparationStoreError> {
    sqlx::query(
        "INSERT INTO project_preparations \
         (id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
          development_suite_id, sealed_suite_id, benchmark_bundle_id, workflow_definition_id, \
          artifact_json, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(value.id)
    .bind(&value.name)
    .bind(&value.manifest_fingerprint)
    .bind(value.dataset_id)
    .bind(value.project_configuration_id)
    .bind(value.development_suite_id)
    .bind(value.sealed_suite_id)
    .bind(value.benchmark_bundle_id)
    .bind(value.workflow_definition_id)
    .bind(to_json(value)?)
    .bind(&value.fingerprint)
    .bind(value.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn validate_bundle(bundle: &PreparationBundle) -> Result<(), PreparationStoreError> {
    let configuration = &bundle.project_configuration;
    if bundle.default_generation_plan.dataset_id != bundle.dataset.id
        || configuration.dataset_id != bundle.dataset.id
        || configuration.generation_plan_id != bundle.default_generation_plan.id
        || configuration.resolved.fingerprint().map_err(store_error)? != configuration.fingerprint
    {
        return Err(PreparationStoreError(
            "dataset, plan, and project configuration references are inconsistent".into(),
        ));
    }
    if bundle.cohorts.len() != bundle.role_decisions.len() {
        return Err(PreparationStoreError(
            "every cohort must have exactly one initial role".into(),
        ));
    }
    for cohort in &bundle.cohorts {
        if cohort.reproduce_fingerprint().map_err(store_error)? != cohort.fingerprint {
            return Err(PreparationStoreError(
                "cohort fingerprint does not reproduce".into(),
            ));
        }
        let roles = bundle
            .role_decisions
            .iter()
            .filter(|role| role.cohort_id == cohort.id)
            .collect::<Vec<_>>();
        if roles.len() != 1
            || roles[0].predecessor_id.is_some()
            || roles[0].predecessor_fingerprint.is_some()
            || roles[0].reproduce_fingerprint().map_err(store_error)? != roles[0].fingerprint
        {
            return Err(PreparationStoreError("invalid initial cohort role".into()));
        }
    }
    for report in &bundle.contamination_reports {
        if report.reproduce_fingerprint().map_err(store_error)? != report.fingerprint {
            return Err(PreparationStoreError(
                "contamination report fingerprint does not reproduce".into(),
            ));
        }
    }
    let reports = bundle
        .contamination_reports
        .iter()
        .map(|report| (report.id, report.fingerprint.as_str()))
        .collect::<BTreeMap<_, _>>();
    for suite in std::iter::once(&bundle.development_suite).chain(bundle.sealed_suite.iter()) {
        if suite.reproduce_fingerprint().map_err(store_error)? != suite.fingerprint
            || reports.get(&suite.contamination_report_id).copied()
                != Some(suite.contamination_report_fingerprint.as_str())
        {
            return Err(PreparationStoreError(
                "benchmark suite does not match its contamination report".into(),
            ));
        }
    }
    let global_report = bundle
        .contamination_reports
        .iter()
        .find(|report| report.id == bundle.benchmark_bundle.contamination_report_id)
        .ok_or_else(|| {
            PreparationStoreError("benchmark bundle contamination report is absent".into())
        })?;
    let expected_bundle = build_benchmark_bundle(
        &bundle.development_suite,
        bundle.sealed_suite.as_ref(),
        global_report,
    )
    .map_err(store_error)?;
    if bundle.benchmark_bundle.validate_fingerprint().is_err()
        || bundle.benchmark_bundle.development_suite_id != expected_bundle.development_suite_id
        || bundle.benchmark_bundle.development_suite_fingerprint
            != expected_bundle.development_suite_fingerprint
        || bundle.benchmark_bundle.sealed_suite_id != expected_bundle.sealed_suite_id
        || bundle.benchmark_bundle.sealed_suite_fingerprint
            != expected_bundle.sealed_suite_fingerprint
        || bundle.benchmark_bundle.contamination_report_id
            != expected_bundle.contamination_report_id
        || bundle.benchmark_bundle.contamination_report_fingerprint
            != expected_bundle.contamination_report_fingerprint
        || bundle.benchmark_bundle.fingerprint != expected_bundle.fingerprint
    {
        return Err(PreparationStoreError(
            "benchmark bundle does not match the prepared suites and global report".into(),
        ));
    }
    let definition = &bundle.workflow_definition;
    if definition.reproduce_fingerprint().map_err(store_error)? != definition.fingerprint
        || definition.dataset_id != bundle.dataset.id
        || definition.project_configuration_id != configuration.id
        || definition.development_suite_id != bundle.development_suite.id
        || definition.sealed_suite_id != bundle.sealed_suite.as_ref().map(|suite| suite.id)
        || definition
            .benchmark_bundle
            .as_ref()
            .is_none_or(|binding| binding.validate_bundle(&bundle.benchmark_bundle).is_err())
    {
        return Err(PreparationStoreError(
            "workflow definition references are inconsistent".into(),
        ));
    }
    let preparation = &bundle.preparation;
    if preparation.reproduce_fingerprint().map_err(store_error)? != preparation.fingerprint
        || preparation.dataset_id != bundle.dataset.id
        || preparation.project_configuration_id != configuration.id
        || preparation.development_suite_id != bundle.development_suite.id
        || preparation.sealed_suite_id != bundle.sealed_suite.as_ref().map(|suite| suite.id)
        || preparation.benchmark_bundle_id != Some(bundle.benchmark_bundle.id)
        || preparation.benchmark_bundle_fingerprint.as_deref()
            != Some(bundle.benchmark_bundle.fingerprint.as_str())
        || preparation.workflow_definition_id != definition.id
    {
        return Err(PreparationStoreError(
            "preparation summary references are inconsistent".into(),
        ));
    }
    Ok(())
}

const PREPARATION_SELECT_BY_ID: &str = "SELECT id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
     development_suite_id, sealed_suite_id, benchmark_bundle_id, workflow_definition_id, artifact_json, \
     fingerprint, created_at FROM project_preparations WHERE id = ?";

async fn get_by_manifest<'e, E>(
    executor: E,
    manifest_fingerprint: &str,
) -> Result<Option<PreparedProject>, PreparationStoreError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, PreparationRow>(
        "SELECT id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
         development_suite_id, sealed_suite_id, benchmark_bundle_id, workflow_definition_id, artifact_json, \
         fingerprint, created_at FROM project_preparations WHERE manifest_fingerprint = ?",
    )
    .bind(manifest_fingerprint)
    .fetch_optional(executor)
    .await
    .map_err(store_error)?
    .map(PreparationRow::into_domain)
    .transpose()
}

#[derive(Debug, FromRow)]
struct PreparationRow {
    id: Uuid,
    name: String,
    manifest_fingerprint: String,
    dataset_id: Uuid,
    project_configuration_id: Uuid,
    development_suite_id: Uuid,
    sealed_suite_id: Option<Uuid>,
    benchmark_bundle_id: Option<Uuid>,
    workflow_definition_id: Uuid,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl PreparationRow {
    fn into_domain(self) -> Result<PreparedProject, PreparationStoreError> {
        let value: PreparedProject =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if value.benchmark_bundle_id.is_some() != value.benchmark_bundle_fingerprint.is_some()
            || value.id != self.id
            || value.name != self.name
            || value.manifest_fingerprint != self.manifest_fingerprint
            || value.dataset_id != self.dataset_id
            || value.project_configuration_id != self.project_configuration_id
            || value.development_suite_id != self.development_suite_id
            || value.sealed_suite_id != self.sealed_suite_id
            || value.benchmark_bundle_id != self.benchmark_bundle_id
            || value.workflow_definition_id != self.workflow_definition_id
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
            || value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint
        {
            return Err(PreparationStoreError(
                "prepared project normalized columns differ from immutable artifact".into(),
            ));
        }
        Ok(value)
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, PreparationStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> PreparationStoreError {
    PreparationStoreError(error.to_string())
}
