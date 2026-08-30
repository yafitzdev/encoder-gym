use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    ports::{
        BoxFuture, WorkflowDefinitionQuery, WorkflowRunQuery, WorkflowRunStore, WorkflowStoreError,
    },
    workflow::{WorkflowDefinition, WorkflowRun, WorkflowStageAttempt},
};

use crate::SqliteStore;

impl WorkflowRunStore for SqliteStore {
    fn create_workflow_definition(
        &self,
        definition: &WorkflowDefinition,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let definition = definition.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_workflow_definition(&mut transaction, &definition).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_workflow_definition(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowDefinition>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, DefinitionRow>(
                "SELECT id, name, dataset_id, project_configuration_id, development_suite_id, \
                 sealed_suite_id, artifact_json, fingerprint, created_at \
                 FROM workflow_definitions WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(DefinitionRow::into_domain)
            .transpose()
        })
    }

    fn query_workflow_definitions(
        &self,
        query: WorkflowDefinitionQuery,
    ) -> BoxFuture<'_, Result<Vec<WorkflowDefinition>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, name, dataset_id, project_configuration_id, development_suite_id, \
                 sealed_suite_id, artifact_json, fingerprint, created_at FROM workflow_definitions",
            );
            if let Some(dataset_id) = query.dataset_id {
                builder.push(" WHERE dataset_id = ").push_bind(dataset_id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<DefinitionRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(DefinitionRow::into_domain)
                .collect()
        })
    }

    fn create_workflow_run(
        &self,
        run: &WorkflowRun,
        initial_attempt: &WorkflowStageAttempt,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let run = run.clone();
        let attempt = initial_attempt.clone();
        Box::pin(async move {
            validate_run_attempt(&run, &attempt)?;
            if attempt.sequence != 0
                || attempt.predecessor_id.is_some()
                || attempt.predecessor_fingerprint.is_some()
            {
                return Err(WorkflowStoreError(
                    "initial workflow attempt must be the first event".into(),
                ));
            }
            let definition_fingerprint: Option<String> =
                sqlx::query_scalar("SELECT fingerprint FROM workflow_definitions WHERE id = ?")
                    .bind(run.definition_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(store_error)?;
            if definition_fingerprint.as_deref() != Some(&run.definition_fingerprint) {
                return Err(WorkflowStoreError(
                    "workflow run definition does not match persistence".into(),
                ));
            }
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_run(&mut transaction, &run).await?;
            insert_attempt(&mut transaction, &attempt).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_workflow_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowRun>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, RunRow>(
                "SELECT id, definition_id, state, current_stage, iteration, latest_attempt_id, \
                 latest_attempt_fingerprint, cancel_requested, artifact_json, created_at, updated_at \
                 FROM workflow_runs WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(RunRow::into_domain)
            .transpose()
        })
    }

    fn query_workflow_runs(
        &self,
        query: WorkflowRunQuery,
    ) -> BoxFuture<'_, Result<Vec<WorkflowRun>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, definition_id, state, current_stage, iteration, latest_attempt_id, \
                 latest_attempt_fingerprint, cancel_requested, artifact_json, created_at, updated_at \
                 FROM workflow_runs",
            );
            let mut filtered = false;
            if let Some(definition_id) = query.definition_id {
                builder
                    .push(" WHERE definition_id = ")
                    .push_bind(definition_id);
                filtered = true;
            }
            if let Some(state) = query.state {
                builder
                    .push(if filtered { " AND " } else { " WHERE " })
                    .push("state = ")
                    .push_bind(enum_string(&state)?);
            }
            builder
                .push(" ORDER BY updated_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<RunRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(RunRow::into_domain)
                .collect()
        })
    }

    fn commit_workflow_attempt(
        &self,
        run: &WorkflowRun,
        attempt: &WorkflowStageAttempt,
        expected_previous_attempt_id: Uuid,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let run = run.clone();
        let attempt = attempt.clone();
        Box::pin(async move {
            validate_run_attempt(&run, &attempt)?;
            if attempt.predecessor_id != Some(expected_previous_attempt_id) {
                return Err(WorkflowStoreError(
                    "attempt predecessor does not match expected event".into(),
                ));
            }
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            insert_attempt(&mut transaction, &attempt).await?;
            let updated =
                update_run(&mut transaction, &run, Some(expected_previous_attempt_id)).await?;
            if updated != 1 {
                return Err(WorkflowStoreError(
                    "workflow run changed concurrently; attempt was not committed".into(),
                ));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_workflow_attempts(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowStageAttempt>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AttemptRow>(
                "SELECT id, workflow_run_id, sequence, iteration, stage, attempt, state, \
                 predecessor_id, predecessor_fingerprint, retryable, artifact_json, fingerprint, \
                 started_at, finished_at FROM workflow_stage_attempts \
                 WHERE workflow_run_id = ? ORDER BY sequence ASC",
            )
            .bind(run_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(AttemptRow::into_domain)
            .collect()
        })
    }

    fn save_workflow_run(
        &self,
        run: &WorkflowRun,
        expected_latest_attempt_id: Option<Uuid>,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let run = run.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let updated = update_run(&mut transaction, &run, expected_latest_attempt_id).await?;
            if updated != 1 {
                return Err(WorkflowStoreError(
                    "workflow run changed concurrently; update was rejected".into(),
                ));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }
}

async fn validate_definition_references(
    connection: &mut SqliteConnection,
    definition: &WorkflowDefinition,
) -> Result<(), WorkflowStoreError> {
    let configuration: Option<(Uuid, String)> =
        sqlx::query_as("SELECT dataset_id, fingerprint FROM project_configurations WHERE id = ?")
            .bind(definition.project_configuration_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(store_error)?;
    if configuration.as_ref()
        != Some(&(
            definition.dataset_id,
            definition.project_configuration_fingerprint.clone(),
        ))
    {
        return Err(WorkflowStoreError(
            "workflow project configuration does not match persistence".into(),
        ));
    }
    let development: Option<(String, String)> =
        sqlx::query_as("SELECT kind, fingerprint FROM workflow_benchmark_suites WHERE id = ?")
            .bind(definition.development_suite_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(store_error)?;
    if development.as_ref()
        != Some(&(
            "development".into(),
            definition.development_suite_fingerprint.clone(),
        ))
    {
        return Err(WorkflowStoreError(
            "workflow development suite does not match persistence".into(),
        ));
    }
    if let (Some(id), Some(fingerprint)) = (
        definition.sealed_suite_id,
        definition.sealed_suite_fingerprint.as_ref(),
    ) {
        let sealed: Option<(String, String)> =
            sqlx::query_as("SELECT kind, fingerprint FROM workflow_benchmark_suites WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *connection)
                .await
                .map_err(store_error)?;
        if sealed.as_ref() != Some(&("sealed_acceptance".into(), fingerprint.clone())) {
            return Err(WorkflowStoreError(
                "workflow sealed suite does not match persistence".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn insert_workflow_definition(
    connection: &mut SqliteConnection,
    definition: &WorkflowDefinition,
) -> Result<(), WorkflowStoreError> {
    validate_definition(definition)?;
    validate_definition_references(connection, definition).await?;
    sqlx::query(
        "INSERT INTO workflow_definitions \
         (id, name, dataset_id, project_configuration_id, development_suite_id, \
          sealed_suite_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(definition.id)
    .bind(&definition.name)
    .bind(definition.dataset_id)
    .bind(definition.project_configuration_id)
    .bind(definition.development_suite_id)
    .bind(definition.sealed_suite_id)
    .bind(to_json(definition)?)
    .bind(&definition.fingerprint)
    .bind(definition.created_at)
    .execute(connection)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_run(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    run: &WorkflowRun,
) -> Result<(), WorkflowStoreError> {
    sqlx::query(
        "INSERT INTO workflow_runs \
         (id, definition_id, state, current_stage, iteration, latest_attempt_id, \
          latest_attempt_fingerprint, cancel_requested, artifact_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(run.id)
    .bind(run.definition_id)
    .bind(enum_string(&run.state)?)
    .bind(
        run.current_stage
            .map(|value| enum_string(&value))
            .transpose()?,
    )
    .bind(run.iteration)
    .bind(run.latest_attempt_id)
    .bind(&run.latest_attempt_fingerprint)
    .bind(run.cancel_requested)
    .bind(to_json(run)?)
    .bind(run.created_at)
    .bind(run.updated_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn update_run(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    run: &WorkflowRun,
    expected_latest_attempt_id: Option<Uuid>,
) -> Result<u64, WorkflowStoreError> {
    let result = sqlx::query(
        "UPDATE workflow_runs SET state = ?, current_stage = ?, iteration = ?, \
         latest_attempt_id = ?, latest_attempt_fingerprint = ?, cancel_requested = ?, \
         artifact_json = ?, updated_at = ? WHERE id = ? AND latest_attempt_id IS ?",
    )
    .bind(enum_string(&run.state)?)
    .bind(
        run.current_stage
            .map(|value| enum_string(&value))
            .transpose()?,
    )
    .bind(run.iteration)
    .bind(run.latest_attempt_id)
    .bind(&run.latest_attempt_fingerprint)
    .bind(run.cancel_requested)
    .bind(to_json(run)?)
    .bind(run.updated_at)
    .bind(run.id)
    .bind(expected_latest_attempt_id)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(result.rows_affected())
}

async fn insert_attempt(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    attempt: &WorkflowStageAttempt,
) -> Result<(), WorkflowStoreError> {
    sqlx::query(
        "INSERT INTO workflow_stage_attempts \
         (id, workflow_run_id, sequence, iteration, stage, attempt, state, predecessor_id, \
          predecessor_fingerprint, retryable, artifact_json, fingerprint, started_at, finished_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.id)
    .bind(attempt.workflow_run_id)
    .bind(attempt.sequence)
    .bind(attempt.iteration)
    .bind(enum_string(&attempt.stage)?)
    .bind(attempt.attempt)
    .bind(enum_string(&attempt.state)?)
    .bind(attempt.predecessor_id)
    .bind(&attempt.predecessor_fingerprint)
    .bind(attempt.retryable)
    .bind(to_json(attempt)?)
    .bind(&attempt.fingerprint)
    .bind(attempt.started_at)
    .bind(attempt.finished_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    for artifact in &attempt.artifacts {
        sqlx::query(
            "INSERT INTO workflow_artifact_links \
             (workflow_stage_attempt_id, artifact_kind, artifact_id, artifact_fingerprint) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(attempt.id)
        .bind(&artifact.kind)
        .bind(artifact.artifact_id)
        .bind(&artifact.artifact_fingerprint)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct DefinitionRow {
    id: Uuid,
    name: String,
    dataset_id: Uuid,
    project_configuration_id: Uuid,
    development_suite_id: Uuid,
    sealed_suite_id: Option<Uuid>,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl DefinitionRow {
    fn into_domain(self) -> Result<WorkflowDefinition, WorkflowStoreError> {
        let value: WorkflowDefinition = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.name != self.name
            || value.dataset_id != self.dataset_id
            || value.project_configuration_id != self.project_configuration_id
            || value.development_suite_id != self.development_suite_id
            || value.sealed_suite_id != self.sealed_suite_id
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "workflow definition normalized fields do not match artifact".into(),
            ));
        }
        validate_definition(&value)?;
        Ok(value)
    }
}

#[derive(Debug, FromRow)]
struct RunRow {
    id: Uuid,
    definition_id: Uuid,
    state: String,
    current_stage: Option<String>,
    iteration: i64,
    latest_attempt_id: Option<Uuid>,
    latest_attempt_fingerprint: Option<String>,
    cancel_requested: bool,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl RunRow {
    fn into_domain(self) -> Result<WorkflowRun, WorkflowStoreError> {
        let value: WorkflowRun = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.definition_id != self.definition_id
            || enum_string(&value.state)? != self.state
            || value
                .current_stage
                .map(|item| enum_string(&item))
                .transpose()?
                != self.current_stage
            || i64::from(value.iteration) != self.iteration
            || value.latest_attempt_id != self.latest_attempt_id
            || value.latest_attempt_fingerprint != self.latest_attempt_fingerprint
            || value.cancel_requested != self.cancel_requested
            || value.created_at != self.created_at
            || value.updated_at != self.updated_at
        {
            return Err(WorkflowStoreError(
                "workflow run normalized fields do not match artifact".into(),
            ));
        }
        Ok(value)
    }
}

#[derive(Debug, FromRow)]
struct AttemptRow {
    id: Uuid,
    workflow_run_id: Uuid,
    sequence: i64,
    iteration: i64,
    stage: String,
    attempt: i64,
    state: String,
    predecessor_id: Option<Uuid>,
    predecessor_fingerprint: Option<String>,
    retryable: bool,
    artifact_json: String,
    fingerprint: String,
    started_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AttemptRow {
    fn into_domain(self) -> Result<WorkflowStageAttempt, WorkflowStoreError> {
        let value: WorkflowStageAttempt = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.workflow_run_id != self.workflow_run_id
            || i64::from(value.sequence) != self.sequence
            || i64::from(value.iteration) != self.iteration
            || enum_string(&value.stage)? != self.stage
            || i64::from(value.attempt) != self.attempt
            || enum_string(&value.state)? != self.state
            || value.predecessor_id != self.predecessor_id
            || value.predecessor_fingerprint != self.predecessor_fingerprint
            || value.retryable != self.retryable
            || value.fingerprint != self.fingerprint
            || value.started_at != self.started_at
            || value.finished_at != self.finished_at
        {
            return Err(WorkflowStoreError(
                "workflow attempt normalized fields do not match artifact".into(),
            ));
        }
        if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
            return Err(WorkflowStoreError(
                "workflow attempt fingerprint mismatch".into(),
            ));
        }
        Ok(value)
    }
}

fn validate_definition(value: &WorkflowDefinition) -> Result<(), WorkflowStoreError> {
    if value.reproduce_fingerprint().map_err(store_error)? != value.fingerprint {
        return Err(WorkflowStoreError(
            "workflow definition fingerprint mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_run_attempt(
    run: &WorkflowRun,
    attempt: &WorkflowStageAttempt,
) -> Result<(), WorkflowStoreError> {
    if attempt.workflow_run_id != run.id
        || run.latest_attempt_id != Some(attempt.id)
        || run.latest_attempt_fingerprint.as_deref() != Some(&attempt.fingerprint)
        || attempt.reproduce_fingerprint().map_err(store_error)? != attempt.fingerprint
    {
        return Err(WorkflowStoreError(
            "workflow run and stage attempt are inconsistent".into(),
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
