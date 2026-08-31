use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    benchmark::BenchmarkSuite,
    benchmark_bundle::{BenchmarkBundle, build_benchmark_bundle},
    contamination::ContaminationReport,
    ports::{BenchmarkBundleQuery, BenchmarkBundleStore, BoxFuture, WorkflowStoreError},
};

use crate::SqliteStore;

impl BenchmarkBundleStore for SqliteStore {
    fn create_benchmark_bundle(
        &self,
        bundle: &BenchmarkBundle,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let bundle = bundle.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_benchmark_bundle(&mut transaction, &bundle).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_benchmark_bundle(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkBundle>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_benchmark_bundle(&mut connection, id).await
        })
    }

    fn query_benchmark_bundles(
        &self,
        query: BenchmarkBundleQuery,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkBundle>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT id FROM workflow_benchmark_bundles");
            let mut has_filter = false;
            if let Some(id) = query.development_suite_id {
                builder.push(" WHERE development_suite_id = ").push_bind(id);
                has_filter = true;
            }
            if let Some(id) = query.sealed_suite_id {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("sealed_suite_id = ")
                    .push_bind(id);
                has_filter = true;
            }
            if let Some(id) = query.contamination_report_id {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("contamination_report_id = ")
                    .push_bind(id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            let ids = builder
                .build_query_scalar::<Uuid>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?;
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            let mut bundles = Vec::with_capacity(ids.len());
            for id in ids {
                let bundle = load_benchmark_bundle(&mut connection, id)
                    .await?
                    .ok_or_else(|| {
                        WorkflowStoreError(format!(
                            "benchmark bundle disappeared while listing: {id}"
                        ))
                    })?;
                bundles.push(bundle);
            }
            Ok(bundles)
        })
    }
}

pub(crate) async fn insert_benchmark_bundle(
    connection: &mut SqliteConnection,
    bundle: &BenchmarkBundle,
) -> Result<(), WorkflowStoreError> {
    validate_bundle_identity(bundle)?;
    let development = load_suite(connection, bundle.development_suite_id, true).await?;
    let sealed = match bundle.sealed_suite_id {
        Some(id) => Some(load_suite(connection, id, true).await?),
        None => None,
    };
    let report = load_report(connection, bundle.contamination_report_id).await?;
    validate_bundle_authority(bundle, &development, sealed.as_ref(), &report)?;

    sqlx::query(
        "INSERT INTO workflow_benchmark_bundles \
         (id, development_suite_id, sealed_suite_id, contamination_report_id, artifact_json, \
          fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(bundle.id)
    .bind(bundle.development_suite_id)
    .bind(bundle.sealed_suite_id)
    .bind(bundle.contamination_report_id)
    .bind(to_json(bundle)?)
    .bind(&bundle.fingerprint)
    .bind(bundle.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

pub(crate) async fn load_benchmark_bundle(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<BenchmarkBundle>, WorkflowStoreError> {
    load_benchmark_bundle_with_role_mode(connection, id, false).await
}

pub(crate) async fn load_executable_benchmark_bundle(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<BenchmarkBundle>, WorkflowStoreError> {
    load_benchmark_bundle_with_role_mode(connection, id, true).await
}

async fn load_benchmark_bundle_with_role_mode(
    connection: &mut SqliteConnection,
    id: Uuid,
    require_current_roles: bool,
) -> Result<Option<BenchmarkBundle>, WorkflowStoreError> {
    let Some(row) = sqlx::query_as::<_, BundleRow>(
        "SELECT id, development_suite_id, sealed_suite_id, contamination_report_id, \
         artifact_json, fingerprint, created_at FROM workflow_benchmark_bundles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let bundle = row.into_domain()?;
    let development = load_suite(
        connection,
        bundle.development_suite_id,
        require_current_roles,
    )
    .await?;
    let sealed = match bundle.sealed_suite_id {
        Some(id) => Some(load_suite(connection, id, require_current_roles).await?),
        None => None,
    };
    let report = load_report(connection, bundle.contamination_report_id).await?;
    validate_bundle_authority(&bundle, &development, sealed.as_ref(), &report)?;
    Ok(Some(bundle))
}

fn validate_bundle_identity(bundle: &BenchmarkBundle) -> Result<(), WorkflowStoreError> {
    if bundle.id.is_nil() {
        return Err(WorkflowStoreError(
            "benchmark bundle identity must not be nil".into(),
        ));
    }
    bundle.validate_fingerprint().map_err(store_error)
}

fn validate_bundle_authority(
    bundle: &BenchmarkBundle,
    development: &BenchmarkSuite,
    sealed: Option<&BenchmarkSuite>,
    report: &ContaminationReport,
) -> Result<(), WorkflowStoreError> {
    validate_bundle_identity(bundle)?;
    let expected = build_benchmark_bundle(development, sealed, report).map_err(store_error)?;
    if bundle.development_suite_id != expected.development_suite_id
        || bundle.development_suite_fingerprint != expected.development_suite_fingerprint
        || bundle.sealed_suite_id != expected.sealed_suite_id
        || bundle.sealed_suite_fingerprint != expected.sealed_suite_fingerprint
        || bundle.contamination_report_id != expected.contamination_report_id
        || bundle.contamination_report_fingerprint != expected.contamination_report_fingerprint
        || bundle.fingerprint != expected.fingerprint
    {
        return Err(WorkflowStoreError(
            "benchmark bundle does not match persisted suite and contamination evidence".into(),
        ));
    }
    Ok(())
}

async fn load_suite(
    connection: &mut SqliteConnection,
    id: Uuid,
    require_current_roles: bool,
) -> Result<BenchmarkSuite, WorkflowStoreError> {
    crate::benchmark::load_benchmark_suite(connection, id, require_current_roles)
        .await?
        .ok_or_else(|| WorkflowStoreError(format!("benchmark suite not found: {id}")))
}

async fn load_report(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<ContaminationReport, WorkflowStoreError> {
    crate::contamination::load_contamination_report(connection, id)
        .await?
        .ok_or_else(|| WorkflowStoreError(format!("contamination report not found: {id}")))
}

#[derive(Debug, FromRow)]
struct BundleRow {
    id: Uuid,
    development_suite_id: Uuid,
    sealed_suite_id: Option<Uuid>,
    contamination_report_id: Uuid,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl BundleRow {
    fn into_domain(self) -> Result<BenchmarkBundle, WorkflowStoreError> {
        let value: BenchmarkBundle = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.development_suite_id != self.development_suite_id
            || value.sealed_suite_id != self.sealed_suite_id
            || value.contamination_report_id != self.contamination_report_id
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "benchmark bundle normalized fields differ from immutable artifact".into(),
            ));
        }
        validate_bundle_identity(&value)?;
        Ok(value)
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, WorkflowStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, WorkflowStoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
