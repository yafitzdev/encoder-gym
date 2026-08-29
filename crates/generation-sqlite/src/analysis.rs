use std::collections::BTreeMap;

use analysis_core::{
    domain::{
        AnalysisFinding, AnalysisReport, EvidenceCategory, FindingIdentity, FindingKind,
        FindingReviewRecord, FindingReviewState,
    },
    ports::{
        AnalysisEvidenceSource, AnalysisEvidenceSourceError, AnalysisFindingQuery,
        AnalysisFindingSort, AnalysisPredictionPage, AnalysisReportQuery, AnalysisStore,
        AnalysisStoreError, BoxFuture, FindingEvidenceQuery, FindingReviewQuery,
        PairedEvaluationPrediction,
    },
};
use chrono::{DateTime, Utc};
use evaluation_core::{
    domain::{EvaluationComparisonReport, EvaluationPrediction, EvaluationRun},
    ports::{EvaluationStore, PredictionQuery},
};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use training_core::domain::LabelProbability;
use uuid::Uuid;

use super::SqliteStore;

impl AnalysisStore for SqliteStore {
    fn create_analysis_report(
        &self,
        report: &AnalysisReport,
    ) -> BoxFuture<'_, Result<(), AnalysisStoreError>> {
        let report = report.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO analysis_reports \
                 (id, evaluation_run_id, minimum_support, prediction_count, error_count, \
                  report_json, fingerprint, created_at, protocol_json, protocol_fingerprint, \
                  source_identity_json, comparison_id, comparison_fingerprint) \
                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(report.id)
            .bind(report.evaluation_run_id)
            .bind(to_i64(report.minimum_support)?)
            .bind(to_i64(report.prediction_count)?)
            .bind(to_i64(report.error_count)?)
            .bind(to_json(&report)?)
            .bind(&report.fingerprint)
            .bind(report.created_at)
            .bind(to_json(&report.protocol)?)
            .bind(&report.protocol_fingerprint)
            .bind(report.source_identity.as_ref().map(to_json).transpose()?)
            .bind(
                report
                    .source_identity
                    .as_ref()
                    .and_then(|source| source.comparison_id),
            )
            .bind(
                report
                    .source_identity
                    .as_ref()
                    .and_then(|source| source.comparison_fingerprint.as_deref()),
            )
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            for finding in &report.findings {
                let identity = FindingIdentity {
                    kind: finding.kind,
                    attributes: finding.attributes.clone(),
                };
                if identity.key() != finding.key {
                    return Err(AnalysisStoreError(format!(
                        "finding key does not match canonical identity: {}",
                        finding.key
                    )));
                }
                sqlx::query(
                    "INSERT INTO analysis_findings \
                     (analysis_report_id, finding_key, rank, kind, support, error_count, \
                      identity_json, finding_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(report.id)
                .bind(&finding.key)
                .bind(to_i64(finding.rank)?)
                .bind(kind_text(finding.kind))
                .bind(to_i64(finding.support)?)
                .bind(to_i64(finding.error_count)?)
                .bind(to_json(&identity)?)
                .bind(to_json(finding)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                for (order, evidence) in report
                    .finding_evidence
                    .get(&finding.key)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    sqlx::query(
                        "INSERT INTO analysis_finding_evidence \
                         (analysis_report_id, finding_key, evidence_order, category, prediction_id, \
                          snapshot_member_id, source_row_id, evidence_json) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(report.id)
                    .bind(&finding.key)
                    .bind(i64::try_from(order).map_err(store_error)?)
                    .bind(category_text(evidence.category))
                    .bind(evidence.prediction_id)
                    .bind(evidence.snapshot_member_id)
                    .bind(evidence.source_row_id)
                    .bind(to_json(evidence)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(store_error)?;
                }
            }
            transaction.commit().await.map_err(store_error)
        })
    }

