use std::collections::BTreeMap;

use chrono::Utc;
use evaluation_core::domain::{EvaluationRun, EvaluationRunState};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    comparison::{ComparisonAccumulator, ComparisonError},
    diagnostics::{DiagnosticAccumulator, DiagnosticError, OverlapAccumulator},
    domain::{
        AnalysisFinding, AnalysisReport, AnalysisSourceIdentity, ComparisonDiagnosis,
        FindingEvidenceReference,
    },
    ports::{
        AnalysisEvidenceSource, AnalysisEvidenceSourceError, AnalysisPredictionPage, AnalysisStore,
        AnalysisStoreError,
    },
    protocol::{AnalysisProtocol, AnalysisProtocolError},
};

const ANALYSIS_PAGE_SIZE: u32 = 1_000;

#[derive(Debug, Error)]
pub enum AnalysisRunnerError {
    #[error(transparent)]
    Protocol(#[from] AnalysisProtocolError),
    #[error(transparent)]
    Evidence(#[from] AnalysisEvidenceSourceError),
    #[error(transparent)]
    Diagnostics(#[from] DiagnosticError),
    #[error(transparent)]
    Comparison(#[from] ComparisonError),
    #[error(transparent)]
    Store(#[from] AnalysisStoreError),
    #[error("evaluation run not found: {0}")]
    EvaluationNotFound(Uuid),
    #[error("evaluation run {id} must be completed, but is {state:?}")]
    EvaluationIncomplete { id: Uuid, state: EvaluationRunState },
    #[error("evaluation run {0} has no completed metrics")]
    EvaluationMetrics(Uuid),
    #[error(
        "evaluation run {id} is inconsistent: persisted={persisted}, examples={examples}, processed={processed}, metric_total={metric_total}"
    )]
    EvaluationCounts {
        id: Uuid,
        persisted: u64,
        examples: u64,
        processed: u64,
        metric_total: u64,
    },
    #[error("comparison report not found: {0}")]
    ComparisonNotFound(Uuid),
    #[error(
        "comparison report {comparison_id} is incompatible with evaluation run {run_id}: {reason}"
    )]
    ComparisonIncompatible {
        comparison_id: Uuid,
        run_id: Uuid,
        reason: &'static str,
    },
    #[error(
        "prediction page for run {run_id} contained prediction {prediction_id} from run {actual_run_id}"
    )]
    ForeignPrediction {
        run_id: Uuid,
        prediction_id: Uuid,
        actual_run_id: Uuid,
    },
    #[error("prediction source returned more than the declared {0} predictions")]
    ExcessPredictions(u64),
    #[error("could not fingerprint analysis report: {0}")]
    Fingerprint(String),
    #[error("analysis report {0} does not reproduce from persisted prediction evidence")]
    ReportEvidenceMismatch(Uuid),
    #[error("analysis report {id} does not reproduce from persisted {reason}")]
    ReportEvidenceDetail { id: Uuid, reason: &'static str },
}

