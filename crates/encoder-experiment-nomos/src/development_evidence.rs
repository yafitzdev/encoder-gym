//! Development-only inspection of already-persisted native retrieval failures.
//! This path never invokes Python or an evaluator and never opens suite rows.

use std::collections::{BTreeMap, BTreeSet};

use encoder_optimization_core::agent::InspectionItem;
use serde::Serialize;

use crate::{
    EncoderTaskAdapterError, EvaluationReport, EvidenceRole, ExternalProjectSnapshot,
    MetricContract, NativeAgentEvaluation, NomosBackend, Value, adapter_error,
    evaluation_component_root, evaluation_model_root, workspace_relative,
};

pub const DEVELOPMENT_EVIDENCE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosDevelopmentEvidence {
    pub schema_version: u32,
    pub report_id: uuid::Uuid,
    pub report_fingerprint: String,
    pub suite_key: String,
    /// Canonical fingerprint of the saved native retrieval report.
    pub artifact_fingerprint: String,
    /// The scientific report's conservative cross-component support. This is
    /// not a retrieval denominator when Agent sessions are also evaluated.
    pub report_support: u64,
    /// Exact number of retrieval states from the saved native retrieval report.
    pub retrieval_support: u64,
    pub agent_population: NomosAgentPopulation,
    /// Complete aggregate evaluator slices. These contain no row text and are
    /// safe for development-only dataset-landscape analysis.
    pub clusters: Vec<NomosDevelopmentCluster>,
    pub failures: Vec<InspectionItem>,
    /// The native evaluator stores at most fifty retrieval disagreements.
    /// This is a recorded diagnostic sample, not all predictions.
    pub failure_sample_limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum NomosAgentPopulation {
    NotRequired,
    Missing,
    Corrupt,
    Incompatible,
    Available {
        #[serde(rename = "artifactFingerprint")]
        artifact_fingerprint: String,
        sessions: u64,
        #[serde(rename = "toolCallAttempts")]
        tool_call_attempts: u64,
        #[serde(rename = "validExecutionAttempts")]
        valid_execution_attempts: u64,
        /// A null value means the saved native report does not expose the
        /// underlying count needed to state that metric's denominator.
        #[serde(rename = "metricDenominators")]
        metric_denominators: BTreeMap<String, Option<u64>>,
    },
}

#[derive(Debug)]
pub(crate) enum NomosDevelopmentEvidenceRead {
    Available(Box<NomosDevelopmentEvidence>),
    Missing,
    Corrupt,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosDevelopmentCluster {
    pub dimension: String,
    pub value: String,
    pub support: u64,
    pub metrics: BTreeMap<String, f64>,
}

impl NomosBackend {
    /// A bounded task-visible projection, not the full repeated registry in
    /// every source row. The owning dataset adapter supplies stable membership.
    pub fn training_row_for_agent(
        member_id: &str,
        row: &Value,
    ) -> Result<InspectionItem, EncoderTaskAdapterError> {
        crate::managed_training::validate_native_training_row(row)?;
        let legal: BTreeSet<_> = row["legal_candidate_ids"]
            .as_array()
            .expect("validated legal IDs")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let tools: Vec<_> = row["tool_registry"]["tools"].as_array().expect("validated registry").iter().filter(|tool| tool["tool_id"].as_str().is_some_and(|id| legal.contains(id))).map(|tool| serde_json::json!({
            "id":tool["tool_id"], "description":tool["description"], "capabilities":tool["capabilities"], "sideEffects":tool["side_effect_class"],
        })).collect();
        let state: serde_json::Map<String, Value> = [
            "expansion_context",
            "history",
            "plan",
            "agent_state",
            "source_state",
            "query_state",
            "governance",
            "resource_state",
        ]
        .into_iter()
        .filter_map(|key| row.get(key).cloned().map(|value| (key.to_owned(), value)))
        .collect();
        let content = serde_json::json!({"question":row["question"],"taskKind":row["task_kind"],"label":row["label"],"previousCandidateIds":row["previous_candidate_ids"],"tools":tools,"state":state,"generationPolicy":{"writableFields":["question"],"preservedFields":["state","label","toolRegistry","taskKind","previousCandidateIds"]}});
        if serde_json::to_vec(&content).map_err(adapter_error)?.len() > 32768 {
            return Err(adapter_error(
                "Training example exceeds the Agent's 32 KiB inspection limit",
            ));
        }
        Ok(InspectionItem {
            id: member_id.into(),
            fingerprint: artifact_core::fingerprint(&content).map_err(adapter_error)?,
            content,
        })
    }

    pub fn read_development_evidence(
        &self,
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        report: &EvaluationReport,
    ) -> Result<NomosDevelopmentEvidence, EncoderTaskAdapterError> {
        match self.read_development_evidence_status(project, contract, report)? {
            NomosDevelopmentEvidenceRead::Available(evidence) => Ok(*evidence),
            NomosDevelopmentEvidenceRead::Missing => Err(adapter_error(
                "The pinned development report has no saved retrieval diagnostics; no evaluation was rerun",
            )),
            NomosDevelopmentEvidenceRead::Corrupt => Err(adapter_error(
                "The pinned development report's saved retrieval diagnostics are corrupt",
            )),
            NomosDevelopmentEvidenceRead::Incompatible => Err(adapter_error(
                "The pinned development report's saved retrieval diagnostics are incompatible",
            )),
        }
    }

    pub(crate) fn read_development_evidence_status(
        &self,
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        report: &EvaluationReport,
    ) -> Result<NomosDevelopmentEvidenceRead, EncoderTaskAdapterError> {
        // Check the role and full report binding before even resolving a path.
        if report.evidence_role != EvidenceRole::Development {
            return Err(adapter_error(
                "Only development evidence may enter optimization",
            ));
        }
        report
            .validate_integrity(project, contract)
            .map_err(adapter_error)?;
        project.validate_integrity().map_err(adapter_error)?;
        if !self.supports_identity(&project.backend) {
            return Err(adapter_error(
                "Development evidence belongs to another task adapter",
            ));
        }
        let configuration = Self::task_configuration(project)?;
        let suite = configuration.suites.get(&report.suite_key).ok_or_else(|| {
            adapter_error("Development report suite is not in the pinned project")
        })?;
        if suite.role != EvidenceRole::Development || suite.fingerprint != report.suite_fingerprint
        {
            return Err(adapter_error(
                "Development report does not match its pinned suite",
            ));
        }
        let model_root = evaluation_model_root(&self.root, &report.model)?;
        let directory =
            evaluation_component_root(&model_root, "retrieval", &suite.retrieval_fingerprint)?;
        let expected = directory.join(format!("{}.json", report.suite_key));
        if !expected.exists() {
            return Ok(NomosDevelopmentEvidenceRead::Missing);
        }
        let path = match self.resolve_existing(&workspace_relative(&self.root, &expected)?) {
            Ok(path) => path,
            Err(_) => return Ok(NomosDevelopmentEvidenceRead::Corrupt),
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 4_194_304 => bytes,
            _ => return Ok(NomosDevelopmentEvidenceRead::Corrupt),
        };
        let raw: Value = match serde_json::from_slice(&bytes) {
            Ok(raw) => raw,
            Err(_) => return Ok(NomosDevelopmentEvidenceRead::Corrupt),
        };
        // Analysis consumes saved predictions, not checkpoint weights. Their
        // immutable model identity already determines the diagnostic path.
        // Do not rehash a multi-gigabyte checkpoint merely to read its failures.
        crate::validate_relative(&report.model.key)?;
        let expected_model = report.model.key.replace('\\', "/");
        let (retrieval_support, failures, clusters) =
            match normalize_development_evidence(&raw, &expected_model, &suite.path, report) {
                Ok(evidence) => evidence,
                Err(_) => return Ok(NomosDevelopmentEvidenceRead::Incompatible),
            };
        let agent_population =
            self.read_agent_population(&model_root, &configuration, suite, contract, report)?;
        Ok(NomosDevelopmentEvidenceRead::Available(Box::new(
            NomosDevelopmentEvidence {
                schema_version: DEVELOPMENT_EVIDENCE_SCHEMA_VERSION,
                report_id: report.id,
                report_fingerprint: report.fingerprint.clone(),
                suite_key: report.suite_key.clone(),
                artifact_fingerprint: artifact_core::fingerprint(&raw).map_err(adapter_error)?,
                report_support: report.support,
                retrieval_support,
                agent_population,
                clusters,
                failures,
                failure_sample_limit: 50,
            },
        )))
    }

    fn read_agent_population(
        &self,
        model_root: &std::path::Path,
        configuration: &crate::TaskConfiguration,
        suite: &crate::SuiteConfiguration,
        contract: &MetricContract,
        report: &EvaluationReport,
    ) -> Result<NomosAgentPopulation, EncoderTaskAdapterError> {
        let agent_metrics: Vec<_> = contract
            .definitions
            .iter()
            .filter(|definition| definition.key.starts_with("agent_"))
            .map(|definition| definition.key.clone())
            .collect();
        if agent_metrics.is_empty() {
            return Ok(NomosAgentPopulation::NotRequired);
        }
        let agent = &configuration.agent_evaluation;
        let agent_suite = agent.suite(suite.role);
        let root = evaluation_component_root(model_root, "agent", &suite.agent_fingerprint)?;
        let output = root.join(format!("{}.json", report.suite_key));
        let trace = root.join(format!("{}.trace.jsonl", report.suite_key));
        if !output.is_file() || !trace.is_file() {
            return Ok(NomosAgentPopulation::Missing);
        }
        let output = match self.resolve_existing(&workspace_relative(&self.root, &output)?) {
            Ok(path) => path,
            Err(_) => return Ok(NomosAgentPopulation::Corrupt),
        };
        if self
            .resolve_existing(&workspace_relative(&self.root, &trace)?)
            .is_err()
        {
            return Ok(NomosAgentPopulation::Corrupt);
        }
        let bytes = match std::fs::read(output) {
            Ok(bytes) if bytes.len() <= 4_194_304 => bytes,
            _ => return Ok(NomosAgentPopulation::Corrupt),
        };
        let raw: Value = match serde_json::from_slice(&bytes) {
            Ok(raw) => raw,
            Err(_) => return Ok(NomosAgentPopulation::Corrupt),
        };
        let native: NativeAgentEvaluation = match serde_json::from_value(raw.clone()) {
            Ok(native) => native,
            Err(_) => return Ok(NomosAgentPopulation::Incompatible),
        };
        let summary = match native.validate_and_summary(agent, agent_suite) {
            Ok(summary) => summary,
            Err(_) => return Ok(NomosAgentPopulation::Incompatible),
        };
        let normalized = match summary.normalized_metrics() {
            Ok(normalized) => normalized,
            Err(_) => return Ok(NomosAgentPopulation::Incompatible),
        };
        if agent_metrics
            .iter()
            .any(|key| normalized.get(key).copied() != report.metrics.get(key).copied())
        {
            return Ok(NomosAgentPopulation::Incompatible);
        }
        let valid_execution_attempts = summary.tool_call_attempts - summary.invalid_calls;
        let mut metric_denominators = BTreeMap::new();
        for key in agent_metrics {
            let denominator = match key.as_str() {
                "agent_success_rate" | "agent_mean_completed_stage_rate" => Some(summary.sessions),
                "agent_successful_execution_rate"
                | "agent_tool_selection_accuracy"
                | "agent_schema_valid_call_rate"
                | "agent_visible_oracle_hit_rate"
                | "agent_invalid_call_rate"
                | "agent_prompt_tokens_per_attempt" => Some(summary.tool_call_attempts),
                "agent_wrong_tool_execution_rate" => Some(valid_execution_attempts),
                // The native report stores only this rate, not the underlying
                // sum of available tool descriptions.
                "agent_tool_description_reduction" => None,
                _ => None,
            };
            metric_denominators.insert(key, denominator);
        }
        Ok(NomosAgentPopulation::Available {
            artifact_fingerprint: artifact_core::fingerprint(&raw).map_err(adapter_error)?,
            sessions: summary.sessions,
            tool_call_attempts: summary.tool_call_attempts,
            valid_execution_attempts,
            metric_denominators,
        })
    }
}

fn normalize_development_evidence(
    raw: &Value,
    model: &str,
    input: &str,
    report: &EvaluationReport,
) -> Result<(u64, Vec<InspectionItem>, Vec<NomosDevelopmentCluster>), EncoderTaskAdapterError> {
    if report.evidence_role != EvidenceRole::Development
        || raw
            .get("model")
            .and_then(Value::as_str)
            .map(|v| v.replace('\\', "/"))
            .as_deref()
            != Some(model)
    {
        return Err(adapter_error(
            "Native diagnostics model or evidence role does not match",
        ));
    }
    let inputs = raw
        .get("inputs")
        .and_then(Value::as_object)
        .ok_or_else(|| adapter_error("Native diagnostics have no input binding"))?;
    if inputs.len() != 1 {
        return Err(adapter_error(
            "Native diagnostics must belong to exactly one development suite",
        ));
    }
    let (path, value) = inputs.iter().next().expect("exactly one input");
    if path.replace('\\', "/") != input {
        return Err(adapter_error(
            "Native diagnostics input is not the pinned development suite",
        ));
    }
    let metrics = value
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| adapter_error("Native diagnostics metrics are missing"))?;
    let retrieval_support = metrics
        .get("states")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| adapter_error("Native diagnostics retrieval support is invalid"))?;
    if report.support > retrieval_support {
        return Err(adapter_error(
            "Scientific report support exceeds its retrieval population",
        ));
    }
    for (name, score) in &report.metrics {
        if name.starts_with("agent_") {
            continue;
        }
        if metrics.get(name).and_then(Value::as_f64) != Some(*score) {
            return Err(adapter_error(
                "Native diagnostics metrics differ from the persisted development report",
            ));
        }
    }
    let clusters = normalize_clusters(value)?;
    if clusters
        .iter()
        .any(|cluster| cluster.support > retrieval_support)
    {
        return Err(adapter_error(
            "Native development cluster exceeds its retrieval population",
        ));
    }
    let disagreements = value
        .get("disagreements")
        .and_then(Value::as_array)
        .ok_or_else(|| adapter_error("Native report has no recorded failure sample"))?;
    if disagreements.len() > 50 {
        return Err(adapter_error(
            "Native failure sample exceeds its evaluator contract",
        ));
    }
    let mut ids = BTreeSet::new();
    let failures = disagreements
        .iter()
        .map(|failure| {
            let source_id = failure
                .get("decision_state_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty() && id.len() <= 200)
                .ok_or_else(|| {
                    adapter_error("Development failure has no stable source-row identity")
                })?;
            if !ids.insert(source_id) {
                return Err(adapter_error(
                    "Development failure sample repeats a source row",
                ));
            }
            // Explicit field projection: file paths, native traces and unrelated
            // report fields cannot leak through the generic inspection port.
            let content = serde_json::json!({
                "reportId": report.id, "suite": report.suite_key, "sourceRowId": source_id,
                "evidenceScope": "retrieval_failure_sample", "sampleLimit": 50,
                "taskKind": failure.get("task_kind"), "question": failure.get("question"),
                "expectedCapabilities": failure.get("expected_capabilities"),
                "predictedCapabilities": failure.get("predicted_capabilities"),
                "expectedRank": failure.get("expected_rank"),
            });
            if serde_json::to_vec(&content).map_err(adapter_error)?.len() > 16384 {
                return Err(adapter_error(
                    "Development failure exceeds the bounded inspection size",
                ));
            }
            let fingerprint = artifact_core::fingerprint(&content).map_err(adapter_error)?;
            Ok(InspectionItem {
                id: format!("{}:{}", report.id, &fingerprint[7..23]),
                fingerprint,
                content,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if failures.len() as u64 > retrieval_support {
        return Err(adapter_error(
            "Native failure sample exceeds its retrieval population",
        ));
    }
    Ok((retrieval_support, failures, clusters))
}

fn normalize_clusters(
    value: &Value,
) -> Result<Vec<NomosDevelopmentCluster>, EncoderTaskAdapterError> {
    let dimensions = [
        ("by_expected_capability", "expected_capability"),
        ("by_task_kind", "task_kind"),
        ("by_scenario_family", "scenario_family"),
        ("by_pool_size", "candidate_pool_size"),
    ];
    let mut clusters = Vec::new();
    for (field, dimension) in dimensions {
        let Some(groups) = value.get(field) else {
            // Historical reports predate aggregate slices. They remain valid
            // V1 evidence but cannot satisfy a V2 landscape.
            continue;
        };
        let groups = groups
            .as_object()
            .ok_or_else(|| adapter_error("Native development cluster map is invalid"))?;
        for (group, raw_metrics) in groups {
            if group.trim().is_empty() || group.len() > 200 {
                return Err(adapter_error("Native development cluster value is invalid"));
            }
            let raw_metrics = raw_metrics
                .as_object()
                .ok_or_else(|| adapter_error("Native development cluster metrics are invalid"))?;
            let support = raw_metrics
                .get("states")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .ok_or_else(|| adapter_error("Native development cluster support is invalid"))?;
            let mut metrics = BTreeMap::new();
            for name in [
                "recall_at_1",
                "recall_at_2",
                "recall_at_3",
                "mrr",
                "mean_positive_margin",
            ] {
                let metric = raw_metrics
                    .get(name)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| adapter_error("Native development cluster metric is invalid"))?;
                if name != "mean_positive_margin" && !(0.0..=1.0).contains(&metric) {
                    return Err(adapter_error("Native development cluster rate is invalid"));
                }
                metrics.insert(name.into(), metric);
            }
            clusters.push(NomosDevelopmentCluster {
                dimension: dimension.into(),
                value: group.clone(),
                support,
                metrics,
            });
        }
    }
    clusters.sort_by(|left, right| {
        left.dimension
            .cmp(&right.dimension)
            .then_with(|| left.value.cmp(&right.value))
    });
    Ok(clusters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoder_experiment_core::domain::ModelArtifactIdentity;
    use std::collections::BTreeMap;

    fn report() -> EvaluationReport {
        EvaluationReport {
            schema_version: 1,
            id: uuid::Uuid::new_v4(),
            project_snapshot_id: uuid::Uuid::new_v4(),
            project_snapshot_fingerprint: format!("sha256:{}", "1".repeat(64)),
            model: ModelArtifactIdentity {
                id: uuid::Uuid::new_v4(),
                key: "candidate".into(),
                format: "sentence-transformers".into(),
                fingerprint: format!("sha256:{}", "2".repeat(64)),
                bytes: 100,
            },
            evidence_role: EvidenceRole::Development,
            suite_key: "development".into(),
            suite_fingerprint: format!("sha256:{}", "3".repeat(64)),
            metric_contract_fingerprint: format!("sha256:{}", "4".repeat(64)),
            metrics: BTreeMap::from([("mrr".into(), 0.75)]),
            support: 4,
            created_at: chrono::Utc::now(),
            reference: None,
            fingerprint: format!("sha256:{}", "5".repeat(64)),
        }
    }

    fn raw() -> Value {
        serde_json::json!({"model":"models/candidate", "private_native_field":"must not enter inspection", "inputs":{"data/development.jsonl":{"metrics":{"states":100,"mrr":0.75},"disagreements":[{"decision_state_id":"dev-1","task_kind":"read","question":"Find the relevant reference","expected_capabilities":["read"],"predicted_capabilities":["write"],"expected_rank":2,"unrelated_trace":"must not enter inspection"}]}}})
    }

    #[test]
    fn persisted_failures_are_scoped_to_the_exact_model_input_and_report_metrics() {
        let report = report();
        let (retrieval_support, failures, clusters) = normalize_development_evidence(
            &raw(),
            "models/candidate",
            "data/development.jsonl",
            &report,
        )
        .unwrap();
        assert_eq!(retrieval_support, 100);
        assert_eq!(failures.len(), 1);
        assert!(clusters.is_empty());
        assert_eq!(failures[0].content["sourceRowId"], "dev-1");
        assert_eq!(failures[0].content["expectedRank"], 2);
        assert_eq!(
            failures[0].content["evidenceScope"],
            "retrieval_failure_sample"
        );
        assert_eq!(failures[0].content["sampleLimit"], 50);
        assert!(
            !serde_json::to_string(&failures)
                .unwrap()
                .contains("must not enter inspection")
        );
        assert!(
            normalize_development_evidence(
                &raw(),
                "models/other",
                "data/development.jsonl",
                &report
            )
            .is_err()
        );
        assert!(
            normalize_development_evidence(&raw(), "models/candidate", "data/other.jsonl", &report)
                .is_err()
        );
        let mut changed = raw();
        changed["inputs"]["data/development.jsonl"]["metrics"]["mrr"] = 0.5.into();
        assert!(
            normalize_development_evidence(
                &changed,
                "models/candidate",
                "data/development.jsonl",
                &report
            )
            .is_err()
        );
    }

    #[test]
    fn sealed_reports_and_repeated_or_missing_failure_identities_are_rejected() {
        let mut report = report();
        report.evidence_role = EvidenceRole::SealedAcceptance;
        assert!(
            normalize_development_evidence(
                &raw(),
                "models/candidate",
                "data/development.jsonl",
                &report
            )
            .is_err()
        );
        report.evidence_role = EvidenceRole::Development;
        let mut repeated = raw();
        let failure = repeated["inputs"]["data/development.jsonl"]["disagreements"][0].clone();
        repeated["inputs"]["data/development.jsonl"]["disagreements"]
            .as_array_mut()
            .unwrap()
            .push(failure);
        assert!(
            normalize_development_evidence(
                &repeated,
                "models/candidate",
                "data/development.jsonl",
                &report
            )
            .is_err()
        );
        repeated["inputs"]["data/development.jsonl"]["disagreements"][0]["decision_state_id"] =
            Value::Null;
        assert!(
            normalize_development_evidence(
                &repeated,
                "models/candidate",
                "data/development.jsonl",
                &report
            )
            .is_err()
        );
    }

    #[test]
    fn training_inspection_cannot_admit_a_protected_partition() {
        let row = serde_json::json!({"schema_version":"decision-state.v2","evaluation_partition":"sealed","accepted":true});
        assert!(NomosBackend::training_row_for_agent("row-1", &row).is_err());
    }

    #[test]
    fn native_aggregate_dimensions_are_normalized_for_dataset_intelligence() {
        let metrics = serde_json::json!({
            "states": 4,
            "recall_at_1": 0.5,
            "recall_at_2": 0.75,
            "recall_at_3": 1.0,
            "mrr": 0.7,
            "mean_positive_margin": -0.1,
        });
        let value = serde_json::json!({
            "by_expected_capability": {"search": metrics},
            "by_task_kind": {"route": metrics},
            "by_scenario_family": {"politics": metrics},
            "by_pool_size": {"8": metrics},
        });
        let clusters = normalize_clusters(&value).unwrap();
        assert_eq!(clusters.len(), 4);
        assert!(clusters.iter().any(|cluster| {
            cluster.dimension == "expected_capability"
                && cluster.value == "search"
                && cluster.support == 4
                && cluster.metrics["recall_at_1"] == 0.5
        }));
        assert!(clusters.iter().any(|cluster| {
            cluster.dimension == "scenario_family" && cluster.value == "politics"
        }));
    }
}
