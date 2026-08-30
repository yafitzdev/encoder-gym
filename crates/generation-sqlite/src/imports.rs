use chrono::{DateTime, Utc};
use dataset_core::{
    domain::{
        DatasetImport, ImportFieldMapping, ImportFormat, ImportRowStatus, ImportState, ImportedRow,
        SourceProvenance,
    },
    ports::{BoxFuture, DatasetStoreError, ImportStore},
};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;

use super::SqliteStore;

impl ImportStore for SqliteStore {
    fn create_import(
        &self,
        dataset_import: &DatasetImport,
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>> {
        let dataset_import = dataset_import.clone();
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(store_error)?;
            insert_import(&mut connection, &dataset_import).await?;
            Ok(())
        })
    }

    fn save_import(
        &self,
        dataset_import: &DatasetImport,
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>> {
        let dataset_import = dataset_import.clone();
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE dataset_imports SET state = ?, processed_rows = ?, accepted_rows = ?, \
                 rejected_rows = ?, error_message = ?, updated_at = ? WHERE id = ?",
            )
            .bind(import_state_text(dataset_import.state))
            .bind(to_i64(dataset_import.processed_rows)?)
            .bind(to_i64(dataset_import.accepted_rows)?)
            .bind(to_i64(dataset_import.rejected_rows)?)
            .bind(dataset_import.error_message)
            .bind(dataset_import.updated_at)
            .bind(dataset_import.id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(DatasetStoreError(format!(
                    "dataset import not found: {}",
                    dataset_import.id
                )));
            }
            Ok(())
        })
    }

    fn get_import(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetImport>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ImportRecord>(
                "SELECT id, dataset_id, source_path, source_format, mapping_json, state, \
                 processed_rows, accepted_rows, rejected_rows, error_message, created_at, updated_at \
                 FROM dataset_imports WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(ImportRecord::into_domain)
            .transpose()
        })
    }

    fn list_imports(
        &self,
        dataset_id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<DatasetImport>, DatasetStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, dataset_id, source_path, source_format, mapping_json, state, \
                 processed_rows, accepted_rows, rejected_rows, error_message, created_at, updated_at \
                 FROM dataset_imports WHERE 1 = 1",
            );
            if let Some(dataset_id) = dataset_id {
                builder.push(" AND dataset_id = ").push_bind(dataset_id);
            }
            builder
                .push(" ORDER BY created_at, id LIMIT ")
                .push_bind(i64::from(bounded_limit(limit)))
                .push(" OFFSET ")
                .push_bind(i64::from(offset));
            builder
                .build_query_as::<ImportRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ImportRecord::into_domain)
                .collect()
        })
    }

    fn insert_imported_rows(
        &self,
        dataset_import: &DatasetImport,
        rows: &[ImportedRow],
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>> {
        let dataset_import = dataset_import.clone();
        let rows = rows.to_vec();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            insert_import_rows(&mut transaction, &dataset_import, &rows).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_imported_rows(
        &self,
        import_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<ImportedRow>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ImportedRowRecord>(
                "SELECT id, import_id, dataset_id, source_row_number, cell_key, text, \
                 normalized_text, label, dimensions_json, validation_status, \
                 validation_errors_json, created_at FROM imported_rows WHERE import_id = ? \
                 ORDER BY source_row_number LIMIT ? OFFSET ?",
            )
            .bind(import_id)
            .bind(i64::from(bounded_limit(limit)))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(ImportedRowRecord::into_domain)
            .collect()
        })
    }

    fn accepted_normalized_source_texts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<String>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT normalized_text FROM dataset_source_rows WHERE dataset_id = ?",
            )
            .bind(dataset_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)
        })
    }
}

pub(crate) async fn insert_import_bundle(
    connection: &mut SqliteConnection,
    dataset_import: &DatasetImport,
    rows: &[ImportedRow],
) -> Result<(), DatasetStoreError> {
    insert_import(connection, dataset_import).await?;
    insert_import_rows(connection, dataset_import, rows).await
}

