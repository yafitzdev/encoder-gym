//! Durable native semantic-review calls and row-level admission evidence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use dataset_quality_core::{
    native_assessment::{
        NativeAdmissionDecision, NativeAdmissionRecord, NativeReviewCallOutcome,
        NativeReviewCallReservation, NativeReviewOperationCategory, NativeReviewRequest,
        NativeReviewUsage,
    },
    ports::{BoxFuture, NativeReviewStore, QualityAdapterError},
};
use encoder_optimization_core::{agent::conservative_charge, fingerprint};
use project_workspace_core::{OptimizationLaunchAuthorization, ProviderLimits};
use sqlx::{Connection, Row, SqliteConnection};
use sysinfo::{Pid, System};
use uuid::Uuid;

use crate::{
    connect, open_workspace, optimization_agent, optimization_iterations, optimization_launch,
    optimization_runs,
};

pub struct ProjectNativeReviewStore {
    folder: PathBuf,
    run_id: Uuid,
    launch: OptimizationLaunchAuthorization,
}

impl ProjectNativeReviewStore {
    pub async fn open(folder: &Path, run_id: Uuid) -> Result<Self> {
        let workspace = open_workspace(folder, false).await?;
        let run = optimization_runs::show(folder, run_id).await?;
        let launch = optimization_launch::list(folder)
            .await?
            .into_iter()
            .find(|value| value.id.to_string() == run.run.launch.id)
            .context("Run launch was not found")?;
        ensure!(
            launch
                .scope
                .agentic
                .as_ref()
                .is_some_and(|settings| settings.analysis_protocol == 3),
            "Native semantic review requires an analysis-protocol V3 run"
        );
        let mut database = connect(Path::new(&workspace.folder), false, false).await?;
        sqlx::migrate!("./migrations").run(&mut database).await?;
        database.close().await?;
        Ok(Self {
            folder: workspace.folder.into(),
            run_id,
            launch,
        })
    }

    async fn validate_request(&self, request: &NativeReviewRequest) -> Result<()> {
        request.validate()?;
        ensure!(
            request.run_id() == self.run_id,
            "Native review request belongs to another run"
        );
        let iterations = optimization_iterations::list(&self.folder, self.run_id).await?;
        ensure!(
            iterations
                .iter()
                .any(|value| value.scope.iteration == request.iteration()),
            "Native review request has no exact authorized iteration"
        );
        Ok(())
    }

    /// Convert only a reservation owned by a no-longer-running exact process
    /// into an interrupted outcome. Recovery never dispatches its replacement.
    pub async fn recover_interrupted(&self) -> Result<()> {
        let mut database = connect(&self.folder, false, false).await?;
        let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
        let rows = sqlx::query("SELECT c.metadata_json,c.fingerprint,c.process_id,c.process_started_at FROM optimization_native_review_calls c LEFT JOIN optimization_native_review_outcomes o ON o.call_id=c.id WHERE c.run_id=? AND o.call_id IS NULL ORDER BY c.iteration,c.created_at")
            .bind(self.run_id.to_string())
            .fetch_all(&mut *transaction)
            .await?;
        let processes = System::new_all();
        for row in rows {
            let pid = u32::try_from(row.get::<i64, _>("process_id"))?;
            let started = u64::try_from(row.get::<i64, _>("process_started_at"))?;
            ensure!(
                !processes
                    .process(Pid::from_u32(pid))
                    .is_some_and(|process| process.start_time() == started),
                "Reserved native review call still belongs to a live process"
            );
            let reservation: NativeReviewCallReservation =
                serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
            ensure!(
                fingerprint(&reservation)? == row.get::<String, _>("fingerprint"),
                "Native review reservation fingerprint changed"
            );
            insert_outcome(
                &mut transaction,
                &NativeReviewCallOutcome {
                    reservation,
                    usage: NativeReviewUsage::default(),
                    response: None,
                    failure: None,
                    interrupted: true,
                },
            )
            .await?;
        }
        transaction.commit().await?;
        database.close().await?;
        Ok(())
    }
}

