//! Append-only accounting at the actual native-training dispatch boundary.
use crate::{
    connect, optimization_execution, optimization_iteration_execution as custody,
    optimization_launch, optimization_runs,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    domain::TrainingCandidate,
    ports::{BoxFuture, EncoderTaskAdapterError},
    training_budget::*,
};
use project_workspace_core::{
    BoundIdentity, optimization_iteration_execution::IterationTrainingBinding,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug)]
pub struct ProjectTrainingAccounting {
    folder: PathBuf,
    run_id: Uuid,
    iteration_id: Uuid,
    binding: IterationTrainingBinding,
    limits: TrainingTimeLimits,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reservation {
    run_id: Uuid,
    training_fingerprint: String,
    candidate: BoundIdentity,
    attempt: TrainingAttempt,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Completion {
    permit: TrainingPermit,
    completion: TrainingAttemptCompletion,
    created_at: DateTime<Utc>,
}

impl ProjectTrainingAccounting {
    pub async fn open(
        folder: &Path,
        run_id: Uuid,
        iteration_id: Uuid,
        prior_training_started: bool,
    ) -> Result<Self> {
        let binding = custody::training(folder, run_id, iteration_id)
            .await?
            .context("Training binding missing")?;
        let run = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|launch| launch.id.to_string() == run.run.launch.id)
            .context("Launch missing")?;
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .context("Agent settings missing")?;
        let value = Self {
            folder: folder.into(),
            run_id,
            iteration_id,
            binding,
            limits: TrainingTimeLimits {
                iteration_millis: u64::from(settings.training.maximum_seconds_per_iteration)
                    .checked_mul(1000)
                    .context("Training limit overflow")?,
                run_millis: launch
                    .scope
                    .limits
                    .maximum_training_seconds
                    .checked_mul(1000)
                    .context("Training limit overflow")?,
            },
        };
        let mut db = connect(folder, false, false).await?;
        sqlx::migrate!("./migrations").run(&mut db).await?;
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        let attempts = read(&mut tx, run_id).await?;
        value.limits.remaining(&attempts, iteration_id)?;
        // Upgrading a previously started, unmetered iteration must not grant a
        // fresh allowance. Preserve its unknown charge even if its artifact is reusable.
        if prior_training_started && !attempts.iter().any(|a| a.iteration_id == iteration_id) {
            value.insert_reservation(&mut tx, &attempts).await?;
        }
        tx.commit().await?;
        db.close().await?;
        Ok(value)
    }

    async fn insert_reservation(
        &self,
        db: &mut SqliteConnection,
        attempts: &[TrainingAttempt],
    ) -> Result<TrainingPermit> {
        let attempt = self.limits.reserve(attempts, self.iteration_id)?;
        let record = Reservation {
            run_id: self.run_id,
            training_fingerprint: self.binding.fingerprint.clone(),
            candidate: self.binding.candidate.clone(),
            attempt,
            created_at: Utc::now(),
        };
        sqlx::query("INSERT INTO optimization_training_attempts(id,run_id,iteration_id,fingerprint,metadata_json) VALUES(?,?,?,?,?)")
            .bind(record.attempt.permit.id.to_string()).bind(self.run_id.to_string()).bind(self.iteration_id.to_string())
            .bind(artifact_core::fingerprint(&record)?).bind(serde_json::to_string(&record)?).execute(db).await?;
        Ok(record.attempt.permit)
    }
}

impl TrainingAccounting for ProjectTrainingAccounting {
    fn reserve(
        &self,
        candidate: &TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainingPermit, EncoderTaskAdapterError>> {
        let candidate = candidate.clone();
        Box::pin(async move {
            async {
                ensure!(
                    self.binding.candidate.id == candidate.id.to_string()
                        && self.binding.candidate.fingerprint == candidate.fingerprint
                        && candidate.maximum_training_seconds.checked_mul(1000)
                            == Some(self.limits.iteration_millis),
                    "Training candidate or time authority changed"
                );
                let mut db = connect(&self.folder, false, false).await?;
                let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
                ensure!(
                    !optimization_execution::dispatch_stopped(&mut tx, self.run_id).await?,
                    "Run stopped before training dispatch"
                );
                let stored = custody::read::<IterationTrainingBinding>(
                    &mut tx,
                    "optimization_iteration_training",
                    self.iteration_id,
                )
                .await?;
                ensure!(
                    stored.as_ref() == Some(&self.binding),
                    "Training binding changed before dispatch"
                );
                let attempts = read(&mut tx, self.run_id).await?;
                let permit = self.insert_reservation(&mut tx, &attempts).await?;
                tx.commit().await?;
                db.close().await?;
                Ok(permit)
            }
            .await
            .map_err(adapter_error)
        })
    }

    fn finish(
        &self,
        permit: TrainingPermit,
        outcome: TrainingAttemptOutcome,
        elapsed_millis: u64,
    ) -> BoxFuture<'_, Result<(), EncoderTaskAdapterError>> {
        Box::pin(async move {
            async {
            let mut db = connect(&self.folder, false, false).await?;
            let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
            let attempts = read(&mut tx, self.run_id).await?;
            let mut attempt = attempts.into_iter().find(|a| a.permit == permit && a.iteration_id == self.iteration_id)
                .context("Training completion has no exact reservation")?;
            let already_finished = attempt.completion.is_some();
            attempt.finish(outcome, elapsed_millis)?;
            if !already_finished {
                let record = Completion { permit, completion: attempt.completion.unwrap(), created_at: Utc::now() };
                sqlx::query("INSERT INTO optimization_training_completions(attempt_id,fingerprint,metadata_json) VALUES(?,?,?)")
                    .bind(record.permit.id.to_string()).bind(artifact_core::fingerprint(&record)?)
                    .bind(serde_json::to_string(&record)?).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            db.close().await?;
            Ok(())
        }.await.map_err(adapter_error)
        })
    }
}

/// Read-only persisted facts; pending records retain their full conservative charge.
pub async fn history(folder: &Path, run_id: Uuid) -> Result<Vec<TrainingAttempt>> {
    let run = optimization_runs::show(folder, run_id).await?;
    let mut db = connect(folder, true, false).await?;
    let attempts = read(&mut db, run_id).await?;
    db.close().await?;
    if let Some(first) = attempts.first() {
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|launch| launch.id.to_string() == run.run.launch.id)
            .context("Launch missing")?;
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .context("Training ledger has no Agent authority")?;
        TrainingTimeLimits {
            iteration_millis: u64::from(settings.training.maximum_seconds_per_iteration)
                .checked_mul(1000)
                .context("Training limit overflow")?,
            run_millis: launch
                .scope
                .limits
                .maximum_training_seconds
                .checked_mul(1000)
                .context("Training limit overflow")?,
        }
        .remaining(&attempts, first.iteration_id)?;
    }
    Ok(attempts)
}

async fn read(db: &mut SqliteConnection, run_id: Uuid) -> Result<Vec<TrainingAttempt>> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_training_attempts'").fetch_one(&mut *db).await?;
    if exists == 0 {
        return Ok(vec![]);
    }
    let rows = sqlx::query(
        "SELECT * FROM optimization_training_attempts WHERE run_id=? ORDER BY sequence",
    )
    .bind(run_id.to_string())
    .fetch_all(&mut *db)
    .await?;
    let mut result = Vec::new();
    for row in rows {
        let record: Reservation = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        ensure!(
            record.run_id == run_id
                && record.attempt.permit.id.to_string() == row.get::<String, _>("id")
                && record.attempt.iteration_id.to_string() == row.get::<String, _>("iteration_id")
                && record.attempt.completion.is_none()
                && artifact_core::fingerprint(&record)? == row.get::<String, _>("fingerprint"),
            "Training reservation storage changed"
        );
        let binding = custody::read::<IterationTrainingBinding>(
            db,
            "optimization_iteration_training",
            record.attempt.iteration_id,
        )
        .await?
        .context("Training custody missing")?;
        ensure!(
            binding.fingerprint == record.training_fingerprint
                && binding.candidate == record.candidate,
            "Training reservation custody changed"
        );
        let mut attempt = record.attempt;
        let owner: String =
            sqlx::query_scalar("SELECT run_id FROM project_optimization_iterations WHERE id=?")
                .bind(attempt.iteration_id.to_string())
                .fetch_one(&mut *db)
                .await?;
        ensure!(
            owner == run_id.to_string(),
            "Training iteration belongs to another run"
        );
        if let Some(row) =
            sqlx::query("SELECT * FROM optimization_training_completions WHERE attempt_id=?")
                .bind(attempt.permit.id.to_string())
                .fetch_optional(&mut *db)
                .await?
        {
            let completion: Completion =
                serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
            ensure!(
                completion.permit == attempt.permit
                    && completion.created_at >= record.created_at
                    && artifact_core::fingerprint(&completion)?
                        == row.get::<String, _>("fingerprint"),
                "Training completion storage changed"
            );
            attempt.finish(
                completion.completion.outcome,
                completion.completion.elapsed_millis,
            )?;
        }
        result.push(attempt);
    }
    Ok(result)
}

fn adapter_error(error: anyhow::Error) -> EncoderTaskAdapterError {
    error
        .downcast_ref::<EncoderTaskAdapterError>()
        .cloned()
        .unwrap_or_else(|| EncoderTaskAdapterError::TrainingAccounting(error.to_string()))
}
