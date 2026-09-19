//! Deterministic, development-only dataset intelligence for Agent protocol V2.
//! The complete training population is scanned locally; only aggregate cluster
//! summaries and explicitly sampled task-visible rows leave the adapter.

use std::collections::{BTreeMap, BTreeSet};

use encoder_optimization_core::agent::InspectionItem;
use serde_json::{Value, json};

use crate::{
    EncoderTaskAdapterError, NomosBackend, NomosDevelopmentCluster, NomosDevelopmentEvidence,
    adapter_error,
};

const LANDSCAPE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterKey {
    dimension: String,
    value: String,
}

#[derive(Debug, Clone)]
pub struct NomosDatasetLandscape {
    summaries: Vec<InspectionItem>,
    members: BTreeMap<String, Vec<String>>,
}

impl NomosDatasetLandscape {
    pub fn build(
        rows: &BTreeMap<String, Value>,
        current: &[NomosDevelopmentEvidence],
        baseline: &[NomosDevelopmentEvidence],
    ) -> Result<Self, EncoderTaskAdapterError> {
        if rows.is_empty() || current.is_empty() || baseline.is_empty() {
            return Err(adapter_error(
                "Dataset landscape requires training rows and development reports",
            ));
        }
        let mut training_members: BTreeMap<ClusterKey, Vec<String>> = BTreeMap::new();
        for (id, row) in rows {
            for key in row_clusters(row)? {
                training_members.entry(key).or_default().push(id.clone());
            }
        }
        let current_clusters = development_clusters(current)?;
        let baseline_clusters = development_clusters(baseline)?;
        let current_support: BTreeMap<_, _> = current
            .iter()
            .map(|evidence| (evidence.suite_key.clone(), evidence.retrieval_support))
            .collect();
        let mut keys: BTreeSet<_> = training_members.keys().cloned().collect();
        keys.extend(current_clusters.keys().map(|(_, key)| key.clone()));
        keys.extend(baseline_clusters.keys().map(|(_, key)| key.clone()));

        let total_rows = rows.len() as u64;
        let mut ranked = Vec::new();
        for key in keys {
            let training_rows = training_members.get(&key).map_or(0, Vec::len) as u64;
            let training_share_ppm = share_ppm(training_rows, total_rows);
            let mut development = Vec::new();
            let mut error_count = 0_u64;
            let mut regression_ppm = 0_u64;
            let mut underrepresentation_ppm = 0_u64;
            for evidence in current {
                let lookup = (evidence.suite_key.clone(), key.clone());
                let Some(observed) = current_clusters.get(&lookup) else {
                    continue;
                };
                let reference = baseline_clusters.get(&lookup);
                let recall = observed.metrics["recall_at_1"];
                let errors = ((observed.support as f64) * (1.0 - recall)).round() as u64;
                error_count = error_count.saturating_add(errors);
                let regression = reference
                    .map(|value| value.metrics["recall_at_1"] - recall)
                    .unwrap_or(0.0)
                    .max(0.0);
                regression_ppm = regression_ppm.max((regression * 1_000_000.0).round() as u64);
                let baseline_metrics = reference.map(|value| &value.metrics);
                let deltas =
                    baseline_metrics.map(|metrics| metric_deltas(&observed.metrics, metrics));
                let support_share_ppm =
                    share_ppm(observed.support, current_support[&evidence.suite_key]);
                let coverage_gap_ppm = support_share_ppm as i64 - training_share_ppm as i64;
                underrepresentation_ppm =
                    underrepresentation_ppm.max(coverage_gap_ppm.max(0) as u64);
                development.push(json!({
                    "suite": evidence.suite_key,
                    "support": observed.support,
                    "population": {
                        "retrievalStates": evidence.retrieval_support,
                        "scientificReportSupport": evidence.report_support,
                        "agent": evidence.agent_population,
                    },
                    "supportSharePpm": support_share_ppm,
                    "trainingSharePpm": training_share_ppm,
                    "coverageGapPpm": coverage_gap_ppm,
                    "estimatedTop1Errors": errors,
                    "current": observed.metrics,
                    "originalBaseline": baseline_metrics,
                    "deltaFromOriginalBaseline": deltas,
                    "sampledFailureDiagnostics": sampled_failure_diagnostics(evidence, &key),
                }));
            }
            let content = json!({
                "kind": "dataset_cluster",
                "schemaVersion": LANDSCAPE_SCHEMA_VERSION,
                "cluster": {
                    "dimension": key.dimension,
                    "value": key.value,
                    "overlapsOtherClusters": key.dimension == "expected_capability",
                },
                "training": {
                    "rows": training_rows,
                    "totalRows": total_rows,
                    "sharePpm": training_share_ppm,
                },
                "development": development,
                "priority": {
                    "estimatedTop1Errors": error_count,
                    "maximumRecallAt1RegressionPpm": regression_ppm,
                    "maximumUnderrepresentationPpm": underrepresentation_ppm,
                },
                "interpretation": "Training coverage is descriptive, not proof that more rows will improve the metric.",
            });
            let fingerprint = artifact_core::fingerprint(&content).map_err(adapter_error)?;
            let id = format!("dataset-cluster-{fingerprint}");
            ranked.push((
                std::cmp::Reverse(error_count),
                std::cmp::Reverse(regression_ppm),
                std::cmp::Reverse(underrepresentation_ppm),
                key.dimension.clone(),
                key.value.clone(),
                id,
                fingerprint,
                content,
                training_members.remove(&key).unwrap_or_default(),
            ));
        }
        ranked.sort_by(|left, right| {
            (&left.0, &left.1, &left.2, &left.3, &left.4)
                .cmp(&(&right.0, &right.1, &right.2, &right.3, &right.4))
        });
        let mut summaries = Vec::with_capacity(ranked.len());
        let mut members = BTreeMap::new();
        for (_, _, _, _, _, id, fingerprint, content, cluster_members) in ranked {
            members.insert(id.clone(), cluster_members);
            summaries.push(InspectionItem {
                id,
                fingerprint,
                content,
            });
        }
        Ok(Self { summaries, members })
    }

