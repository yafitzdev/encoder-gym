use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;
use workflow_core::{
    advisor::{AdvisorEgressPolicy, AdvisoryAssessment},
    ports::{AdvisorStore, AdvisoryAssessmentQuery, BoxFuture, WorkflowStoreError},
};

use crate::SqliteStore;

impl AdvisorStore for SqliteStore {
    fn create_advisory_assessment(
        &self,
        assessment: &AdvisoryAssessment,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let assessment = assessment.clone();
        Box::pin(async move {
            if assessment.reproduce_fingerprint().map_err(store_error)? != assessment.fingerprint {
                return Err(WorkflowStoreError(
                    "advisory assessment fingerprint does not reproduce".into(),
                ));
            }
            let input_tokens = sqlite_u64(assessment.usage.input_tokens)?;
            let output_tokens = sqlite_u64(assessment.usage.output_tokens)?;
            let total_tokens = sqlite_u64(assessment.usage.total_tokens)?;
            sqlx::query(
                "INSERT INTO advisory_assessments \
                 (id, workflow_run_id, workflow_iteration, analysis_report_id, \
                  acceptance_assessment_id, backend, model, egress_policy, prompt_version, \
                  prompt_fingerprint, input_tokens, output_tokens, total_tokens, fingerprint, \
                  artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(assessment.id)
            .bind(assessment.request.workflow_run_id)
            .bind(assessment.request.workflow_iteration)
            .bind(assessment.request.analysis_report_id)
            .bind(assessment.request.acceptance_assessment_id)
            .bind(&assessment.backend)
            .bind(&assessment.model)
            .bind(egress_policy(assessment.request.egress_policy))
            .bind(&assessment.prompt_version)
            .bind(&assessment.prompt_fingerprint)
            .bind(input_tokens)
            .bind(output_tokens)
            .bind(total_tokens)
            .bind(&assessment.fingerprint)
            .bind(serde_json::to_string(&assessment).map_err(store_error)?)
            .bind(assessment.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_advisory_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AdvisoryAssessment>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AdvisoryRow>(
                "SELECT artifact_json FROM advisory_assessments WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(AdvisoryRow::into_domain)
            .transpose()
        })
    }

    fn query_advisory_assessments(
        &self,
        query: AdvisoryAssessmentQuery,
    ) -> BoxFuture<'_, Result<Vec<AdvisoryAssessment>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT artifact_json FROM advisory_assessments");
            let mut has_filter = false;
            if let Some(id) = query.workflow_run_id {
                builder.push(" WHERE workflow_run_id = ").push_bind(id);
                has_filter = true;
            }
            if let Some(id) = query.analysis_report_id {
                builder
                    .push(if has_filter {
                        " AND analysis_report_id = "
                    } else {
                        " WHERE analysis_report_id = "
                    })
                    .push_bind(id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<AdvisoryRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(AdvisoryRow::into_domain)
                .collect()
        })
    }
}

#[derive(Debug, FromRow)]
struct AdvisoryRow {
    artifact_json: String,
}

impl AdvisoryRow {
    fn into_domain(self) -> Result<AdvisoryAssessment, WorkflowStoreError> {
        let value: AdvisoryAssessment =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
            return Err(WorkflowStoreError(
                "advisory assessment fingerprint does not reproduce".into(),
            ));
        }
        Ok(value)
    }
}

const fn egress_policy(value: AdvisorEgressPolicy) -> &'static str {
    match value {
        AdvisorEgressPolicy::AggregateOnly => "aggregate_only",
        AdvisorEgressPolicy::DevelopmentText => "development_text",
    }
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}

fn sqlite_u64(value: Option<u64>) -> Result<Option<i64>, WorkflowStoreError> {
    value
        .map(|value| {
            i64::try_from(value)
                .map_err(|_| WorkflowStoreError("token usage exceeds SQLite limits".into()))
        })
        .transpose()
}
