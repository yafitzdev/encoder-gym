//! Development-only inspection of already-persisted native retrieval failures.
//! This path never invokes Python or an evaluator and never opens suite rows.

use std::collections::BTreeSet;

use encoder_optimization_core::agent::InspectionItem;
use serde::Serialize;

use crate::{
    EncoderTaskAdapterError, EvaluationReport, EvidenceRole, ExternalProjectSnapshot,
    MetricContract, NomosBackend, Value, adapter_error, evaluation_component_root,
    evaluation_model_root, workspace_relative,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosDevelopmentEvidence {
    pub report_id: uuid::Uuid,
    pub report_fingerprint: String,
    pub suite_key: String,
    pub artifact_fingerprint: String,
    pub failures: Vec<InspectionItem>,
    /// The native evaluator stores at most fifty retrieval disagreements.
    /// This is a recorded diagnostic sample, not all predictions.
    pub failure_sample_limit: u32,
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
        let path = self.resolve_existing(&workspace_relative(&self.root, &expected)?).map_err(|_| adapter_error("The pinned development report has no saved retrieval diagnostics; no evaluation was rerun"))?;
        if std::fs::metadata(&path).map_err(adapter_error)?.len() > 4_194_304 {
            return Err(adapter_error(
                "Development diagnostics exceed the 4 MiB inspection limit",
            ));
        }
        let raw: Value = serde_json::from_slice(&std::fs::read(path).map_err(adapter_error)?)
            .map_err(|_| adapter_error("Saved development diagnostics are not valid JSON"))?;
        // Analysis consumes saved predictions, not checkpoint weights. Their
        // immutable model identity already determines the diagnostic path.
        // Do not rehash a multi-gigabyte checkpoint merely to read its failures.
        crate::validate_relative(&report.model.key)?;
        let expected_model = report.model.key.replace('\\', "/");
        let failures = normalize_failures(&raw, &expected_model, &suite.path, report)?;
        Ok(NomosDevelopmentEvidence {
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            suite_key: report.suite_key.clone(),
            artifact_fingerprint: artifact_core::fingerprint(&raw).map_err(adapter_error)?,
            failures,
            failure_sample_limit: 50,
        })
    }
}

fn normalize_failures(
    raw: &Value,
    model: &str,
    input: &str,
    report: &EvaluationReport,
) -> Result<Vec<InspectionItem>, EncoderTaskAdapterError> {
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
    disagreements
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
        .collect()
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
        serde_json::json!({"model":"models/candidate", "private_native_field":"must not enter inspection", "inputs":{"data/development.jsonl":{"metrics":{"mrr":0.75},"disagreements":[{"decision_state_id":"dev-1","task_kind":"read","question":"Find the relevant reference","expected_capabilities":["read"],"predicted_capabilities":["write"],"expected_rank":2,"unrelated_trace":"must not enter inspection"}]}}})
    }

    #[test]
    fn persisted_failures_are_scoped_to_the_exact_model_input_and_report_metrics() {
        let report = report();
        let failures = normalize_failures(
            &raw(),
            "models/candidate",
            "data/development.jsonl",
            &report,
        )
        .unwrap();
        assert_eq!(failures.len(), 1);
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
            normalize_failures(&raw(), "models/other", "data/development.jsonl", &report).is_err()
        );
        assert!(
            normalize_failures(&raw(), "models/candidate", "data/other.jsonl", &report).is_err()
        );
        let mut changed = raw();
        changed["inputs"]["data/development.jsonl"]["metrics"]["mrr"] = 0.5.into();
        assert!(
            normalize_failures(
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
            normalize_failures(
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
            normalize_failures(
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
            normalize_failures(
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
}
