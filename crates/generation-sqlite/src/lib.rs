//! SQLite implementations of persistence ports owned by `synthetic-data-core`.

mod advisor;
mod analysis;
mod approval;
mod benchmark;
mod contamination;
mod evaluation;
mod generation_execution;
mod governance;
mod imports;
mod optimization;
mod project;
mod project_bootstrap;
mod project_preparation;
mod promotion;
mod provenance;
mod recovery;
mod semantic;
mod stop;
mod training;
mod workflow;
mod workflow_run;

use std::{collections::BTreeMap, str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use dataset_core::{
    domain::{
        DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceProvenance, SourceRow,
        SplitConfiguration,
    },
    ports::{
        AcceptedRowSource, BoxFuture as DatasetBoxFuture, DatasetStoreError, SnapshotQuery,
        SnapshotStore,
    },
};
use generation_core::{
    coverage::CellCounts,
    domain::{
        BackendConfiguration, DatasetDefinition, DimensionDefinition, GeneratedRow,
        GenerationParameters, GenerationPlan, PlannedCell, ValidationStatus,
    },
    jobs::{GenerationJob, JobState},
    ports::{
        BackendConfigurationStore, BoxFuture, DatasetQuery, DatasetStore, JobQuery, JobStore,
        PlanStore, RowQuery, RowStore, StoreError,
    },
};
use sqlx::{
    FromRow, QueryBuilder, Sqlite, SqliteConnection, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(store_error)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(if database_url.contains(":memory:") {
                1
            } else {
                5
            })
            .connect_with(options)
            .await
            .map_err(store_error)?;
        MIGRATOR.run(&pool).await.map_err(store_error)?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

impl DatasetStore for SqliteStore {
    fn create_dataset(&self, dataset: &DatasetDefinition) -> BoxFuture<'_, Result<(), StoreError>> {
        let dataset = dataset.clone();
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(store_error)?;
            insert_dataset(&mut connection, &dataset).await?;
            Ok(())
        })
    }

    fn get_dataset(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetDefinition>, StoreError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, DatasetRecord>(
                "SELECT id, name, task_description, labels_json, dimensions_json, created_at \
                 FROM dataset_definitions WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            row.map(DatasetRecord::into_domain).transpose()
        })
    }

    fn list_datasets(&self) -> BoxFuture<'_, Result<Vec<DatasetDefinition>, StoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, DatasetRecord>(
                "SELECT id, name, task_description, labels_json, dimensions_json, created_at \
                 FROM dataset_definitions ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(DatasetRecord::into_domain)
            .collect()
        })
    }

    fn query_datasets(
        &self,
        query: DatasetQuery,
    ) -> BoxFuture<'_, Result<Vec<DatasetDefinition>, StoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, name, task_description, labels_json, dimensions_json, created_at \
                 FROM dataset_definitions",
            );
            if let Some(name) = query.name_contains {
                builder.push(" WHERE instr(lower(name), lower(");
                builder.push_bind(name);
                builder.push(")) > 0");
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<DatasetRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(DatasetRecord::into_domain)
                .collect()
        })
    }
}

impl PlanStore for SqliteStore {
    fn create_plan(&self, plan: &GenerationPlan) -> BoxFuture<'_, Result<(), StoreError>> {
        let plan = plan.clone();
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(store_error)?;
            insert_plan(&mut connection, &plan).await?;
            Ok(())
        })
    }

    fn get_plan(&self, id: Uuid) -> BoxFuture<'_, Result<Option<GenerationPlan>, StoreError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, PlanRecord>(
                "SELECT id, dataset_id, cells_json, created_at FROM generation_plans WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            row.map(PlanRecord::into_domain).transpose()
        })
    }
}

