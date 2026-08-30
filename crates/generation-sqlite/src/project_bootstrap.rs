use project_preparation::{
    BootstrapBundle, BootstrapStore, BoxFuture, PreparationStoreError, ProjectBootstrap,
};
use sqlx::{FromRow, Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::SqliteStore;

impl BootstrapStore for SqliteStore {
    fn create_bootstrap(
        &self,
        bundle: &BootstrapBundle,
    ) -> BoxFuture<'_, Result<ProjectBootstrap, PreparationStoreError>> {
        let bundle = bundle.clone();
        Box::pin(async move {
            validate_bundle(&bundle)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            if let Some(existing) =
                get_by_fingerprint(&mut *transaction, &bundle.bootstrap.bootstrap_fingerprint)
                    .await?
            {
                transaction.rollback().await.map_err(store_error)?;
                return Ok(existing);
            }
            for source in &bundle.sources {
                crate::insert_dataset(&mut transaction, &source.dataset)
                    .await
                    .map_err(store_error)?;
                crate::imports::insert_import_bundle(
                    &mut transaction,
                    &source.dataset_import,
                    &source.imported_rows,
                )
                .await
                .map_err(store_error)?;
                crate::insert_snapshot(&mut transaction, &source.snapshot, &source.members)
                    .await
                    .map_err(store_error)?;
            }
            crate::project_preparation::insert_preparation_bundle(
                &mut transaction,
                &bundle.preparation,
            )
            .await?;
            insert_bootstrap(&mut transaction, &bundle.bootstrap).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(bundle.bootstrap)
        })
    }

    fn get_bootstrap(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProjectBootstrap>, PreparationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, BootstrapRow>(BOOTSTRAP_SELECT_BY_ID)
                .bind(id)
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?
                .map(BootstrapRow::into_domain)
                .transpose()
        })
    }

    fn get_bootstrap_by_fingerprint(
        &self,
        bootstrap_fingerprint: &str,
    ) -> BoxFuture<'_, Result<Option<ProjectBootstrap>, PreparationStoreError>> {
        let bootstrap_fingerprint = bootstrap_fingerprint.to_owned();
        Box::pin(async move { get_by_fingerprint(self.pool(), &bootstrap_fingerprint).await })
    }

    fn list_bootstraps(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<ProjectBootstrap>, PreparationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, BootstrapRow>(
                "SELECT id, name, bootstrap_fingerprint, preparation_id, artifact_json, \
                 fingerprint, created_at FROM project_bootstraps \
                 ORDER BY created_at DESC, id ASC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(BootstrapRow::into_domain)
            .collect()
        })
    }
}

fn validate_bundle(bundle: &BootstrapBundle) -> Result<(), PreparationStoreError> {
    let bootstrap = &bundle.bootstrap;
    if bootstrap.reproduce_fingerprint().map_err(store_error)? != bootstrap.fingerprint
        || bootstrap.preparation_id != bundle.preparation.preparation.id
        || bootstrap.sources.len() != bundle.sources.len()
    {
        return Err(PreparationStoreError(
            "bootstrap summary and preparation references are inconsistent".into(),
        ));
    }
    for (summary, source) in bootstrap.sources.iter().zip(&bundle.sources) {
        if summary.key != source.key
            || summary.content_fingerprint != source.content_fingerprint
            || summary.dataset_id != source.dataset.id
            || summary.import_id != source.dataset_import.id
            || summary.snapshot_id != source.snapshot.id
            || summary.accepted_rows != source.dataset_import.accepted_rows
        {
            return Err(PreparationStoreError(format!(
                "bootstrap source summary differs from artifact bundle: {}",
                source.key
            )));
        }
    }
    Ok(())
}

async fn insert_bootstrap(
    connection: &mut SqliteConnection,
    value: &ProjectBootstrap,
) -> Result<(), PreparationStoreError> {
    sqlx::query(
        "INSERT INTO project_bootstraps \
         (id, name, bootstrap_fingerprint, preparation_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(value.id)
    .bind(&value.name)
    .bind(&value.bootstrap_fingerprint)
    .bind(value.preparation_id)
    .bind(to_json(value)?)
    .bind(&value.fingerprint)
    .bind(value.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

const BOOTSTRAP_SELECT_BY_ID: &str = "SELECT id, name, bootstrap_fingerprint, preparation_id, artifact_json, fingerprint, \
     created_at FROM project_bootstraps WHERE id = ?";

async fn get_by_fingerprint<'e, E>(
    executor: E,
    bootstrap_fingerprint: &str,
) -> Result<Option<ProjectBootstrap>, PreparationStoreError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, BootstrapRow>(
        "SELECT id, name, bootstrap_fingerprint, preparation_id, artifact_json, fingerprint, \
         created_at FROM project_bootstraps WHERE bootstrap_fingerprint = ?",
    )
    .bind(bootstrap_fingerprint)
    .fetch_optional(executor)
    .await
    .map_err(store_error)?
    .map(BootstrapRow::into_domain)
    .transpose()
}

#[derive(Debug, FromRow)]
struct BootstrapRow {
    id: Uuid,
    name: String,
    bootstrap_fingerprint: String,
    preparation_id: Uuid,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl BootstrapRow {
    fn into_domain(self) -> Result<ProjectBootstrap, PreparationStoreError> {
        let value: ProjectBootstrap =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if value.id != self.id
            || value.name != self.name
            || value.bootstrap_fingerprint != self.bootstrap_fingerprint
            || value.preparation_id != self.preparation_id
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
            || value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint
        {
            return Err(PreparationStoreError(
                "project bootstrap columns differ from immutable artifact".into(),
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
