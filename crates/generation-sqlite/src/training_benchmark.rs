use std::collections::BTreeMap;

use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    contamination::ContaminationReport,
    governance::{CohortDisposition, CohortRole},
    ports::{
        BoxFuture, TrainingBenchmarkCheckQuery, TrainingBenchmarkCheckStore, WorkflowStoreError,
    },
    training_benchmark::{
        TrainingBenchmarkCheck, TrainingCohortEvidence, build_training_benchmark_check,
    },
};

use crate::SqliteStore;

impl TrainingBenchmarkCheckStore for SqliteStore {
    fn create_training_benchmark_check(
        &self,
        new_training_cohorts: &[TrainingCohortEvidence],
        report: &ContaminationReport,
        check: &TrainingBenchmarkCheck,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let new_training_cohorts = new_training_cohorts.to_vec();
        let report = report.clone();
        let check = check.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let supplied = new_training_cohorts
                .iter()
                .map(|evidence| (evidence.cohort.id, evidence))
                .collect::<BTreeMap<_, _>>();
            if supplied.len() != new_training_cohorts.len()
                || supplied.iter().any(|(id, evidence)| {
                    !check.training_cohorts.iter().any(|binding| {
                        binding.cohort_id == *id
                            && binding.split == evidence.cohort.split
                            && binding.cohort_fingerprint == evidence.cohort.fingerprint
                            && binding.role_decision_id == evidence.role.id
                            && binding.role_decision_fingerprint == evidence.role.fingerprint
                    })
                })
            {
                return Err(WorkflowStoreError(
                    "new training cohorts are not an exact subset of the check bindings".into(),
                ));
            }
            for binding in &check.training_cohorts {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM workflow_evaluation_cohorts WHERE id = ?)",
                )
                .bind(binding.cohort_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(store_error)?;
                if supplied.contains_key(&binding.cohort_id) == exists {
                    return Err(WorkflowStoreError(
                        "new training cohort set differs from missing persisted bindings".into(),
                    ));
                }
            }
            for evidence in &new_training_cohorts {
                crate::governance::insert_cohort_with_initial_role(
                    &mut transaction,
                    &evidence.cohort,
                    &evidence.role,
                )
                .await?;
            }
            crate::contamination::insert_contamination_report(&mut transaction, &report).await?;
            insert_training_benchmark_check(&mut transaction, &check).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_training_benchmark_check(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingBenchmarkCheck>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_training_benchmark_check(&mut connection, id, false).await
        })
    }

    fn get_executable_training_benchmark_check(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingBenchmarkCheck>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_training_benchmark_check(&mut connection, id, true).await
        })
    }

    fn query_training_benchmark_checks(
        &self,
        query: TrainingBenchmarkCheckQuery,
    ) -> BoxFuture<'_, Result<Vec<TrainingBenchmarkCheck>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT id FROM workflow_training_benchmark_checks");
            let mut has_filter = false;
            if let Some(id) = query.training_snapshot_id {
                builder.push(" WHERE training_snapshot_id = ").push_bind(id);
                has_filter = true;
            }
            if let Some(id) = query.benchmark_bundle_id {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("benchmark_bundle_id = ")
                    .push_bind(id);
                has_filter = true;
            }
            if let Some(status) = query.status {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("status = ")
                    .push_bind(enum_string(&status)?);
                has_filter = true;
            }
            if let Some(protocol) = query.protocol {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("training_input_protocol = ")
                    .push_bind(enum_string(&protocol)?);
                has_filter = true;
            }
            if let Some(version) = query.check_protocol_version {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("check_protocol_version = ")
                    .push_bind(version);
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
            let mut checks = Vec::with_capacity(ids.len());
            for id in ids {
                checks.push(
                    load_training_benchmark_check(&mut connection, id, false)
                        .await?
                        .ok_or_else(|| {
                            WorkflowStoreError(format!(
                                "training-benchmark check disappeared while listing: {id}"
                            ))
                        })?,
                );
            }
            Ok(checks)
        })
    }
}

