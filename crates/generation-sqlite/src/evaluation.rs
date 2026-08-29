use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotSplit;
use evaluation_core::{
    domain::{
        EvaluationComparisonReport, EvaluationExample, EvaluationPrediction, EvaluationProtocol,
        EvaluationRun, EvaluationRunState, EvaluationSourceIdentity, ModelSelectionReport,
    },
    ports::{
        BoxFuture, EvaluationExampleSource, EvaluationExampleSourceError, EvaluationRunQuery,
        EvaluationStore, EvaluationStoreError, PredictionQuery,
    },
};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;

use super::SqliteStore;

const RUN_SELECT: &str = "SELECT id, checkpoint_id, snapshot_id, split, input_fingerprint, protocol_json, protocol_fingerprint, source_identity_json, state, total_examples, processed_examples, completed_batches, current_batch, correct_predictions, elapsed_milliseconds, cancel_requested, example_count, metrics_json, error_message, created_at, updated_at FROM evaluation_runs";
const PREDICTION_SELECT: &str = "SELECT id, evaluation_run_id, snapshot_member_id, source_row_id, text, expected_label, predicted_label, confidence, probabilities_json, dimensions_json, created_at FROM evaluation_predictions";

impl EvaluationStore for SqliteStore {
    fn create_evaluation_run(
        &self,
        run: &EvaluationRun,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let run = run.clone();
        Box::pin(async move {
            sqlx::query("INSERT INTO evaluation_runs (id, checkpoint_id, snapshot_id, split, input_fingerprint, protocol_json, protocol_fingerprint, source_identity_json, state, total_examples, processed_examples, completed_batches, current_batch, correct_predictions, elapsed_milliseconds, cancel_requested, example_count, metrics_json, error_message, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(run.id).bind(run.checkpoint_id).bind(run.snapshot_id).bind(split_text(run.split))
                .bind(run.input_fingerprint).bind(to_json(&run.protocol)?).bind(run.protocol_fingerprint)
                .bind(to_json(&run.source_identity)?).bind(run_state_text(run.state))
                .bind(to_i64(run.total_examples)?).bind(to_i64(run.processed_examples)?)
                .bind(to_i64(run.completed_batches)?).bind(to_i64(run.current_batch)?).bind(to_i64(run.correct_predictions)?)
                .bind(to_i64(run.elapsed_milliseconds)?).bind(run.cancel_requested)
                .bind(to_i64(run.example_count)?).bind(run.metrics.as_ref().map(to_json).transpose()?)
                .bind(run.error_message).bind(run.created_at).bind(run.updated_at)
                .execute(&self.pool).await.map_err(store_error)?;
            Ok(())
        })
    }

    fn save_evaluation_run(
        &self,
        run: &EvaluationRun,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let run = run.clone();
        Box::pin(async move {
            let result = sqlx::query("UPDATE evaluation_runs SET split = ?, input_fingerprint = ?, protocol_json = ?, protocol_fingerprint = ?, source_identity_json = ?, state = ?, total_examples = ?, processed_examples = ?, completed_batches = ?, current_batch = ?, correct_predictions = ?, elapsed_milliseconds = ?, cancel_requested = CASE WHEN ? THEN 1 ELSE cancel_requested END, example_count = ?, metrics_json = ?, error_message = ?, updated_at = ? WHERE id = ?")
                .bind(split_text(run.split)).bind(run.input_fingerprint).bind(to_json(&run.protocol)?)
                .bind(run.protocol_fingerprint).bind(to_json(&run.source_identity)?).bind(run_state_text(run.state))
                .bind(to_i64(run.total_examples)?).bind(to_i64(run.processed_examples)?)
                .bind(to_i64(run.completed_batches)?).bind(to_i64(run.current_batch)?).bind(to_i64(run.correct_predictions)?)
                .bind(to_i64(run.elapsed_milliseconds)?).bind(run.cancel_requested)
                .bind(to_i64(run.example_count)?).bind(run.metrics.as_ref().map(to_json).transpose()?)
                .bind(run.error_message).bind(run.updated_at).bind(run.id)
                .execute(&self.pool).await.map_err(store_error)?;
            require_one(result.rows_affected(), run.id)
        })
    }

    fn get_evaluation_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationRun>, EvaluationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, EvaluationRunRecord>(&format!("{RUN_SELECT} WHERE id = ?"))
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(store_error)?
                .map(EvaluationRunRecord::into_domain)
                .transpose()
        })
    }

    fn list_evaluation_runs(
        &self,
    ) -> BoxFuture<'_, Result<Vec<EvaluationRun>, EvaluationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, EvaluationRunRecord>(&format!(
                "{RUN_SELECT} ORDER BY created_at, id"
            ))
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(EvaluationRunRecord::into_domain)
            .collect()
        })
    }

    fn query_evaluation_runs(
        &self,
        query: EvaluationRunQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationRun>, EvaluationStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(RUN_SELECT);
            let mut conditions = Vec::new();
            if query.checkpoint_id.is_some() {
                conditions.push("checkpoint_id");
            }
            if query.snapshot_id.is_some() {
                conditions.push("snapshot_id");
            }
            if query.state.is_some() {
                conditions.push("state");
            }
            for (index, condition) in conditions.iter().enumerate() {
                builder
                    .push(if index == 0 { " WHERE " } else { " AND " })
                    .push(*condition)
                    .push(" = ");
                match *condition {
                    "checkpoint_id" => {
                        builder.push_bind(query.checkpoint_id.expect("present"));
                    }
                    "snapshot_id" => {
                        builder.push_bind(query.snapshot_id.expect("present"));
                    }
                    _ => {
                        builder.push_bind(run_state_text(query.state.expect("present")));
                    }
                };
            }
            builder
                .push(" ORDER BY created_at, id LIMIT ")
                .push_bind(i64::from(query.limit));
            builder.push(" OFFSET ").push_bind(i64::from(query.offset));
            builder
                .build_query_as::<EvaluationRunRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(EvaluationRunRecord::into_domain)
                .collect()
        })
    }

    fn insert_predictions(
        &self,
        predictions: &[EvaluationPrediction],
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let predictions = predictions.to_vec();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            insert_prediction_rows(&mut transaction, predictions).await?;
            transaction.commit().await.map_err(store_error)
        })
    }

    fn commit_evaluation_batch(
        &self,
        run: &EvaluationRun,
        predictions: &[EvaluationPrediction],
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let run = run.clone();
        let predictions = predictions.to_vec();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            insert_prediction_rows(&mut transaction, predictions).await?;
            let result = sqlx::query("UPDATE evaluation_runs SET state = ?, total_examples = ?, processed_examples = ?, completed_batches = ?, current_batch = ?, correct_predictions = ?, elapsed_milliseconds = ?, example_count = ?, updated_at = ? WHERE id = ?")
                .bind(run_state_text(run.state)).bind(to_i64(run.total_examples)?)
                .bind(to_i64(run.processed_examples)?).bind(to_i64(run.completed_batches)?).bind(to_i64(run.current_batch)?)
                .bind(to_i64(run.correct_predictions)?).bind(to_i64(run.elapsed_milliseconds)?)
                .bind(to_i64(run.example_count)?).bind(run.updated_at).bind(run.id)
                .execute(&mut *transaction).await.map_err(store_error)?;
            require_one(result.rows_affected(), run.id)?;
            transaction.commit().await.map_err(store_error)
        })
    }

    fn request_evaluation_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, EvaluationStoreError>> {
        Box::pin(async move {
            let result = sqlx::query("UPDATE evaluation_runs SET cancel_requested = 1, updated_at = ? WHERE id = ? AND state IN ('queued', 'running')")
                .bind(Utc::now()).bind(id).execute(&self.pool).await.map_err(store_error)?;
            Ok(result.rows_affected() == 1)
        })
    }

    fn list_predictions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, EvaluationStoreError>> {
        self.query_predictions(PredictionQuery::page(run_id, u32::MAX, 0))
    }

    fn query_predictions(
        &self,
        query: PredictionQuery,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, EvaluationStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(PREDICTION_SELECT);
            push_prediction_filters(&mut builder, &query);
            builder
                .push(" ORDER BY snapshot_member_id, id LIMIT ")
                .push_bind(i64::from(query.limit));
            builder.push(" OFFSET ").push_bind(i64::from(query.offset));
            builder
                .build_query_as::<PredictionRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(PredictionRecord::into_domain)
                .collect()
        })
    }

    fn count_predictions(&self, run_id: Uuid) -> BoxFuture<'_, Result<u64, EvaluationStoreError>> {
        Box::pin(async move {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM evaluation_predictions WHERE evaluation_run_id = ?",
            )
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .map_err(store_error)?;
            u64::try_from(count).map_err(store_error)
        })
    }

    fn create_comparison(
        &self,
        report: &EvaluationComparisonReport,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let report = report.clone();
        Box::pin(async move {
            sqlx::query("INSERT INTO evaluation_comparisons (id, left_run_id, right_run_id, cohort_fingerprint, protocol_fingerprint, report_json, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(report.id).bind(report.left_run_id).bind(report.right_run_id)
                .bind(&report.cohort_fingerprint).bind(&report.protocol_fingerprint)
                .bind(to_json(&report)?).bind(&report.fingerprint).bind(report.created_at)
                .execute(&self.pool).await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_comparison(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationComparisonReport>, EvaluationStoreError>> {
        Box::pin(async move {
            let json: Option<String> =
                sqlx::query_scalar("SELECT report_json FROM evaluation_comparisons WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(store_error)?;
            json.as_deref().map(from_json).transpose()
        })
    }

    fn list_comparisons(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<EvaluationComparisonReport>, EvaluationStoreError>> {
        Box::pin(async move {
            sqlx::query_scalar::<_, String>("SELECT report_json FROM evaluation_comparisons ORDER BY created_at, id LIMIT ? OFFSET ?")
                .bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&self.pool).await.map_err(store_error)?
                .iter().map(|json| from_json(json)).collect()
        })
    }

    fn create_selection(
        &self,
        report: &ModelSelectionReport,
    ) -> BoxFuture<'_, Result<(), EvaluationStoreError>> {
        let report = report.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query("INSERT INTO model_selection_reports (id, report_json, fingerprint, selected_evaluation_run_id, selected_checkpoint_id, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(report.id).bind(to_json(&report)?).bind(&report.fingerprint)
                .bind(report.selected_run_id).bind(report.selected_checkpoint_id).bind(report.created_at)
                .execute(&mut *transaction).await.map_err(store_error)?;
            for (order, run_id) in report.candidate_run_ids.iter().enumerate() {
                sqlx::query("INSERT INTO model_selection_candidates (selection_report_id, evaluation_run_id, candidate_order) VALUES (?, ?, ?)")
                    .bind(report.id).bind(run_id).bind(i64::try_from(order).map_err(store_error)?)
                    .execute(&mut *transaction).await.map_err(store_error)?;
            }
            transaction.commit().await.map_err(store_error)
        })
    }

    fn get_selection(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ModelSelectionReport>, EvaluationStoreError>> {
        Box::pin(async move {
            let json: Option<String> =
                sqlx::query_scalar("SELECT report_json FROM model_selection_reports WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(store_error)?;
            json.as_deref().map(from_json).transpose()
        })
    }

    fn list_selections(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<ModelSelectionReport>, EvaluationStoreError>> {
        Box::pin(async move {
            sqlx::query_scalar::<_, String>("SELECT report_json FROM model_selection_reports ORDER BY created_at, id LIMIT ? OFFSET ?")
                .bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&self.pool).await.map_err(store_error)?
                .iter().map(|json| from_json(json)).collect()
        })
    }
}

impl EvaluationExampleSource for SqliteStore {
    fn count_examples(
        &self,
        snapshot_id: Uuid,
        split: SnapshotSplit,
    ) -> BoxFuture<'_, Result<u64, EvaluationExampleSourceError>> {
        Box::pin(async move {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM dataset_snapshot_members WHERE snapshot_id = ? AND split = ?",
            )
            .bind(snapshot_id)
            .bind(split_text(split))
            .fetch_one(&self.pool)
            .await
            .map_err(source_error)?;
            u64::try_from(count).map_err(source_error)
        })
    }

    fn query_examples(
        &self,
        snapshot_id: Uuid,
        split: SnapshotSplit,
        limit: u32,
        offset: u64,
    ) -> BoxFuture<'_, Result<Vec<EvaluationExample>, EvaluationExampleSourceError>> {
        Box::pin(async move {
            sqlx::query_as::<_, EvaluationExampleRecord>("SELECT id, source_row_id, text, label, dimensions_json FROM dataset_snapshot_members WHERE snapshot_id = ? AND split = ? ORDER BY id LIMIT ? OFFSET ?")
                .bind(snapshot_id).bind(split_text(split)).bind(i64::from(limit))
                .bind(i64::try_from(offset).map_err(source_error)?)
                .fetch_all(&self.pool).await.map_err(source_error)?.into_iter()
                .map(EvaluationExampleRecord::into_domain).collect()
        })
    }
}

async fn insert_prediction_rows(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    predictions: Vec<EvaluationPrediction>,
) -> Result<(), EvaluationStoreError> {
    for prediction in predictions {
        sqlx::query("INSERT INTO evaluation_predictions (id, evaluation_run_id, snapshot_member_id, source_row_id, text, expected_label, predicted_label, confidence, probabilities_json, dimensions_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(prediction.id).bind(prediction.evaluation_run_id).bind(prediction.snapshot_member_id)
            .bind(prediction.source_row_id).bind(prediction.text).bind(prediction.expected_label)
            .bind(prediction.predicted_label).bind(prediction.confidence)
            .bind(to_json(&prediction.probabilities)?).bind(to_json(&prediction.dimensions)?)
            .bind(prediction.created_at).execute(&mut **transaction).await.map_err(store_error)?;
    }
    Ok(())
}

fn push_prediction_filters(builder: &mut QueryBuilder<'_, Sqlite>, query: &PredictionQuery) {
    builder
        .push(" WHERE evaluation_run_id = ")
        .push_bind(query.run_id);
    if let Some(correct) = query.correct {
        builder.push(if correct {
            " AND expected_label = predicted_label"
        } else {
            " AND expected_label <> predicted_label"
        });
    }
    if let Some(value) = &query.expected_label {
        builder
            .push(" AND expected_label = ")
            .push_bind(value.clone());
    }
    if let Some(value) = &query.predicted_label {
        builder
            .push(" AND predicted_label = ")
            .push_bind(value.clone());
    }
    if let Some(value) = query.minimum_confidence {
        builder.push(" AND confidence >= ").push_bind(value);
    }
    if let Some(value) = query.maximum_confidence {
        builder.push(" AND confidence <= ").push_bind(value);
    }
    if let Some(value) = query.snapshot_member_id {
        builder.push(" AND snapshot_member_id = ").push_bind(value);
    }
    if let Some(value) = query.source_row_id {
        builder.push(" AND source_row_id = ").push_bind(value);
    }
    for (name, value) in &query.dimensions {
        let path = format!(
            "$.{}",
            serde_json::to_string(name).expect("string serialization")
        );
        builder
            .push(" AND json_extract(dimensions_json, ")
            .push_bind(path)
            .push(") = ")
            .push_bind(value.clone());
    }
}

#[derive(Debug, FromRow)]
struct EvaluationRunRecord {
    id: Uuid,
    checkpoint_id: Uuid,
    snapshot_id: Uuid,
    split: String,
    input_fingerprint: Option<String>,
    protocol_json: String,
    protocol_fingerprint: String,
    source_identity_json: String,
    state: String,
    total_examples: i64,
    processed_examples: i64,
    completed_batches: i64,
    current_batch: i64,
    correct_predictions: i64,
    elapsed_milliseconds: i64,
    cancel_requested: bool,
    example_count: i64,
    metrics_json: Option<String>,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl EvaluationRunRecord {
    fn into_domain(self) -> Result<EvaluationRun, EvaluationStoreError> {
        Ok(EvaluationRun {
            id: self.id,
            checkpoint_id: self.checkpoint_id,
            snapshot_id: self.snapshot_id,
            split: parse_split(&self.split)?,
            input_fingerprint: self
                .input_fingerprint
                .unwrap_or_else(|| "legacy:unavailable".into()),
            protocol: from_json::<EvaluationProtocol>(&self.protocol_json)?,
            protocol_fingerprint: self.protocol_fingerprint,
            source_identity: from_json::<EvaluationSourceIdentity>(&self.source_identity_json)?,
            state: parse_run_state(&self.state)?,
            total_examples: to_u64(self.total_examples)?,
            processed_examples: to_u64(self.processed_examples)?,
            completed_batches: to_u64(self.completed_batches)?,
            current_batch: to_u64(self.current_batch)?,
            correct_predictions: to_u64(self.correct_predictions)?,
            elapsed_milliseconds: to_u64(self.elapsed_milliseconds)?,
            cancel_requested: self.cancel_requested,
            example_count: to_u64(self.example_count)?,
            metrics: self.metrics_json.as_deref().map(from_json).transpose()?,
            error_message: self.error_message,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct PredictionRecord {
    id: Uuid,
    evaluation_run_id: Uuid,
    snapshot_member_id: Uuid,
    source_row_id: Uuid,
    text: String,
    expected_label: String,
    predicted_label: String,
    confidence: f64,
    probabilities_json: String,
    dimensions_json: String,
    created_at: DateTime<Utc>,
}

impl PredictionRecord {
    fn into_domain(self) -> Result<EvaluationPrediction, EvaluationStoreError> {
        Ok(EvaluationPrediction {
            id: self.id,
            evaluation_run_id: self.evaluation_run_id,
            snapshot_member_id: self.snapshot_member_id,
            source_row_id: self.source_row_id,
            text: self.text,
            expected_label: self.expected_label,
            predicted_label: self.predicted_label,
            confidence: self.confidence,
            probabilities: from_json(&self.probabilities_json)?,
            dimensions: from_json(&self.dimensions_json)?,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct EvaluationExampleRecord {
    id: Uuid,
    source_row_id: Uuid,
    text: String,
    label: String,
    dimensions_json: String,
}
impl EvaluationExampleRecord {
    fn into_domain(self) -> Result<EvaluationExample, EvaluationExampleSourceError> {
        Ok(EvaluationExample {
            snapshot_member_id: self.id,
            source_row_id: self.source_row_id,
            text: self.text,
            expected_label: self.label,
            dimensions: serde_json::from_str(&self.dimensions_json).map_err(source_error)?,
        })
    }
}

fn split_text(split: SnapshotSplit) -> &'static str {
    match split {
        SnapshotSplit::Train => "train",
        SnapshotSplit::Validation => "validation",
        SnapshotSplit::Test => "test",
    }
}
fn parse_split(value: &str) -> Result<SnapshotSplit, EvaluationStoreError> {
    match value {
        "train" => Ok(SnapshotSplit::Train),
        "validation" => Ok(SnapshotSplit::Validation),
        "test" => Ok(SnapshotSplit::Test),
        other => Err(EvaluationStoreError(format!(
            "unknown snapshot split: {other}"
        ))),
    }
}
fn run_state_text(state: EvaluationRunState) -> &'static str {
    match state {
        EvaluationRunState::Queued => "queued",
        EvaluationRunState::Running => "running",
        EvaluationRunState::Completed => "completed",
        EvaluationRunState::Failed => "failed",
        EvaluationRunState::Cancelled => "cancelled",
    }
}
fn parse_run_state(value: &str) -> Result<EvaluationRunState, EvaluationStoreError> {
    match value {
        "queued" => Ok(EvaluationRunState::Queued),
        "running" => Ok(EvaluationRunState::Running),
        "completed" => Ok(EvaluationRunState::Completed),
        "failed" => Ok(EvaluationRunState::Failed),
        "cancelled" => Ok(EvaluationRunState::Cancelled),
        other => Err(EvaluationStoreError(format!(
            "unknown evaluation run state: {other}"
        ))),
    }
}
fn require_one(rows: u64, id: Uuid) -> Result<(), EvaluationStoreError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(EvaluationStoreError(format!(
            "evaluation run not found: {id}"
        )))
    }
}
fn to_i64(value: u64) -> Result<i64, EvaluationStoreError> {
    i64::try_from(value).map_err(store_error)
}
fn to_u64(value: i64) -> Result<u64, EvaluationStoreError> {
    u64::try_from(value).map_err(store_error)
}
fn to_json<T: serde::Serialize>(value: &T) -> Result<String, EvaluationStoreError> {
    serde_json::to_string(value).map_err(store_error)
}
fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, EvaluationStoreError> {
    serde_json::from_str(value).map_err(store_error)
}
fn store_error(error: impl std::fmt::Display) -> EvaluationStoreError {
    EvaluationStoreError(error.to_string())
}
fn source_error(error: impl std::fmt::Display) -> EvaluationExampleSourceError {
    EvaluationExampleSourceError(error.to_string())
}