pub async fn run_analysis(
    source: &impl AnalysisEvidenceSource,
    store: &impl AnalysisStore,
    evaluation_run_id: Uuid,
    protocol: AnalysisProtocol,
) -> Result<AnalysisReport, AnalysisRunnerError> {
    protocol.validate()?;
    let run = source
        .get_evaluation_run(evaluation_run_id)
        .await?
        .ok_or(AnalysisRunnerError::EvaluationNotFound(evaluation_run_id))?;
    let persisted_count = source
        .count_evaluation_predictions(evaluation_run_id)
        .await?;
    validate_evaluation(&run, persisted_count)?;
    let comparison = if let Some(comparison_id) = protocol.comparison_id {
        let comparison = source
            .get_evaluation_comparison(comparison_id)
            .await?
            .ok_or(AnalysisRunnerError::ComparisonNotFound(comparison_id))?;
        validate_comparison(&run, &comparison)?;
        let other_run_id = if comparison.left_run_id == run.id {
            comparison.right_run_id
        } else {
            comparison.left_run_id
        };
        let other_run = source
            .get_evaluation_run(other_run_id)
            .await?
            .ok_or(AnalysisRunnerError::EvaluationNotFound(other_run_id))?;
        let other_count = source.count_evaluation_predictions(other_run_id).await?;
        validate_evaluation(&other_run, other_count)?;
        if other_count != persisted_count {
            return Err(AnalysisRunnerError::ComparisonIncompatible {
                comparison_id,
                run_id: run.id,
                reason: "paired evaluation prediction counts differ",
            });
        }
        Some(comparison)
    } else {
        None
    };

    let mut accumulator = DiagnosticAccumulator::new(protocol.clone())?;
    page_predictions(source, evaluation_run_id, persisted_count, |prediction| {
        accumulator.add(prediction)
    })
    .await?;
    let mut aggregation = accumulator.finish()?;

    let mut overlap = OverlapAccumulator::new(
        &aggregation.findings,
        aggregation.error_count,
        protocol.clone(),
    )?;
    page_predictions(source, evaluation_run_id, persisted_count, |prediction| {
        overlap.add(prediction)
    })
    .await?;
    overlap.finish(&mut aggregation.findings)?;
    refresh_finding_fingerprints(&mut aggregation.findings)?;
    let finding_evidence = persistence_normalize_evidence(aggregation.evidence)?;

    let comparison_diagnosis = match &comparison {
        Some(comparison) => Some(
            build_comparison_diagnosis(source, comparison, &protocol, persisted_count, run.id)
                .await?,
        ),
        None => None,
    }
    .as_ref()
    .map(persistence_normalize)
    .transpose()?;

    let protocol_fingerprint = protocol
        .fingerprint()
        .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))?;
    let source_identity = AnalysisSourceIdentity {
        evaluation_run_id,
        evaluation_input_fingerprint: run.input_fingerprint.clone(),
        evaluation_protocol_fingerprint: run.protocol_fingerprint.clone(),
        cohort_fingerprint: run.source_identity.cohort_fingerprint.clone(),
        prediction_count: persisted_count,
        comparison_id: comparison.as_ref().map(|value| value.id),
        comparison_fingerprint: comparison.as_ref().map(|value| value.fingerprint.clone()),
    };
    let fingerprint = report_fingerprint(
        &protocol_fingerprint,
        &source_identity,
        aggregation.prediction_count,
        aggregation.error_count,
        &aggregation.findings,
        &finding_evidence,
        comparison_diagnosis.as_ref(),
    )?;
    let report = AnalysisReport {
        id: Uuid::new_v4(),
        evaluation_run_id,
        minimum_support: protocol.minimum_support,
        prediction_count: aggregation.prediction_count,
        error_count: aggregation.error_count,
        findings: aggregation.findings,
        errors: Vec::new(),
        protocol,
        protocol_fingerprint,
        source_identity: Some(source_identity),
        finding_evidence,
        comparison_diagnosis,
        fingerprint,
        created_at: Utc::now(),
    };
    store.create_analysis_report(&report).await?;
    Ok(report)
}

pub fn reproduce_report_fingerprint(
    report: &AnalysisReport,
) -> Result<String, AnalysisRunnerError> {
    let source_identity = report
        .source_identity
        .as_ref()
        .ok_or_else(|| AnalysisRunnerError::Fingerprint("missing source identity".into()))?;
    report_fingerprint(
        &report.protocol_fingerprint,
        source_identity,
        report.prediction_count,
        report.error_count,
        &report.findings,
        &report.finding_evidence,
        report.comparison_diagnosis.as_ref(),
    )
}

