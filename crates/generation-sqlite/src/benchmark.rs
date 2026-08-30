use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    benchmark::{AcceptanceAssessment, BenchmarkSuite},
    ports::{
        AcceptanceAssessmentQuery, BenchmarkStore, BenchmarkSuiteQuery, BoxFuture,
        WorkflowStoreError,
    },
};

use crate::SqliteStore;

impl BenchmarkStore for SqliteStore {
    fn create_benchmark_suite(
        &self,
        suite: &BenchmarkSuite,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let suite = suite.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_benchmark_suite(&mut transaction, &suite).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_benchmark_suite(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkSuite>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, SuiteRow>(
                "SELECT id, name, kind, contamination_report_id, \
                 contamination_override_fingerprint, artifact_json, fingerprint, created_at \
                 FROM workflow_benchmark_suites WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(SuiteRow::into_domain)
            .transpose()
        })
    }

    fn query_benchmark_suites(
        &self,
        query: BenchmarkSuiteQuery,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkSuite>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT DISTINCT s.id, s.name, s.kind, s.contamination_report_id, \
                 s.contamination_override_fingerprint, s.artifact_json, s.fingerprint, \
                 s.created_at FROM workflow_benchmark_suites s",
            );
            if query.cohort_id.is_some() {
                builder.push(" JOIN workflow_benchmark_suite_cohorts c ON c.suite_id = s.id");
            }
            let mut has_filter = false;
            if let Some(kind) = query.kind {
                builder
                    .push(" WHERE s.kind = ")
                    .push_bind(enum_string(&kind)?);
                has_filter = true;
            }
            if let Some(cohort_id) = query.cohort_id {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("c.cohort_id = ")
                    .push_bind(cohort_id);
            }
            builder
                .push(" ORDER BY s.created_at DESC, s.id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<SuiteRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(SuiteRow::into_domain)
                .collect()
        })
    }

    fn create_acceptance_assessment(
        &self,
        assessment: &AcceptanceAssessment,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let assessment = assessment.clone();
        Box::pin(async move {
            validate_assessment(&assessment)?;
            let suite: Option<String> = sqlx::query_scalar(
                "SELECT fingerprint FROM workflow_benchmark_suites WHERE id = ?",
            )
            .bind(assessment.suite_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            if suite.as_deref() != Some(&assessment.suite_fingerprint) {
                return Err(WorkflowStoreError(
                    "assessment suite does not match persistence".into(),
                ));
            }
            for run_id in assessment.evaluation_run_ids.values() {
                let exists: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM evaluation_runs WHERE id = ?")
                        .bind(run_id)
                        .fetch_one(self.pool())
                        .await
                        .map_err(store_error)?;
                if exists != 1 {
                    return Err(WorkflowStoreError(format!(
                        "assessment evaluation run not found: {run_id}"
                    )));
                }
            }
            for comparison_id in assessment.comparison_ids.values() {
                let exists: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM evaluation_comparisons WHERE id = ?")
                        .bind(comparison_id)
                        .fetch_one(self.pool())
                        .await
                        .map_err(store_error)?;
                if exists != 1 {
                    return Err(WorkflowStoreError(format!(
                        "assessment comparison not found: {comparison_id}"
                    )));
                }
            }
            sqlx::query(
                "INSERT INTO workflow_acceptance_assessments \
                 (id, suite_id, checkpoint_id, state, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(assessment.id)
            .bind(assessment.suite_id)
            .bind(assessment.checkpoint_id)
            .bind(enum_string(&assessment.state)?)
            .bind(to_json(&assessment)?)
            .bind(&assessment.fingerprint)
            .bind(assessment.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_acceptance_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AcceptanceAssessment>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AssessmentRow>(
                "SELECT id, suite_id, checkpoint_id, state, artifact_json, fingerprint, \
                 created_at FROM workflow_acceptance_assessments WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(AssessmentRow::into_domain)
            .transpose()
        })
    }

    fn query_acceptance_assessments(
        &self,
        query: AcceptanceAssessmentQuery,
    ) -> BoxFuture<'_, Result<Vec<AcceptanceAssessment>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, suite_id, checkpoint_id, state, artifact_json, fingerprint, \
                 created_at FROM workflow_acceptance_assessments",
            );
            let mut has_filter = false;
            if let Some(suite_id) = query.suite_id {
                builder.push(" WHERE suite_id = ").push_bind(suite_id);
                has_filter = true;
            }
            if let Some(checkpoint_id) = query.checkpoint_id {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("checkpoint_id = ")
                    .push_bind(checkpoint_id);
                has_filter = true;
            }
            if let Some(state) = query.state {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("state = ")
                    .push_bind(enum_string(&state)?);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<AssessmentRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(AssessmentRow::into_domain)
                .collect()
        })
    }
}

