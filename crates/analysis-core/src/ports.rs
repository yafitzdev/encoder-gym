use std::{future::Future, pin::Pin};

use thiserror::Error;
use uuid::Uuid;

use evaluation_core::domain::{EvaluationComparisonReport, EvaluationPrediction, EvaluationRun};

use crate::domain::{
    AnalysisFinding, AnalysisReport, EvidenceCategory, FindingEvidenceReference, FindingKind,
    FindingReviewRecord, FindingReviewState,
};

#[derive(Debug, Clone, Copy)]
pub struct AnalysisReportQuery {
    pub evaluation_run_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct AnalysisFindingQuery {
    pub analysis_report_id: Uuid,
    pub kind: Option<FindingKind>,
    pub minimum_support: Option<u64>,
    pub minimum_error_count: Option<u64>,
    pub sort: AnalysisFindingSort,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnalysisFindingSort {
    #[default]
    Rank,
    ErrorRate,
    ErrorCount,
    ErrorRateLift,
    ErrorShare,
    HighConfidenceErrorSeverity,
    Support,
}

#[derive(Debug, Clone)]
pub struct FindingEvidenceQuery {
    pub analysis_report_id: Uuid,
    pub finding_key: String,
    pub category: Option<EvidenceCategory>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct FindingReviewQuery {
    pub analysis_report_id: Option<Uuid>,
    pub state: Option<FindingReviewState>,
    pub limit: u32,
    pub offset: u32,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("analysis persistence operation failed: {0}")]
pub struct AnalysisStoreError(pub String);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("analysis evidence source failed: {0}")]
pub struct AnalysisEvidenceSourceError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisPredictionPage {
    pub evaluation_run_id: Uuid,
    pub limit: u32,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairedEvaluationPrediction {
    pub left: EvaluationPrediction,
    pub right: EvaluationPrediction,
}

/// Read-only, backend-neutral access to immutable evaluation evidence.
pub trait AnalysisEvidenceSource: Send + Sync {
    fn get_evaluation_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationRun>, AnalysisEvidenceSourceError>>;

    fn get_evaluation_comparison(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluationComparisonReport>, AnalysisEvidenceSourceError>>;

    fn count_evaluation_predictions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<u64, AnalysisEvidenceSourceError>>;

    fn page_evaluation_predictions(
        &self,
        page: AnalysisPredictionPage,
    ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, AnalysisEvidenceSourceError>>;

    fn page_paired_evaluation_predictions(
        &self,
        left_run_id: Uuid,
        right_run_id: Uuid,
        limit: u32,
        offset: u64,
    ) -> BoxFuture<'_, Result<Vec<PairedEvaluationPrediction>, AnalysisEvidenceSourceError>>;
}

pub trait AnalysisStore: Send + Sync {
    fn create_analysis_report(
        &self,
        report: &AnalysisReport,
    ) -> BoxFuture<'_, Result<(), AnalysisStoreError>>;

    fn get_analysis_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AnalysisReport>, AnalysisStoreError>>;

    fn list_analysis_reports(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AnalysisReport>, AnalysisStoreError>>;
    fn query_analysis_reports(
        &self,
        query: AnalysisReportQuery,
    ) -> BoxFuture<'_, Result<Vec<AnalysisReport>, AnalysisStoreError>>;

    fn query_analysis_findings(
        &self,
        query: AnalysisFindingQuery,
    ) -> BoxFuture<'_, Result<Vec<AnalysisFinding>, AnalysisStoreError>>;

    fn get_analysis_finding(
        &self,
        report_id: Uuid,
        finding_key: &str,
    ) -> BoxFuture<'_, Result<Option<AnalysisFinding>, AnalysisStoreError>>;

    fn query_finding_evidence(
        &self,
        query: FindingEvidenceQuery,
    ) -> BoxFuture<'_, Result<Vec<FindingEvidenceReference>, AnalysisStoreError>>;

    fn append_finding_review(
        &self,
        review: &FindingReviewRecord,
    ) -> BoxFuture<'_, Result<(), AnalysisStoreError>>;

    fn query_finding_reviews(
        &self,
        query: FindingReviewQuery,
    ) -> BoxFuture<'_, Result<Vec<FindingReviewRecord>, AnalysisStoreError>>;
}