impl NativeReviewStore for ProjectNativeReviewStore {
    fn history(
        &self,
        request: NativeReviewRequest,
    ) -> BoxFuture<'_, Result<Vec<NativeReviewCallOutcome>, QualityAdapterError>> {
        Box::pin(async move {
            self.validate_request(&request).await.map_err(adapter)?;
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let values = read_calls(&mut database, self.run_id)
                .await
                .map_err(adapter)?
                .into_iter()
                .filter(|(reservation, _)| {
                    reservation.request.fingerprint() == request.fingerprint()
                })
                .filter_map(|(_, outcome)| outcome)
                .collect();
            database.close().await.map_err(adapter)?;
            Ok(values)
        })
    }

    fn reserve(
        &self,
        reservation: NativeReviewCallReservation,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        Box::pin(async move {
            self.validate_request(&reservation.request)
                .await
                .map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            reserve(&mut transaction, &reservation, &self.launch.scope.advisor)
                .await
                .map_err(adapter)?;
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(())
        })
    }

    fn finish(
        &self,
        outcome: NativeReviewCallOutcome,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        Box::pin(async move {
            self.validate_request(&outcome.reservation.request)
                .await
                .map_err(adapter)?;
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            insert_outcome(&mut transaction, &outcome)
                .await
                .map_err(adapter)?;
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(())
        })
    }

    fn record_admissions(
        &self,
        records: Vec<NativeAdmissionRecord>,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        Box::pin(async move {
            let mut database = connect(&self.folder, false, false).await.map_err(adapter)?;
            let mut transaction = database
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(adapter)?;
            for record in records {
                record.validate().map_err(adapter)?;
                if record.run_id != self.run_id {
                    return Err(adapter("Native admission belongs to another run"));
                }
                let existing = sqlx::query_scalar::<_, String>("SELECT metadata_json FROM optimization_native_admissions WHERE run_id=? AND iteration=? AND row_id=?")
                    .bind(record.run_id.to_string())
                    .bind(i64::from(record.iteration))
                    .bind(&record.authority.row_id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(adapter)?;
                if let Some(existing) = existing {
                    let existing: NativeAdmissionRecord =
                        serde_json::from_str(&existing).map_err(adapter)?;
                    if existing != record {
                        return Err(adapter("Native admission evidence changed on replay"));
                    }
                    continue;
                }
                sqlx::query("INSERT INTO optimization_native_admissions(run_id,iteration,row_id,row_fingerprint,decision,fingerprint,metadata_json,created_at) VALUES(?,?,?,?,?,?,?,?)")
                    .bind(record.run_id.to_string())
                    .bind(i64::from(record.iteration))
                    .bind(&record.authority.row_id)
                    .bind(&record.authority.row_fingerprint)
                    .bind(decision_name(record.admission.decision))
                    .bind(&record.fingerprint)
                    .bind(serde_json::to_string(&record).map_err(adapter)?)
                    .bind(Utc::now().to_rfc3339())
                    .execute(&mut *transaction)
                    .await
                    .map_err(adapter)?;
            }
            transaction.commit().await.map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(())
        })
    }

    fn admissions(
        &self,
        run_id: Uuid,
        iteration: u32,
    ) -> BoxFuture<'_, Result<Vec<NativeAdmissionRecord>, QualityAdapterError>> {
        Box::pin(async move {
            if run_id != self.run_id || !(1..=10).contains(&iteration) {
                return Err(adapter("Native admission query is outside this run"));
            }
            let mut database = connect(&self.folder, true, false).await.map_err(adapter)?;
            let result = read_admissions(&mut database, run_id, iteration)
                .await
                .map_err(adapter)?;
            database.close().await.map_err(adapter)?;
            Ok(result)
        })
    }
}

pub(crate) async fn read_admissions(
    database: &mut SqliteConnection,
    run_id: Uuid,
    iteration: u32,
) -> Result<Vec<NativeAdmissionRecord>> {
    let rows = sqlx::query("SELECT row_id,row_fingerprint,fingerprint,metadata_json FROM optimization_native_admissions WHERE run_id=? AND iteration=? ORDER BY row_id")
        .bind(run_id.to_string())
        .bind(i64::from(iteration))
        .fetch_all(database)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let value: NativeAdmissionRecord =
            serde_json::from_str(&row.get::<String, _>("metadata_json"))?;
        value.validate()?;
        ensure!(
            value.run_id == run_id
                && value.iteration == iteration
                && value.authority.row_id == row.get::<String, _>("row_id")
                && value.authority.row_fingerprint == row.get::<String, _>("row_fingerprint")
                && value.fingerprint == row.get::<String, _>("fingerprint"),
            "Native admission evidence binding changed"
        );
        result.push(value);
    }
    Ok(result)
}