impl JobStore for SqliteStore {
    fn create_job(&self, job: &GenerationJob) -> BoxFuture<'_, Result<(), StoreError>> {
        let job = job.clone();
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(store_error)?;
            insert_generation_job(&mut connection, &job).await?;
            Ok(())
        })
    }

    fn save_job(&self, job: &GenerationJob) -> BoxFuture<'_, Result<(), StoreError>> {
        let job = job.clone();
        Box::pin(async move {
            sqlx::query(
                "UPDATE generation_jobs SET state = ?, generated_rows = ?, accepted_rows = ?, \
                 rejected_rows = ?, failed_requests = ?, \
                 cancel_requested = MAX(cancel_requested, ?), error_message = ?, \
                 updated_at = ? WHERE id = ?",
            )
            .bind(job_state_text(job.state))
            .bind(as_i64(job.generated_rows)?)
            .bind(as_i64(job.accepted_rows)?)
            .bind(as_i64(job.rejected_rows)?)
            .bind(as_i64(job.failed_requests)?)
            .bind(job.cancel_requested)
            .bind(job.error_message)
            .bind(job.updated_at)
            .bind(job.id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_job(&self, id: Uuid) -> BoxFuture<'_, Result<Option<GenerationJob>, StoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, JobRecord>(
                "SELECT id, dataset_id, plan_id, backend_name, backend_model, state, requested_rows, \
                 generated_rows, accepted_rows, rejected_rows, failed_requests, cancel_requested, \
                 error_message, created_at, updated_at FROM generation_jobs WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(JobRecord::into_domain)
            .transpose()
        })
    }

    fn request_job_cancellation(&self, id: Uuid) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE generation_jobs SET cancel_requested = 1, updated_at = ? \
                 WHERE id = ? AND state IN ('queued', 'running')",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn list_jobs(&self, query: JobQuery) -> BoxFuture<'_, Result<Vec<GenerationJob>, StoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, dataset_id, plan_id, backend_name, backend_model, state, \
                 requested_rows, generated_rows, accepted_rows, rejected_rows, failed_requests, \
                 cancel_requested, error_message, created_at, updated_at FROM generation_jobs",
            );
            let mut has_condition = false;
            if let Some(dataset_id) = query.dataset_id {
                builder.push(" WHERE dataset_id = ").push_bind(dataset_id);
                has_condition = true;
            }
            if let Some(plan_id) = query.plan_id {
                builder.push(if has_condition { " AND " } else { " WHERE " });
                builder.push("plan_id = ").push_bind(plan_id);
                has_condition = true;
            }
            if let Some(state) = query.state {
                builder.push(if has_condition { " AND " } else { " WHERE " });
                builder.push("state = ").push_bind(job_state_text(state));
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<JobRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(JobRecord::into_domain)
                .collect()
        })
    }
}

