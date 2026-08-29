use std::{fs::File, io::BufWriter};

use analysis_core::{
    domain::{
        ComparisonEvidenceCategory, EvidenceCategory, FindingKind, FindingRankingPolicy,
        FindingReviewRecord, FindingReviewState,
    },
    ports::{
        AnalysisEvidenceSource, AnalysisFindingQuery, AnalysisFindingSort, AnalysisPredictionPage,
        AnalysisReportQuery, AnalysisStore, FindingEvidenceQuery, FindingReviewQuery,
    },
    protocol::AnalysisProtocol,
    runner::run_analysis,
};
use anyhow::Context;
use chrono::Utc;
use evaluation_core::{
    domain::EvaluationPrediction,
    ports::{EvaluationStore, PredictionQuery},
};
use serde::Serialize;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::cli::{
    AnalysisCommand, AnalysisEvidenceCategoryArg, AnalysisFindingKindArg, AnalysisFindingSortArg,
    AnalysisRankingArg, ComparisonDiagnosisCategoryArg, EvidenceExportFormatArg,
    FindingReviewStateArg,
};

pub async fn execute(command: AnalysisCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        AnalysisCommand::Create {
            evaluation_run_id,
            minimum_support,
            comparison_id,
            maximum_examples,
            confidence_thresholds,
            ranking,
            dimension_intersections,
            include_correct_contrasts,
            seed,
        } => {
            let protocol = AnalysisProtocol {
                minimum_support,
                finding_kinds: vec![
                    FindingKind::ExpectedLabel,
                    FindingKind::PredictedLabel,
                    FindingKind::ConfusionPair,
                    FindingKind::DimensionValue,
                    FindingKind::Cell,
                    FindingKind::DimensionIntersection,
                    FindingKind::ConfidenceBand,
                    FindingKind::CorrectnessConfidence,
                ],
                dimension_intersections: parse_intersections(&dimension_intersections),
                maximum_representative_examples: maximum_examples,
                confidence_thresholds,
                ranking_policy: ranking.into(),
                include_correct_contrasts,
                deterministic_seed: seed,
                comparison_id,
            };
            print_json(&run_analysis(store, store, evaluation_run_id, protocol).await?)?;
        }
        AnalysisCommand::List {
            evaluation_run_id,
            page,
        } => {
            let reports = store
                .query_analysis_reports(AnalysisReportQuery {
                    evaluation_run_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&reports, reports.len(), page)?;
        }
        AnalysisCommand::Show { id } => print_json(&require_report(store, id).await?)?,
        AnalysisCommand::Findings {
            report_id,
            kind,
            minimum_support,
            minimum_error_count,
            sort,
            page,
        } => {
            require_report(store, report_id).await?;
            let findings = store
                .query_analysis_findings(AnalysisFindingQuery {
                    analysis_report_id: report_id,
                    kind: kind.map(Into::into),
                    minimum_support,
                    minimum_error_count,
                    sort: sort.into(),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&findings, findings.len(), page)?;
        }
        AnalysisCommand::Finding {
            report_id,
            finding_key,
        } => print_json(&require_finding(store, report_id, &finding_key).await?)?,
        AnalysisCommand::Evidence {
            report_id,
            finding_key,
            category,
            format,
            output,
            page,
        } => {
            let report = require_report(store, report_id).await?;
            require_finding(store, report_id, &finding_key).await?;
            let references = store
                .query_finding_evidence(FindingEvidenceQuery {
                    analysis_report_id: report_id,
                    finding_key,
                    category: category.map(Into::into),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            let mut evidence = Vec::with_capacity(references.len());
            for reference in references {
                let mut query = PredictionQuery::page(report.evaluation_run_id, 1, 0);
                query.snapshot_member_id = Some(reference.snapshot_member_id);
                let prediction = store
                    .query_predictions(query)
                    .await?
                    .into_iter()
                    .next()
                    .with_context(|| {
                        format!("evidence prediction not found: {}", reference.prediction_id)
                    })?;
                anyhow::ensure!(
                    prediction.id == reference.prediction_id,
                    "evidence prediction identity mismatch for {}",
                    reference.prediction_id
                );
                evidence.push(ResolvedEvidence {
                    reference,
                    prediction,
                });
            }
            if let Some(format) = format {
                let output = output.context("--file is required when --format is supplied")?;
                export_evidence(&output, format, &evidence)?;
                print_json(&serde_json::json!({
                    "exported": true,
                    "artifact": "analysis_finding_evidence",
                    "format": format_name(format),
                    "rows": evidence.len(),
                    "output": output,
                }))?;
            } else {
                anyhow::ensure!(output.is_none(), "--format is required with --file");
                crate::presentation::print_page(&evidence, evidence.len(), page)?;
            }
        }
        AnalysisCommand::HighConfidenceErrors { report_id, page } => {
            let report = require_report(store, report_id).await?;
            let threshold = report
                .protocol
                .confidence_thresholds
                .last()
                .copied()
                .unwrap_or(0.8);
            let mut source_offset = 0_u64;
            let mut skipped = 0_u64;
            let mut predictions = Vec::new();
            while source_offset < report.prediction_count && predictions.len() < page.limit as usize
            {
                let batch = store
                    .page_evaluation_predictions(AnalysisPredictionPage {
                        evaluation_run_id: report.evaluation_run_id,
                        limit: 1_000,
                        offset: source_offset,
                    })
                    .await?;
                if batch.is_empty() {
                    break;
                }
                source_offset += batch.len() as u64;
                for prediction in batch {
                    if prediction.expected_label == prediction.predicted_label
                        || prediction.confidence < threshold
                    {
                        continue;
                    }
                    if skipped < u64::from(page.offset) {
                        skipped += 1;
                    } else if predictions.len() < page.limit as usize {
                        predictions.push(prediction);
                    }
                }
            }
            crate::presentation::print_page(&predictions, predictions.len(), page)?;
        }
        AnalysisCommand::WeakCells { report_id, page } => {
            require_report(store, report_id).await?;
            let findings = store
                .query_analysis_findings(AnalysisFindingQuery {
                    analysis_report_id: report_id,
                    kind: Some(FindingKind::Cell),
                    minimum_support: None,
                    minimum_error_count: Some(1),
                    sort: AnalysisFindingSort::ErrorRate,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&findings, findings.len(), page)?;
        }
        AnalysisCommand::ComparisonGroup {
            report_id,
            category,
            page,
        } => {
            let report = require_report(store, report_id).await?;
            let diagnosis = report
                .comparison_diagnosis
                .context("analysis report has no comparison diagnosis")?;
            let values = diagnosis
                .evidence
                .get(&comparison_category(category))
                .cloned()
                .unwrap_or_default();
            let start = (page.offset as usize).min(values.len());
            let end = start.saturating_add(page.limit as usize).min(values.len());
            let values = values[start..end].to_vec();
            crate::presentation::print_page(&values, values.len(), page)?;
        }
        AnalysisCommand::Review {
            report_id,
            finding_key,
            state,
            note,
            resolution_evaluation_run_id,
            resolution_comparison_id,
        } => {
            require_finding(store, report_id, &finding_key).await?;
            let state = state.into();
            if state == FindingReviewState::ResolvedByLaterEvidence {
                anyhow::ensure!(
                    resolution_evaluation_run_id.is_some() || resolution_comparison_id.is_some(),
                    "resolved reviews require --resolution-evaluation-run-id or --resolution-comparison-id"
                );
            }
            let review = FindingReviewRecord {
                id: Uuid::new_v4(),
                analysis_report_id: report_id,
                finding_key,
                state,
                note: note
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty()),
                resolution_evaluation_run_id,
                resolution_comparison_id,
                created_at: Utc::now(),
            };
            store.append_finding_review(&review).await?;
            print_json(&review)?;
        }
        AnalysisCommand::Reviews {
            report_id,
            state,
            page,
        } => {
            let reviews = store
                .query_finding_reviews(FindingReviewQuery {
                    analysis_report_id: report_id,
                    state: state.map(Into::into),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&reviews, reviews.len(), page)?;
        }
        AnalysisCommand::Export { id, output } => {
            let report = require_report(store, id).await?;
            std::fs::write(&output, serde_json::to_vec_pretty(&report)?)
                .with_context(|| format!("could not write {}", output.display()))?;
            print_json(&serde_json::json!({
                "exported": true,
                "artifact": "analysis_report",
                "output": output,
            }))?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ResolvedEvidence {
    reference: analysis_core::domain::FindingEvidenceReference,
    prediction: EvaluationPrediction,
}

fn export_evidence(
    output: &std::path::Path,
    format: EvidenceExportFormatArg,
    evidence: &[ResolvedEvidence],
) -> anyhow::Result<()> {
    let file =
        File::create(output).with_context(|| format!("could not create {}", output.display()))?;
    match format {
        EvidenceExportFormatArg::Jsonl => {
            use std::io::Write;
            let mut writer = BufWriter::new(file);
            for row in evidence {
                serde_json::to_writer(&mut writer, row)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        EvidenceExportFormatArg::Csv => {
            let mut writer = csv::Writer::from_writer(BufWriter::new(file));
            for row in evidence {
                writer.serialize(EvidenceCsvRow::from(row))?;
            }
            writer.flush()?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct EvidenceCsvRow<'a> {
    category: &'static str,
    prediction_id: Uuid,
    snapshot_member_id: Uuid,
    source_row_id: Uuid,
    text: &'a str,
    expected_label: &'a str,
    predicted_label: &'a str,
    confidence: f64,
    prediction_margin: f64,
    probabilities_json: String,
    dimensions_json: String,
}

impl<'a> From<&'a ResolvedEvidence> for EvidenceCsvRow<'a> {
    fn from(value: &'a ResolvedEvidence) -> Self {
        Self {
            category: evidence_category_name(value.reference.category),
            prediction_id: value.prediction.id,
            snapshot_member_id: value.prediction.snapshot_member_id,
            source_row_id: value.prediction.source_row_id,
            text: &value.prediction.text,
            expected_label: &value.prediction.expected_label,
            predicted_label: &value.prediction.predicted_label,
            confidence: value.prediction.confidence,
            prediction_margin: value.reference.prediction_margin,
            probabilities_json: serde_json::to_string(&value.prediction.probabilities)
                .expect("probabilities serialize"),
            dimensions_json: serde_json::to_string(&value.prediction.dimensions)
                .expect("dimensions serialize"),
        }
    }
}

async fn require_report(
    store: &SqliteStore,
    id: Uuid,
) -> anyhow::Result<analysis_core::domain::AnalysisReport> {
    store
        .get_analysis_report(id)
        .await?
        .with_context(|| format!("analysis report not found: {id}"))
}

async fn require_finding(
    store: &SqliteStore,
    report_id: Uuid,
    finding_key: &str,
) -> anyhow::Result<analysis_core::domain::AnalysisFinding> {
    store
        .get_analysis_finding(report_id, finding_key)
        .await?
        .with_context(|| format!("analysis finding not found: {finding_key}"))
}

fn parse_intersections(values: &[String]) -> Vec<Vec<String>> {
    let mut intersections = values
        .iter()
        .map(|value| {
            let mut names = value
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        })
        .collect::<Vec<_>>();
    intersections.sort();
    intersections.dedup();
    intersections
}

impl From<AnalysisRankingArg> for FindingRankingPolicy {
    fn from(value: AnalysisRankingArg) -> Self {
        match value {
            AnalysisRankingArg::ErrorRate => Self::ErrorRate,
            AnalysisRankingArg::ErrorCount => Self::ErrorCount,
            AnalysisRankingArg::ErrorRateLift => Self::ErrorRateLift,
            AnalysisRankingArg::ErrorShare => Self::ErrorShare,
            AnalysisRankingArg::HighConfidenceError => Self::HighConfidenceErrorSeverity,
        }
    }
}

impl From<AnalysisFindingKindArg> for FindingKind {
    fn from(value: AnalysisFindingKindArg) -> Self {
        match value {
            AnalysisFindingKindArg::ExpectedLabel => Self::ExpectedLabel,
            AnalysisFindingKindArg::PredictedLabel => Self::PredictedLabel,
            AnalysisFindingKindArg::ConfusionPair => Self::ConfusionPair,
            AnalysisFindingKindArg::DimensionValue => Self::DimensionValue,
            AnalysisFindingKindArg::Cell => Self::Cell,
            AnalysisFindingKindArg::DimensionIntersection => Self::DimensionIntersection,
            AnalysisFindingKindArg::ConfidenceBand => Self::ConfidenceBand,
            AnalysisFindingKindArg::CorrectnessConfidence => Self::CorrectnessConfidence,
        }
    }
}

impl From<AnalysisFindingSortArg> for AnalysisFindingSort {
    fn from(value: AnalysisFindingSortArg) -> Self {
        match value {
            AnalysisFindingSortArg::Rank => Self::Rank,
            AnalysisFindingSortArg::ErrorRate => Self::ErrorRate,
            AnalysisFindingSortArg::ErrorCount => Self::ErrorCount,
            AnalysisFindingSortArg::ErrorRateLift => Self::ErrorRateLift,
            AnalysisFindingSortArg::ErrorShare => Self::ErrorShare,
            AnalysisFindingSortArg::HighConfidenceError => Self::HighConfidenceErrorSeverity,
            AnalysisFindingSortArg::Support => Self::Support,
        }
    }
}

impl From<AnalysisEvidenceCategoryArg> for EvidenceCategory {
    fn from(value: AnalysisEvidenceCategoryArg) -> Self {
        match value {
            AnalysisEvidenceCategoryArg::HighestConfidenceError => Self::HighestConfidenceError,
            AnalysisEvidenceCategoryArg::LowestMarginError => Self::LowestMarginError,
            AnalysisEvidenceCategoryArg::MedianConfidenceError => Self::MedianConfidenceError,
            AnalysisEvidenceCategoryArg::StableError => Self::StableError,
            AnalysisEvidenceCategoryArg::CorrectContrast => Self::CorrectContrast,
        }
    }
}

impl From<FindingReviewStateArg> for FindingReviewState {
    fn from(value: FindingReviewStateArg) -> Self {
        match value {
            FindingReviewStateArg::Open => Self::Open,
            FindingReviewStateArg::Acknowledged => Self::Acknowledged,
            FindingReviewStateArg::AcceptedLimitation => Self::AcceptedLimitation,
            FindingReviewStateArg::CandidateForMoreData => Self::CandidateForMoreData,
            FindingReviewStateArg::CandidateForLabelSchemaReview => {
                Self::CandidateForLabelSchemaReview
            }
            FindingReviewStateArg::ResolvedByLaterEvidence => Self::ResolvedByLaterEvidence,
        }
    }
}

fn evidence_category_name(category: EvidenceCategory) -> &'static str {
    match category {
        EvidenceCategory::HighestConfidenceError => "highest_confidence_error",
        EvidenceCategory::LowestMarginError => "lowest_margin_error",
        EvidenceCategory::MedianConfidenceError => "median_confidence_error",
        EvidenceCategory::StableError => "stable_error",
        EvidenceCategory::CorrectContrast => "correct_contrast",
    }
}

fn format_name(format: EvidenceExportFormatArg) -> &'static str {
    match format {
        EvidenceExportFormatArg::Jsonl => "jsonl",
        EvidenceExportFormatArg::Csv => "csv",
    }
}

fn comparison_category(value: ComparisonDiagnosisCategoryArg) -> ComparisonEvidenceCategory {
    match value {
        ComparisonDiagnosisCategoryArg::Fixed => ComparisonEvidenceCategory::FixedByRight,
        ComparisonDiagnosisCategoryArg::Regressed => ComparisonEvidenceCategory::RegressedByRight,
        ComparisonDiagnosisCategoryArg::Persistent => ComparisonEvidenceCategory::PersistentError,
        ComparisonDiagnosisCategoryArg::HighConfidenceRegression => {
            ComparisonEvidenceCategory::HighConfidenceRegression
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}
