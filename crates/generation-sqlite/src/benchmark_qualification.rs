use std::collections::{BTreeMap, BTreeSet};

use dataset_core::domain::{DatasetSnapshot, SnapshotMember};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    benchmark::{BenchmarkSuite, BenchmarkSuiteKind},
    benchmark_qualification::{
        BenchmarkQualification, BenchmarkQualificationReview, BenchmarkQualificationReviewDecision,
        BenchmarkReadiness, QualificationPopulation, qualify_benchmark_bundle,
        validate_qualification_review,
    },
    ports::{
        BenchmarkQualificationQuery, BenchmarkQualificationStore, BoxFuture, WorkflowStoreError,
    },
};

use crate::SqliteStore;

impl BenchmarkQualificationStore for SqliteStore {
    fn create_benchmark_qualification(
        &self,
        qualification: &BenchmarkQualification,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let qualification = qualification.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            validate_against_persisted(&mut transaction, &qualification, true).await?;
            sqlx::query(
                "INSERT INTO workflow_benchmark_qualifications \
                 (id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, \
                  artifact_json, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(qualification.id)
            .bind(qualification.benchmark_bundle_id)
            .bind(&qualification.protocol)
            .bind(readiness_name(qualification.readiness))
            .bind(policy_fingerprint(&qualification)?)
            .bind(to_json(&qualification)?)
            .bind(&qualification.fingerprint)
            .bind(qualification.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_benchmark_qualification(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkQualification>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_qualification(&mut connection, id, false).await
        })
    }

    fn get_executable_benchmark_qualification(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkQualification>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_qualification(&mut connection, id, true).await
        })
    }

    fn query_benchmark_qualifications(
        &self,
        query: BenchmarkQualificationQuery,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkQualification>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder =
                QueryBuilder::<Sqlite>::new("SELECT id FROM workflow_benchmark_qualifications");
            let mut has_filter = false;
            if let Some(id) = query.benchmark_bundle_id {
                builder.push(" WHERE benchmark_bundle_id = ").push_bind(id);
                has_filter = true;
            }
            if let Some(readiness) = query.readiness {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("readiness = ")
                    .push_bind(readiness_name(readiness));
                has_filter = true;
            }
            if let Some(protocol) = query.protocol {
                builder
                    .push(if has_filter { " AND " } else { " WHERE " })
                    .push("protocol = ")
                    .push_bind(protocol);
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
            let mut qualifications = Vec::with_capacity(ids.len());
            for id in ids {
                qualifications.push(
                    load_qualification(&mut connection, id, false)
                        .await?
                        .ok_or_else(|| {
                            WorkflowStoreError(format!(
                                "benchmark qualification disappeared while listing: {id}"
                            ))
                        })?,
                );
            }
            Ok(qualifications)
        })
    }

    fn create_benchmark_qualification_review(
        &self,
        review: &BenchmarkQualificationReview,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let review = review.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let qualification = load_qualification(&mut transaction, review.qualification_id, true)
                .await?
                .ok_or_else(|| WorkflowStoreError("review qualification not found".into()))?;
            validate_qualification_review(&qualification, &review).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO workflow_benchmark_qualification_reviews \
                 (id, qualification_id, decision, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.qualification_id)
            .bind(review_decision_name(review.decision))
            .bind(to_json(&review)?)
            .bind(&review.fingerprint)
            .bind(review.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_benchmark_qualification_review(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkQualificationReview>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_review_by(&mut connection, "id", id).await
        })
    }

    fn get_benchmark_qualification_review_for_qualification(
        &self,
        qualification_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkQualificationReview>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_review_by(&mut connection, "qualification_id", qualification_id).await
        })
    }
}

async fn load_review_by(
    connection: &mut SqliteConnection,
    column: &'static str,
    id: Uuid,
) -> Result<Option<BenchmarkQualificationReview>, WorkflowStoreError> {
    let query = format!(
        "SELECT id, qualification_id, decision, artifact_json, fingerprint, created_at \
         FROM workflow_benchmark_qualification_reviews WHERE {column} = ?"
    );
    let Some(row) = sqlx::query_as::<_, ReviewRow>(&query)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let review = row.into_domain()?;
    let qualification = load_qualification(connection, review.qualification_id, false)
        .await?
        .ok_or_else(|| WorkflowStoreError("review qualification not found".into()))?;
    validate_qualification_review(&qualification, &review).map_err(store_error)?;
    Ok(Some(review))
}

