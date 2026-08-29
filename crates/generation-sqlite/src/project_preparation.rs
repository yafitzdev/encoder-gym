use std::collections::BTreeMap;

use project_preparation::{
    BoxFuture, PreparationBundle, PreparationStore, PreparationStoreError, PreparedProject,
};
use sqlx::{FromRow, Sqlite, Transaction};
use uuid::Uuid;

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

            insert_dataset(&mut transaction, &bundle).await?;
            insert_project_configuration(&mut transaction, &bundle).await?;
            insert_cohorts(&mut transaction, &bundle).await?;
            insert_contamination_reports(&mut transaction, &bundle).await?;
            insert_suite(&mut transaction, &bundle.development_suite).await?;
            if let Some(suite) = &bundle.sealed_suite {
                insert_suite(&mut transaction, suite).await?;
            }
            insert_workflow_definition(&mut transaction, &bundle).await?;
            insert_preparation(&mut transaction, &bundle.preparation).await?;
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
                 development_suite_id, sealed_suite_id, workflow_definition_id, artifact_json, \
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

async fn insert_dataset(
    transaction: &mut Transaction<'_, Sqlite>,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    let dataset = &bundle.dataset;
    sqlx::query(
        "INSERT INTO dataset_definitions \
         (id, name, task_description, labels_json, dimensions_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(dataset.id)
    .bind(&dataset.name)
    .bind(&dataset.task_description)
    .bind(to_json(&dataset.labels)?)
    .bind(to_json(&dataset.dimensions)?)
    .bind(dataset.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;

    let plan = &bundle.default_generation_plan;
    sqlx::query(
        "INSERT INTO generation_plans (id, dataset_id, cells_json, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(plan.id)
    .bind(plan.dataset_id)
    .bind(to_json(&plan.cells)?)
    .bind(plan.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;

    if let Some(backend) = &bundle.backend_configuration {
        sqlx::query(
            "INSERT INTO backend_configurations \
             (name, base_url, model, parameters_json, updated_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(name) DO UPDATE SET base_url = excluded.base_url, \
             model = excluded.model, parameters_json = excluded.parameters_json, \
             updated_at = excluded.updated_at",
        )
        .bind(&backend.name)
        .bind(&backend.base_url)
        .bind(&backend.model)
        .bind(to_json(&backend.parameters)?)
        .bind(backend.updated_at)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

async fn insert_project_configuration(
    transaction: &mut Transaction<'_, Sqlite>,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    let configuration = &bundle.project_configuration;
    sqlx::query(
        "INSERT INTO project_configurations \
         (id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(configuration.id)
    .bind(&configuration.fingerprint)
    .bind(configuration.dataset_id)
    .bind(configuration.generation_plan_id)
    .bind(to_json(&configuration.resolved)?)
    .bind(configuration.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_cohorts(
    transaction: &mut Transaction<'_, Sqlite>,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    let roles = bundle
        .role_decisions
        .iter()
        .map(|role| (role.cohort_id, role))
        .collect::<BTreeMap<_, _>>();
    for cohort in &bundle.cohorts {
        let persisted_snapshot_fingerprint: Option<String> =
            sqlx::query_scalar("SELECT fingerprint FROM dataset_snapshots WHERE id = ?")
                .bind(cohort.snapshot_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(store_error)?;
        if persisted_snapshot_fingerprint.as_deref() != Some(&cohort.snapshot_fingerprint) {
            return Err(PreparationStoreError(format!(
                "cohort snapshot fingerprint does not match persistence: {}",
                cohort.snapshot_id
            )));
        }
        sqlx::query(
            "INSERT INTO workflow_evaluation_cohorts \
             (id, snapshot_id, split, origin, name, fingerprint, artifact_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(cohort.id)
        .bind(cohort.snapshot_id)
        .bind(enum_string(&cohort.split)?)
        .bind(enum_string(&cohort.origin)?)
        .bind(&cohort.name)
        .bind(&cohort.fingerprint)
        .bind(to_json(cohort)?)
        .bind(cohort.created_at)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
        let role = roles.get(&cohort.id).ok_or_else(|| {
            PreparationStoreError(format!("cohort has no initial role: {}", cohort.id))
        })?;
        sqlx::query(
            "INSERT INTO workflow_cohort_role_decisions \
             (id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
              artifact_json, created_at) VALUES (?, ?, 0, ?, ?, ?, ?, ?, ?)",
        )
        .bind(role.id)
        .bind(role.cohort_id)
        .bind(enum_string(&role.role)?)
        .bind(enum_string(&role.disposition)?)
        .bind(role.predecessor_id)
        .bind(&role.fingerprint)
        .bind(to_json(role)?)
        .bind(role.created_at)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

async fn insert_contamination_reports(
    transaction: &mut Transaction<'_, Sqlite>,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    for report in &bundle.contamination_reports {
        sqlx::query(
            "INSERT INTO workflow_contamination_reports \
             (id, status, cohort_ids_json, artifact_json, fingerprint, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(report.id)
        .bind(enum_string(&report.status)?)
        .bind(to_json(&report.cohort_ids)?)
        .bind(to_json(report)?)
        .bind(&report.fingerprint)
        .bind(report.created_at)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
        for cohort_id in &report.cohort_ids {
            sqlx::query(
                "INSERT INTO workflow_contamination_report_cohorts (report_id, cohort_id) \
                 VALUES (?, ?)",
            )
            .bind(report.id)
            .bind(cohort_id)
            .execute(&mut **transaction)
            .await
            .map_err(store_error)?;
        }
    }
    Ok(())
}

async fn insert_suite(
    transaction: &mut Transaction<'_, Sqlite>,
    suite: &workflow_core::benchmark::BenchmarkSuite,
) -> Result<(), PreparationStoreError> {
    sqlx::query(
        "INSERT INTO workflow_benchmark_suites \
         (id, name, kind, contamination_report_id, contamination_override_fingerprint, \
          artifact_json, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(suite.id)
    .bind(&suite.name)
    .bind(enum_string(&suite.kind)?)
    .bind(suite.contamination_report_id)
    .bind(&suite.contamination_override_fingerprint)
    .bind(to_json(suite)?)
    .bind(&suite.fingerprint)
    .bind(suite.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    for cohort in &suite.cohorts {
        sqlx::query(
            "INSERT INTO workflow_benchmark_suite_cohorts \
             (suite_id, cohort_id, role_decision_id, protocol_fingerprint) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(suite.id)
        .bind(cohort.cohort_id)
        .bind(cohort.role_decision_id)
        .bind(&cohort.protocol_fingerprint)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

async fn insert_workflow_definition(
    transaction: &mut Transaction<'_, Sqlite>,
    bundle: &PreparationBundle,
) -> Result<(), PreparationStoreError> {
    let definition = &bundle.workflow_definition;
    sqlx::query(
        "INSERT INTO workflow_definitions \
         (id, name, dataset_id, project_configuration_id, development_suite_id, \
          sealed_suite_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(definition.id)
    .bind(&definition.name)
    .bind(definition.dataset_id)
    .bind(definition.project_configuration_id)
    .bind(definition.development_suite_id)
    .bind(definition.sealed_suite_id)
    .bind(to_json(definition)?)
    .bind(&definition.fingerprint)
    .bind(definition.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_preparation(
    transaction: &mut Transaction<'_, Sqlite>,
    value: &PreparedProject,
) -> Result<(), PreparationStoreError> {
    sqlx::query(
        "INSERT INTO project_preparations \
         (id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
          development_suite_id, sealed_suite_id, workflow_definition_id, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(value.id)
    .bind(&value.name)
    .bind(&value.manifest_fingerprint)
    .bind(value.dataset_id)
    .bind(value.project_configuration_id)
    .bind(value.development_suite_id)
    .bind(value.sealed_suite_id)
    .bind(value.workflow_definition_id)
    .bind(to_json(value)?)
    .bind(&value.fingerprint)
    .bind(value.created_at)
    .execute(&mut **transaction)
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
    let definition = &bundle.workflow_definition;
    if definition.reproduce_fingerprint().map_err(store_error)? != definition.fingerprint
        || definition.dataset_id != bundle.dataset.id
        || definition.project_configuration_id != configuration.id
        || definition.development_suite_id != bundle.development_suite.id
        || definition.sealed_suite_id != bundle.sealed_suite.as_ref().map(|suite| suite.id)
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
        || preparation.workflow_definition_id != definition.id
    {
        return Err(PreparationStoreError(
            "preparation summary references are inconsistent".into(),
        ));
    }
    Ok(())
}

const PREPARATION_SELECT_BY_ID: &str = "SELECT id, name, manifest_fingerprint, dataset_id, project_configuration_id, \
     development_suite_id, sealed_suite_id, workflow_definition_id, artifact_json, \
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
         development_suite_id, sealed_suite_id, workflow_definition_id, artifact_json, \
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
    workflow_definition_id: Uuid,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl PreparationRow {
    fn into_domain(self) -> Result<PreparedProject, PreparationStoreError> {
        let value: PreparedProject =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if value.id != self.id
            || value.name != self.name
            || value.manifest_fingerprint != self.manifest_fingerprint
            || value.dataset_id != self.dataset_id
            || value.project_configuration_id != self.project_configuration_id
            || value.development_suite_id != self.development_suite_id
            || value.sealed_suite_id != self.sealed_suite_id
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

fn enum_string(value: &impl serde::Serialize) -> Result<String, PreparationStoreError> {
    serde_json::to_value(value)
        .map_err(store_error)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| PreparationStoreError("enum did not serialize as a string".into()))
}

fn to_json(value: &impl serde::Serialize) -> Result<String, PreparationStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> PreparationStoreError {
    PreparationStoreError(error.to_string())
}
