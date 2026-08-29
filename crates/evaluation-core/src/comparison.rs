use std::collections::BTreeMap;

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{
        ConfidenceInterval, EvaluationComparisonReport, EvaluationPrediction, EvaluationRun,
        EvaluationRunState, LeaderboardEntry, LeaderboardMetric, McNemarResult,
        ModelSelectionReport, PairedSliceDelta,
    },
    metrics::calculate_metrics_with_protocol,
};

pub fn paired_comparison(
    left: &EvaluationRun,
    right: &EvaluationRun,
    left_predictions: &[EvaluationPrediction],
    right_predictions: &[EvaluationPrediction],
) -> Result<EvaluationComparisonReport, PairedComparisonError> {
    validate_compatibility(left, right)?;
    let left_by_member = left_predictions
        .iter()
        .map(|p| (p.snapshot_member_id, p))
        .collect::<BTreeMap<_, _>>();
    let right_by_member = right_predictions
        .iter()
        .map(|p| (p.snapshot_member_id, p))
        .collect::<BTreeMap<_, _>>();
    if left_by_member.keys().collect::<Vec<_>>() != right_by_member.keys().collect::<Vec<_>>()
        || left_by_member.len() as u64 != left.total_examples
    {
        return Err(PairedComparisonError::PredictionCohort);
    }
    let labels = &left.source_identity.labels;
    let left_metrics = calculate_metrics_with_protocol(labels, left_predictions, &left.protocol);
    let right_metrics = calculate_metrics_with_protocol(labels, right_predictions, &right.protocol);
    let mut both_correct = 0;
    let mut both_wrong = 0;
    let mut left_only = 0;
    let mut right_only = 0;
    let mut fixed = Vec::new();
    let mut regressed = Vec::new();
    let paired = left_by_member
        .iter()
        .map(|(id, left_prediction)| {
            let right_prediction = right_by_member[id];
            let l = left_prediction.expected_label == left_prediction.predicted_label;
            let r = right_prediction.expected_label == right_prediction.predicted_label;
            match (l, r) {
                (true, true) => both_correct += 1,
                (false, false) => both_wrong += 1,
                (true, false) => {
                    left_only += 1;
                    regressed.push(*id);
                }
                (false, true) => {
                    right_only += 1;
                    fixed.push(*id);
                }
            }
            (*left_prediction, right_prediction)
        })
        .collect::<Vec<_>>();
    let (accuracy_interval, macro_interval) = bootstrap_intervals(
        &paired,
        labels,
        &left.protocol,
        left.protocol.bootstrap_samples,
        left.protocol.statistical_seed,
        left.protocol.confidence_level,
    );
    let slice_deltas = left_metrics
        .slices
        .iter()
        .filter_map(|(key, left_slice)| {
            right_metrics.slices.get(key).and_then(|right_slice| {
                let support = left_slice.support.min(right_slice.support);
                (support >= left.protocol.minimum_slice_support).then(|| {
                    (
                        key.clone(),
                        PairedSliceDelta {
                            identity: left_slice.identity.clone(),
                            support,
                            accuracy_delta: right_slice.metrics.accuracy
                                - left_slice.metrics.accuracy,
                        },
                    )
                })
            })
        })
        .collect();
    let discordant = left_only + right_only;
    let p_value = exact_mcnemar(left_only, right_only);
    let mut report = EvaluationComparisonReport {
        id: Uuid::new_v4(),
        left_run_id: left.id,
        right_run_id: right.id,
        cohort_fingerprint: left.source_identity.cohort_fingerprint.clone(),
        protocol_fingerprint: left.protocol_fingerprint.clone(),
        left_metrics: left_metrics.clone(),
        right_metrics: right_metrics.clone(),
        accuracy_delta: right_metrics.overall.accuracy - left_metrics.overall.accuracy,
        macro_f1_delta: right_metrics.overall.macro_f1 - left_metrics.overall.macro_f1,
        per_label_f1_delta: labels
            .iter()
            .map(|label| {
                (
                    label.clone(),
                    right_metrics.overall.per_label[label].f1
                        - left_metrics.overall.per_label[label].f1,
                )
            })
            .collect(),
        both_correct,
        both_wrong,
        left_only_correct: left_only,
        right_only_correct: right_only,
        fixed_snapshot_member_ids: fixed,
        regressed_snapshot_member_ids: regressed,
        accuracy_delta_interval: accuracy_interval,
        macro_f1_delta_interval: macro_interval,
        mcnemar: McNemarResult {
            left_only_correct: left_only,
            right_only_correct: right_only,
            two_sided_p_value: p_value,
            significant: discordant > 0 && p_value < 1.0 - left.protocol.confidence_level,
        },
        slice_deltas,
        fingerprint: String::new(),
        created_at: Utc::now(),
    };
    report.fingerprint = comparison_fingerprint(&report)
        .map_err(|e| PairedComparisonError::Fingerprint(e.to_string()))?;
    Ok(report)
}

