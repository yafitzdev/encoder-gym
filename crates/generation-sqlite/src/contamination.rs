use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    contamination::{ContaminationOverride, ContaminationReport, ContaminationStatus},
    ports::{BoxFuture, ContaminationQuery, ContaminationStore, WorkflowStoreError},
};

use crate::SqliteStore;

impl ContaminationStore for SqliteStore {
    fn create_contamination_report(
        &self,
        report: &ContaminationReport,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let report = report.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_contamination_report(&mut transaction, &report).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_contamination_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ContaminationReport>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ReportRow>(
                "SELECT id, status, cohort_ids_json, artifact_json, fingerprint, created_at \
                 FROM workflow_contamination_reports WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(ReportRow::into_domain)
            .transpose()
        })
    }

    fn query_contamination_reports(
        &self,
        query: ContaminationQuery,
    ) -> BoxFuture<'_, Result<Vec<ContaminationReport>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT DISTINCT r.id, r.status, r.cohort_ids_json, r.artifact_json, \
                 r.fingerprint, r.created_at FROM workflow_contamination_reports r",
            );
            if query.cohort_id.is_some() {
                builder.push(" JOIN workflow_contamination_report_cohorts c ON c.report_id = r.id");
            }
            let has_cohort = query.cohort_id.is_some();
            if let Some(cohort_id) = query.cohort_id {
                builder.push(" WHERE c.cohort_id = ").push_bind(cohort_id);
            }
            if let Some(status) = query.status {
                builder
                    .push(if has_cohort { " AND " } else { " WHERE " })
                    .push("r.status = ")
                    .push_bind(enum_string(&status)?);
            }
            builder
                .push(" ORDER BY r.created_at DESC, r.id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<ReportRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ReportRow::into_domain)
                .collect()
        })
    }

    fn append_contamination_override(
        &self,
        value: &ContaminationOverride,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let value = value.clone();
        Box::pin(async move {
            validate_override(&value)?;
            let report = self
                .get_contamination_report(value.report_id)
                .await?
                .ok_or_else(|| WorkflowStoreError("contamination report not found".into()))?;
            if report.fingerprint != value.report_fingerprint
                || report.status != ContaminationStatus::Blocked
            {
                return Err(WorkflowStoreError(
                    "override does not reference the persisted blocked report".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO workflow_contamination_overrides \
                 (id, report_id, report_fingerprint, reason, approved_by, artifact_json, \
                  fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(value.id)
            .bind(value.report_id)
            .bind(&value.report_fingerprint)
            .bind(&value.reason)
            .bind(&value.approved_by)
            .bind(to_json(&value)?)
            .bind(&value.fingerprint)
            .bind(value.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_contamination_override(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ContaminationOverride>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, OverrideRow>(
                "SELECT id, report_id, report_fingerprint, reason, approved_by, artifact_json, \
                 fingerprint, created_at FROM workflow_contamination_overrides \
                 WHERE report_id = ?",
            )
            .bind(report_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(OverrideRow::into_domain)
            .transpose()
        })
    }
}

pub(crate) async fn insert_contamination_report(
    connection: &mut SqliteConnection,
    report: &ContaminationReport,
) -> Result<(), WorkflowStoreError> {
    validate_report(report)?;
    for cohort_id in &report.cohort_ids {
        let persisted: Option<String> =
            sqlx::query_scalar("SELECT fingerprint FROM workflow_evaluation_cohorts WHERE id = ?")
                .bind(cohort_id)
                .fetch_optional(&mut *connection)
                .await
                .map_err(store_error)?;
        if persisted.as_deref()
            != report
                .cohort_fingerprints
                .get(cohort_id)
                .map(String::as_str)
        {
            return Err(WorkflowStoreError(format!(
                "cohort fingerprint does not match persistence: {cohort_id}"
            )));
        }
    }
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
    .execute(&mut *connection)
    .await
    .map_err(store_error)?;
    for cohort_id in &report.cohort_ids {
        sqlx::query(
            "INSERT INTO workflow_contamination_report_cohorts \
             (report_id, cohort_id) VALUES (?, ?)",
        )
        .bind(report.id)
        .bind(cohort_id)
        .execute(&mut *connection)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
struct ReportRow {
    id: Uuid,
    status: String,
    cohort_ids_json: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ReportRow {
    fn into_domain(self) -> Result<ContaminationReport, WorkflowStoreError> {
        let value: ContaminationReport = from_json(&self.artifact_json)?;
        if value.id != self.id
            || enum_string(&value.status)? != self.status
            || to_json(&value.cohort_ids)? != self.cohort_ids_json
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "contamination report normalized fields do not match artifact".into(),
            ));
        }
        validate_report(&value)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, FromRow)]
struct OverrideRow {
    id: Uuid,
    report_id: Uuid,
    report_fingerprint: String,
    reason: String,
    approved_by: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl OverrideRow {
    fn into_domain(self) -> Result<ContaminationOverride, WorkflowStoreError> {
        let value: ContaminationOverride = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.report_id != self.report_id
            || value.report_fingerprint != self.report_fingerprint
            || value.reason != self.reason
            || value.approved_by != self.approved_by
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "contamination override normalized fields do not match artifact".into(),
            ));
        }
        validate_override(&value)?;
        Ok(value)
    }
}

fn validate_report(value: &ContaminationReport) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "contamination report fingerprint mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_override(value: &ContaminationOverride) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "contamination override fingerprint mismatch".into(),
        ));
    }
    Ok(())
}

fn enum_string(value: &impl serde::Serialize) -> Result<String, WorkflowStoreError> {
    let json = serde_json::to_value(value).map_err(store_error)?;
    json.as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| WorkflowStoreError("enum did not serialize as a string".into()))
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