pub(crate) async fn reserve(
    database: &mut SqliteConnection,
    reservation: &NativeReviewCallReservation,
    limits: &ProviderLimits,
) -> Result<()> {
    reservation.validate()?;
    ensure!(
        !crate::optimization_execution::dispatch_stopped(database, reservation.request.run_id())
            .await?,
        "Optimization run stopped before native semantic review dispatch"
    );
    let agent =
        optimization_agent::read_history(database, reservation.request.run_id(), None).await?;
    let native = read_calls(database, reservation.request.run_id()).await?;
    ensure!(
        agent.iter().all(|(_, outcome)| outcome.is_some())
            && native.iter().all(|(_, outcome)| outcome.is_some()),
        "Another advisor call is still pending; recover its exact process first"
    );
    let request_history = native
        .iter()
        .filter(|(call, _)| call.request.fingerprint() == reservation.request.fingerprint())
        .collect::<Vec<_>>();
    match reservation.attempt {
        1 => ensure!(
            request_history.is_empty(),
            "Native review request already has an attempt"
        ),
        2 => ensure!(
            request_history.len() == 1
                && request_history[0].0.attempt == 1
                && request_history[0]
                    .1
                    .as_ref()
                    .is_some_and(|outcome| outcome.interrupted),
            "Native review retry requires one interrupted first attempt"
        ),
        _ => unreachable!("validated native review attempt"),
    }
    let requests = agent
        .len()
        .checked_add(native.len())
        .and_then(|value| value.checked_add(1))
        .context("Advisor request accounting overflow")?;
    ensure!(
        requests <= limits.maximum_requests as usize,
        "Advisor request budget exhausted by native semantic review"
    );
    let (mut input, mut output, mut cost) = (
        reservation.input_token_ceiling,
        reservation.output_token_ceiling,
        reservation.cost_ceiling_microusd,
    );
    for (call, outcome) in agent {
        let interrupted = outcome.as_ref().is_none_or(|value| value.interrupted);
        input = input
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.input_tokens),
                call.input_token_ceiling,
                interrupted,
            ))
            .context("Advisor input accounting overflow")?;
        output = output
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.output_tokens),
                call.output_token_ceiling,
                interrupted,
            ))
            .context("Advisor output accounting overflow")?;
        cost = cost
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.cost_microusd),
                call.cost_ceiling_microusd,
                interrupted,
            ))
            .context("Advisor spend accounting overflow")?;
    }
    for (call, outcome) in native {
        let interrupted = outcome.as_ref().is_none_or(|value| value.interrupted);
        input = input
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.input_tokens),
                call.input_token_ceiling,
                interrupted,
            ))
            .context("Advisor input accounting overflow")?;
        output = output
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.output_tokens),
                call.output_token_ceiling,
                interrupted,
            ))
            .context("Advisor output accounting overflow")?;
        cost = cost
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.cost_microusd),
                call.cost_ceiling_microusd,
                interrupted,
            ))
            .context("Advisor spend accounting overflow")?;
    }
    ensure!(
        input <= limits.maximum_input_tokens,
        "Advisor input token budget exhausted by native semantic review"
    );
    ensure!(
        output <= limits.maximum_output_tokens,
        "Advisor output token budget exhausted by native semantic review"
    );
    ensure!(
        cost <= limits.maximum_cost_microusd,
        "Advisor spend budget exhausted by native semantic review"
    );
    let pid = std::process::id();
    let processes = System::new_all();
    let started = processes
        .process(Pid::from_u32(pid))
        .context("Native reviewer process identity is unavailable")?
        .start_time();
    sqlx::query("INSERT INTO optimization_native_review_calls(id,run_id,iteration,category,request_id,request_fingerprint,attempt,fingerprint,metadata_json,process_id,process_started_at,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(reservation.id.to_string())
        .bind(reservation.request.run_id().to_string())
        .bind(i64::from(reservation.request.iteration()))
        .bind(category_name(reservation.category))
        .bind(reservation.request.id().to_string())
        .bind(reservation.request.fingerprint())
        .bind(i64::from(reservation.attempt))
        .bind(fingerprint(reservation)?)
        .bind(serde_json::to_string(reservation)?)
        .bind(i64::from(pid))
        .bind(i64::try_from(started)?)
        .bind(Utc::now().to_rfc3339())
        .execute(database)
        .await?;
    Ok(())
}