async fn insert_training_benchmark_check(
    connection: &mut SqliteConnection,
    check: &TrainingBenchmarkCheck,
) -> Result<(), WorkflowStoreError> {
    check.validate_integrity().map_err(store_error)?;
    validate_persisted_authority(connection, check, true).await?;
    let has_override: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workflow_contamination_overrides WHERE report_id = ?)",
    )
    .bind(check.contamination_report_id)
    .fetch_one(&mut *connection)
    .await
    .map_err(store_error)?;
    if has_override {
        return Err(WorkflowStoreError(
            "training-benchmark contamination report has a forbidden override".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO workflow_training_benchmark_checks \
         (id, training_snapshot_id, benchmark_bundle_id, contamination_report_id, \
          check_protocol_version, training_input_protocol, status, artifact_json, fingerprint, \
          created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(check.id)
    .bind(check.training_snapshot_id)
    .bind(check.benchmark_bundle_id)
    .bind(check.contamination_report_id)
    .bind(&check.check_protocol_version)
    .bind(enum_string(&check.training_input_protocol)?)
    .bind(enum_string(&check.status)?)
    .bind(to_json(check)?)
    .bind(&check.fingerprint)
    .bind(check.created_at)
    .execute(&mut *connection)
    .await
    .map_err(store_error)?;
    for binding in &check.training_cohorts {
        sqlx::query(
            "INSERT INTO workflow_training_benchmark_check_cohorts \
             (check_id, cohort_id, role_decision_id, split, member_count) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(check.id)
        .bind(binding.cohort_id)
        .bind(binding.role_decision_id)
        .bind(enum_string(&binding.split)?)
        .bind(i64::try_from(binding.member_count).map_err(store_error)?)
        .execute(&mut *connection)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

pub(crate) async fn load_training_benchmark_check(
    connection: &mut SqliteConnection,
    id: Uuid,
    require_current_roles: bool,
) -> Result<Option<TrainingBenchmarkCheck>, WorkflowStoreError> {
    let Some(row) = sqlx::query_as::<_, CheckRow>(
        "SELECT id, training_snapshot_id, benchmark_bundle_id, contamination_report_id, \
         check_protocol_version, training_input_protocol, status, artifact_json, fingerprint, \
         created_at FROM workflow_training_benchmark_checks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let check = row.into_domain()?;
    validate_cohort_index(connection, &check).await?;
    validate_persisted_authority(connection, &check, require_current_roles).await?;
    Ok(Some(check))
}

async fn validate_cohort_index(
    connection: &mut SqliteConnection,
    check: &TrainingBenchmarkCheck,
) -> Result<(), WorkflowStoreError> {
    let rows = sqlx::query_as::<_, CohortIndexRow>(
        "SELECT cohort_id, role_decision_id, split, member_count \
         FROM workflow_training_benchmark_check_cohorts \
         WHERE check_id = ? ORDER BY split, cohort_id",
    )
    .bind(check.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(store_error)?;
    if rows.len() != check.training_cohorts.len() {
        return Err(WorkflowStoreError(
            "training-benchmark check cohort index is incomplete".into(),
        ));
    }
    for (row, binding) in rows.iter().zip(&check.training_cohorts) {
        if row.cohort_id != binding.cohort_id
            || row.role_decision_id != binding.role_decision_id
            || parse_enum::<dataset_core::domain::SnapshotSplit>(&row.split)? != binding.split
            || u64::try_from(row.member_count).map_err(store_error)? != binding.member_count
        {
            return Err(WorkflowStoreError(
                "training-benchmark check cohort index differs from artifact".into(),
            ));
        }
    }
    Ok(())
}

async fn validate_persisted_authority(
    connection: &mut SqliteConnection,
    check: &TrainingBenchmarkCheck,
    require_current_roles: bool,
) -> Result<(), WorkflowStoreError> {
    let (snapshot, members) =
        crate::load_verified_snapshot_with_members(connection, check.training_snapshot_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| WorkflowStoreError("training snapshot not found".into()))?;
    let bundle = if require_current_roles {
        crate::benchmark_bundle::load_executable_benchmark_bundle(
            connection,
            check.benchmark_bundle_id,
        )
        .await?
    } else {
        crate::benchmark_bundle::load_benchmark_bundle(connection, check.benchmark_bundle_id)
            .await?
    }
    .ok_or_else(|| WorkflowStoreError("benchmark bundle not found".into()))?;
    let benchmark_report =
        crate::contamination::load_contamination_report(connection, bundle.contamination_report_id)
            .await?
            .ok_or_else(|| WorkflowStoreError("benchmark contamination report not found".into()))?;
    let report =
        crate::contamination::load_contamination_report(connection, check.contamination_report_id)
            .await?
            .ok_or_else(|| WorkflowStoreError("training contamination report not found".into()))?;

    let mut evidence = Vec::with_capacity(check.training_cohorts.len());
    for binding in &check.training_cohorts {
        let cohort = crate::governance::load_cohort(connection, binding.cohort_id)
            .await?
            .ok_or_else(|| WorkflowStoreError("training cohort not found".into()))?;
        let role = crate::governance::load_role_decision(
            connection,
            binding.role_decision_id,
            binding.cohort_id,
        )
        .await?
        .ok_or_else(|| WorkflowStoreError("training role decision not found".into()))?;
        if require_current_roles {
            let current_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM workflow_cohort_role_decisions \
                 WHERE cohort_id = ? ORDER BY sequence DESC LIMIT 1",
            )
            .bind(binding.cohort_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(store_error)?;
            if current_id != Some(role.id)
                || role.role != CohortRole::Training
                || role.disposition != CohortDisposition::Active
            {
                return Err(WorkflowStoreError(
                    "training cohort role is no longer current and active".into(),
                ));
            }
        }
        evidence.push(TrainingCohortEvidence { cohort, role });
    }
    let expected = build_training_benchmark_check(
        &snapshot,
        &members,
        check.training_input_protocol,
        evidence,
        &bundle,
        &benchmark_report,
        &report,
    )
    .map_err(store_error)?;
    if expected.fingerprint != check.fingerprint {
        return Err(WorkflowStoreError(
            "training-benchmark check differs from persisted authority".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct CheckRow {
    id: Uuid,
    training_snapshot_id: Uuid,
    benchmark_bundle_id: Uuid,
    contamination_report_id: Uuid,
    check_protocol_version: String,
    training_input_protocol: String,
    status: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl CheckRow {
    fn into_domain(self) -> Result<TrainingBenchmarkCheck, WorkflowStoreError> {
        let value: TrainingBenchmarkCheck = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.training_snapshot_id != self.training_snapshot_id
            || value.benchmark_bundle_id != self.benchmark_bundle_id
            || value.contamination_report_id != self.contamination_report_id
            || value.check_protocol_version != self.check_protocol_version
            || enum_string(&value.training_input_protocol)? != self.training_input_protocol
            || enum_string(&value.status)? != self.status
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "training-benchmark check normalized fields differ from artifact".into(),
            ));
        }
        value.validate_integrity().map_err(store_error)?;
        Ok(value)
    }
}

#[derive(Debug, FromRow)]
struct CohortIndexRow {
    cohort_id: Uuid,
    role_decision_id: Uuid,
    split: String,
    member_count: i64,
}

fn enum_string(value: &impl serde::Serialize) -> Result<String, WorkflowStoreError> {
    serde_json::to_value(value)
        .map_err(store_error)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| WorkflowStoreError("enum did not serialize as a string".into()))
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, WorkflowStoreError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(store_error)
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