/// Rebuilds bounded diagnostic facts from persisted predictions without
/// writing a report or invoking a predictor.
pub async fn verify_report_evidence(
    source: &impl AnalysisEvidenceSource,
    report: &AnalysisReport,
) -> Result<(), AnalysisRunnerError> {
    report.protocol.validate()?;
    let protocol_fingerprint = report
        .protocol
        .fingerprint()
        .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))?;
    if protocol_fingerprint != report.protocol_fingerprint
        || reproduce_report_fingerprint(report)? != report.fingerprint
    {
        return Err(AnalysisRunnerError::ReportEvidenceMismatch(report.id));
    }
    let run = source
        .get_evaluation_run(report.evaluation_run_id)
        .await?
        .ok_or(AnalysisRunnerError::EvaluationNotFound(
            report.evaluation_run_id,
        ))?;
    let persisted_count = source
        .count_evaluation_predictions(report.evaluation_run_id)
        .await?;
    validate_evaluation(&run, persisted_count)?;
    let expected_source = AnalysisSourceIdentity {
        evaluation_run_id: run.id,
        evaluation_input_fingerprint: run.input_fingerprint.clone(),
        evaluation_protocol_fingerprint: run.protocol_fingerprint.clone(),
        cohort_fingerprint: run.source_identity.cohort_fingerprint.clone(),
        prediction_count: persisted_count,
        comparison_id: report
            .source_identity
            .as_ref()
            .and_then(|source| source.comparison_id),
        comparison_fingerprint: report
            .source_identity
            .as_ref()
            .and_then(|source| source.comparison_fingerprint.clone()),
    };
    if report.source_identity.as_ref() != Some(&expected_source) {
        return Err(AnalysisRunnerError::ReportEvidenceMismatch(report.id));
    }
    let reproduced_comparison = if let Some(comparison_id) = expected_source.comparison_id {
        let comparison = source
            .get_evaluation_comparison(comparison_id)
            .await?
            .ok_or(AnalysisRunnerError::ComparisonNotFound(comparison_id))?;
        validate_comparison(&run, &comparison)?;
        if expected_source.comparison_fingerprint.as_deref()
            != Some(comparison.fingerprint.as_str())
        {
            return Err(AnalysisRunnerError::ReportEvidenceMismatch(report.id));
        }
        Some(
            build_comparison_diagnosis(
                source,
                &comparison,
                &report.protocol,
                persisted_count,
                run.id,
            )
            .await?,
        )
    } else {
        None
    }
    .as_ref()
    .map(persistence_normalize)
    .transpose()?;
    let mut accumulator = DiagnosticAccumulator::new(report.protocol.clone())?;
    page_predictions(source, run.id, persisted_count, |prediction| {
        accumulator.add(prediction)
    })
    .await?;
    let mut aggregation = accumulator.finish()?;
    let mut overlap = OverlapAccumulator::new(
        &aggregation.findings,
        aggregation.error_count,
        report.protocol.clone(),
    )?;
    page_predictions(source, run.id, persisted_count, |prediction| {
        overlap.add(prediction)
    })
    .await?;
    overlap.finish(&mut aggregation.findings)?;
    refresh_finding_fingerprints(&mut aggregation.findings)?;
    let finding_evidence = persistence_normalize_evidence(aggregation.evidence)?;
    verify_reproduced(
        aggregation.prediction_count == report.prediction_count,
        report.id,
        "prediction count",
    )?;
    verify_reproduced(
        aggregation.error_count == report.error_count,
        report.id,
        "error count",
    )?;
    verify_reproduced(
        aggregation.findings == report.findings,
        report.id,
        "findings",
    )?;
    verify_reproduced(
        finding_evidence == report.finding_evidence,
        report.id,
        "finding evidence",
    )?;
    verify_reproduced(
        reproduced_comparison == report.comparison_diagnosis,
        report.id,
        "comparison diagnosis",
    )?;
    Ok(())
}

fn verify_reproduced(
    condition: bool,
    id: Uuid,
    reason: &'static str,
) -> Result<(), AnalysisRunnerError> {
    if condition {
        Ok(())
    } else {
        Err(AnalysisRunnerError::ReportEvidenceDetail { id, reason })
    }
}

fn persistence_normalize_evidence(
    evidence: BTreeMap<String, Vec<FindingEvidenceReference>>,
) -> Result<BTreeMap<String, Vec<FindingEvidenceReference>>, AnalysisRunnerError> {
    persistence_normalize(&evidence)
}

fn persistence_normalize<T>(value: &T) -> Result<T, AnalysisRunnerError>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let bytes = serde_json::to_vec(value)
        .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))
}

async fn build_comparison_diagnosis(
    source: &impl AnalysisEvidenceSource,
    comparison: &evaluation_core::domain::EvaluationComparisonReport,
    protocol: &AnalysisProtocol,
    expected_count: u64,
    analyzed_run_id: Uuid,
) -> Result<ComparisonDiagnosis, AnalysisRunnerError> {
    let mut accumulator = ComparisonAccumulator::new(comparison, protocol);
    let mut offset = 0_u64;
    while offset < expected_count {
        let remaining = expected_count - offset;
        let pairs = source
            .page_paired_evaluation_predictions(
                comparison.left_run_id,
                comparison.right_run_id,
                ANALYSIS_PAGE_SIZE.min(u32::try_from(remaining).unwrap_or(u32::MAX)),
                offset,
            )
            .await?;
        if pairs.is_empty() {
            break;
        }
        for pair in &pairs {
            accumulator.add(pair)?;
        }
        offset = offset
            .checked_add(pairs.len() as u64)
            .ok_or(AnalysisRunnerError::ExcessPredictions(expected_count))?;
        if offset > expected_count {
            return Err(AnalysisRunnerError::ExcessPredictions(expected_count));
        }
    }
    if offset != expected_count {
        return Err(AnalysisRunnerError::ComparisonIncompatible {
            comparison_id: comparison.id,
            run_id: analyzed_run_id,
            reason: "paired prediction evidence is incomplete",
        });
    }
    accumulator.finish().map_err(Into::into)
}

