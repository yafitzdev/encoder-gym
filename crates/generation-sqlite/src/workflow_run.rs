use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use uuid::Uuid;
use workflow_core::{
    benchmark_bundle::BenchmarkBundle,
    contamination::ContaminationReport,
    execution::WorkflowChildExecution,
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

    fn create_workflow_definition_with_authority(
        &self,
        new_contamination_report: Option<&ContaminationReport>,
        new_benchmark_bundle: Option<&BenchmarkBundle>,
        definition: &WorkflowDefinition,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let report = new_contamination_report.cloned();
        let bundle = new_benchmark_bundle.cloned();
        let definition = definition.clone();
        Box::pin(async move {
            let binding = definition.benchmark_bundle.as_ref().ok_or_else(|| {
                WorkflowStoreError("workflow definition requires a benchmark bundle binding".into())
            })?;
            if let Some(bundle) = &bundle {
                binding.validate_bundle(bundle).map_err(store_error)?;
            }
            if let Some(report) = &report {
                let bundle = bundle.as_ref().ok_or_else(|| {
                    WorkflowStoreError(
                        "a new contamination report requires its new benchmark bundle".into(),
                    )
                })?;
                if report.id != bundle.contamination_report_id
                    || report.fingerprint != bundle.contamination_report_fingerprint
                {
                    return Err(WorkflowStoreError(
                        "new contamination report does not belong to the workflow benchmark bundle"
                            .into(),
                    ));
                }
            }
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            if let Some(report) = &report {
                crate::contamination::insert_contamination_report(&mut transaction, report).await?;
            }
            if let Some(bundle) = &bundle {
                crate::benchmark_bundle::insert_benchmark_bundle(&mut transaction, bundle).await?;
            }
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
            let mut connection = self.pool().acquire().await.map_err(store_error)?;
            load_workflow_definition(&mut connection, id).await
        })
    }

    fn query_workflow_definitions(
        &self,
        query: WorkflowDefinitionQuery,
    ) -> BoxFuture<'_, Result<Vec<WorkflowDefinition>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new("SELECT id FROM workflow_definitions");
            if let Some(dataset_id) = query.dataset_id {
                builder.push(" WHERE dataset_id = ").push_bind(dataset_id);
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
            let mut definitions = Vec::with_capacity(ids.len());
            for id in ids {
                let definition = load_workflow_definition(&mut connection, id)
                    .await?
                    .ok_or_else(|| {
                        WorkflowStoreError(format!(
                            "workflow definition disappeared while listing: {id}"
                        ))
                    })?;
                definitions.push(definition);
            }
            Ok(definitions)
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
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let definition = load_workflow_definition(&mut transaction, run.definition_id)
                .await?
                .ok_or_else(|| WorkflowStoreError("workflow run definition not found".into()))?;
            if definition.fingerprint != run.definition_fingerprint {
                return Err(WorkflowStoreError(
                    "workflow run definition does not match persistence".into(),
                ));
            }
            let binding = definition.benchmark_bundle.as_ref().ok_or_else(|| {
                WorkflowStoreError(
                    "legacy workflow definitions without benchmark authority are not executable"
                        .into(),
                )
            })?;
            let executable_bundle = crate::benchmark_bundle::load_executable_benchmark_bundle(
                &mut transaction,
                binding.bundle_id,
            )
            .await?
            .ok_or_else(|| WorkflowStoreError("workflow benchmark bundle not found".into()))?;
            binding
                .validate_bundle(&executable_bundle)
                .map_err(store_error)?;
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

    fn create_workflow_child_execution(
        &self,
        execution: &WorkflowChildExecution,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let execution = execution.clone();
        Box::pin(async move {
            execution.validate_integrity().map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            let attempt = sqlx::query_as::<_, AttemptRow>(
                "SELECT id, workflow_run_id, sequence, iteration, stage, attempt, state, \
                 predecessor_id, predecessor_fingerprint, retryable, artifact_json, fingerprint, \
                 started_at, finished_at FROM workflow_stage_attempts WHERE id = ?",
            )
            .bind(execution.workflow_stage_attempt_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(store_error)?
            .ok_or_else(|| WorkflowStoreError("workflow child parent attempt not found".into()))?
            .into_domain()?;
            execution
                .validate_for_attempt(&attempt)
                .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO workflow_child_executions \
                 (id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
                  logical_key, child_execution_id, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(execution.id)
            .bind(execution.workflow_run_id)
            .bind(execution.workflow_stage_attempt_id)
            .bind(enum_string(&execution.stage)?)
            .bind(execution.ordinal)
            .bind(enum_string(&execution.child_kind)?)
            .bind(&execution.logical_key)
            .bind(execution.child_execution_id)
            .bind(to_json(&execution)?)
            .bind(&execution.fingerprint)
            .bind(execution.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_workflow_child_executions(
        &self,
        attempt_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<WorkflowChildExecution>, WorkflowStoreError>> {
        Box::pin(async move {
            let attempt = sqlx::query_as::<_, AttemptRow>(
                "SELECT id, workflow_run_id, sequence, iteration, stage, attempt, state, \
                 predecessor_id, predecessor_fingerprint, retryable, artifact_json, fingerprint, \
                 started_at, finished_at FROM workflow_stage_attempts WHERE id = ?",
            )
            .bind(attempt_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .ok_or_else(|| WorkflowStoreError("workflow child parent attempt not found".into()))?
            .into_domain()?;
            let rows = sqlx::query_as::<_, ChildExecutionRow>(
                "SELECT id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
                 logical_key, child_execution_id, artifact_json, fingerprint, created_at \
                 FROM workflow_child_executions WHERE workflow_stage_attempt_id = ? \
                 ORDER BY ordinal ASC",
            )
            .bind(attempt_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            rows.into_iter()
                .map(ChildExecutionRow::into_domain)
                .map(|result| {
                    let execution = result?;
                    execution
                        .validate_for_attempt(&attempt)
                        .map_err(store_error)?;
                    Ok(execution)
                })
                .collect()
        })
    }

    fn get_workflow_child_execution(
        &self,
        child_kind: workflow_core::execution::WorkflowChildKind,
        child_execution_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<WorkflowChildExecution>, WorkflowStoreError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, ChildExecutionRow>(
                "SELECT id, workflow_run_id, workflow_stage_attempt_id, stage, ordinal, child_kind, \
                 logical_key, child_execution_id, artifact_json, fingerprint, created_at \
                 FROM workflow_child_executions WHERE child_kind = ? AND child_execution_id = ? \
                 ORDER BY created_at, id LIMIT 1",
            )
            .bind(enum_string(&child_kind)?)
            .bind(child_execution_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(execution) = row.map(ChildExecutionRow::into_domain).transpose()? else {
                return Ok(None);
            };
            let attempt = sqlx::query_as::<_, AttemptRow>(
                "SELECT id, workflow_run_id, sequence, iteration, stage, attempt, state, \
                 predecessor_id, predecessor_fingerprint, retryable, artifact_json, fingerprint, \
                 started_at, finished_at FROM workflow_stage_attempts WHERE id = ?",
            )
            .bind(execution.workflow_stage_attempt_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .ok_or_else(|| WorkflowStoreError("workflow child parent attempt not found".into()))?
            .into_domain()?;
            execution
                .validate_for_attempt(&attempt)
                .map_err(store_error)?;
            Ok(Some(execution))
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

pub(crate) async fn load_workflow_definition(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<WorkflowDefinition>, WorkflowStoreError> {
    let Some(row) = sqlx::query_as::<_, DefinitionRow>(
        "SELECT id, name, dataset_id, project_configuration_id, development_suite_id, \
         sealed_suite_id, benchmark_bundle_id, benchmark_qualification_id, \
         benchmark_qualification_review_id, artifact_json, fingerprint, created_at \
         FROM workflow_definitions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let definition = row.into_domain()?;
    if let Some(binding) = &definition.benchmark_bundle {
        let bundle = crate::benchmark_bundle::load_benchmark_bundle(connection, binding.bundle_id)
            .await?
            .ok_or_else(|| {
                WorkflowStoreError(format!(
                    "workflow benchmark bundle not found: {}",
                    binding.bundle_id
                ))
            })?;
        binding.validate_bundle(&bundle).map_err(store_error)?;
    }
    validate_persisted_qualification_binding(connection, &definition, false).await?;
    Ok(Some(definition))
}

async fn validate_persisted_qualification_binding(
    connection: &mut SqliteConnection,
    definition: &WorkflowDefinition,
    require_current_roles: bool,
) -> Result<(), WorkflowStoreError> {
    let Some(binding) = definition.benchmark_qualification.as_ref() else {
        return Ok(());
    };
    let qualification = crate::benchmark_qualification::load_qualification(
        connection,
        binding.qualification_id,
        require_current_roles,
    )
    .await?
    .ok_or_else(|| WorkflowStoreError("workflow benchmark qualification not found".into()))?;
    let review =
        crate::benchmark_qualification::load_review_by(connection, "id", binding.review_id)
            .await?
            .ok_or_else(|| {
                WorkflowStoreError("workflow benchmark qualification review not found".into())
            })?;
    binding
        .validate(&qualification, &review)
        .map_err(store_error)
}

async fn validate_definition_references(
    connection: &mut SqliteConnection,
    definition: &WorkflowDefinition,
) -> Result<(), WorkflowStoreError> {
    let binding = definition.benchmark_bundle.as_ref().ok_or_else(|| {
        WorkflowStoreError("new workflow definitions require a benchmark bundle".into())
    })?;
    let persisted_bundle =
        crate::benchmark_bundle::load_benchmark_bundle(connection, binding.bundle_id)
            .await?
            .ok_or_else(|| WorkflowStoreError("workflow benchmark bundle not found".into()))?;
    binding
        .validate_bundle(&persisted_bundle)
        .map_err(store_error)?;
    validate_persisted_qualification_binding(connection, definition, true).await?;
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
          sealed_suite_id, benchmark_bundle_id, benchmark_qualification_id, \
          benchmark_qualification_review_id, artifact_json, fingerprint, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(definition.id)
    .bind(&definition.name)
    .bind(definition.dataset_id)
    .bind(definition.project_configuration_id)
    .bind(definition.development_suite_id)
    .bind(definition.sealed_suite_id)
    .bind(
        definition
            .benchmark_bundle
            .as_ref()
            .map(|binding| binding.bundle_id),
    )
    .bind(
        definition
            .benchmark_qualification
            .as_ref()
            .map(|binding| binding.qualification_id),
    )
    .bind(
        definition
            .benchmark_qualification
            .as_ref()
            .map(|binding| binding.review_id),
    )
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
    benchmark_bundle_id: Option<Uuid>,
    benchmark_qualification_id: Option<Uuid>,
    benchmark_qualification_review_id: Option<Uuid>,
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
            || value
                .benchmark_bundle
                .as_ref()
                .map(|binding| binding.bundle_id)
                != self.benchmark_bundle_id
            || value
                .benchmark_qualification
                .as_ref()
                .map(|binding| binding.qualification_id)
                != self.benchmark_qualification_id
            || value
                .benchmark_qualification
                .as_ref()
                .map(|binding| binding.review_id)
                != self.benchmark_qualification_review_id
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

#[derive(Debug, FromRow)]
struct ChildExecutionRow {
    id: Uuid,
    workflow_run_id: Uuid,
    workflow_stage_attempt_id: Uuid,
    stage: String,
    ordinal: i64,
    child_kind: String,
    logical_key: String,
    child_execution_id: Uuid,
    artifact_json: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ChildExecutionRow {
    fn into_domain(self) -> Result<WorkflowChildExecution, WorkflowStoreError> {
        let value: WorkflowChildExecution = from_json(&self.artifact_json)?;
        if value.id != self.id
            || value.workflow_run_id != self.workflow_run_id
            || value.workflow_stage_attempt_id != self.workflow_stage_attempt_id
            || enum_string(&value.stage)? != self.stage
            || i64::from(value.ordinal) != self.ordinal
            || enum_string(&value.child_kind)? != self.child_kind
            || value.logical_key != self.logical_key
            || value.child_execution_id != self.child_execution_id
            || value.fingerprint != self.fingerprint
            || value.created_at != self.created_at
        {
            return Err(WorkflowStoreError(
                "workflow child execution normalized fields do not match artifact".into(),
            ));
        }
        value.validate_integrity().map_err(store_error)?;
        Ok(value)
    }
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
