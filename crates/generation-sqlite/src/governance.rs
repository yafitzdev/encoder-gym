use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    governance::{
        CohortRoleDecision, EvaluationCohort, EvidenceExposure, validate_exposure_resolution,
    },
    ports::{BoxFuture, CohortQuery, ExposureQuery, GovernanceStore, WorkflowStoreError},
};

use crate::SqliteStore;

impl GovernanceStore for SqliteStore {
    fn create_cohort(
        &self,
        cohort: &EvaluationCohort,
        initial_role: &CohortRoleDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let cohort = cohort.clone();
        let initial_role = initial_role.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_cohort_with_initial_role(&mut transaction, &cohort, &initial_role).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_cohort(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationCohort>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CohortRow>(
                "SELECT id, snapshot_id, split, origin, name, fingerprint, artifact_json, \
                 created_at FROM workflow_evaluation_cohorts WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(CohortRow::into_domain)
            .transpose()
        })
    }

    fn query_cohorts(
        &self,
        query: CohortQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationCohort>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, snapshot_id, split, origin, name, fingerprint, artifact_json, \
                 created_at FROM workflow_evaluation_cohorts",
            );
            if let Some(snapshot_id) = query.snapshot_id {
                builder.push(" WHERE snapshot_id = ").push_bind(snapshot_id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<CohortRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(CohortRow::into_domain)
                .collect()
        })
    }

    fn get_current_cohort_role(
        &self,
        cohort_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CohortRoleDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            current_role(self.pool(), cohort_id)
                .await?
                .map(RoleRow::into_domain)
                .transpose()
        })
    }

    fn append_cohort_role(
        &self,
        decision: &CohortRoleDecision,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let decision = decision.clone();
        Box::pin(async move {
            validate_role(&decision)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let current = current_role(&mut *transaction, decision.cohort_id)
                .await?
                .ok_or_else(|| WorkflowStoreError("cohort has no current role".into()))?;
            let current_domain = current.clone().into_domain()?;
            validate_role_successor(&current_domain, &decision)?;
            insert_role(&mut transaction, &decision, current.sequence + 1).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_cohort_role_history(
        &self,
        cohort_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CohortRoleDecision>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, RoleRow>(
                "SELECT id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
                 artifact_json, created_at FROM workflow_cohort_role_decisions \
                 WHERE cohort_id = ? ORDER BY sequence ASC",
            )
            .bind(cohort_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(RoleRow::into_domain)
            .collect()
        })
    }

    fn append_exposure(
        &self,
        exposure: &EvidenceExposure,
        resolution: Option<&CohortRoleDecision>,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let exposure = exposure.clone();
        let resolution = resolution.cloned();
        Box::pin(async move {
            validate_exposure(&exposure)?;
            if let Some(resolution) = &resolution {
                validate_role(resolution)?;
            }
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let current = current_role(&mut *transaction, exposure.cohort_id)
                .await?
                .ok_or_else(|| WorkflowStoreError("cohort has no current role".into()))?;
            let current_domain = current.clone().into_domain()?;
            validate_exposure_resolution(&exposure, &current_domain, resolution.as_ref())
                .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO workflow_evidence_exposures \
                 (id, cohort_id, role_decision_id, evaluation_run_id, workflow_run_id, \
                  workflow_iteration, purpose, disclosure, adaptation_eligible, \
                  requires_retirement, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(exposure.id)
            .bind(exposure.cohort_id)
            .bind(exposure.role_decision_id)
            .bind(exposure.evaluation_run_id)
            .bind(exposure.workflow_run_id)
            .bind(exposure.workflow_iteration)
            .bind(enum_string(&exposure.purpose)?)
            .bind(enum_string(&exposure.disclosure)?)
            .bind(exposure.adaptation_eligible)
            .bind(exposure.requires_retirement)
            .bind(&exposure.fingerprint)
            .bind(to_json(&exposure)?)
            .bind(exposure.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if let Some(resolution) = &resolution {
                insert_role(&mut transaction, resolution, current.sequence + 1).await?;
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn query_exposures(
        &self,
        query: ExposureQuery,
    ) -> BoxFuture<'_, Result<Vec<EvidenceExposure>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, cohort_id, role_decision_id, evaluation_run_id, workflow_run_id, \
                 workflow_iteration, purpose, disclosure, adaptation_eligible, \
                 requires_retirement, fingerprint, artifact_json, created_at \
                 FROM workflow_evidence_exposures WHERE cohort_id = ",
            );
            builder.push_bind(query.cohort_id);
            if let Some(purpose) = query.purpose {
                builder
                    .push(" AND purpose = ")
                    .push_bind(enum_string(&purpose)?);
            }
            builder
                .push(" ORDER BY created_at ASC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<ExposureRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ExposureRow::into_domain)
                .collect()
        })
    }
}

/// Loads and fully validates one persisted cohort through the same normalized
/// column checks used by the public governance store, while retaining the
/// caller's transaction/connection boundary.
pub(crate) async fn load_cohort(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<EvaluationCohort>, WorkflowStoreError> {
    sqlx::query_as::<_, CohortRow>(
        "SELECT id, snapshot_id, split, origin, name, fingerprint, artifact_json, \
         created_at FROM workflow_evaluation_cohorts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    .map(CohortRow::into_domain)
    .transpose()
}

/// Loads and fully validates an exact historical role decision. This is
/// intentionally distinct from resolving the current role: immutable suites
/// pin a decision, while bundle creation separately requires that decision to
/// still be current.
pub(crate) async fn load_role_decision(
    connection: &mut SqliteConnection,
    id: Uuid,
    cohort_id: Uuid,
) -> Result<Option<CohortRoleDecision>, WorkflowStoreError> {
    sqlx::query_as::<_, RoleRow>(
        "SELECT id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
         artifact_json, created_at FROM workflow_cohort_role_decisions \
         WHERE id = ? AND cohort_id = ?",
    )
    .bind(id)
    .bind(cohort_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    .map(RoleRow::into_domain)
    .transpose()
}

pub(crate) async fn load_current_role(
    connection: &mut SqliteConnection,
    cohort_id: Uuid,
) -> Result<Option<CohortRoleDecision>, WorkflowStoreError> {
    current_role(&mut *connection, cohort_id)
        .await?
        .map(RoleRow::into_domain)
        .transpose()
}

async fn current_role<'e, E>(
    executor: E,
    cohort_id: Uuid,
) -> Result<Option<RoleRow>, WorkflowStoreError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, RoleRow>(
        "SELECT id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
         artifact_json, created_at FROM workflow_cohort_role_decisions \
         WHERE cohort_id = ? ORDER BY sequence DESC LIMIT 1",
    )
    .bind(cohort_id)
    .fetch_optional(executor)
    .await
    .map_err(store_error)
}

async fn insert_role(
    connection: &mut SqliteConnection,
    decision: &CohortRoleDecision,
    sequence: i64,
) -> Result<(), WorkflowStoreError> {
    sqlx::query(
        "INSERT INTO workflow_cohort_role_decisions \
         (id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
          artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(decision.id)
    .bind(decision.cohort_id)
    .bind(sequence)
    .bind(enum_string(&decision.role)?)
    .bind(enum_string(&decision.disposition)?)
    .bind(decision.predecessor_id)
    .bind(&decision.fingerprint)
    .bind(to_json(decision)?)
    .bind(decision.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

pub(crate) async fn insert_cohort_with_initial_role(
    connection: &mut SqliteConnection,
    cohort: &EvaluationCohort,
    initial_role: &CohortRoleDecision,
) -> Result<(), WorkflowStoreError> {
    validate_cohort(cohort)?;
    validate_role(initial_role)?;
    if initial_role.cohort_id != cohort.id
        || initial_role.predecessor_id.is_some()
        || initial_role.predecessor_fingerprint.is_some()
    {
        return Err(WorkflowStoreError(
            "initial role does not belong to the cohort or has a predecessor".into(),
        ));
    }
    let persisted_snapshot_fingerprint: Option<String> =
        sqlx::query_scalar("SELECT fingerprint FROM dataset_snapshots WHERE id = ?")
            .bind(cohort.snapshot_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(store_error)?;
    if persisted_snapshot_fingerprint.as_deref() != Some(&cohort.snapshot_fingerprint) {
        return Err(WorkflowStoreError(
            "cohort snapshot fingerprint does not match persistence".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO workflow_evaluation_cohorts \
         (id, snapshot_id, split, origin, name, fingerprint, artifact_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(cohort.id)
    .bind(cohort.snapshot_id)
    .bind(enum_string(&cohort.split)?)
    .bind(enum_string(&cohort.origin)?)
    .bind(&cohort.name)
    .bind(&cohort.fingerprint)
    .bind(to_json(cohort)?)
    .bind(cohort.created_at)
    .execute(&mut *connection)
    .await
    .map_err(store_error)?;
    insert_role(connection, initial_role, 0).await
}

fn validate_cohort(cohort: &EvaluationCohort) -> Result<(), WorkflowStoreError> {
    if cohort.reproduce_fingerprint().map_err(store_error)? != cohort.fingerprint {
        Err(WorkflowStoreError(
            "cohort fingerprint does not reproduce".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_role(decision: &CohortRoleDecision) -> Result<(), WorkflowStoreError> {
    if decision.reproduce_fingerprint().map_err(store_error)? != decision.fingerprint {
        Err(WorkflowStoreError(
            "cohort role fingerprint does not reproduce".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_exposure(exposure: &EvidenceExposure) -> Result<(), WorkflowStoreError> {
    if exposure.reproduce_fingerprint().map_err(store_error)? != exposure.fingerprint {
        Err(WorkflowStoreError(
            "evidence exposure fingerprint does not reproduce".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_role_successor(
    current: &CohortRoleDecision,
    successor: &CohortRoleDecision,
) -> Result<(), WorkflowStoreError> {
    if successor.cohort_id != current.cohort_id
        || successor.predecessor_id != Some(current.id)
        || successor.predecessor_fingerprint.as_deref() != Some(&current.fingerprint)
    {
        Err(WorkflowStoreError(
            "role decision does not continue the current append-only chain".into(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, FromRow)]
struct CohortRow {
    id: Uuid,
    snapshot_id: Uuid,
    split: String,
    origin: String,
    name: String,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl CohortRow {
    fn into_domain(self) -> Result<EvaluationCohort, WorkflowStoreError> {
        let cohort: EvaluationCohort = from_json(&self.artifact_json)?;
        if self.id != cohort.id
            || self.snapshot_id != cohort.snapshot_id
            || self.split != enum_string(&cohort.split)?
            || self.origin != enum_string(&cohort.origin)?
            || self.name != cohort.name
            || self.fingerprint != cohort.fingerprint
            || self.created_at != cohort.created_at
        {
            return Err(WorkflowStoreError(
                "normalized cohort columns differ from immutable artifact".into(),
            ));
        }
        validate_cohort(&cohort)?;
        Ok(cohort)
    }
}

#[derive(Clone, FromRow)]
struct RoleRow {
    id: Uuid,
    cohort_id: Uuid,
    sequence: i64,
    role: String,
    disposition: String,
    predecessor_id: Option<Uuid>,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl RoleRow {
    fn into_domain(self) -> Result<CohortRoleDecision, WorkflowStoreError> {
        let decision: CohortRoleDecision = from_json(&self.artifact_json)?;
        if self.id != decision.id
            || self.cohort_id != decision.cohort_id
            || self.role != enum_string(&decision.role)?
            || self.disposition != enum_string(&decision.disposition)?
            || self.predecessor_id != decision.predecessor_id
            || self.fingerprint != decision.fingerprint
            || self.created_at != decision.created_at
        {
            return Err(WorkflowStoreError(
                "normalized role columns differ from immutable artifact".into(),
            ));
        }
        validate_role(&decision)?;
        Ok(decision)
    }
}

#[derive(FromRow)]
struct ExposureRow {
    id: Uuid,
    cohort_id: Uuid,
    role_decision_id: Uuid,
    evaluation_run_id: Option<Uuid>,
    workflow_run_id: Option<Uuid>,
    workflow_iteration: Option<u32>,
    purpose: String,
    disclosure: String,
    adaptation_eligible: bool,
    requires_retirement: bool,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ExposureRow {
    fn into_domain(self) -> Result<EvidenceExposure, WorkflowStoreError> {
        let exposure: EvidenceExposure = from_json(&self.artifact_json)?;
        if self.id != exposure.id
            || self.cohort_id != exposure.cohort_id
            || self.role_decision_id != exposure.role_decision_id
            || self.evaluation_run_id != exposure.evaluation_run_id
            || self.workflow_run_id != exposure.workflow_run_id
            || self.workflow_iteration != exposure.workflow_iteration
            || self.purpose != enum_string(&exposure.purpose)?
            || self.disclosure != enum_string(&exposure.disclosure)?
            || self.adaptation_eligible != exposure.adaptation_eligible
            || self.requires_retirement != exposure.requires_retirement
            || self.fingerprint != exposure.fingerprint
            || self.created_at != exposure.created_at
        {
            return Err(WorkflowStoreError(
                "normalized exposure columns differ from immutable artifact".into(),
            ));
        }
        validate_exposure(&exposure)?;
        Ok(exposure)
    }
}

fn enum_string(value: &impl serde::Serialize) -> Result<String, WorkflowStoreError> {
    match serde_json::to_value(value).map_err(store_error)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(WorkflowStoreError(
            "enum did not serialize as a string".into(),
        )),
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