    pub fn summaries(&self) -> &[InspectionItem] {
        &self.summaries
    }

    pub fn sample_rows(
        &self,
        rows: &BTreeMap<String, Value>,
        cluster_ids: &[String],
        examples_per_cluster: u32,
    ) -> Result<Vec<InspectionItem>, EncoderTaskAdapterError> {
        if cluster_ids.is_empty()
            || cluster_ids.len() > 4
            || !(1..=4).contains(&examples_per_cluster)
            || cluster_ids.iter().collect::<BTreeSet<_>>().len() != cluster_ids.len()
        {
            return Err(adapter_error(
                "Dataset cluster inspection requires 1–4 unique clusters and 1–4 examples each",
            ));
        }
        let mut selected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for cluster_id in cluster_ids {
            let members = self.members.get(cluster_id).ok_or_else(|| {
                adapter_error("Dataset cluster identity was not returned by the landscape")
            })?;
            let mut ranked = members
                .iter()
                .map(|row_id| {
                    let rank = artifact_core::fingerprint(&json!({
                        "sampling": "cluster-content-hash-v1",
                        "cluster": cluster_id,
                        "row": row_id,
                    }))
                    .map_err(adapter_error)?;
                    Ok((rank, row_id))
                })
                .collect::<Result<Vec<_>, EncoderTaskAdapterError>>()?;
            ranked.sort();
            for (_, row_id) in ranked.into_iter().take(examples_per_cluster as usize) {
                selected
                    .entry(row_id.clone())
                    .or_default()
                    .insert(cluster_id.clone());
            }
        }
        let mut output = Vec::with_capacity(selected.len());
        for (row_id, clusters) in selected {
            let row = rows.get(&row_id).ok_or_else(|| {
                adapter_error("Dataset cluster member is missing from the pinned version")
            })?;
            let mut item = NomosBackend::training_row_for_agent(&row_id, row)?;
            let content = item
                .content
                .as_object_mut()
                .ok_or_else(|| adapter_error("Task-visible training row is not an object"))?;
            content.insert(
                "selectedForClusters".into(),
                serde_json::to_value(clusters).map_err(adapter_error)?,
            );
            content.insert(
                "sampling".into(),
                Value::String("cluster-content-hash-v1".into()),
            );
            item.fingerprint = artifact_core::fingerprint(&item.content).map_err(adapter_error)?;
            output.push(item);
        }
        Ok(output)
    }
}

fn development_clusters(
    evidence: &[NomosDevelopmentEvidence],
) -> Result<BTreeMap<(String, ClusterKey), NomosDevelopmentCluster>, EncoderTaskAdapterError> {
    let mut result = BTreeMap::new();
    for report in evidence {
        if report.clusters.is_empty() {
            return Err(adapter_error(
                "Development report predates dataset-intelligence cluster evidence",
            ));
        }
        for cluster in &report.clusters {
            let key = (
                report.suite_key.clone(),
                ClusterKey {
                    dimension: cluster.dimension.clone(),
                    value: cluster.value.clone(),
                },
            );
            if result.insert(key, cluster.clone()).is_some() {
                return Err(adapter_error("Development cluster identity is repeated"));
            }
        }
    }
    Ok(result)
}