pub fn leaderboard(
    runs: &[EvaluationRun],
    metric: LeaderboardMetric,
) -> Result<Vec<LeaderboardEntry>, SelectionError> {
    let first = runs.first().ok_or(SelectionError::NoCandidates)?;
    for run in runs {
        validate_candidate(first, run)?;
    }
    let mut entries = runs
        .iter()
        .map(|run| {
            let metrics = run.metrics.as_ref().expect("validated completed metrics");
            LeaderboardEntry {
                rank: 0,
                evaluation_run_id: run.id,
                checkpoint_id: run.checkpoint_id,
                metric_value: metric.value(&metrics.overall),
                created_at: run.created_at,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let order = left.metric_value.total_cmp(&right.metric_value);
        let order = if metric.lower_is_better() {
            order
        } else {
            order.reverse()
        };
        order
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.evaluation_run_id.cmp(&right.evaluation_run_id))
    });
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.rank = index as u32 + 1;
    }
    Ok(entries)
}

pub fn select_model(
    runs: &[EvaluationRun],
    metric: LeaderboardMetric,
    minimum_improvement: Option<f64>,
    required_confidence: Option<f64>,
) -> Result<ModelSelectionReport, SelectionError> {
    let ranking = leaderboard(runs, metric)?;
    let mut selected = ranking.first();
    let mut reasons = Vec::new();
    if let (Some(minimum), Some(first), Some(second)) =
        (minimum_improvement, ranking.first(), ranking.get(1))
    {
        let improvement = if metric.lower_is_better() {
            second.metric_value - first.metric_value
        } else {
            first.metric_value - second.metric_value
        };
        if improvement < minimum {
            selected = None;
            reasons.push(format!(
                "best candidate improvement {improvement:.6} is below required {minimum:.6}"
            ));
        }
    }
    if let Some(confidence) = required_confidence {
        selected = None;
        reasons.push(format!("selection requires confidence {confidence:.3}; create a paired comparison and review it before selecting"));
    }
    if let Some(best) = selected {
        reasons.push(format!(
            "ranked first by {metric:?} at {:.6}",
            best.metric_value
        ));
    }
    let mut report = ModelSelectionReport {
        id: Uuid::new_v4(),
        candidate_run_ids: ranking.iter().map(|e| e.evaluation_run_id).collect(),
        metric,
        minimum_improvement,
        required_confidence,
        selected_run_id: selected.map(|e| e.evaluation_run_id),
        selected_checkpoint_id: selected.map(|e| e.checkpoint_id),
        reasons,
        fingerprint: String::new(),
        created_at: Utc::now(),
    };
    report.fingerprint =
        selection_fingerprint(&report).map_err(|e| SelectionError::Fingerprint(e.to_string()))?;
    Ok(report)
}

pub fn comparison_fingerprint(
    report: &EvaluationComparisonReport,
) -> Result<String, artifact_core::FingerprintError> {
    artifact_core::fingerprint(&serde_json::json!({
        "left_run_id": report.left_run_id, "right_run_id": report.right_run_id,
        "cohort_fingerprint": report.cohort_fingerprint, "protocol_fingerprint": report.protocol_fingerprint,
        "left_metrics": report.left_metrics, "right_metrics": report.right_metrics,
        "accuracy_delta": report.accuracy_delta, "macro_f1_delta": report.macro_f1_delta,
        "per_label_f1_delta": report.per_label_f1_delta, "both_correct": report.both_correct,
        "both_wrong": report.both_wrong, "left_only_correct": report.left_only_correct,
        "right_only_correct": report.right_only_correct, "fixed": report.fixed_snapshot_member_ids,
        "regressed": report.regressed_snapshot_member_ids, "accuracy_interval": report.accuracy_delta_interval,
        "macro_f1_interval": report.macro_f1_delta_interval, "mcnemar": report.mcnemar,
        "slice_deltas": report.slice_deltas,
    }))
}

pub fn selection_fingerprint(
    report: &ModelSelectionReport,
) -> Result<String, artifact_core::FingerprintError> {
    artifact_core::fingerprint(&serde_json::json!({
        "candidate_run_ids": report.candidate_run_ids, "metric": report.metric,
        "minimum_improvement": report.minimum_improvement, "required_confidence": report.required_confidence,
        "selected_run_id": report.selected_run_id, "selected_checkpoint_id": report.selected_checkpoint_id,
        "reasons": report.reasons,
    }))
}

fn validate_compatibility(
    left: &EvaluationRun,
    right: &EvaluationRun,
) -> Result<(), PairedComparisonError> {
    if left.state != EvaluationRunState::Completed {
        return Err(PairedComparisonError::Incomplete(left.id));
    }
    if right.state != EvaluationRunState::Completed {
        return Err(PairedComparisonError::Incomplete(right.id));
    }
    if left.snapshot_id != right.snapshot_id {
        return Err(PairedComparisonError::Snapshot);
    }
    if left.split != right.split {
        return Err(PairedComparisonError::Split);
    }
    if left.source_identity.cohort_fingerprint != right.source_identity.cohort_fingerprint {
        return Err(PairedComparisonError::Cohort);
    }
    if left.source_identity.labels != right.source_identity.labels {
        return Err(PairedComparisonError::Labels);
    }
    if left.protocol_fingerprint != right.protocol_fingerprint {
        return Err(PairedComparisonError::Protocol);
    }
    Ok(())
}