impl RowStore for SqliteStore {
    fn insert_rows(&self, rows: &[GeneratedRow]) -> BoxFuture<'_, Result<(), StoreError>> {
        let rows = rows.to_vec();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            insert_generated_rows(&mut transaction, &rows).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_rows(&self, query: RowQuery) -> BoxFuture<'_, Result<Vec<GeneratedRow>, StoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, dataset_id, plan_id, generation_job_id, cell_key, text, normalized_text, \
                 label, dimensions_json, generator_backend, generator_model, created_at, \
                 validation_status, validation_errors_json, generation_metadata_json, \
                 fields_json, construction_json \
                 FROM generated_rows WHERE 1 = 1",
            );
            if let Some(dataset_id) = query.dataset_id {
                builder.push(" AND dataset_id = ").push_bind(dataset_id);
            }
            if let Some(job_id) = query.job_id {
                builder.push(" AND generation_job_id = ").push_bind(job_id);
            }
            if let Some(status) = query.status {
                builder
                    .push(" AND validation_status = ")
                    .push_bind(validation_status_text(status));
            }
            let limit = if query.limit == 0 {
                100
            } else {
                query.limit.min(10_000)
            };
            builder
                .push(" ORDER BY created_at, id LIMIT ")
                .push_bind(i64::from(limit))
                .push(" OFFSET ")
                .push_bind(i64::from(query.offset));

            builder
                .build_query_as::<RowRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(RowRecord::into_domain)
                .collect()
        })
    }

    fn accepted_normalized_texts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>> {
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

    fn cell_counts(
        &self,
        plan_id: Uuid,
    ) -> BoxFuture<'_, Result<BTreeMap<String, CellCounts>, StoreError>> {
        Box::pin(async move {
            let records = sqlx::query_as::<_, CountRecord>(
                "SELECT cell_key, COUNT(*) AS attempted, \
                 SUM(CASE WHEN validation_status = 'accepted' THEN 1 ELSE 0 END) AS accepted, \
                 SUM(CASE WHEN validation_status = 'rejected' THEN 1 ELSE 0 END) AS rejected \
                 FROM generated_rows WHERE plan_id = ? GROUP BY cell_key",
            )
            .bind(plan_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            records
                .into_iter()
                .map(|record| {
                    Ok((
                        record.cell_key,
                        CellCounts {
                            attempted: as_u32(record.attempted)?,
                            accepted: as_u32(record.accepted)?,
                            rejected: as_u32(record.rejected)?,
                        },
                    ))
                })
                .collect()
        })
    }

    fn dataset_cell_counts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<BTreeMap<String, CellCounts>, StoreError>> {
        Box::pin(async move {
            let records = sqlx::query_as::<_, CountRecord>(
                "SELECT cell_key, COUNT(*) AS attempted, \
                 SUM(CASE WHEN validation_status = 'accepted' THEN 1 ELSE 0 END) AS accepted, \
                 SUM(CASE WHEN validation_status = 'rejected' THEN 1 ELSE 0 END) AS rejected \
                 FROM ( \
                     SELECT cell_key, validation_status FROM generated_rows WHERE dataset_id = ? \
                     UNION ALL \
                     SELECT cell_key, validation_status FROM imported_rows \
                     WHERE dataset_id = ? AND cell_key IS NOT NULL \
                 ) GROUP BY cell_key",
            )
            .bind(dataset_id)
            .bind(dataset_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            records
                .into_iter()
                .map(|record| {
                    Ok((
                        record.cell_key,
                        CellCounts {
                            attempted: as_u32(record.attempted)?,
                            accepted: as_u32(record.accepted)?,
                            rejected: as_u32(record.rejected)?,
                        },
                    ))
                })
                .collect()
        })
    }
}

impl BackendConfigurationStore for SqliteStore {
    fn save_backend_configuration(
        &self,
        configuration: &BackendConfiguration,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let configuration = configuration.clone();
        Box::pin(async move {
            let mut connection = self.pool.acquire().await.map_err(store_error)?;
            upsert_backend(&mut connection, &configuration).await?;
            Ok(())
        })
    }

    fn get_backend_configuration(
        &self,
        name: &str,
    ) -> BoxFuture<'_, Result<Option<BackendConfiguration>, StoreError>> {
        let name = name.to_owned();
        Box::pin(async move {
            sqlx::query_as::<_, BackendConfigurationRecord>(
                "SELECT name, base_url, model, parameters_json, updated_at \
                 FROM backend_configurations WHERE name = ?",
            )
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(BackendConfigurationRecord::into_domain)
            .transpose()
        })
    }
}

impl AcceptedRowSource for SqliteStore {
    fn list_accepted_source_rows(
        &self,
        dataset_id: Uuid,
    ) -> DatasetBoxFuture<'_, Result<Vec<SourceRow>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SourceRowRecord>(
                "SELECT id, dataset_id, text, label, dimensions_json, fields_json, provenance_json, created_at \
                 FROM dataset_source_rows WHERE dataset_id = ? \
                 ORDER BY created_at, id",
            )
            .bind(dataset_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .into_iter()
            .map(SourceRowRecord::into_domain)
            .collect()
        })
    }

    fn query_accepted_source_rows(
        &self,
        dataset_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> DatasetBoxFuture<'_, Result<Vec<SourceRow>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SourceRowRecord>(
                "SELECT id, dataset_id, text, label, dimensions_json, fields_json, provenance_json, created_at \
                 FROM dataset_source_rows WHERE dataset_id = ? ORDER BY created_at, id \
                 LIMIT ? OFFSET ?",
            )
            .bind(dataset_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .into_iter()
            .map(SourceRowRecord::into_domain)
            .collect()
        })
    }
}