fn row_clusters(row: &Value) -> Result<BTreeSet<ClusterKey>, EncoderTaskAdapterError> {
    crate::managed_training::validate_native_training_row(row)?;
    let task_kind = row["task_kind"]
        .as_str()
        .ok_or_else(|| adapter_error("Validated training row has no task kind"))?;
    let scenario = row
        .get("matrix_cell")
        .and_then(|value| value.get("scenario_family"))
        .and_then(Value::as_str)
        .unwrap_or("unspecified");
    let legal: BTreeSet<_> = row["legal_candidate_ids"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no legal candidate set"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let previous: BTreeSet<_> = row["previous_candidate_ids"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no previous candidate set"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let eligible: BTreeSet<_> = if task_kind == "recover" {
        legal.difference(&previous).copied().collect()
    } else {
        legal
    };
    let pool = eligible.len().to_string();
    let acceptable: BTreeSet<_> = row["label"]["acceptable_tools"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no acceptable tools"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut result = BTreeSet::from([
        ClusterKey {
            dimension: "task_kind".into(),
            value: task_kind.into(),
        },
        ClusterKey {
            dimension: "scenario_family".into(),
            value: scenario.into(),
        },
        ClusterKey {
            dimension: "candidate_pool_size".into(),
            value: pool,
        },
    ]);
    for tool in row["tool_registry"]["tools"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no tool registry"))?
    {
        if !tool["tool_id"]
            .as_str()
            .is_some_and(|id| acceptable.contains(id) && eligible.contains(id))
        {
            continue;
        }
        for capability in tool["capabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            result.insert(ClusterKey {
                dimension: "expected_capability".into(),
                value: capability.into(),
            });
        }
    }
    Ok(result)
}

fn metric_deltas(
    current: &BTreeMap<String, f64>,
    baseline: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    current
        .iter()
        .filter_map(|(name, value)| {
            baseline
                .get(name)
                .map(|reference| (name.clone(), value - reference))
        })
        .collect()
}

fn sampled_failure_diagnostics(evidence: &NomosDevelopmentEvidence, key: &ClusterKey) -> Value {
    let matching: Vec<_> = evidence
        .failures
        .iter()
        .filter(|failure| failure_matches_cluster(&failure.content, key))
        .collect();
    let mut predicted_capabilities = BTreeMap::<String, u64>::new();
    let mut rank_counts = BTreeMap::<String, u64>::new();
    for failure in &matching {
        for capability in failure.content["predictedCapabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            *predicted_capabilities
                .entry(capability.to_owned())
                .or_default() += 1;
        }
        let rank = failure.content["expectedRank"]
            .as_u64()
            .map_or_else(|| "missing".into(), |value| value.to_string());
        *rank_counts.entry(rank).or_default() += 1;
    }
    let examples: Vec<_> = matching
        .iter()
        .take(2)
        .map(|failure| {
            let question = failure.content["question"].as_str().unwrap_or_default();
            let preview: String = question.chars().take(1_000).collect();
            json!({
                "evidenceId": failure.id,
                "questionPreview": preview,
                "questionPreviewTruncated": question.chars().count() > 1_000,
                "expectedCapabilities": failure.content["expectedCapabilities"],
                "predictedCapabilities": failure.content["predictedCapabilities"],
                "expectedRank": failure.content["expectedRank"],
            })
        })
        .collect();
    json!({
        "evidenceScope": "retrieval_failure_sample",
        "reportSampleLimit": evidence.failure_sample_limit,
        "matchingFailures": matching.len(),
        "predictedCapabilityCounts": predicted_capabilities,
        "expectedRankCounts": rank_counts,
        "examples": examples,
        "interpretation": "This is a bounded diagnostic sample, not the complete error population.",
    })
}