async fn load_qualification(
    connection: &mut SqliteConnection,
    id: Uuid,
    require_current_roles: bool,
) -> Result<Option<BenchmarkQualification>, WorkflowStoreError> {
    let Some(row) = sqlx::query_as::<_, QualificationRow>(
        "SELECT id, benchmark_bundle_id, protocol, readiness, policy_fingerprint, \
         artifact_json, fingerprint, created_at FROM workflow_benchmark_qualifications \
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let qualification = row.into_domain()?;
    validate_against_persisted(connection, &qualification, require_current_roles).await?;
    Ok(Some(qualification))
}

async fn validate_against_persisted(
    connection: &mut SqliteConnection,
    qualification: &BenchmarkQualification,
    require_current_roles: bool,
) -> Result<(), WorkflowStoreError> {
    qualification.validate_integrity().map_err(store_error)?;
    let bundle = if require_current_roles {
        crate::benchmark_bundle::load_executable_benchmark_bundle(
            connection,
            qualification.benchmark_bundle_id,
        )
        .await?
    } else {
        crate::benchmark_bundle::load_benchmark_bundle(
            connection,
            qualification.benchmark_bundle_id,
        )
        .await?
    }
    .ok_or_else(|| WorkflowStoreError("qualification benchmark bundle not found".into()))?;
    let development = load_suite(
        connection,
        bundle.development_suite_id,
        require_current_roles,
    )
    .await?;
    if development.kind != BenchmarkSuiteKind::Development {
        return Err(WorkflowStoreError(
            "qualification development suite has the wrong kind".into(),
        ));
    }
    let sealed = match bundle.sealed_suite_id {
        Some(id) => Some(load_suite(connection, id, require_current_roles).await?),
        None => None,
    };
    let populations = load_populations(connection, &development, sealed.as_ref()).await?;
    let inputs = populations
        .iter()
        .map(|(cohort_id, members)| QualificationPopulation {
            cohort_id: *cohort_id,
            members,
        })
        .collect();
    let expected = qualify_benchmark_bundle(
        &bundle,
        &development,
        sealed.as_ref(),
        inputs,
        qualification.policy.clone(),
    )
    .map_err(store_error)?;
    if qualification.protocol != expected.protocol
        || qualification.benchmark_bundle_id != expected.benchmark_bundle_id
        || qualification.benchmark_bundle_fingerprint != expected.benchmark_bundle_fingerprint
        || qualification.development_suite_id != expected.development_suite_id
        || qualification.development_suite_fingerprint != expected.development_suite_fingerprint
        || qualification.sealed_suite_id != expected.sealed_suite_id
        || qualification.sealed_suite_fingerprint != expected.sealed_suite_fingerprint
        || qualification.policy != expected.policy
        || qualification.readiness != expected.readiness
        || qualification.cohorts != expected.cohorts
        || qualification.issues != expected.issues
        || qualification.fingerprint != expected.fingerprint
    {
        return Err(WorkflowStoreError(
            "benchmark qualification differs from recomputed persisted evidence".into(),
        ));
    }
    Ok(())
}

async fn load_populations(
    connection: &mut SqliteConnection,
    development: &BenchmarkSuite,
    sealed: Option<&BenchmarkSuite>,
) -> Result<BTreeMap<Uuid, Vec<SnapshotMember>>, WorkflowStoreError> {
    let mut snapshots = BTreeMap::<Uuid, (DatasetSnapshot, Vec<SnapshotMember>)>::new();
    let snapshot_ids = development
        .cohorts
        .iter()
        .chain(sealed.into_iter().flat_map(|suite| suite.cohorts.iter()))
        .map(|cohort| cohort.snapshot_id)
        .collect::<BTreeSet<_>>();
    for snapshot_id in snapshot_ids {
        let loaded = crate::load_verified_snapshot_with_members(connection, snapshot_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                WorkflowStoreError(format!("qualification snapshot not found: {snapshot_id}"))
            })?;
        snapshots.insert(snapshot_id, loaded);
    }
    let mut populations = BTreeMap::new();
    for cohort in development
        .cohorts
        .iter()
        .chain(sealed.into_iter().flat_map(|suite| suite.cohorts.iter()))
    {
        let (snapshot, members) = &snapshots[&cohort.snapshot_id];
        if snapshot.fingerprint != cohort.snapshot_fingerprint {
            return Err(WorkflowStoreError(format!(
                "qualification snapshot fingerprint differs for cohort {}",
                cohort.cohort_id
            )));
        }
        let selected = members
            .iter()
            .filter(|member| member.split == cohort.split)
            .cloned()
            .collect::<Vec<_>>();
        if populations.insert(cohort.cohort_id, selected).is_some() {
            return Err(WorkflowStoreError(
                "qualification cohort appears more than once".into(),
            ));
        }
    }
    Ok(populations)
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

#[derive(Debug, FromRow)]
struct QualificationRow {
    id: Uuid,
    benchmark_bundle_id: Uuid,
    protocol: String,
    readiness: String,
    policy_fingerprint: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ReviewRow {
    id: Uuid,
    qualification_id: Uuid,
    decision: String,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ReviewRow {
    fn into_domain(self) -> Result<BenchmarkQualificationReview, WorkflowStoreError> {
        let value: BenchmarkQualificationReview = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.qualification_id != self.qualification_id
            || review_decision_name(value.decision) != self.decision
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "benchmark qualification review normalized fields differ from immutable artifact"
                    .into(),
            ));
        }
        value.validate_integrity().map_err(store_error)?;
        Ok(value)
    }
}

impl QualificationRow {
    fn into_domain(self) -> Result<BenchmarkQualification, WorkflowStoreError> {
        let value: BenchmarkQualification = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.benchmark_bundle_id != self.benchmark_bundle_id
            || value.protocol != self.protocol
            || readiness_name(value.readiness) != self.readiness
            || artifact_core::fingerprint(&value.policy).map_err(store_error)?
                != self.policy_fingerprint
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "benchmark qualification normalized fields differ from immutable artifact".into(),
            ));
        }
        value.validate_integrity().map_err(store_error)?;
        Ok(value)
    }
}

fn policy_fingerprint(
    qualification: &BenchmarkQualification,
) -> Result<String, WorkflowStoreError> {
    artifact_core::fingerprint(&qualification.policy).map_err(store_error)
}

const fn readiness_name(value: BenchmarkReadiness) -> &'static str {
    match value {
        BenchmarkReadiness::Ready => "ready",
        BenchmarkReadiness::Blocked => "blocked",
    }
}

const fn review_decision_name(value: BenchmarkQualificationReviewDecision) -> &'static str {
    match value {
        BenchmarkQualificationReviewDecision::Approve => "approve",
        BenchmarkQualificationReviewDecision::Reject => "reject",
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
