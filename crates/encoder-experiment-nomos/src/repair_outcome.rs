//! Pure projection of already-saved native development aggregates into the
//! metric points required by repair outcomes. No evaluation is rerun here.

use std::collections::BTreeMap;

use encoder_optimization_core::repair_strategy::RepairPlan;
use serde::Serialize;

use crate::{
    EncoderTaskAdapterError, NomosDevelopmentCluster, NomosDevelopmentEvidence, adapter_error,
    dataset_landscape::stable_cluster_key,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosRepairMetricPoint {
    pub target_id: String,
    pub cluster_key: String,
    pub suite: String,
    pub metric: String,
    pub support: Option<u64>,
    pub value: Option<f64>,
    pub original_baseline: Option<f64>,
}

pub fn project_repair_metric_points(
    plan: &RepairPlan,
    current: &[NomosDevelopmentEvidence],
    baseline: &[NomosDevelopmentEvidence],
) -> Result<Vec<NomosRepairMetricPoint>, EncoderTaskAdapterError> {
    let current = clusters(current)?;
    let baseline = clusters(baseline)?;
    let suites = current
        .keys()
        .map(|(suite, _)| suite.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut points = Vec::new();
    for target in &plan.targets {
        for cluster_key in &target.cluster_keys {
            for suite in &suites {
                let observed = current.get(&(suite.clone(), cluster_key.clone()));
                let original = baseline.get(&(suite.clone(), cluster_key.clone()));
                points.push(NomosRepairMetricPoint {
                    target_id: target.target_id.clone(),
                    cluster_key: cluster_key.clone(),
                    suite: suite.clone(),
                    metric: target.target_metric.name.clone(),
                    support: observed.map(|cluster| cluster.support),
                    value: observed.and_then(|cluster| {
                        cluster.metrics.get(&target.target_metric.name).copied()
                    }),
                    original_baseline: original.and_then(|cluster| {
                        cluster.metrics.get(&target.target_metric.name).copied()
                    }),
                });
            }
        }
    }
    Ok(points)
}

fn clusters(
    evidence: &[NomosDevelopmentEvidence],
) -> Result<BTreeMap<(String, String), NomosDevelopmentCluster>, EncoderTaskAdapterError> {
    let mut result = BTreeMap::new();
    for report in evidence {
        for cluster in &report.clusters {
            let key = stable_cluster_key(&cluster.dimension, &cluster.value)?;
            if result
                .insert((report.suite_key.clone(), key), cluster.clone())
                .is_some()
            {
                return Err(adapter_error(
                    "Native repair metric cluster is repeated within one suite",
                ));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoder_optimization_core::repair_strategy::{
        AdditionCount, AnchorAllocation, MetricDirection, REPAIR_PLAN_SCHEMA_VERSION,
        RepairOperation, RepairTarget, TargetMetric,
    };
    use serde_json::json;

    fn evidence(suite: &str, recall: f64) -> NomosDevelopmentEvidence {
        NomosDevelopmentEvidence {
            schema_version: crate::development_evidence::DEVELOPMENT_EVIDENCE_SCHEMA_VERSION,
            report_id: uuid::Uuid::new_v4(),
            report_fingerprint: artifact_core::fingerprint(&json!([suite, recall])).unwrap(),
            suite_key: suite.into(),
            artifact_fingerprint: artifact_core::fingerprint(&json!(["artifact", recall])).unwrap(),
            report_support: 100,
            retrieval_support: 100,
            agent_population: crate::NomosAgentPopulation::NotRequired,
            clusters: vec![NomosDevelopmentCluster {
                dimension: "expected_capability".into(),
                value: "search".into(),
                support: 20,
                metrics: BTreeMap::from([("recall_at_1".into(), recall)]),
            }],
            failures: vec![],
            failure_sample_limit: 50,
        }
    }

    #[test]
    fn target_points_keep_suite_support_and_original_baseline_separate() {
        let cluster = stable_cluster_key("expected_capability", "search").unwrap();
        let plan = RepairPlan {
            schema_version: REPAIR_PLAN_SCHEMA_VERSION,
            summary: "Target search".into(),
            stop: false,
            targets: vec![RepairTarget {
                target_id: "search".into(),
                cluster_keys: vec![cluster.clone()],
                evidence_ids: vec![cluster.clone()],
                hypothesis: "A search variant may improve recall".into(),
                evidence_limitations: "One suite".into(),
                intended_failure_pattern: "Search miss".into(),
                alternative_explanation: "Capacity".into(),
                operation: RepairOperation::LabelPreservingVariants {
                    count: AdditionCount::AbsoluteRows { desired_rows: 1 },
                    allocation_rationale: "One anchor".into(),
                    anchors: vec![AnchorAllocation {
                        row_id: "row".into(),
                        row_fingerprint: artifact_core::fingerprint(&"row").unwrap(),
                        additions: 1,
                    }],
                },
                target_metric: TargetMetric {
                    name: "recall_at_1".into(),
                    direction: MetricDirection::Increase,
                },
            }],
        };
        let points = project_repair_metric_points(
            &plan,
            &[evidence("development", 0.6)],
            &[evidence("development", 0.8)],
        )
        .unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].support, Some(20));
        assert_eq!(points[0].value, Some(0.6));
        assert_eq!(points[0].original_baseline, Some(0.8));
    }
}