fn validate_candidate(first: &EvaluationRun, run: &EvaluationRun) -> Result<(), SelectionError> {
    if run.state != EvaluationRunState::Completed || run.metrics.is_none() {
        return Err(SelectionError::Incomplete(run.id));
    }
    if first.snapshot_id != run.snapshot_id
        || first.split != run.split
        || first.source_identity.cohort_fingerprint != run.source_identity.cohort_fingerprint
        || first.protocol_fingerprint != run.protocol_fingerprint
        || first.source_identity.labels != run.source_identity.labels
    {
        return Err(SelectionError::Incompatible(run.id));
    }
    Ok(())
}

fn bootstrap_intervals(
    pairs: &[(&EvaluationPrediction, &EvaluationPrediction)],
    labels: &[String],
    protocol: &crate::domain::EvaluationProtocol,
    samples: u32,
    seed: u64,
    level: f64,
) -> (ConfidenceInterval, ConfidenceInterval) {
    let mut rng = Lcg(seed);
    let mut accuracy = Vec::with_capacity(samples as usize);
    let mut macro_f1 = Vec::with_capacity(samples as usize);
    for _ in 0..samples {
        let mut left = Vec::with_capacity(pairs.len());
        let mut right = Vec::with_capacity(pairs.len());
        for _ in 0..pairs.len() {
            let index = rng.index(pairs.len());
            left.push(pairs[index].0.clone());
            right.push(pairs[index].1.clone());
        }
        let l = calculate_metrics_with_protocol(labels, &left, protocol);
        let r = calculate_metrics_with_protocol(labels, &right, protocol);
        accuracy.push(r.overall.accuracy - l.overall.accuracy);
        macro_f1.push(r.overall.macro_f1 - l.overall.macro_f1);
    }
    (interval(accuracy, level), interval(macro_f1, level))
}

fn interval(mut values: Vec<f64>, level: f64) -> ConfidenceInterval {
    values.sort_by(f64::total_cmp);
    let tail = (1.0 - level) / 2.0;
    let last = values.len().saturating_sub(1);
    ConfidenceInterval {
        level,
        lower: values[((tail * values.len() as f64).floor() as usize).min(last)],
        upper: values[(((1.0 - tail) * values.len() as f64).ceil() as usize)
            .saturating_sub(1)
            .min(last)],
    }
}

fn exact_mcnemar(left_only: u64, right_only: u64) -> f64 {
    let n = left_only + right_only;
    if n == 0 {
        return 1.0;
    }
    let k = left_only.min(right_only);
    let mut log_probability = -(n as f64) * std::f64::consts::LN_2;
    let mut tail = log_probability.exp();
    for value in 1..=k {
        log_probability += ((n - value + 1) as f64).ln() - (value as f64).ln();
        tail += log_probability.exp();
    }
    (2.0 * tail).min(1.0)
}

struct Lcg(u64);
impl Lcg {
    fn index(&mut self, length: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 32) as usize) % length
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PairedComparisonError {
    #[error("evaluation run {0} is incomplete")]
    Incomplete(Uuid),
    #[error("paired comparison requires the same snapshot")]
    Snapshot,
    #[error("paired comparison requires the same split")]
    Split,
    #[error("paired comparison requires the exact same member cohort")]
    Cohort,
    #[error("paired comparison requires the same ordered labels")]
    Labels,
    #[error("paired comparison requires compatible metric protocols")]
    Protocol,
    #[error("persisted prediction member sets do not match the run cohort")]
    PredictionCohort,
    #[error("could not fingerprint comparison: {0}")]
    Fingerprint(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SelectionError {
    #[error("no completed evaluation candidates were supplied")]
    NoCandidates,
    #[error("candidate evaluation {0} is incomplete")]
    Incomplete(Uuid),
    #[error("candidate evaluation {0} is not cohort/protocol compatible")]
    Incompatible(Uuid),
    #[error("could not fingerprint selection: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use super::{Lcg, exact_mcnemar, interval};

    #[test]
    fn exact_mcnemar_and_bootstrap_primitives_are_deterministic() {
        assert!((exact_mcnemar(0, 10) - 0.001953125).abs() < 1e-15);
        assert_eq!(exact_mcnemar(4, 4), 1.0);
        let left = interval(vec![-0.2, 0.0, 0.1, 0.3], 0.5);
        let right = interval(vec![0.3, -0.2, 0.1, 0.0], 0.5);
        assert_eq!(left, right);
        let mut first = Lcg(42);
        let mut second = Lcg(42);
        assert_eq!(
            (0..20).map(|_| first.index(7)).collect::<Vec<_>>(),
            (0..20).map(|_| second.index(7)).collect::<Vec<_>>()
        );
    }
}
