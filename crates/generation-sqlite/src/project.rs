use generation_core::domain::{BackendConfiguration, DatasetDefinition, GenerationPlan};
use project_config::{
    BoxFuture, PersistedProjectConfiguration, ProjectConfigurationStore, ProjectInitializer,
    ProjectStoreError, ResolvedProjectConfig,
};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;

use super::SqliteStore;

impl ProjectInitializer for SqliteStore {
    fn initialize_project(
        &self,
        dataset: &DatasetDefinition,
        plan: &GenerationPlan,
        backend: Option<&BackendConfiguration>,
        configuration: &PersistedProjectConfiguration,
    ) -> BoxFuture<'_, Result<(), ProjectStoreError>> {
        let dataset = dataset.clone();
        let plan = plan.clone();
        let backend = backend.cloned();
        let configuration = configuration.clone();
        Box::pin(async move {
            if plan.dataset_id != dataset.id {
                return Err(ProjectStoreError(
                    "generation plan does not belong to the project dataset".into(),
                ));
            }
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            crate::insert_dataset(&mut transaction, &dataset)
                .await
                .map_err(store_error)?;
            crate::insert_plan(&mut transaction, &plan)
                .await
                .map_err(store_error)?;
            if let Some(backend) = &backend {
                crate::upsert_backend(&mut transaction, backend)
                    .await
                    .map_err(store_error)?;
            }
            if configuration.dataset_id != dataset.id || configuration.generation_plan_id != plan.id
            {
                return Err(ProjectStoreError(
                    "persisted configuration references a different initialized project".into(),
                ));
            }
            insert_project_configuration(&mut transaction, &configuration).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }
}

pub(crate) async fn insert_project_configuration(
    connection: &mut SqliteConnection,
    configuration: &PersistedProjectConfiguration,
) -> Result<(), ProjectStoreError> {
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
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

impl ProjectConfigurationStore for SqliteStore {
    fn get_project_configuration(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PersistedProjectConfiguration>, ProjectStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ProjectConfigurationRow>(
                "SELECT id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, \
                 created_at FROM project_configurations WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(ProjectConfigurationRow::into_domain)
            .transpose()
        })
    }

    fn list_project_configurations(
        &self,
        dataset_id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<PersistedProjectConfiguration>, ProjectStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, \
                 created_at FROM project_configurations",
            );
            if let Some(dataset_id) = dataset_id {
                builder.push(" WHERE dataset_id = ").push_bind(dataset_id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(limit)
                .push(" OFFSET ")
                .push_bind(offset);
            builder
                .build_query_as::<ProjectConfigurationRow>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ProjectConfigurationRow::into_domain)
                .collect()
        })
    }
}

#[derive(Debug, FromRow)]
struct ProjectConfigurationRow {
    id: Uuid,
    fingerprint: String,
    dataset_id: Uuid,
    generation_plan_id: Uuid,
    resolved_toml_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ProjectConfigurationRow {
    fn into_domain(self) -> Result<PersistedProjectConfiguration, ProjectStoreError> {
        let resolved: ResolvedProjectConfig =
            serde_json::from_str(&self.resolved_toml_json).map_err(store_error)?;
        if resolved.fingerprint().map_err(store_error)? != self.fingerprint {
            return Err(ProjectStoreError(
                "persisted project configuration fingerprint mismatch".into(),
            ));
        }
        Ok(PersistedProjectConfiguration {
            id: self.id,
            fingerprint: self.fingerprint,
            dataset_id: self.dataset_id,
            generation_plan_id: self.generation_plan_id,
            resolved,
            created_at: self.created_at,
        })
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, ProjectStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> ProjectStoreError {
    ProjectStoreError(error.to_string())
}
