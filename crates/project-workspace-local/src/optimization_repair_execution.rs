//! Immutable execution-stop evidence for a validated repair plan. Reading this
//! module never dispatches work or grants publication/training authority.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_optimization_core::{fingerprint, generation::RepairNotExecuted};
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
            existing.get::<String, _>("reason") == "canary_rejected"
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
        .bind("canary_rejected")
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
            && row.get::<String, _>("reason") == "canary_rejected"
            && row.get::<String, _>("fingerprint") == fingerprint(&receipt)?,
        "Repair execution stop receipt changed"
    );
    Ok(Some(receipt))
}
