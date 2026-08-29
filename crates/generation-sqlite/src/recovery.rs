use chrono::{DateTime, Utc};
use recovery_core::{
    BoxFuture, RecoveryRecord, RecoveryState, RecoveryStore, RecoveryStoreError, WorkflowKind,
};
use sqlx::{FromRow, Sqlite, Transaction};
use sysinfo::{Pid, System};
use uuid::Uuid;

use super::SqliteStore;

impl RecoveryStore for SqliteStore {
    fn acquire_execution_lease(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>> {
        Box::pin(async move {
            let process_id = std::process::id();
            let process_started_at = current_process_started_at(process_id)?;
            sqlx::query(
                "INSERT INTO workflow_execution_leases \
                 (workflow_kind, workflow_id, process_id, process_started_at, acquired_at) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT(workflow_kind, workflow_id) DO UPDATE SET \
                 process_id = excluded.process_id, process_started_at = excluded.process_started_at, \
                 acquired_at = excluded.acquired_at",
            )
            .bind(kind.as_str())
            .bind(workflow_id)
            .bind(i64::from(process_id))
            .bind(to_i64(process_started_at)?)
            .bind(Utc::now())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn release_execution_lease(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>> {
        Box::pin(async move {
            sqlx::query(
                "DELETE FROM workflow_execution_leases \
                 WHERE workflow_kind = ? AND workflow_id = ?",
            )
            .bind(kind.as_str())
            .bind(workflow_id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn detect_interrupted_workflows(
        &self,
    ) -> BoxFuture<'_, Result<Vec<RecoveryRecord>, RecoveryStoreError>> {
        Box::pin(async move {
            let system = System::new_all();
            let running = sqlx::query_as::<_, RunningWorkflowRecord>(
                "SELECT 'generation' AS workflow_kind, id AS workflow_id \
                 FROM generation_jobs WHERE state = 'running' \
                 UNION ALL SELECT 'training', id FROM training_runs WHERE state = 'running' \
                 UNION ALL SELECT 'evaluation', id FROM evaluation_runs WHERE state = 'running' \
                 ORDER BY workflow_kind, workflow_id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            let mut interrupted = Vec::new();
            for workflow in running {
                let kind = parse_kind(&workflow.workflow_kind)?;
                let lease = sqlx::query_as::<_, LeaseRecord>(
                    "SELECT process_id, process_started_at FROM workflow_execution_leases \
                     WHERE workflow_kind = ? AND workflow_id = ?",
                )
                .bind(kind.as_str())
                .bind(workflow.workflow_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(store_error)?;
                if lease.as_ref().is_some_and(|lease| lease.is_active(&system)) {
                    continue;
                }
                let now = Utc::now();
                let message = format!(
                    "{} execution was interrupted because its owning process is no longer active",
                    kind.as_str()
                );
                mark_failed(&mut transaction, kind, workflow.workflow_id, &message, now).await?;
                let resumable_in_place = kind == WorkflowKind::Generation;
                sqlx::query(
                    "INSERT INTO workflow_recovery_records \
                     (workflow_kind, workflow_id, state, resumable_in_place, message, detected_at, \
                      replacement_id, resolved_at) VALUES (?, ?, 'pending', ?, ?, ?, NULL, NULL) \
                     ON CONFLICT(workflow_kind, workflow_id) DO UPDATE SET state = 'pending', \
                     resumable_in_place = excluded.resumable_in_place, message = excluded.message, \
                     detected_at = excluded.detected_at, replacement_id = NULL, resolved_at = NULL",
                )
                .bind(kind.as_str())
                .bind(workflow.workflow_id)
                .bind(resumable_in_place)
                .bind(&message)
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                sqlx::query(
                    "DELETE FROM workflow_execution_leases \
                     WHERE workflow_kind = ? AND workflow_id = ?",
                )
                .bind(kind.as_str())
                .bind(workflow.workflow_id)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                interrupted.push(RecoveryRecord {
                    workflow_kind: kind,
                    workflow_id: workflow.workflow_id,
                    state: RecoveryState::Pending,
                    resumable_in_place,
                    message,
                    detected_at: now,
                    replacement_id: None,
                    resolved_at: None,
                });
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(interrupted)
        })
    }

    fn list_recovery_records(
        &self,
        include_resolved: bool,
    ) -> BoxFuture<'_, Result<Vec<RecoveryRecord>, RecoveryStoreError>> {
        Box::pin(async move {
            let query = if include_resolved {
                "SELECT workflow_kind, workflow_id, state, resumable_in_place, message, \
                 detected_at, replacement_id, resolved_at FROM workflow_recovery_records \
                 ORDER BY detected_at, workflow_kind, workflow_id"
            } else {
                "SELECT workflow_kind, workflow_id, state, resumable_in_place, message, \
                 detected_at, replacement_id, resolved_at FROM workflow_recovery_records \
                 WHERE state = 'pending' ORDER BY detected_at, workflow_kind, workflow_id"
            };
            sqlx::query_as::<_, RecoveryRecordRow>(query)
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(RecoveryRecordRow::into_domain)
                .collect()
        })
    }

    fn prepare_generation_resume(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>> {
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM workflow_recovery_records \
                 WHERE workflow_kind = 'generation' AND workflow_id = ? AND state = 'pending'",
            )
            .bind(job_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(store_error)?;
            if pending != 1 {
                return Err(RecoveryStoreError(format!(
                    "generation job {job_id} has no pending interruption record"
                )));
            }
            let result = sqlx::query(
                "UPDATE generation_jobs SET state = 'queued', cancel_requested = 0, \
                 error_message = NULL, updated_at = ? WHERE id = ? AND state IN ('failed', 'queued')",
            )
            .bind(Utc::now())
            .bind(job_id)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(RecoveryStoreError(format!(
                    "generation job {job_id} is not eligible for in-place resume"
                )));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn resolve_recovery(
        &self,
        kind: WorkflowKind,
        workflow_id: Uuid,
        state: RecoveryState,
        replacement_id: Option<Uuid>,
    ) -> BoxFuture<'_, Result<(), RecoveryStoreError>> {
        Box::pin(async move {
            if state == RecoveryState::Pending {
                return Err(RecoveryStoreError(
                    "pending is not a terminal recovery resolution".into(),
                ));
            }
            let result = sqlx::query(
                "UPDATE workflow_recovery_records SET state = ?, replacement_id = ?, \
                 resolved_at = ? WHERE workflow_kind = ? AND workflow_id = ? AND state = 'pending'",
            )
            .bind(recovery_state(state))
            .bind(replacement_id)
            .bind(Utc::now())
            .bind(kind.as_str())
            .bind(workflow_id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(RecoveryStoreError(format!(
                    "{} workflow {workflow_id} has no pending recovery",
                    kind.as_str()
                )));
            }
            Ok(())
        })
    }
}

#[derive(Debug, FromRow)]
struct RunningWorkflowRecord {
    workflow_kind: String,
    workflow_id: Uuid,
}

#[derive(Debug, FromRow)]
struct LeaseRecord {
    process_id: i64,
    process_started_at: i64,
}

impl LeaseRecord {
    fn is_active(&self, system: &System) -> bool {
        let Ok(process_id) = u32::try_from(self.process_id) else {
            return false;
        };
        let Ok(started_at) = u64::try_from(self.process_started_at) else {
            return false;
        };
        system
            .process(Pid::from_u32(process_id))
            .is_some_and(|process| process.start_time() == started_at)
    }
}

#[derive(Debug, FromRow)]
struct RecoveryRecordRow {
    workflow_kind: String,
    workflow_id: Uuid,
    state: String,
    resumable_in_place: bool,
    message: String,
    detected_at: DateTime<Utc>,
    replacement_id: Option<Uuid>,
    resolved_at: Option<DateTime<Utc>>,
}

impl RecoveryRecordRow {
    fn into_domain(self) -> Result<RecoveryRecord, RecoveryStoreError> {
        Ok(RecoveryRecord {
            workflow_kind: parse_kind(&self.workflow_kind)?,
            workflow_id: self.workflow_id,
            state: parse_recovery_state(&self.state)?,
            resumable_in_place: self.resumable_in_place,
            message: self.message,
            detected_at: self.detected_at,
            replacement_id: self.replacement_id,
            resolved_at: self.resolved_at,
        })
    }
}

async fn mark_failed(
    transaction: &mut Transaction<'_, Sqlite>,
    kind: WorkflowKind,
    workflow_id: Uuid,
    message: &str,
    now: DateTime<Utc>,
) -> Result<(), RecoveryStoreError> {
    let table = match kind {
        WorkflowKind::Generation => "generation_jobs",
        WorkflowKind::Training => "training_runs",
        WorkflowKind::Evaluation => "evaluation_runs",
    };
    let query = format!(
        "UPDATE {table} SET state = 'failed', error_message = ?, updated_at = ? \
         WHERE id = ? AND state = 'running'"
    );
    let result = sqlx::query(&query)
        .bind(message)
        .bind(now)
        .bind(workflow_id)
        .execute(&mut **transaction)
        .await
        .map_err(store_error)?;
    if result.rows_affected() != 1 {
        return Err(RecoveryStoreError(format!(
            "{} workflow {workflow_id} changed while detecting interruption",
            kind.as_str()
        )));
    }
    Ok(())
}

fn current_process_started_at(process_id: u32) -> Result<u64, RecoveryStoreError> {
    System::new_all()
        .process(Pid::from_u32(process_id))
        .map(sysinfo::Process::start_time)
        .ok_or_else(|| RecoveryStoreError("could not inspect the current process".into()))
}

fn parse_kind(value: &str) -> Result<WorkflowKind, RecoveryStoreError> {
    match value {
        "generation" => Ok(WorkflowKind::Generation),
        "training" => Ok(WorkflowKind::Training),
        "evaluation" => Ok(WorkflowKind::Evaluation),
        _ => Err(RecoveryStoreError(format!(
            "unknown workflow kind in SQLite: {value}"
        ))),
    }
}

fn parse_recovery_state(value: &str) -> Result<RecoveryState, RecoveryStoreError> {
    match value {
        "pending" => Ok(RecoveryState::Pending),
        "resumed" => Ok(RecoveryState::Resumed),
        "restarted" => Ok(RecoveryState::Restarted),
        "dismissed" => Ok(RecoveryState::Dismissed),
        _ => Err(RecoveryStoreError(format!(
            "unknown recovery state in SQLite: {value}"
        ))),
    }
}

const fn recovery_state(state: RecoveryState) -> &'static str {
    match state {
        RecoveryState::Pending => "pending",
        RecoveryState::Resumed => "resumed",
        RecoveryState::Restarted => "restarted",
        RecoveryState::Dismissed => "dismissed",
    }
}

fn to_i64(value: u64) -> Result<i64, RecoveryStoreError> {
    i64::try_from(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> RecoveryStoreError {
    RecoveryStoreError(error.to_string())
}