    fn get_analysis_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AnalysisReport>, AnalysisStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AnalysisReportRecord>(
                "SELECT id, evaluation_run_id, minimum_support, prediction_count, error_count, \
                 report_json, fingerprint, created_at FROM analysis_reports WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(AnalysisReportRecord::into_domain)
            .transpose()
        })
    }

    fn list_analysis_reports(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AnalysisReport>, AnalysisStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, AnalysisReportRecord>(
                "SELECT id, evaluation_run_id, minimum_support, prediction_count, error_count, \
                 report_json, fingerprint, created_at FROM analysis_reports ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(AnalysisReportRecord::into_domain)
            .collect()
        })
    }

    fn query_analysis_reports(
        &self,
        query: AnalysisReportQuery,
    ) -> BoxFuture<'_, Result<Vec<AnalysisReport>, AnalysisStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, evaluation_run_id, minimum_support, prediction_count, error_count, \
                 report_json, fingerprint, created_at FROM analysis_reports",
            );
            if let Some(evaluation_run_id) = query.evaluation_run_id {
                builder
                    .push(" WHERE evaluation_run_id = ")
                    .push_bind(evaluation_run_id);
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<AnalysisReportRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(AnalysisReportRecord::into_domain)
                .collect()
        })
    }

    fn query_analysis_findings(
        &self,
        query: AnalysisFindingQuery,
    ) -> BoxFuture<'_, Result<Vec<AnalysisFinding>, AnalysisStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT finding_json FROM analysis_findings WHERE analysis_report_id = ",
            );
            builder.push_bind(query.analysis_report_id);
            if let Some(kind) = query.kind {
                builder.push(" AND kind = ").push_bind(kind_text(kind));
            }
            if let Some(support) = query.minimum_support {
                builder.push(" AND support >= ").push_bind(to_i64(support)?);
            }
            if let Some(errors) = query.minimum_error_count {
                builder
                    .push(" AND error_count >= ")
                    .push_bind(to_i64(errors)?);
            }
            builder
                .push(finding_order(query.sort))
                .push(" LIMIT ")
                .push_bind(i64::from(query.limit))
                .push(" OFFSET ")
                .push_bind(i64::from(query.offset));
            builder
                .build_query_scalar::<String>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .iter()
                .map(|json| from_json(json))
                .collect()
        })
    }

    fn get_analysis_finding(
        &self,
        report_id: Uuid,
        finding_key: &str,
    ) -> BoxFuture<'_, Result<Option<AnalysisFinding>, AnalysisStoreError>> {
        let finding_key = finding_key.to_owned();
        Box::pin(async move {
            let json: Option<String> = sqlx::query_scalar(
                "SELECT finding_json FROM analysis_findings \
                 WHERE analysis_report_id = ? AND finding_key = ?",
            )
            .bind(report_id)
            .bind(finding_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            json.as_deref().map(from_json).transpose()
        })
    }

    fn query_finding_evidence(
        &self,
        query: FindingEvidenceQuery,
    ) -> BoxFuture<
        '_,
        Result<Vec<analysis_core::domain::FindingEvidenceReference>, AnalysisStoreError>,
    > {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT evidence_json FROM analysis_finding_evidence \
                 WHERE analysis_report_id = ",
            );
            builder
                .push_bind(query.analysis_report_id)
                .push(" AND finding_key = ")
                .push_bind(query.finding_key);
            if let Some(category) = query.category {
                builder
                    .push(" AND category = ")
                    .push_bind(category_text(category));
            }
            builder
                .push(" ORDER BY evidence_order LIMIT ")
                .push_bind(i64::from(query.limit))
                .push(" OFFSET ")
                .push_bind(i64::from(query.offset));
            builder
                .build_query_scalar::<String>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .iter()
                .map(|json| from_json(json))
                .collect()
        })
    }

    fn append_finding_review(
        &self,
        review: &FindingReviewRecord,
    ) -> BoxFuture<'_, Result<(), AnalysisStoreError>> {
        let review = review.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO analysis_finding_reviews \
                 (id, analysis_report_id, finding_key, state, note, \
                  resolution_evaluation_run_id, resolution_comparison_id, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.analysis_report_id)
            .bind(review.finding_key)
            .bind(review_state_text(review.state))
            .bind(review.note)
            .bind(review.resolution_evaluation_run_id)
            .bind(review.resolution_comparison_id)
            .bind(review.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn query_finding_reviews(
        &self,
        query: FindingReviewQuery,
    ) -> BoxFuture<'_, Result<Vec<FindingReviewRecord>, AnalysisStoreError>> {
        Box::pin(async move {
            let rows = match (query.analysis_report_id, query.state) {
                (Some(report_id), Some(state)) => {
                    sqlx::query_as::<_, FindingReviewRecordRow>(
                        "SELECT id, analysis_report_id, finding_key, state, note, \
                     resolution_evaluation_run_id, resolution_comparison_id, created_at \
                     FROM analysis_finding_reviews WHERE analysis_report_id = ? AND state = ? \
                     ORDER BY created_at, id LIMIT ? OFFSET ?",
                    )
                    .bind(report_id)
                    .bind(review_state_text(state))
                    .bind(i64::from(query.limit))
                    .bind(i64::from(query.offset))
                    .fetch_all(&self.pool)
                    .await
                }
                (Some(report_id), None) => {
                    sqlx::query_as::<_, FindingReviewRecordRow>(
                        "SELECT id, analysis_report_id, finding_key, state, note, \
                     resolution_evaluation_run_id, resolution_comparison_id, created_at \
                     FROM analysis_finding_reviews WHERE analysis_report_id = ? \
                     ORDER BY created_at, id LIMIT ? OFFSET ?",
                    )
                    .bind(report_id)
                    .bind(i64::from(query.limit))
                    .bind(i64::from(query.offset))
                    .fetch_all(&self.pool)
                    .await
                }
                (None, Some(state)) => {
                    sqlx::query_as::<_, FindingReviewRecordRow>(
                        "SELECT id, analysis_report_id, finding_key, state, note, \
                     resolution_evaluation_run_id, resolution_comparison_id, created_at \
                     FROM analysis_finding_reviews WHERE state = ? \
                     ORDER BY created_at, id LIMIT ? OFFSET ?",
                    )
                    .bind(review_state_text(state))
                    .bind(i64::from(query.limit))
                    .bind(i64::from(query.offset))
                    .fetch_all(&self.pool)
                    .await
                }
                (None, None) => {
                    sqlx::query_as::<_, FindingReviewRecordRow>(
                        "SELECT id, analysis_report_id, finding_key, state, note, \
                     resolution_evaluation_run_id, resolution_comparison_id, created_at \
                     FROM analysis_finding_reviews ORDER BY created_at, id LIMIT ? OFFSET ?",
                    )
                    .bind(i64::from(query.limit))
                    .bind(i64::from(query.offset))
                    .fetch_all(&self.pool)
                    .await
                }
            }
            .map_err(store_error)?;
            rows.into_iter()
                .map(FindingReviewRecordRow::into_domain)
                .collect()
        })
    }
}