fn refresh_finding_fingerprints(
    findings: &mut [AnalysisFinding],
) -> Result<(), AnalysisRunnerError> {
    for finding in findings {
        finding
            .refresh_fingerprint()
            .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))?;
    }
    Ok(())
}

async fn page_predictions(
    source: &impl AnalysisEvidenceSource,
    run_id: Uuid,
    expected_count: u64,
    mut visit: impl FnMut(&evaluation_core::domain::EvaluationPrediction) -> Result<(), DiagnosticError>,
) -> Result<(), AnalysisRunnerError> {
    let mut offset = 0_u64;
    while offset < expected_count {
        let remaining = expected_count - offset;
        let page = source
            .page_evaluation_predictions(AnalysisPredictionPage {
                evaluation_run_id: run_id,
                limit: ANALYSIS_PAGE_SIZE.min(u32::try_from(remaining).unwrap_or(u32::MAX)),
                offset,
            })
            .await?;
        if page.is_empty() {
            break;
        }
        for prediction in &page {
            if prediction.evaluation_run_id != run_id {
                return Err(AnalysisRunnerError::ForeignPrediction {
                    run_id,
                    prediction_id: prediction.id,
                    actual_run_id: prediction.evaluation_run_id,
                });
            }
            visit(prediction)?;
        }
        offset = offset
            .checked_add(page.len() as u64)
            .ok_or(AnalysisRunnerError::ExcessPredictions(expected_count))?;
        if offset > expected_count {
            return Err(AnalysisRunnerError::ExcessPredictions(expected_count));
        }
    }
    if offset != expected_count {
        return Err(AnalysisRunnerError::EvaluationCounts {
            id: run_id,
            persisted: offset,
            examples: expected_count,
            processed: offset,
            metric_total: expected_count,
        });
    }
    Ok(())
}

fn validate_evaluation(
    run: &EvaluationRun,
    persisted_count: u64,
) -> Result<(), AnalysisRunnerError> {
    if run.state != EvaluationRunState::Completed {
        return Err(AnalysisRunnerError::EvaluationIncomplete {
            id: run.id,
            state: run.state,
        });
    }
    let metrics = run
        .metrics
        .as_ref()
        .ok_or(AnalysisRunnerError::EvaluationMetrics(run.id))?;
    if persisted_count == 0
        || persisted_count != run.example_count
        || persisted_count != run.processed_examples
        || persisted_count != metrics.overall.total
    {
        return Err(AnalysisRunnerError::EvaluationCounts {
            id: run.id,
            persisted: persisted_count,
            examples: run.example_count,
            processed: run.processed_examples,
            metric_total: metrics.overall.total,
        });
    }
    Ok(())
}

fn validate_comparison(
    run: &EvaluationRun,
    comparison: &evaluation_core::domain::EvaluationComparisonReport,
) -> Result<(), AnalysisRunnerError> {
    let reproduced = evaluation_core::comparison::comparison_fingerprint(comparison)
        .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))?;
    if reproduced != comparison.fingerprint {
        return Err(AnalysisRunnerError::ComparisonIncompatible {
            comparison_id: comparison.id,
            run_id: run.id,
            reason: "comparison fingerprint does not reproduce",
        });
    }
    if comparison.left_run_id != run.id && comparison.right_run_id != run.id {
        return Err(AnalysisRunnerError::ComparisonIncompatible {
            comparison_id: comparison.id,
            run_id: run.id,
            reason: "comparison does not contain the evaluation run",
        });
    }
    if comparison.cohort_fingerprint != run.source_identity.cohort_fingerprint {
        return Err(AnalysisRunnerError::ComparisonIncompatible {
            comparison_id: comparison.id,
            run_id: run.id,
            reason: "cohort fingerprint differs",
        });
    }
    if comparison.protocol_fingerprint != run.protocol_fingerprint {
        return Err(AnalysisRunnerError::ComparisonIncompatible {
            comparison_id: comparison.id,
            run_id: run.id,
            reason: "evaluation protocol fingerprint differs",
        });
    }
    Ok(())
}

#[derive(Serialize)]
struct ReportFingerprintInput<'a> {
    protocol_fingerprint: &'a str,
    source_identity: &'a AnalysisSourceIdentity,
    prediction_count: u64,
    error_count: u64,
    findings: &'a [AnalysisFinding],
    evidence: &'a BTreeMap<String, Vec<FindingEvidenceReference>>,
    comparison_diagnosis: Option<&'a ComparisonDiagnosis>,
}