impl SnapshotStore for SqliteStore {
    fn create_snapshot(
        &self,
        snapshot: &DatasetSnapshot,
        members: &[SnapshotMember],
    ) -> DatasetBoxFuture<'_, Result<(), DatasetStoreError>> {
        let snapshot = snapshot.clone();
        let members = members.to_vec();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(dataset_store_error)?;
            insert_snapshot(&mut transaction, &snapshot, &members).await?;
            transaction.commit().await.map_err(dataset_store_error)?;
            Ok(())
        })
    }

    fn get_snapshot(
        &self,
        id: Uuid,
    ) -> DatasetBoxFuture<'_, Result<Option<DatasetSnapshot>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SnapshotRecord>(
                "SELECT id, source_dataset_id, name, description, split_configuration_json, \
                 member_count, fingerprint, created_at FROM dataset_snapshots WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .map(SnapshotRecord::into_domain)
            .transpose()
        })
    }

    fn list_snapshots(
        &self,
    ) -> DatasetBoxFuture<'_, Result<Vec<DatasetSnapshot>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SnapshotRecord>(
                "SELECT id, source_dataset_id, name, description, split_configuration_json, \
                 member_count, fingerprint, created_at FROM dataset_snapshots ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .into_iter()
            .map(SnapshotRecord::into_domain)
            .collect()
        })
    }

    fn query_snapshots(
        &self,
        query: SnapshotQuery,
    ) -> DatasetBoxFuture<'_, Result<Vec<DatasetSnapshot>, DatasetStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, source_dataset_id, name, description, split_configuration_json, \
                 member_count, fingerprint, created_at FROM dataset_snapshots",
            );
            if let Some(dataset_id) = query.dataset_id {
                builder
                    .push(" WHERE source_dataset_id = ")
                    .push_bind(dataset_id);
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<SnapshotRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(dataset_store_error)?
                .into_iter()
                .map(SnapshotRecord::into_domain)
                .collect()
        })
    }

    fn list_snapshot_members(
        &self,
        snapshot_id: Uuid,
    ) -> DatasetBoxFuture<'_, Result<Vec<SnapshotMember>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SnapshotMemberRecord>(
                "SELECT id, snapshot_id, source_row_id, split, text, label, dimensions_json, fields_json, \
                 source_provenance_json, source_created_at FROM dataset_snapshot_members \
                 WHERE snapshot_id = ? ORDER BY source_row_id",
            )
            .bind(snapshot_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .into_iter()
            .map(SnapshotMemberRecord::into_domain)
            .collect()
        })
    }

    fn query_snapshot_members(
        &self,
        snapshot_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> DatasetBoxFuture<'_, Result<Vec<SnapshotMember>, DatasetStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SnapshotMemberRecord>(
                "SELECT id, snapshot_id, source_row_id, split, text, label, dimensions_json, fields_json, \
                 source_provenance_json, source_created_at FROM dataset_snapshot_members \
                 WHERE snapshot_id = ? ORDER BY source_row_id LIMIT ? OFFSET ?",
            )
            .bind(snapshot_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(dataset_store_error)?
            .into_iter()
            .map(SnapshotMemberRecord::into_domain)
            .collect()
        })
    }
}