fn failure_matches_cluster(content: &Value, key: &ClusterKey) -> bool {
    match key.dimension.as_str() {
        "task_kind" => content["taskKind"].as_str() == Some(&key.value),
        "expected_capability" => content["expectedCapabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|capability| capability == key.value),
        // The native failure projection does not expose these fields. Do not
        // guess membership from question text.
        "scenario_family" | "candidate_pool_size" => false,
        _ => false,
    }
}

fn share_ppm(part: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        ((part as f64 / total as f64) * 1_000_000.0).round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(recall: f64) -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("recall_at_1".into(), recall),
            ("recall_at_2".into(), recall),
            ("recall_at_3".into(), recall),
            ("mrr".into(), recall),
            ("mean_positive_margin".into(), 0.1),
        ])
    }

    fn evidence(suite: &str, recall: f64) -> NomosDevelopmentEvidence {
        evidence_with_support(suite, recall, 10, 10, 10)
    }

    fn evidence_with_support(
        suite: &str,
        recall: f64,
        report_support: u64,
        retrieval_support: u64,
        cluster_support: u64,
    ) -> NomosDevelopmentEvidence {
        NomosDevelopmentEvidence {
            schema_version: crate::development_evidence::DEVELOPMENT_EVIDENCE_SCHEMA_VERSION,
            report_id: uuid::Uuid::new_v4(),
            report_fingerprint: format!("sha256:{}", "1".repeat(64)),
            suite_key: suite.into(),
            artifact_fingerprint: format!("sha256:{}", "2".repeat(64)),
            report_support,
            retrieval_support,
            agent_population: crate::NomosAgentPopulation::NotRequired,
            clusters: vec![NomosDevelopmentCluster {
                dimension: "expected_capability".into(),
                value: "search".into(),
                support: cluster_support,
                metrics: metric(recall),
            }],
            failures: vec![],
            failure_sample_limit: 50,
        }
    }

    fn row(question: &str) -> Value {
        json!({
            "schema_version":"decision-state.v2", "evaluation_partition":"train",
            "accepted":true, "decision_state_id":format!("state-{question}"),
            "question":question, "task_kind":"route",
            "previous_candidate_ids":[], "legal_candidate_ids":["tool-1","tool-2"],
            "label":{"acceptable_tools":["tool-1"],"hard_negative_tools":["tool-2"]},
            "tool_registry":{"registry_id":"registry","registry_fingerprint":"sha256:registry","tools":[
                {"tool_id":"tool-1","description":"search", "capabilities":["search"], "side_effect_class":"none", "argument_schema":{}, "evidence_roles":["observation"], "input_modalities":["text"], "output_modalities":["evidence"], "tool_family":"search"},
                {"tool_id":"tool-2","description":"write", "capabilities":["write"], "side_effect_class":"write", "argument_schema":{}, "evidence_roles":["action"], "input_modalities":["text"], "output_modalities":["text"], "tool_family":"write"}
            ]}
        })
    }

    #[test]
    fn complete_training_scan_joins_development_clusters_and_samples_by_cluster() {
        let rows = BTreeMap::from([
            ("row-a".into(), row("find alpha")),
            ("row-b".into(), row("find beta")),
        ]);
        let landscape = NomosDatasetLandscape::build(
            &rows,
            &[evidence("development", 0.5)],
            &[evidence("development", 0.8)],
        )
        .unwrap();
        let search = landscape
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        assert_eq!(search.content["training"]["rows"], 2);
        assert_eq!(search.content["development"][0]["estimatedTop1Errors"], 5);
        assert_eq!(
            search.content["priority"]["maximumRecallAt1RegressionPpm"],
            300_000
        );
        let sampled = landscape
            .sample_rows(&rows, std::slice::from_ref(&search.id), 2)
            .unwrap();
        assert_eq!(sampled.len(), 2);
        assert!(sampled.iter().all(|item| {
            item.content["selectedForClusters"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &search.id))
        }));
    }

    #[test]
    fn retrieval_cluster_share_does_not_use_smaller_agent_report_support() {
        let rows = BTreeMap::from([("row-a".into(), row("find alpha"))]);
        let mut current = evidence_with_support("development", 0.5, 16, 1_000, 100);
        current.agent_population = crate::NomosAgentPopulation::Available {
            artifact_fingerprint: format!("sha256:{}", "3".repeat(64)),
            sessions: 16,
            tool_call_attempts: 32,
            valid_execution_attempts: 30,
            metric_denominators: BTreeMap::from([
                ("agent_success_rate".into(), Some(16)),
                ("agent_wrong_tool_execution_rate".into(), Some(30)),
            ]),
        };
        let landscape = NomosDatasetLandscape::build(
            &rows,
            &[current],
            &[evidence_with_support("development", 0.8, 16, 1_000, 100)],
        )
        .unwrap();
        let search = landscape
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        assert_eq!(search.content["development"][0]["supportSharePpm"], 100_000);
        assert_eq!(
            search.content["development"][0]["population"]["retrievalStates"],
            1_000
        );
        assert_eq!(
            search.content["development"][0]["population"]["scientificReportSupport"],
            16
        );
        assert_eq!(
            search.content["development"][0]["population"]["agent"]["sessions"],
            16
        );
    }
}
