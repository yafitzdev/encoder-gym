//! Append-only final dispatch/result custody, isolated from adaptive history.
use crate::{connect, optimization_completions::IterationScientificEvidence, optimization_final};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use project_workspace_core::{
    optimization_final::AgentFinalAuthorization,
    optimization_final_execution::{AgentFinalDispatch, AgentFinalReceipt, AgentFinalResult},
};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Connection, Row, SqliteConnection};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFinalExecution {
    pub authorization: AgentFinalAuthorization,
    pub dispatch: Option<AgentFinalDispatch>,
    pub result: Option<AgentFinalReceipt>,
}

impl AgentFinalExecution {
    /// Persisted dispatch alone is not evidence of a live worker or a failure.
    pub fn state(&self) -> &'static str {
        if self.result.is_some() {
            "completed"
        } else if self.dispatch.is_some() {
            "outcome_unknown"
        } else {
            "authorized"
        }
    }
}

pub fn show<'a>(
    folder: &'a Path,
    run_id: Uuid,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<Option<AgentFinalExecution>>> + 'a {
    Box::pin(async move {
        let Some(authorization) = optimization_final::show(folder, run_id, scientific).await?
        else {
            return Ok(None);
        };
        let mut db = connect(folder, true, false).await?;
        let dispatch = read_dispatch(&mut db, &authorization).await?;
        let result: Option<(String, AgentFinalReceipt)> = read(
            &mut db,
            "optimization_agent_final_results",
            authorization.id,
        )
        .await?;
        db.close().await?;
        let result = result
            .map(|(fingerprint, value)| {
                let dispatch = dispatch
                    .as_ref()
                    .context("Final result has no dispatch reservation")?;
                value.validate(&authorization, dispatch)?;
                ensure!(
                    value.fingerprint == fingerprint,
                    "Final result storage changed"
                );
                Ok::<_, anyhow::Error>(value)
            })
            .transpose()?;
        Ok(Some(AgentFinalExecution {
            authorization,
            dispatch,
            result,
        }))
    })
}

/// Only the caller that inserts this row receives fresh=true. Every other call
/// must use the adapter's read-only recovery port, including after process death.
/// The CLI also holds the ordinary exclusive execution lease during dispatch.
pub fn reserve<'a>(
    folder: &'a Path,
    run_id: Uuid,
    authorization_id: Uuid,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<(AgentFinalDispatch, bool)>> + 'a {
    Box::pin(async move {
        let view = show(folder, run_id, scientific)
            .await?
            .context("Explicit final consent is required")?;
        ensure!(
            view.authorization.id == authorization_id,
            "Final consent differs from the requested authorization"
        );
        if let Some(dispatch) = view.dispatch {
            return Ok((dispatch, false));
        }
        let mut db = connect(folder, false, false).await?;
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::migrate!("./migrations").run(&mut *tx).await?;
        ensure!(
            optimization_final::read(&mut tx, run_id).await?.as_ref() == Some(&view.authorization),
            "Final consent changed before dispatch"
        );
        let (dispatch, fresh) = if let Some(existing) =
            read_dispatch(&mut tx, &view.authorization).await?
        {
            (existing, false)
        } else {
            let dispatch = AgentFinalDispatch::create(&view.authorization, Utc::now())?;
            sqlx::query("INSERT INTO optimization_agent_final_dispatches(authorization_id,fingerprint,metadata_json) VALUES(?,?,?)")
                .bind(authorization_id.to_string()).bind(&dispatch.fingerprint).bind(serde_json::to_string(&dispatch)?)
                .execute(&mut *tx).await?;
            (dispatch, true)
        };
        tx.commit().await?;
        db.close().await?;
        Ok((dispatch, fresh))
    })
}

pub fn record_result<'a>(
    folder: &'a Path,
    run_id: Uuid,
    result: AgentFinalResult,
    scientific: &'a [IterationScientificEvidence],
) -> impl std::future::Future<Output = Result<AgentFinalReceipt>> + 'a {
    Box::pin(async move {
        let view = show(folder, run_id, scientific)
            .await?
            .context("Final consent is missing")?;
        let dispatch = view
            .dispatch
            .as_ref()
            .context("Final dispatch was not reserved")?;
        let source = selected_source(&view.authorization, scientific)?;
        result.validate(
            &view.authorization,
            dispatch,
            &source.project,
            &source.protocol,
        )?;
        let receipt = AgentFinalReceipt::from_result(&result)?;
        let mut db = connect(folder, false, false).await?;
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        ensure!(
            optimization_final::read(&mut tx, run_id).await?.as_ref() == Some(&view.authorization)
                && read_dispatch(&mut tx, &view.authorization).await?.as_ref() == Some(dispatch),
            "Final authority changed before result persistence"
        );
        let saved = if let Some((fingerprint, existing)) = read::<AgentFinalReceipt>(
            &mut tx,
            "optimization_agent_final_results",
            view.authorization.id,
        )
        .await?
        {
            existing.recover(
                &view.authorization,
                dispatch,
                &source.project,
                &source.protocol,
                result.report,
            )?;
            ensure!(
                existing.fingerprint == fingerprint,
                "Final result storage changed"
            );
            existing
        } else {
            sqlx::query("INSERT INTO optimization_agent_final_results(authorization_id,fingerprint,metadata_json) VALUES(?,?,?)")
                .bind(view.authorization.id.to_string()).bind(&receipt.fingerprint).bind(serde_json::to_string(&receipt)?)
                .execute(&mut *tx).await?;
            receipt
        };
        tx.commit().await?;
        db.close().await?;
        Ok(saved)
    })
}

fn selected_source<'a>(
    authorization: &AgentFinalAuthorization,
    scientific: &'a [IterationScientificEvidence],
) -> Result<&'a IterationScientificEvidence> {
    scientific
        .iter()
        .find(|source| source.iteration_id.to_string() == authorization.scope.iteration.id)
        .context("Selected final scientific evidence is missing")
}

async fn read_dispatch(
    db: &mut SqliteConnection,
    authorization: &AgentFinalAuthorization,
) -> Result<Option<AgentFinalDispatch>> {
    let record: Option<(String, AgentFinalDispatch)> =
        read(db, "optimization_agent_final_dispatches", authorization.id).await?;
    record
        .map(|(fingerprint, value)| {
            value.validate(authorization)?;
            ensure!(
                value.fingerprint == fingerprint,
                "Final dispatch storage changed"
            );
            Ok(value)
        })
        .transpose()
}

async fn read<T: DeserializeOwned>(
    db: &mut SqliteConnection,
    table: &str,
    authorization_id: Uuid,
) -> Result<Option<(String, T)>> {
    ensure!(
        matches!(
            table,
            "optimization_agent_final_dispatches" | "optimization_agent_final_results"
        ),
        "Unknown final record"
    );
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(&mut *db)
            .await?;
    if exists == 0 {
        return Ok(None);
    }
    let row = sqlx::query(&format!(
        "SELECT fingerprint,metadata_json FROM {table} WHERE authorization_id=?"
    ))
    .bind(authorization_id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    row.map(|row| {
        Ok((
            row.get("fingerprint"),
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?,
        ))
    })
    .transpose()
}