async fn insert_import(
    connection: &mut SqliteConnection,
    dataset_import: &DatasetImport,
) -> Result<(), DatasetStoreError> {
    sqlx::query(
        "INSERT INTO dataset_imports \
         (id, dataset_id, source_path, source_format, mapping_json, state, \
          processed_rows, accepted_rows, rejected_rows, error_message, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(dataset_import.id)
    .bind(dataset_import.dataset_id)
    .bind(&dataset_import.source_path)
    .bind(dataset_import.format.as_str())
    .bind(to_json(&dataset_import.mapping)?)
    .bind(import_state_text(dataset_import.state))
    .bind(to_i64(dataset_import.processed_rows)?)
    .bind(to_i64(dataset_import.accepted_rows)?)
    .bind(to_i64(dataset_import.rejected_rows)?)
    .bind(&dataset_import.error_message)
    .bind(dataset_import.created_at)
    .bind(dataset_import.updated_at)
    .execute(&mut *connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_import_rows(
    connection: &mut SqliteConnection,
    dataset_import: &DatasetImport,
    rows: &[ImportedRow],
) -> Result<(), DatasetStoreError> {
    for row in rows {
        if row.import_id != dataset_import.id || row.dataset_id != dataset_import.dataset_id {
            return Err(DatasetStoreError(
                "imported row does not belong to the supplied import".into(),
            ));
        }
        let inserted = sqlx::query(
            "INSERT INTO imported_rows \
             (id, import_id, dataset_id, source_row_number, cell_key, text, normalized_text, \
              label, dimensions_json, validation_status, validation_errors_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(import_id, source_row_number) DO NOTHING",
        )
        .bind(row.id)
        .bind(row.import_id)
        .bind(row.dataset_id)
        .bind(to_i64(row.source_row_number)?)
        .bind(&row.cell_key)
        .bind(&row.text)
        .bind(&row.normalized_text)
        .bind(&row.label)
        .bind(to_json(&row.dimensions)?)
        .bind(row_status_text(row.status))
        .bind(to_json(&row.issues)?)
        .bind(row.created_at)
        .execute(&mut *connection)
        .await
        .map_err(store_error)?;
        if inserted.rows_affected() == 1 && row.status == ImportRowStatus::Accepted {
            let provenance = SourceProvenance::Imported {
                import_id: dataset_import.id,
                source_path: dataset_import.source_path.clone(),
                source_row_number: row.source_row_number,
            };
            sqlx::query(
                "INSERT INTO dataset_source_rows \
                 (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, \
                  label, dimensions_json, provenance_json, created_at) \
                 VALUES (?, ?, 'imported', ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id)
            .bind(row.dataset_id)
            .bind(row.id)
            .bind(row.cell_key.as_deref().ok_or_else(|| {
                DatasetStoreError("accepted imported row has no valid cell key".into())
            })?)
            .bind(&row.text)
            .bind(&row.normalized_text)
            .bind(&row.label)
            .bind(to_json(&row.dimensions)?)
            .bind(to_json(&provenance)?)
            .bind(row.created_at)
            .execute(&mut *connection)
            .await
            .map_err(store_error)?;
        }
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct ImportRecord {
    id: Uuid,
    dataset_id: Uuid,
    source_path: String,
    source_format: String,
    mapping_json: String,
    state: String,
    processed_rows: i64,
    accepted_rows: i64,
    rejected_rows: i64,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ImportRecord {
    fn into_domain(self) -> Result<DatasetImport, DatasetStoreError> {
        Ok(DatasetImport {
            id: self.id,
            dataset_id: self.dataset_id,
            source_path: self.source_path,
            format: parse_format(&self.source_format)?,
            mapping: from_json::<ImportFieldMapping>(&self.mapping_json)?,
            state: parse_import_state(&self.state)?,
            processed_rows: to_u64(self.processed_rows)?,
            accepted_rows: to_u64(self.accepted_rows)?,
            rejected_rows: to_u64(self.rejected_rows)?,
            error_message: self.error_message,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct ImportedRowRecord {
    id: Uuid,
    import_id: Uuid,
    dataset_id: Uuid,
    source_row_number: i64,
    cell_key: Option<String>,
    text: String,
    normalized_text: String,
    label: String,
    dimensions_json: String,
    validation_status: String,
    validation_errors_json: String,
    created_at: DateTime<Utc>,
}

impl ImportedRowRecord {
    fn into_domain(self) -> Result<ImportedRow, DatasetStoreError> {
        Ok(ImportedRow {
            id: self.id,
            import_id: self.import_id,
            dataset_id: self.dataset_id,
            source_row_number: to_u64(self.source_row_number)?,
            text: self.text,
            normalized_text: self.normalized_text,
            label: self.label,
            dimensions: from_json(&self.dimensions_json)?,
            cell_key: self.cell_key,
            status: parse_row_status(&self.validation_status)?,
            issues: from_json(&self.validation_errors_json)?,
            created_at: self.created_at,
        })
    }
}

fn bounded_limit(limit: u32) -> u32 {
    if limit == 0 { 100 } else { limit.min(10_000) }
}

fn import_state_text(state: ImportState) -> &'static str {
    match state {
        ImportState::Queued => "queued",
        ImportState::Running => "running",
        ImportState::Completed => "completed",
        ImportState::Failed => "failed",
    }
}

fn parse_import_state(value: &str) -> Result<ImportState, DatasetStoreError> {
    match value {
        "queued" => Ok(ImportState::Queued),
        "running" => Ok(ImportState::Running),
        "completed" => Ok(ImportState::Completed),
        "failed" => Ok(ImportState::Failed),
        other => Err(DatasetStoreError(format!("unknown import state: {other}"))),
    }
}

fn parse_format(value: &str) -> Result<ImportFormat, DatasetStoreError> {
    match value {
        "jsonl" => Ok(ImportFormat::Jsonl),
        "csv" => Ok(ImportFormat::Csv),
        other => Err(DatasetStoreError(format!("unknown import format: {other}"))),
    }
}

fn row_status_text(status: ImportRowStatus) -> &'static str {
    match status {
        ImportRowStatus::Accepted => "accepted",
        ImportRowStatus::Rejected => "rejected",
    }
}

fn parse_row_status(value: &str) -> Result<ImportRowStatus, DatasetStoreError> {
    match value {
        "accepted" => Ok(ImportRowStatus::Accepted),
        "rejected" => Ok(ImportRowStatus::Rejected),
        other => Err(DatasetStoreError(format!(
            "unknown imported-row status: {other}"
        ))),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, DatasetStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DatasetStoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn to_i64(value: u64) -> Result<i64, DatasetStoreError> {
    i64::try_from(value).map_err(store_error)
}

fn to_u64(value: i64) -> Result<u64, DatasetStoreError> {
    u64::try_from(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> DatasetStoreError {
    DatasetStoreError(error.to_string())
}
