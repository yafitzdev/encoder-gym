use std::collections::BTreeMap;

use chrono::Utc;
use evaluation_core::domain::EvaluationPrediction;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    AnalysisFinding, AnalysisReport, FindingIdentity, FindingKind, MisclassifiedExample,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    #[error("minimum support must be greater than zero")]
    MinimumSupport,
    #[error("analysis requires at least one persisted prediction")]
    EmptyPredictions,
    #[error("could not fingerprint analysis: {0}")]
    Fingerprint(String),
}

pub fn analyze_predictions(
    evaluation_run_id: Uuid,
    predictions: &[EvaluationPrediction],
    minimum_support: u64,
) -> Result<AnalysisReport, AnalysisError> {
    if minimum_support == 0 {
        return Err(AnalysisError::MinimumSupport);
    }
    if predictions.is_empty() {
        return Err(AnalysisError::EmptyPredictions);
    }

    let mut groups = BTreeMap::<GroupKey, GroupCounts>::new();
    let mut errors = Vec::new();
    for prediction in predictions {
        let is_error = prediction.expected_label != prediction.predicted_label;
        record_group(
            &mut groups,
            FindingKind::ExpectedLabel,
            BTreeMap::from([("label".into(), prediction.expected_label.clone())]),
            is_error,
        );
        record_group(
            &mut groups,
            FindingKind::PredictedLabel,
            BTreeMap::from([("predicted_label".into(), prediction.predicted_label.clone())]),
            is_error,
        );
        record_group(
            &mut groups,
            FindingKind::ConfusionPair,
            BTreeMap::from([
                ("expected_label".into(), prediction.expected_label.clone()),
                ("predicted_label".into(), prediction.predicted_label.clone()),
            ]),
            is_error,
        );
        for (name, value) in &prediction.dimensions {
            record_group(
                &mut groups,
                FindingKind::DimensionValue,
                BTreeMap::from([
                    ("dimension".into(), name.clone()),
                    ("value".into(), value.clone()),
                ]),
                is_error,
            );
        }
        let mut cell = prediction.dimensions.clone();
        cell.insert("label".into(), prediction.expected_label.clone());
        record_group(&mut groups, FindingKind::Cell, cell, is_error);

        if is_error {
            errors.push(MisclassifiedExample {
                prediction_id: prediction.id,
                snapshot_member_id: prediction.snapshot_member_id,
                source_row_id: prediction.source_row_id,
                text: prediction.text.clone(),
                expected_label: prediction.expected_label.clone(),
                predicted_label: prediction.predicted_label.clone(),
                confidence: prediction.confidence,
                dimensions: prediction.dimensions.clone(),
            });
        }
    }

    errors.sort_by(|left, right| {
        left.expected_label
            .cmp(&right.expected_label)
            .then_with(|| left.predicted_label.cmp(&right.predicted_label))
            .then_with(|| left.prediction_id.cmp(&right.prediction_id))
    });
    let mut findings = groups
        .into_iter()
        .filter(|(_, counts)| counts.support >= minimum_support && counts.errors > 0)
        .map(|(group, counts)| AnalysisFinding {
            rank: 0,
            kind: group.kind,
            key: group.key,
            attributes: group.attributes,
            support: counts.support,
            error_count: counts.errors,
            error_rate: counts.errors as f64 / counts.support as f64,
            mean_error_confidence: 0.0,
            median_error_confidence: 0.0,
            mean_expected_probability: 0.0,
            mean_prediction_margin: 0.0,
            mean_entropy: 0.0,
            error_rate_lift: 0.0,
            error_share: 0.0,
            high_confidence_error_severity: 0.0,
            marginal_error_count: 0,
            cumulative_error_count: 0,
            cumulative_error_coverage: 0.0,
            fingerprint: String::new(),
        })
        .collect::<Vec<_>>();
    findings.sort_by(|left, right| {
        right
            .error_rate
            .total_cmp(&left.error_rate)
            .then_with(|| right.error_count.cmp(&left.error_count))
            .then_with(|| left.key.cmp(&right.key))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    for (index, finding) in findings.iter_mut().enumerate() {
        finding.rank = index as u64 + 1;
        finding
            .refresh_fingerprint()
            .map_err(|error| AnalysisError::Fingerprint(error.to_string()))?;
    }

    let fingerprint = artifact_core::fingerprint(&AnalysisFingerprintInput {
        evaluation_run_id,
        minimum_support,
        prediction_count: predictions.len() as u64,
        findings: &findings,
        errors: &errors,
    })
    .map_err(|error| AnalysisError::Fingerprint(error.to_string()))?;
    let protocol = crate::protocol::AnalysisProtocol {
        minimum_support,
        ..crate::protocol::AnalysisProtocol::default()
    };
    let protocol_fingerprint = protocol
        .fingerprint()
        .map_err(|error| AnalysisError::Fingerprint(error.to_string()))?;
    Ok(AnalysisReport {
        id: Uuid::new_v4(),
        evaluation_run_id,
        minimum_support,
        prediction_count: predictions.len() as u64,
        error_count: errors.len() as u64,
        findings,
        errors,
        protocol,
        protocol_fingerprint,
        source_identity: Some(crate::domain::AnalysisSourceIdentity::legacy(
            evaluation_run_id,
            predictions.len() as u64,
        )),
        finding_evidence: BTreeMap::new(),
        comparison_diagnosis: None,
        fingerprint,
        created_at: Utc::now(),
    })
}

#[derive(Serialize)]
struct AnalysisFingerprintInput<'a> {
    evaluation_run_id: Uuid,
    minimum_support: u64,
    prediction_count: u64,
    findings: &'a [AnalysisFinding],
    errors: &'a [MisclassifiedExample],
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    kind: FindingKind,
    key: String,
    attributes: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct GroupCounts {
    support: u64,
    errors: u64,
}

fn record_group(
    groups: &mut BTreeMap<GroupKey, GroupCounts>,
    kind: FindingKind,
    attributes: BTreeMap<String, String>,
    is_error: bool,
) {
    let key = FindingIdentity {
        kind,
        attributes: attributes.clone(),
    }
    .key();
    let counts = groups
        .entry(GroupKey {
            kind,
            key,
            attributes,
        })
        .or_default();
    counts.support += 1;
    counts.errors += u64::from(is_error);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use evaluation_core::domain::EvaluationPrediction;
    use uuid::Uuid;

    use super::analyze_predictions;
    use crate::domain::FindingKind;

    #[test]
    fn ranks_groups_stably_and_applies_minimum_support() {
        let run_id = Uuid::new_v4();
        let predictions = vec![
            prediction(run_id, "a", "b", "hard"),
            prediction(run_id, "a", "a", "hard"),
            prediction(run_id, "b", "a", "easy"),
            prediction(run_id, "b", "b", "easy"),
        ];
        let report = analyze_predictions(run_id, &predictions, 2).expect("report");
        let repeated = analyze_predictions(run_id, &predictions, 2).expect("repeated report");

        assert_eq!(report.error_count, 2);
        assert_eq!(report.errors.len(), 2);
        assert_eq!(report.fingerprint, repeated.fingerprint);
        assert!(report.findings.iter().all(|finding| finding.support >= 2));
        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| finding.rank)
                .collect::<Vec<_>>(),
            (1..=report.findings.len() as u64).collect::<Vec<_>>()
        );
        assert!(report.findings.iter().any(|finding| {
            finding.kind == FindingKind::DimensionValue
                && finding.attributes["dimension"] == "difficulty"
                && finding.attributes["value"] == "hard"
                && finding.error_count == 1
                && finding.support == 2
        }));
        assert!(
            !report.findings.iter().any(|finding| {
                finding.kind == FindingKind::ConfusionPair && finding.support == 1
            })
        );
    }

    fn prediction(
        run_id: Uuid,
        expected: &str,
        predicted: &str,
        difficulty: &str,
    ) -> EvaluationPrediction {
        EvaluationPrediction {
            id: Uuid::new_v4(),
            evaluation_run_id: run_id,
            snapshot_member_id: Uuid::new_v4(),
            source_row_id: Uuid::new_v4(),
            text: format!("{expected} example"),
            expected_label: expected.into(),
            predicted_label: predicted.into(),
            confidence: 0.7,
            probabilities: vec![],
            dimensions: BTreeMap::from([("difficulty".into(), difficulty.into())]),
            created_at: Utc::now(),
        }
    }
}
