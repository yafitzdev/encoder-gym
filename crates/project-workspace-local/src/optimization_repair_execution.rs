//! Immutable execution-stop evidence for a validated repair plan. Reading this
//! module never dispatches work or grants publication/training authority.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{
    fingerprint,
    generation::{RepairNotExecuted, RepairNotExecutedReason},
};
use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::connect;

pub async fn record_not_executed(folder: &Path, receipt: &RepairNotExecuted) -> Result<()> {
    receipt.validate()?;
    let mut database = connect(folder, false, false).await?;
    sqlx::migrate!("./migrations").run(&mut database).await?;
    let encoded = serde_json::to_string(receipt)?;
    let receipt_fingerprint = fingerprint(receipt)?;
    let existing = sqlx::query(
        "SELECT reason,fingerprint,metadata_json FROM optimization_repair_not_executed WHERE run_id=? AND iteration=?",
    )
    .bind(receipt.run_id.to_string())
    .bind(i64::from(receipt.iteration))
    .fetch_optional(&mut database)
    .await?;
    if let Some(existing) = existing {
        ensure!(
            existing.get::<String, _>("reason") == reason_name(receipt.reason)
                && existing.get::<String, _>("fingerprint") == receipt_fingerprint
                && existing.get::<String, _>("metadata_json") == encoded,
            "Repair execution stop receipt changed"
        );
    } else {
        sqlx::query(
            "INSERT INTO optimization_repair_not_executed(run_id,iteration,reason,fingerprint,metadata_json,created_at) VALUES(?,?,?,?,?,?)",
        )
        .bind(receipt.run_id.to_string())
        .bind(i64::from(receipt.iteration))
        .bind(reason_name(receipt.reason))
        .bind(receipt_fingerprint)
        .bind(encoded)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut database)
        .await?;
    }
    database.close().await?;
    Ok(())
}

pub async fn not_executed(
    folder: &Path,
    run_id: Uuid,
    iteration: u32,
) -> Result<Option<RepairNotExecuted>> {
    let mut database = connect(folder, true, false).await?;
    let receipt = read_not_executed(&mut database, run_id, iteration).await?;
    database.close().await?;
    Ok(receipt)
}

pub(crate) async fn read_not_executed(
    database: &mut sqlx::SqliteConnection,
    run_id: Uuid,
    iteration: u32,
) -> Result<Option<RepairNotExecuted>> {
    if !storage_exists(database).await? {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT reason,fingerprint,metadata_json FROM optimization_repair_not_executed WHERE run_id=? AND iteration=?",
    )
    .bind(run_id.to_string())
    .bind(i64::from(iteration))
    .fetch_optional(database)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let receipt: RepairNotExecuted = serde_json::from_str(&row.get::<String, _>("metadata_json"))
        .context("Repair execution stop receipt is corrupt")?;
    receipt.validate()?;
    ensure!(
        receipt.run_id == run_id
            && receipt.iteration == iteration
            && row.get::<String, _>("reason") == reason_name(receipt.reason)
            && row.get::<String, _>("fingerprint") == fingerprint(&receipt)?,
        "Repair execution stop receipt changed"
    );
    Ok(Some(receipt))
}

async fn storage_exists(database: &mut sqlx::SqliteConnection) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_repair_not_executed'",
    )
    .fetch_one(database)
    .await?
        == 1)
}

fn reason_name(reason: RepairNotExecutedReason) -> &'static str {
    match reason {
        RepairNotExecutedReason::CanaryRejected => "canary_rejected",
        RepairNotExecutedReason::ZeroSurvivingEdits => "zero_surviving_edits",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqliteConnection;

    #[tokio::test]
    async fn pre_repair_schema_reads_as_legacy_absence() {
        let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        assert_eq!(
            read_not_executed(&mut database, Uuid::new_v4(), 1)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn stop_reason_migration_preserves_canary_and_admits_zero_surviving() {
        let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE project_optimization_runs(id TEXT PRIMARY KEY); \
             INSERT INTO project_optimization_runs VALUES('run-1'),('run-2');",
        )
        .execute(&mut database)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/0026_optimization_repair_execution.sql"
        ))
        .execute(&mut database)
        .await
        .unwrap();
        sqlx::query("INSERT INTO optimization_repair_not_executed VALUES(?,?,?,?,?,?)")
            .bind("run-1")
            .bind(1_i64)
            .bind("canary_rejected")
            .bind("fingerprint-1")
            .bind("{}")
            .bind("2026-09-19T00:00:00Z")
            .execute(&mut database)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../migrations/0028_optimization_repair_stop_reasons.sql"
        ))
        .execute(&mut database)
        .await
        .unwrap();
        sqlx::query("INSERT INTO optimization_repair_not_executed VALUES(?,?,?,?,?,?)")
            .bind("run-2")
            .bind(1_i64)
            .bind("zero_surviving_edits")
            .bind("fingerprint-2")
            .bind("{}")
            .bind("2026-09-19T00:00:00Z")
            .execute(&mut database)
            .await
            .unwrap();
        let reasons = sqlx::query_scalar::<_, String>(
            "SELECT reason FROM optimization_repair_not_executed ORDER BY run_id",
        )
        .fetch_all(&mut database)
        .await
        .unwrap();
        assert_eq!(reasons, ["canary_rejected", "zero_surviving_edits"]);
    }
}