fn report_fingerprint(
    protocol_fingerprint: &str,
    source_identity: &AnalysisSourceIdentity,
    prediction_count: u64,
    error_count: u64,
    findings: &[AnalysisFinding],
    evidence: &BTreeMap<String, Vec<FindingEvidenceReference>>,
    comparison_diagnosis: Option<&ComparisonDiagnosis>,
) -> Result<String, AnalysisRunnerError> {
    artifact_core::fingerprint(&ReportFingerprintInput {
        protocol_fingerprint,
        source_identity,
        prediction_count,
        error_count,
        findings,
        evidence,
        comparison_diagnosis,
    })
    .map_err(|error| AnalysisRunnerError::Fingerprint(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use evaluation_core::domain::{
        EvaluationComparisonReport, EvaluationPrediction, EvaluationRun,
    };
    use uuid::Uuid;

    use super::page_predictions;
    use crate::{
        diagnostics::DiagnosticError,
        ports::{
            AnalysisEvidenceSource, AnalysisEvidenceSourceError, AnalysisPredictionPage, BoxFuture,
            PairedEvaluationPrediction,
        },
    };

    struct PagingSource {
        predictions: Vec<EvaluationPrediction>,
        pages: Arc<Mutex<Vec<AnalysisPredictionPage>>>,
    }

    impl AnalysisEvidenceSource for PagingSource {
        fn get_evaluation_run(
            &self,
            _: Uuid,
        ) -> BoxFuture<'_, Result<Option<EvaluationRun>, AnalysisEvidenceSourceError>> {
            Box::pin(async { unreachable!() })
        }

        fn get_evaluation_comparison(
            &self,
            _: Uuid,
        ) -> BoxFuture<'_, Result<Option<EvaluationComparisonReport>, AnalysisEvidenceSourceError>>
        {
            Box::pin(async { unreachable!() })
        }

        fn count_evaluation_predictions(
            &self,
            _: Uuid,
        ) -> BoxFuture<'_, Result<u64, AnalysisEvidenceSourceError>> {
            Box::pin(async { unreachable!() })
        }

        fn page_evaluation_predictions(
            &self,
            page: AnalysisPredictionPage,
        ) -> BoxFuture<'_, Result<Vec<EvaluationPrediction>, AnalysisEvidenceSourceError>> {
            Box::pin(async move {
                self.pages.lock().expect("pages").push(page);
                let start = page.offset as usize;
                let end = start
                    .saturating_add(page.limit as usize)
                    .min(self.predictions.len());
                Ok(self.predictions[start..end].to_vec())
            })
        }

        fn page_paired_evaluation_predictions(
            &self,
            _: Uuid,
            _: Uuid,
            _: u32,
            _: u64,
        ) -> BoxFuture<'_, Result<Vec<PairedEvaluationPrediction>, AnalysisEvidenceSourceError>>
        {
            Box::pin(async { unreachable!() })
        }
    }

    #[tokio::test]
    async fn prediction_reader_enforces_deterministic_bounded_pages() {
        let run_id = Uuid::new_v4();
        let predictions = (0..2_001)
            .map(|index| EvaluationPrediction {
                id: Uuid::from_u128(index + 1),
                evaluation_run_id: run_id,
                snapshot_member_id: Uuid::from_u128(index + 10_000),
                source_row_id: Uuid::from_u128(index + 20_000),
                text: String::new(),
                expected_label: String::new(),
                predicted_label: String::new(),
                confidence: 0.0,
                probabilities: Vec::new(),
                dimensions: Default::default(),
                created_at: Utc::now(),
            })
            .collect::<Vec<_>>();
        let pages = Arc::new(Mutex::new(Vec::new()));
        let source = PagingSource {
            predictions,
            pages: Arc::clone(&pages),
        };
        let mut visited = 0_u64;
        page_predictions(&source, run_id, 2_001, |_| {
            visited += 1;
            Ok::<_, DiagnosticError>(())
        })
        .await
        .expect("paged");
        assert_eq!(visited, 2_001);
        let pages = pages.lock().expect("pages");
        assert_eq!(
            pages.iter().map(|page| page.offset).collect::<Vec<_>>(),
            vec![0, 1_000, 2_000]
        );
        assert!(pages.iter().all(|page| page.limit <= 1_000));
    }
}
