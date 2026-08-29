use generation_core::domain::{BackendConfiguration, DatasetDefinition, GenerationPlan};
use project_config::{
    BoxFuture, PersistedProjectConfiguration, ProjectInitializer, ProjectStoreError,
};

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
            sqlx::query(
                "INSERT INTO dataset_definitions \
                 (id, name, task_description, labels_json, dimensions_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(dataset.id)
            .bind(dataset.name)
            .bind(dataset.task_description)
            .bind(to_json(&dataset.labels)?)
            .bind(to_json(&dataset.dimensions)?)
            .bind(dataset.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO generation_plans (id, dataset_id, cells_json, created_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(plan.id)
            .bind(plan.dataset_id)
            .bind(to_json(&plan.cells)?)
            .bind(plan.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if let Some(backend) = backend {
                sqlx::query(
                    "INSERT INTO backend_configurations \
                     (name, base_url, model, parameters_json, updated_at) VALUES (?, ?, ?, ?, ?) \
                     ON CONFLICT(name) DO UPDATE SET base_url = excluded.base_url, \
                     model = excluded.model, parameters_json = excluded.parameters_json, \
                     updated_at = excluded.updated_at",
                )
                .bind(backend.name)
                .bind(backend.base_url)
                .bind(backend.model)
                .bind(to_json(&backend.parameters)?)
                .bind(backend.updated_at)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            if configuration.dataset_id != dataset.id || configuration.generation_plan_id != plan.id
            {
                return Err(ProjectStoreError(
                    "persisted configuration references a different initialized project".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO project_configurations \
                 (id, fingerprint, dataset_id, generation_plan_id, resolved_toml_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(configuration.id)
            .bind(configuration.fingerprint)
            .bind(configuration.dataset_id)
            .bind(configuration.generation_plan_id)
            .bind(to_json(&configuration.resolved)?)
            .bind(configuration.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, ProjectStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> ProjectStoreError {
    ProjectStoreError(error.to_string())
}