impl AnalysisEvidenceSource for SqliteStore {
    fn get_evaluation_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationRun>, AnalysisEvidenceSourceError>> {
        Box::pin(async move {
            EvaluationStore::get_evaluation_run(self, id)
                .await
                .map_err(evidence_error)
        })
    }

    fn get_evaluation_comparison(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationComparisonReport>, AnalysisEvidenceSourceError>>
    {
        Box::pin(async move {
            EvaluationStore::get_comparison(self, id)
                .await
                .map_err(evidence_error)
        })
    }

    fn count_evaluation_predictions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<u64, AnalysisEvidenceSourceError>> {
        Box::pin(async move {
            EvaluationStore::count_predictions(self, run_id)
                .await
                .map_err(evidence_error)
        })
    }

    fn page_evaluation_predictions(
        &self,
        page: AnalysisPredictionPage,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, AnalysisEvidenceSourceError>> {
        Box::pin(async move {
            let offset = u32::try_from(page.offset).map_err(evidence_error)?;
            EvaluationStore::query_predictions(
                self,
                PredictionQuery::page(page.evaluation_run_id, page.limit, offset),
            )
            .await
            .map_err(evidence_error)
        })
    }

    fn page_paired_evaluation_predictions(
        &self,
        left_run_id: Uuid,
        right_run_id: Uuid,
        limit: u32,
        offset: u64,
    ) -> BoxFuture<'_, Result<Vec<PairedEvaluationPrediction>, AnalysisEvidenceSourceError>> {
        Box::pin(async move {
            let offset = i64::try_from(offset).map_err(evidence_error)?;
            sqlx::query_as::<_, PairedPredictionRecord>(
                "SELECT l.id AS left_id, l.evaluation_run_id AS left_run_id, \
                 l.snapshot_member_id, l.source_row_id, l.text, l.expected_label, \
                 l.predicted_label AS left_predicted_label, l.confidence AS left_confidence, \
                 l.probabilities_json AS left_probabilities_json, l.dimensions_json, \
                 l.created_at AS left_created_at, r.id AS right_id, \
                 r.evaluation_run_id AS right_run_id, \
                 r.predicted_label AS right_predicted_label, r.confidence AS right_confidence, \
                 r.probabilities_json AS right_probabilities_json, \
                 r.created_at AS right_created_at \
                 FROM evaluation_predictions l JOIN evaluation_predictions r \
                 ON r.snapshot_member_id = l.snapshot_member_id \
                 WHERE l.evaluation_run_id = ? AND r.evaluation_run_id = ? \
                 ORDER BY l.snapshot_member_id, l.id, r.id LIMIT ? OFFSET ?",
            )
            .bind(left_run_id)
            .bind(right_run_id)
            .bind(i64::from(limit))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(evidence_error)?
            .into_iter()
            .map(PairedPredictionRecord::into_domain)
            .collect()
        })
    }
}

#[derive(Debug, FromRow)]
struct AnalysisReportRecord {
    id: Uuid,
    evaluation_run_id: Uuid,
    minimum_support: i64,
    prediction_count: i64,
    error_count: i64,
    report_json: String,
    fingerprint: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct FindingReviewRecordRow {
    id: Uuid,
    analysis_report_id: Uuid,
    finding_key: String,
    state: String,
    note: Option<String>,
    resolution_evaluation_run_id: Option<Uuid>,
    resolution_comparison_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct PairedPredictionRecord {
    left_id: Uuid,
    left_run_id: Uuid,
    snapshot_member_id: Uuid,
    source_row_id: Uuid,
    text: String,
    expected_label: String,
    left_predicted_label: String,
    left_confidence: f64,
    left_probabilities_json: String,
    dimensions_json: String,
    left_created_at: DateTime<Utc>,
    right_id: Uuid,
    right_run_id: Uuid,
    right_predicted_label: String,
    right_confidence: f64,
    right_probabilities_json: String,
    right_created_at: DateTime<Utc>,
}

impl PairedPredictionRecord {
    fn into_domain(self) -> Result<PairedEvaluationPrediction, AnalysisEvidenceSourceError> {
        let dimensions: BTreeMap<String, String> =
            serde_json::from_str(&self.dimensions_json).map_err(evidence_error)?;
        let left_probabilities: Vec<LabelProbability> =
            serde_json::from_str(&self.left_probabilities_json).map_err(evidence_error)?;
        let right_probabilities: Vec<LabelProbability> =
            serde_json::from_str(&self.right_probabilities_json).map_err(evidence_error)?;
        Ok(PairedEvaluationPrediction {
            left: EvaluationPrediction {
                id: self.left_id,
                evaluation_run_id: self.left_run_id,
                snapshot_member_id: self.snapshot_member_id,
                source_row_id: self.source_row_id,
                text: self.text.clone(),
                expected_label: self.expected_label.clone(),
                predicted_label: self.left_predicted_label,
                confidence: self.left_confidence,
                probabilities: left_probabilities,
                dimensions: dimensions.clone(),
                created_at: self.left_created_at,
            },
            right: EvaluationPrediction {
                id: self.right_id,
                evaluation_run_id: self.right_run_id,
                snapshot_member_id: self.snapshot_member_id,
                source_row_id: self.source_row_id,
                text: self.text,
                expected_label: self.expected_label,
                predicted_label: self.right_predicted_label,
                confidence: self.right_confidence,
                probabilities: right_probabilities,
                dimensions,
                created_at: self.right_created_at,
            },
        })
    }
}

impl FindingReviewRecordRow {
    fn into_domain(self) -> Result<FindingReviewRecord, AnalysisStoreError> {
        Ok(FindingReviewRecord {
            id: self.id,
            analysis_report_id: self.analysis_report_id,
            finding_key: self.finding_key,
            state: parse_review_state(&self.state)?,
            note: self.note,
            resolution_evaluation_run_id: self.resolution_evaluation_run_id,
            resolution_comparison_id: self.resolution_comparison_id,
            created_at: self.created_at,
        })
    }
}

impl AnalysisReportRecord {
    fn into_domain(self) -> Result<AnalysisReport, AnalysisStoreError> {
        let report: AnalysisReport =
            serde_json::from_str(&self.report_json).map_err(store_error)?;
        if report.id != self.id
            || report.evaluation_run_id != self.evaluation_run_id
            || report.minimum_support != to_u64(self.minimum_support)?
            || report.prediction_count != to_u64(self.prediction_count)?
            || report.error_count != to_u64(self.error_count)?
            || self
                .fingerprint
                .as_deref()
                .is_some_and(|fingerprint| report.fingerprint != fingerprint)
            || report.created_at != self.created_at
        {
            return Err(AnalysisStoreError(
                "analysis report columns do not match its immutable payload".into(),
            ));
        }
        Ok(report)
    }
}

fn to_i64(value: u64) -> Result<i64, AnalysisStoreError> {
    i64::try_from(value).map_err(store_error)
}

fn to_u64(value: i64) -> Result<u64, AnalysisStoreError> {
    u64::try_from(value).map_err(store_error)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, AnalysisStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, AnalysisStoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn kind_text(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::ExpectedLabel => "expected_label",
        FindingKind::PredictedLabel => "predicted_label",
        FindingKind::ConfusionPair => "confusion_pair",
        FindingKind::DimensionValue => "dimension_value",
        FindingKind::Cell => "cell",
        FindingKind::DimensionIntersection => "dimension_intersection",
        FindingKind::ConfidenceBand => "confidence_band",
        FindingKind::CorrectnessConfidence => "correctness_confidence",
    }
}

fn finding_order(sort: AnalysisFindingSort) -> &'static str {
    match sort {
        AnalysisFindingSort::Rank => " ORDER BY rank, finding_key",
        AnalysisFindingSort::ErrorRate => {
            " ORDER BY CAST(json_extract(finding_json, '$.error_rate') AS REAL) DESC, rank, finding_key"
        }
        AnalysisFindingSort::ErrorCount => " ORDER BY error_count DESC, rank, finding_key",
        AnalysisFindingSort::ErrorRateLift => {
            " ORDER BY CAST(json_extract(finding_json, '$.error_rate_lift') AS REAL) DESC, rank, finding_key"
        }
        AnalysisFindingSort::ErrorShare => {
            " ORDER BY CAST(json_extract(finding_json, '$.error_share') AS REAL) DESC, rank, finding_key"
        }
        AnalysisFindingSort::HighConfidenceErrorSeverity => {
            " ORDER BY CAST(json_extract(finding_json, '$.high_confidence_error_severity') AS REAL) DESC, rank, finding_key"
        }
        AnalysisFindingSort::Support => " ORDER BY support DESC, rank, finding_key",
    }
}

fn category_text(category: EvidenceCategory) -> &'static str {
    match category {
        EvidenceCategory::HighestConfidenceError => "highest_confidence_error",
        EvidenceCategory::LowestMarginError => "lowest_margin_error",
        EvidenceCategory::MedianConfidenceError => "median_confidence_error",
        EvidenceCategory::StableError => "stable_error",
        EvidenceCategory::CorrectContrast => "correct_contrast",
    }
}

fn review_state_text(state: FindingReviewState) -> &'static str {
    match state {
        FindingReviewState::Open => "open",
        FindingReviewState::Acknowledged => "acknowledged",
        FindingReviewState::AcceptedLimitation => "accepted_limitation",
        FindingReviewState::CandidateForMoreData => "candidate_for_more_data",
        FindingReviewState::CandidateForLabelSchemaReview => "candidate_for_label_schema_review",
        FindingReviewState::ResolvedByLaterEvidence => "resolved_by_later_evidence",
    }
}

fn parse_review_state(value: &str) -> Result<FindingReviewState, AnalysisStoreError> {
    match value {
        "open" => Ok(FindingReviewState::Open),
        "acknowledged" => Ok(FindingReviewState::Acknowledged),
        "accepted_limitation" => Ok(FindingReviewState::AcceptedLimitation),
        "candidate_for_more_data" => Ok(FindingReviewState::CandidateForMoreData),
        "candidate_for_label_schema_review" => {
            Ok(FindingReviewState::CandidateForLabelSchemaReview)
        }
        "resolved_by_later_evidence" => Ok(FindingReviewState::ResolvedByLaterEvidence),
        other => Err(AnalysisStoreError(format!(
            "unknown finding review state: {other}"
        ))),
    }
}

fn store_error(error: impl std::fmt::Display) -> AnalysisStoreError {
    AnalysisStoreError(error.to_string())
}

fn evidence_error(error: impl std::fmt::Display) -> AnalysisEvidenceSourceError {
    AnalysisEvidenceSourceError(error.to_string())
}