pub(crate) async fn insert_snapshot(
    connection: &mut SqliteConnection,
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<(), DatasetStoreError> {
    if snapshot.member_count != members.len() as u64 {
        return Err(DatasetStoreError(format!(
            "snapshot declares {} members but {} were supplied",
            snapshot.member_count,
            members.len()
        )));
    }
    if members
        .iter()
        .any(|member| member.snapshot_id != snapshot.id)
    {
        return Err(DatasetStoreError(
            "all members must reference the persisted snapshot".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO dataset_snapshots \
         (id, source_dataset_id, name, description, split_configuration_json, \
          member_count, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(snapshot.id)
    .bind(snapshot.source_dataset_id)
    .bind(&snapshot.name)
    .bind(&snapshot.description)
    .bind(dataset_to_json(&snapshot.split_configuration)?)
    .bind(i64::try_from(snapshot.member_count).map_err(dataset_store_error)?)
    .bind(&snapshot.fingerprint)
    .bind(snapshot.created_at)
    .execute(&mut *connection)
    .await
    .map_err(dataset_store_error)?;
    for member in members {
        sqlx::query(
            "INSERT INTO dataset_snapshot_members \
             (id, snapshot_id, source_row_id, split, text, label, dimensions_json, fields_json, \
              source_provenance_json, source_created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(member.id)
        .bind(member.snapshot_id)
        .bind(member.source_row_id)
        .bind(member.split.as_str())
        .bind(&member.text)
        .bind(&member.label)
        .bind(dataset_to_json(&member.dimensions)?)
        .bind(dataset_to_json(&member.fields)?)
        .bind(dataset_to_json(&member.source_provenance)?)
        .bind(member.source_created_at)
        .execute(&mut *connection)
        .await
        .map_err(dataset_store_error)?;
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct SourceRowRecord {
    id: Uuid,
    dataset_id: Uuid,
    text: String,
    label: String,
    dimensions_json: String,
    fields_json: String,
    provenance_json: String,
    created_at: DateTime<Utc>,
}

impl SourceRowRecord {
    fn into_domain(self) -> Result<SourceRow, DatasetStoreError> {
        Ok(SourceRow {
            id: self.id,
            dataset_id: self.dataset_id,
            text: self.text,
            label: self.label,
            dimensions: dataset_from_json(&self.dimensions_json)?,
            fields: dataset_from_json(&self.fields_json)?,
            provenance: dataset_from_json(&self.provenance_json)?,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct SnapshotRecord {
    id: Uuid,
    source_dataset_id: Uuid,
    name: String,
    description: Option<String>,
    split_configuration_json: String,
    member_count: i64,
    fingerprint: Option<String>,
    created_at: DateTime<Utc>,
}

impl SnapshotRecord {
    fn into_domain(self) -> Result<DatasetSnapshot, DatasetStoreError> {
        Ok(DatasetSnapshot {
            id: self.id,
            source_dataset_id: self.source_dataset_id,
            name: self.name,
            description: self.description,
            split_configuration: dataset_from_json::<SplitConfiguration>(
                &self.split_configuration_json,
            )?,
            member_count: u64::try_from(self.member_count).map_err(dataset_store_error)?,
            fingerprint: self
                .fingerprint
                .unwrap_or_else(|| "legacy:unavailable".into()),
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct SnapshotMemberRecord {
    id: Uuid,
    snapshot_id: Uuid,
    source_row_id: Uuid,
    split: String,
    text: String,
    label: String,
    dimensions_json: String,
    fields_json: String,
    source_provenance_json: String,
    source_created_at: DateTime<Utc>,
}

impl SnapshotMemberRecord {
    fn into_domain(self) -> Result<SnapshotMember, DatasetStoreError> {
        Ok(SnapshotMember {
            id: self.id,
            snapshot_id: self.snapshot_id,
            source_row_id: self.source_row_id,
            split: parse_snapshot_split(&self.split)?,
            text: self.text,
            label: self.label,
            dimensions: dataset_from_json(&self.dimensions_json)?,
            fields: dataset_from_json(&self.fields_json)?,
            source_provenance: dataset_from_json(&self.source_provenance_json)?,
            source_created_at: self.source_created_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct DatasetRecord {
    id: Uuid,
    name: String,
    task_description: String,
    labels_json: String,
    dimensions_json: String,
    created_at: DateTime<Utc>,
}

impl DatasetRecord {
    fn into_domain(self) -> Result<DatasetDefinition, StoreError> {
        DatasetDefinition::with_identity(
            self.id,
            self.name,
            self.task_description,
            from_json::<Vec<String>>(&self.labels_json)?,
            from_json::<Vec<DimensionDefinition>>(&self.dimensions_json)?,
            self.created_at,
        )
        .map_err(store_error)
    }
}

#[derive(Debug, FromRow)]
struct PlanRecord {
    id: Uuid,
    dataset_id: Uuid,
    cells_json: String,
    created_at: DateTime<Utc>,
}

impl PlanRecord {
    fn into_domain(self) -> Result<GenerationPlan, StoreError> {
        GenerationPlan::with_identity(
            self.id,
            self.dataset_id,
            from_json::<Vec<PlannedCell>>(&self.cells_json)?,
            self.created_at,
        )
        .map_err(store_error)
    }
}

#[derive(Debug, FromRow)]
struct JobRecord {
    id: Uuid,
    dataset_id: Uuid,
    plan_id: Uuid,
    backend_name: String,
    backend_model: String,
    state: String,
    requested_rows: i64,
    generated_rows: i64,
    accepted_rows: i64,
    rejected_rows: i64,
    failed_requests: i64,
    cancel_requested: bool,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl JobRecord {
    fn into_domain(self) -> Result<GenerationJob, StoreError> {
        Ok(GenerationJob {
            id: self.id,
            dataset_id: self.dataset_id,
            plan_id: self.plan_id,
            backend_name: self.backend_name,
            backend_model: self.backend_model,
            state: parse_job_state(&self.state)?,
            requested_rows: as_u64(self.requested_rows)?,
            generated_rows: as_u64(self.generated_rows)?,
            accepted_rows: as_u64(self.accepted_rows)?,
            rejected_rows: as_u64(self.rejected_rows)?,
            failed_requests: as_u64(self.failed_requests)?,
            cancel_requested: self.cancel_requested,
            error_message: self.error_message,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct RowRecord {
    id: Uuid,
    dataset_id: Uuid,
    plan_id: Uuid,
    generation_job_id: Uuid,
    cell_key: String,
    text: String,
    normalized_text: String,
    label: String,
    dimensions_json: String,
    generator_backend: String,
    generator_model: String,
    created_at: DateTime<Utc>,
    validation_status: String,
    validation_errors_json: String,
    generation_metadata_json: String,
    fields_json: String,
    construction_json: Option<String>,
}

impl RowRecord {
    fn into_domain(self) -> Result<GeneratedRow, StoreError> {
        Ok(GeneratedRow {
            id: self.id,
            dataset_id: self.dataset_id,
            plan_id: self.plan_id,
            generation_job_id: self.generation_job_id,
            cell_key: self.cell_key,
            text: self.text,
            normalized_text: self.normalized_text,
            label: self.label,
            dimensions: from_json(&self.dimensions_json)?,
            fields: from_json(&self.fields_json)?,
            construction: self
                .construction_json
                .as_deref()
                .map(from_json)
                .transpose()?,
            generator_backend: self.generator_backend,
            generator_model: self.generator_model,
            created_at: self.created_at,
            validation_status: parse_validation_status(&self.validation_status)?,
            validation_errors: from_json(&self.validation_errors_json)?,
            generation_metadata: from_json(&self.generation_metadata_json)?,
        })
    }
}

#[derive(Debug, FromRow)]
struct CountRecord {
    cell_key: String,
    attempted: i64,
    accepted: i64,
    rejected: i64,
}

#[derive(Debug, FromRow)]
struct BackendConfigurationRecord {
    name: String,
    base_url: Option<String>,
    model: String,
    parameters_json: String,
    updated_at: DateTime<Utc>,
}

impl BackendConfigurationRecord {
    fn into_domain(self) -> Result<BackendConfiguration, StoreError> {
        Ok(BackendConfiguration {
            name: self.name,
            base_url: self.base_url,
            model: self.model,
            parameters: from_json::<GenerationParameters>(&self.parameters_json)?,
            updated_at: self.updated_at,
        })
    }
}

pub(crate) async fn insert_dataset(
    connection: &mut SqliteConnection,
    dataset: &DatasetDefinition,
) -> Result<(), StoreError> {
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
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

pub(crate) async fn insert_plan(
    connection: &mut SqliteConnection,
    plan: &GenerationPlan,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO generation_plans (id, dataset_id, cells_json, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(plan.id)
    .bind(plan.dataset_id)
    .bind(to_json(&plan.cells)?)
    .bind(plan.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

pub(crate) async fn insert_generation_job(
    connection: &mut SqliteConnection,
    job: &GenerationJob,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO generation_jobs \
         (id, dataset_id, plan_id, backend_name, backend_model, state, requested_rows, \
          generated_rows, accepted_rows, rejected_rows, failed_requests, cancel_requested, \
          error_message, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job.id)
    .bind(job.dataset_id)
    .bind(job.plan_id)
    .bind(&job.backend_name)
    .bind(&job.backend_model)
    .bind(job_state_text(job.state))
    .bind(as_i64(job.requested_rows)?)
    .bind(as_i64(job.generated_rows)?)
    .bind(as_i64(job.accepted_rows)?)
    .bind(as_i64(job.rejected_rows)?)
    .bind(as_i64(job.failed_requests)?)
    .bind(job.cancel_requested)
    .bind(&job.error_message)
    .bind(job.created_at)
    .bind(job.updated_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

pub(crate) async fn insert_generated_rows(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    rows: &[GeneratedRow],
) -> Result<(), StoreError> {
    for row in rows {
        sqlx::query(
            "INSERT INTO generated_rows \
             (id, dataset_id, plan_id, generation_job_id, cell_key, text, normalized_text, \
              label, dimensions_json, generator_backend, generator_model, created_at, \
              validation_status, validation_errors_json, generation_metadata_json, fields_json, \
              construction_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.id)
        .bind(row.dataset_id)
        .bind(row.plan_id)
        .bind(row.generation_job_id)
        .bind(&row.cell_key)
        .bind(&row.text)
        .bind(&row.normalized_text)
        .bind(&row.label)
        .bind(to_json(&row.dimensions)?)
        .bind(&row.generator_backend)
        .bind(&row.generator_model)
        .bind(row.created_at)
        .bind(validation_status_text(row.validation_status))
        .bind(to_json(&row.validation_errors)?)
        .bind(to_json(&row.generation_metadata)?)
        .bind(to_json(&row.fields)?)
        .bind(row.construction.as_ref().map(to_json).transpose()?)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
        if row.validation_status == ValidationStatus::Accepted {
            let provenance = SourceProvenance::Generated {
                generation_job_id: row.generation_job_id,
                backend: row.generator_backend.clone(),
                model: row.generator_model.clone(),
                construction_plan_fingerprint: row
                    .construction
                    .as_ref()
                    .map(|trace| trace.plan_fingerprint.clone()),
            };
            sqlx::query(
                "INSERT INTO dataset_source_rows \
                 (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, \
                  label, dimensions_json, provenance_json, created_at, fields_json) \
                 VALUES (?, ?, 'generated', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id)
            .bind(row.dataset_id)
            .bind(row.id)
            .bind(&row.cell_key)
            .bind(&row.text)
            .bind(&row.normalized_text)
            .bind(&row.label)
            .bind(to_json(&row.dimensions)?)
            .bind(to_json(&provenance)?)
            .bind(row.created_at)
            .bind(to_json(&row.fields)?)
            .execute(&mut **transaction)
            .await
            .map_err(store_error)?;
        }
    }
    Ok(())
}

pub(crate) async fn upsert_backend(
    connection: &mut SqliteConnection,
    configuration: &BackendConfiguration,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO backend_configurations \
         (name, base_url, model, parameters_json, updated_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(name) DO UPDATE SET base_url = excluded.base_url, \
         model = excluded.model, parameters_json = excluded.parameters_json, \
         updated_at = excluded.updated_at",
    )
    .bind(&configuration.name)
    .bind(&configuration.base_url)
    .bind(&configuration.model)
    .bind(to_json(&configuration.parameters)?)
    .bind(configuration.updated_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, StoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> StoreError {
    StoreError(error.to_string())
}

fn dataset_to_json<T: serde::Serialize>(value: &T) -> Result<String, DatasetStoreError> {
    serde_json::to_string(value).map_err(dataset_store_error)
}

fn dataset_from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DatasetStoreError> {
    serde_json::from_str(value).map_err(dataset_store_error)
}

fn dataset_store_error(error: impl std::fmt::Display) -> DatasetStoreError {
    DatasetStoreError(error.to_string())
}

fn parse_snapshot_split(value: &str) -> Result<SnapshotSplit, DatasetStoreError> {
    match value {
        "train" => Ok(SnapshotSplit::Train),
        "validation" => Ok(SnapshotSplit::Validation),
        "test" => Ok(SnapshotSplit::Test),
        other => Err(DatasetStoreError(format!(
            "unknown snapshot split: {other}"
        ))),
    }
}

fn as_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(store_error)
}

fn as_u64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(store_error)
}

fn as_u32(value: i64) -> Result<u32, StoreError> {
    u32::try_from(value).map_err(store_error)
}

fn job_state_text(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
    }
}

fn parse_job_state(value: &str) -> Result<JobState, StoreError> {
    match value {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running),
        "completed" => Ok(JobState::Completed),
        "failed" => Ok(JobState::Failed),
        "cancelled" => Ok(JobState::Cancelled),
        other => Err(StoreError(format!("unknown job state: {other}"))),
    }
}

fn validation_status_text(status: ValidationStatus) -> &'static str {
    match status {
        ValidationStatus::Accepted => "accepted",
        ValidationStatus::Rejected => "rejected",
    }
}

fn parse_validation_status(value: &str) -> Result<ValidationStatus, StoreError> {
    match value {
        "accepted" => Ok(ValidationStatus::Accepted),
        "rejected" => Ok(ValidationStatus::Rejected),
        other => Err(StoreError(format!("unknown validation status: {other}"))),
    }
}
