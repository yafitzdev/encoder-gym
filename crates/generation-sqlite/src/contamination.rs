use std::collections::BTreeSet;

use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    contamination::{
        CohortContaminationInput, ContaminationMember, ContaminationOverride, ContaminationReport,
        ContaminationStatus, check_contamination,
    },
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
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_contamination_report(&mut connection, id).await
        })
    }

    fn query_contamination_reports(
        &self,
        query: ContaminationQuery,
    ) -> BoxFuture<'_, Result<Vec<ContaminationReport>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT DISTINCT r.id FROM workflow_contamination_reports r",
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
            let ids = builder
                .build_query_scalar::<Uuid>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?;
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            let mut reports = Vec::with_capacity(ids.len());
            for id in ids {
                let report = load_contamination_report(&mut connection, id)
                    .await?
                    .ok_or_else(|| {
                        WorkflowStoreError(format!(
                            "contamination report disappeared while listing: {id}"
                        ))
                    })?;
                reports.push(report);
            }
            Ok(reports)
        })
    }

    fn append_contamination_override(
        &self,
        value: &ContaminationOverride,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let value = value.clone();
        Box::pin(async move {
            validate_override(&value)?;
            let training_authority: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM workflow_training_benchmark_checks \
                 WHERE contamination_report_id = ?)",
            )
            .bind(value.report_id)
            .fetch_one(self.pool())
            .await
            .map_err(store_error)?;
            if training_authority {
                return Err(WorkflowStoreError(
                    "training-benchmark contamination reports cannot be overridden".into(),
                ));
            }
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

pub(crate) async fn load_contamination_report(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<ContaminationReport>, WorkflowStoreError> {
    let Some(row) = sqlx::query_as::<_, ReportRow>(
        "SELECT id, status, cohort_ids_json, artifact_json, fingerprint, created_at \
         FROM workflow_contamination_reports WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let report = row.into_domain()?;
    let persisted_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT cohort_id FROM workflow_contamination_report_cohorts \
         WHERE report_id = ? ORDER BY cohort_id",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await
    .map_err(store_error)?;
    let mut artifact_ids = report.cohort_ids.clone();
    artifact_ids.sort();
    if artifact_ids != persisted_ids {
        return Err(WorkflowStoreError(format!(
            "contamination report cohort index differs from artifact: {id}"
        )));
    }
    validate_persisted_report_bindings(connection, &report).await?;
    Ok(Some(report))
}

pub(crate) async fn insert_contamination_report(
    connection: &mut SqliteConnection,
    report: &ContaminationReport,
) -> Result<(), WorkflowStoreError> {
    validate_report(report)?;
    validate_persisted_report_bindings(connection, report).await?;
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

async fn validate_persisted_report_bindings(
    connection: &mut SqliteConnection,
    report: &ContaminationReport,
) -> Result<(), WorkflowStoreError> {
    let cohort_ids = report.cohort_ids.iter().copied().collect::<BTreeSet<_>>();
    if cohort_ids.len() != report.cohort_ids.len()
        || report
            .cohort_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != cohort_ids
        || report
            .role_decision_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != cohort_ids
    {
        return Err(WorkflowStoreError(
            "contamination report evidence bindings are incomplete or noncanonical".into(),
        ));
    }
    let mut inputs = Vec::with_capacity(report.cohort_ids.len());
    for cohort_id in &report.cohort_ids {
        let cohort = crate::governance::load_cohort(connection, *cohort_id)
            .await?
            .ok_or_else(|| WorkflowStoreError(format!("cohort not found: {cohort_id}")))?;
        if report.cohort_fingerprints.get(cohort_id) != Some(&cohort.fingerprint) {
            return Err(WorkflowStoreError(format!(
                "cohort fingerprint does not match persistence: {cohort_id}"
            )));
        }
        let role_fingerprint = report
            .role_decision_fingerprints
            .get(cohort_id)
            .ok_or_else(|| {
                WorkflowStoreError(format!(
                    "contamination report has no role binding: {cohort_id}"
                ))
            })?;
        let role_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM workflow_cohort_role_decisions \
             WHERE cohort_id = ? AND fingerprint = ?",
        )
        .bind(cohort_id)
        .bind(role_fingerprint)
        .fetch_optional(&mut *connection)
        .await
        .map_err(store_error)?;
        let role_id = role_id.ok_or_else(|| {
            WorkflowStoreError(format!(
                "cohort role fingerprint does not match persistence: {cohort_id}"
            ))
        })?;
        let role = crate::governance::load_role_decision(connection, role_id, *cohort_id)
            .await?
            .ok_or_else(|| {
                WorkflowStoreError(format!(
                    "cohort role decision disappeared while validating report: {cohort_id}"
                ))
            })?;
        let (snapshot, members) =
            crate::load_verified_snapshot_with_members(connection, cohort.snapshot_id)
                .await
                .map_err(store_error)?
                .ok_or_else(|| {
                    WorkflowStoreError(format!(
                        "contamination cohort snapshot not found: {}",
                        cohort.snapshot_id
                    ))
                })?;
        if snapshot.fingerprint != cohort.snapshot_fingerprint {
            return Err(WorkflowStoreError(format!(
                "contamination cohort snapshot differs from persistence: {cohort_id}"
            )));
        }
        let members = members
            .iter()
            .filter(|member| member.split == cohort.split)
            .map(|member| {
                ContaminationMember::from_snapshot_member(member, report.group_dimension.as_deref())
            })
            .collect::<Vec<_>>();
        if members.is_empty() {
            return Err(WorkflowStoreError(format!(
                "contamination cohort has no members in its pinned split: {cohort_id}"
            )));
        }
        inputs.push(CohortContaminationInput {
            cohort,
            role,
            members,
        });
    }
    let recomputed = check_contamination(
        inputs,
        report.group_dimension.clone(),
        report.policy.clone(),
    )
    .map_err(store_error)?;
    if recomputed.cohort_ids != report.cohort_ids
        || recomputed.cohort_fingerprints != report.cohort_fingerprints
        || recomputed.role_decision_fingerprints != report.role_decision_fingerprints
        || recomputed.group_dimension != report.group_dimension
        || recomputed.policy != report.policy
        || recomputed.counts != report.counts
        || recomputed.findings != report.findings
        || recomputed.status != report.status
        || recomputed.reasons != report.reasons
        || recomputed.fingerprint != report.fingerprint
    {
        return Err(WorkflowStoreError(
            "contamination report differs from recomputed persisted-member evidence".into(),
        ));
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
