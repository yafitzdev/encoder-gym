//! Append-only measured repair outcomes. Reads are passive and never rerun an
//! evaluator or authorize another intervention.

use std::{collections::BTreeSet, path::Path};

use anyhow::{Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{fingerprint, repair_outcome::RepairOutcome};
use sqlx::{Connection, Row, SqliteConnection};
use uuid::Uuid;

use crate::connect;

pub async fn record(folder: &Path, outcomes: &[RepairOutcome]) -> Result<()> {
    ensure!(!outcomes.is_empty(), "Repair outcome batch is empty");
    let run_id = outcomes[0].run_id;
    let iteration = outcomes[0].iteration;
    let mut targets = BTreeSet::new();
    for outcome in outcomes {
        outcome.validate()?;
        ensure!(
            outcome.run_id == run_id
                && outcome.iteration == iteration
                && targets.insert(outcome.target_id.clone()),
            "Repair outcome batch mixes iterations or repeats a target"
        );
    }
    let mut database = connect(folder, false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let existing = read_iteration(&mut transaction, run_id, iteration).await?;
    if existing.is_empty() {
        for outcome in outcomes {
            sqlx::query("INSERT INTO optimization_repair_outcomes(run_id,iteration,target_id,output_evidence_fingerprint,intervention_fingerprint,fingerprint,metadata_json,created_at) VALUES(?,?,?,?,?,?,?,?)")
                .bind(run_id.to_string())
                .bind(i64::from(iteration))
                .bind(&outcome.target_id)
                .bind(&outcome.output_development_evidence_fingerprint)
                .bind(&outcome.intervention_fingerprint)
                .bind(fingerprint(outcome)?)
                .bind(serde_json::to_string(outcome)?)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *transaction)
                .await?;
        }
    } else {
        ensure!(existing == outcomes, "Recorded repair outcomes changed");
    }
    transaction.commit().await?;
    database.close().await?;
    Ok(())
}

pub async fn list(folder: &Path, run_id: Uuid) -> Result<Vec<RepairOutcome>> {
    let mut database = connect(folder, true, false).await?;
    if !storage_exists(&mut database).await? {
        database.close().await?;
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT iteration FROM optimization_repair_outcomes WHERE run_id=? GROUP BY iteration ORDER BY iteration")
        .bind(run_id.to_string())
        .fetch_all(&mut database)
        .await?;
    let mut result = Vec::new();
    for row in rows {
        let iteration = u32::try_from(row.get::<i64, _>("iteration"))?;
        result.extend(read_iteration(&mut database, run_id, iteration).await?);
    }
    database.close().await?;
    Ok(result)
}

pub(crate) async fn read_iteration(
    database: &mut SqliteConnection,
    run_id: Uuid,
    iteration: u32,
) -> Result<Vec<RepairOutcome>> {
    if !storage_exists(database).await? {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT target_id,output_evidence_fingerprint,intervention_fingerprint,fingerprint,metadata_json FROM optimization_repair_outcomes WHERE run_id=? AND iteration=? ORDER BY target_id")
        .bind(run_id.to_string())
        .bind(i64::from(iteration))
        .fetch_all(database)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let outcome: RepairOutcome = serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        outcome.validate()?;
        ensure!(
            outcome.run_id == run_id
                && outcome.iteration == iteration
                && outcome.target_id == row.get::<String, _>("target_id")
                && outcome.output_development_evidence_fingerprint
                    == row.get::<String, _>("output_evidence_fingerprint")
                && outcome.intervention_fingerprint
                    == row.get::<String, _>("intervention_fingerprint")
                && fingerprint(&outcome)? == row.get::<String, _>("fingerprint"),
            "Repair outcome storage changed"
        );
        result.push(outcome);
    }
    Ok(result)
}

async fn storage_exists(database: &mut SqliteConnection) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_repair_outcomes'",
    )
    .fetch_one(database)
    .await?
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pre_outcome_schema_reads_as_legacy_absence() {
        let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        assert!(
            read_iteration(&mut database, Uuid::new_v4(), 1)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