pub(crate) async fn read_calls(
    database: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<(NativeReviewCallReservation, Option<NativeReviewCallOutcome>)>> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='optimization_native_review_calls'")
        .fetch_one(&mut *database)
        .await?
        == 0
    {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT c.id,c.iteration,c.category,c.request_id,c.request_fingerprint,c.attempt,c.fingerprint AS call_fingerprint,c.metadata_json AS call_json,o.fingerprint AS outcome_fingerprint,o.metadata_json AS outcome_json FROM optimization_native_review_calls c LEFT JOIN optimization_native_review_outcomes o ON o.call_id=c.id WHERE c.run_id=? ORDER BY c.iteration,c.created_at,c.attempt")
        .bind(run_id.to_string())
        .fetch_all(database)
        .await?;
    rows.into_iter()
        .map(|row| {
            let reservation: NativeReviewCallReservation =
                serde_json::from_str(&row.get::<String, _>("call_json"))?;
            reservation.validate()?;
            ensure!(
                reservation.id.to_string() == row.get::<String, _>("id")
                    && reservation.request.run_id() == run_id
                    && i64::from(reservation.request.iteration()) == row.get::<i64, _>("iteration")
                    && category_name(reservation.category) == row.get::<String, _>("category")
                    && reservation.request.id().to_string() == row.get::<String, _>("request_id")
                    && reservation.request.fingerprint()
                        == row.get::<String, _>("request_fingerprint")
                    && i64::from(reservation.attempt) == row.get::<i64, _>("attempt")
                    && fingerprint(&reservation)? == row.get::<String, _>("call_fingerprint"),
                "Native review reservation binding changed"
            );
            let outcome = row
                .get::<Option<String>, _>("outcome_json")
                .map(|raw| serde_json::from_str::<NativeReviewCallOutcome>(&raw))
                .transpose()?;
            if let Some(value) = &outcome {
                value.validate()?;
                ensure!(
                    value.reservation == reservation
                        && Some(fingerprint(value)?)
                            == row.get::<Option<String>, _>("outcome_fingerprint"),
                    "Native review outcome binding changed"
                );
            }
            Ok((reservation, outcome))
        })
        .collect()
}

pub(crate) async fn insert_outcome(
    database: &mut SqliteConnection,
    outcome: &NativeReviewCallOutcome,
) -> Result<()> {
    outcome.validate()?;
    let history = read_calls(database, outcome.reservation.request.run_id()).await?;
    let (reservation, previous) = history
        .iter()
        .find(|(reservation, _)| reservation.id == outcome.reservation.id)
        .context("Native review call was never reserved")?;
    ensure!(
        reservation == &outcome.reservation,
        "Native review outcome belongs to another reservation"
    );
    if let Some(previous) = previous {
        ensure!(
            previous == outcome,
            "Native review outcome changed on replay"
        );
        return Ok(());
    }
    sqlx::query("INSERT INTO optimization_native_review_outcomes(call_id,fingerprint,metadata_json,created_at) VALUES(?,?,?,?)")
        .bind(outcome.reservation.id.to_string())
        .bind(fingerprint(outcome)?)
        .bind(serde_json::to_string(outcome)?)
        .bind(Utc::now().to_rfc3339())
        .execute(database)
        .await?;
    Ok(())
}

pub(crate) async fn budget_usage(
    database: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<(usize, usize, u64, u64, u64)> {
    let mut input = 0_u64;
    let mut output = 0_u64;
    let mut cost = 0_u64;
    let history = read_calls(database, run_id).await?;
    let pending = history
        .iter()
        .filter(|(_, outcome)| outcome.is_none())
        .count();
    for (call, outcome) in &history {
        let interrupted = outcome.as_ref().is_none_or(|value| value.interrupted);
        input = input
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.input_tokens),
                call.input_token_ceiling,
                interrupted,
            ))
            .context("Native review input accounting overflow")?;
        output = output
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.output_tokens),
                call.output_token_ceiling,
                interrupted,
            ))
            .context("Native review output accounting overflow")?;
        cost = cost
            .checked_add(conservative_charge(
                outcome.as_ref().and_then(|value| value.usage.cost_microusd),
                call.cost_ceiling_microusd,
                interrupted,
            ))
            .context("Native review spend accounting overflow")?;
    }
    Ok((history.len(), pending, input, output, cost))
}

fn category_name(value: NativeReviewOperationCategory) -> &'static str {
    match value {
        NativeReviewOperationCategory::BlindSemanticAssessment => "blind_semantic_assessment",
        NativeReviewOperationCategory::RepairTargetFitAssessment => "repair_target_fit_assessment",
    }
}

fn decision_name(value: NativeAdmissionDecision) -> &'static str {
    match value {
        NativeAdmissionDecision::Admitted => "admitted",
        NativeAdmissionDecision::BlindAssessmentMissing => "blind_assessment_missing",
        NativeAdmissionDecision::TargetFitMissing => "target_fit_missing",
        NativeAdmissionDecision::Unsupported => "unsupported",
        NativeAdmissionDecision::Ambiguous => "ambiguous",
        NativeAdmissionDecision::ContextInconsistent => "context_inconsistent",
        NativeAdmissionDecision::LabelMismatch => "label_mismatch",
        NativeAdmissionDecision::TargetMismatch => "target_mismatch",
        NativeAdmissionDecision::ReviewerIssues => "reviewer_issues",
    }
}

fn adapter(error: impl std::fmt::Display) -> QualityAdapterError {
    QualityAdapterError(error.to_string())
}

#[cfg(test)]
mod tests;