pub(crate) async fn insert_benchmark_suite(
    connection: &mut SqliteConnection,
    suite: &BenchmarkSuite,
) -> Result<(), WorkflowStoreError> {
    validate_suite(suite)?;
    let report_fingerprint: Option<String> =
        sqlx::query_scalar("SELECT fingerprint FROM workflow_contamination_reports WHERE id = ?")
            .bind(suite.contamination_report_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(store_error)?;
    if report_fingerprint.as_deref() != Some(&suite.contamination_report_fingerprint) {
        return Err(WorkflowStoreError(
            "benchmark contamination report does not match persistence".into(),
        ));
    }
    if let Some(fingerprint) = &suite.contamination_override_fingerprint {
        let persisted: Option<String> = sqlx::query_scalar(
            "SELECT fingerprint FROM workflow_contamination_overrides WHERE report_id = ?",
        )
        .bind(suite.contamination_report_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(store_error)?;
        if persisted.as_deref() != Some(fingerprint) {
            return Err(WorkflowStoreError(
                "benchmark contamination override does not match persistence".into(),
            ));
        }
    }
    for cohort in &suite.cohorts {
        let persisted_cohort: Option<String> =
            sqlx::query_scalar("SELECT fingerprint FROM workflow_evaluation_cohorts WHERE id = ?")
                .bind(cohort.cohort_id)
                .fetch_optional(&mut *connection)
                .await
                .map_err(store_error)?;
        let persisted_role: Option<String> = sqlx::query_scalar(
            "SELECT fingerprint FROM workflow_cohort_role_decisions WHERE id = ? AND cohort_id = ?",
        )
        .bind(cohort.role_decision_id)
        .bind(cohort.cohort_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(store_error)?;
        if persisted_cohort.as_deref() != Some(&cohort.cohort_fingerprint)
            || persisted_role.as_deref() != Some(&cohort.role_decision_fingerprint)
        {
            return Err(WorkflowStoreError(format!(
                "benchmark cohort or role does not match persistence: {}",
                cohort.cohort_id
            )));
        }
    }
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
    .execute(&mut *connection)
    .await
    .map_err(store_error)?;
    for cohort in &suite.cohorts {
        sqlx::query(
            "INSERT INTO workflow_benchmark_suite_cohorts \
             (suite_id, cohort_id, role_decision_id, protocol_fingerprint) VALUES (?, ?, ?, ?)",
        )
        .bind(suite.id)
        .bind(cohort.cohort_id)
        .bind(cohort.role_decision_id)
        .bind(&cohort.protocol_fingerprint)
        .execute(&mut *connection)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct SuiteRow {
    id: Uuid,
    name: String,
    kind: String,
    contamination_report_id: Uuid,
    contamination_override_fingerprint: Option<String>,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl SuiteRow {
    fn into_domain(self) -> Result<BenchmarkSuite, WorkflowStoreError> {
        let value: BenchmarkSuite = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.name != self.name
            || enum_string(&value.kind)? != self.kind
            || value.contamination_report_id != self.contamination_report_id
            || value.contamination_override_fingerprint != self.contamination_override_fingerprint
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "benchmark suite normalized fields do not match artifact".into(),
            ));
        }
        validate_suite(&value)?;
        Ok(value)
    }
}

#[derive(Debug, FromRow)]
struct AssessmentRow {
    id: Uuid,
    suite_id: Uuid,
    checkpoint_id: Option<Uuid>,
    state: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl AssessmentRow {
    fn into_domain(self) -> Result<AcceptanceAssessment, WorkflowStoreError> {
        let value: AcceptanceAssessment = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.suite_id != self.suite_id
            || value.checkpoint_id != self.checkpoint_id
            || enum_string(&value.state)? != self.state
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "acceptance assessment normalized fields do not match artifact".into(),
            ));
        }
        validate_assessment(&value)?;
        Ok(value)
    }
}

fn validate_suite(value: &BenchmarkSuite) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "benchmark suite fingerprint mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_assessment(value: &AcceptanceAssessment) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "acceptance assessment fingerprint mismatch".into(),
        ));
    }
    Ok(())
}

fn enum_string(value: &impl serde::Serialize) -> Result<String, WorkflowStoreError> {
    serde_json::to_value(value)
        .map_err(store_error)?
        .as_str()
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
