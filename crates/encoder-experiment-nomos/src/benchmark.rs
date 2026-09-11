//! Normalize recorded evaluation context without reading native or sealed files.
use super::*;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition,
    metrics::{MetricContract, MetricDefinition, MetricDirection, MetricGate, MetricGateCondition},
    protocol::ExperimentProtocol,
};
use serde::Serialize;

pub const INITIAL_BENCHMARK_MAXIMUM_EVALUATION_SECONDS: u64 = 1_800;

/// Adapter-owned defaults for creating the first shared project benchmark.
/// The user selects the project; native suite and metric details remain
/// inspectable but do not become renderer-authored execution input.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosBenchmarkPlan {
    pub metric_contract: MetricContract,
    pub development_suite_keys: Vec<String>,
    pub sealed_suite_key: String,
    pub maximum_evaluation_seconds: u64,
}

impl NomosBackend {
    pub fn initial_benchmark_plan(
        project: &ExternalProjectSnapshot,
    ) -> Result<NomosBenchmarkPlan, EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        if project.backend.name != ADAPTER_NAME
            || project.backend.protocol_version != ADAPTER_PROTOCOL_VERSION
        {
            return Err(adapter_error("The project uses another evaluation adapter"));
        }
        let configuration = Self::task_configuration(project)?;
        configuration.validate()?;
        let mut development_suite_keys = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::Development)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        development_suite_keys.sort();
        let sealed = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::SealedAcceptance)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if development_suite_keys.is_empty() || sealed.len() != 1 {
            return Err(adapter_error(
                "The project must define development tests and one final holdout",
            ));
        }

        let higher = [
            "recall_at_1",
            "recall_at_2",
            "recall_at_3",
            "mrr",
            "mean_positive_margin",
            "agent_success_rate",
            "agent_mean_completed_stage_rate",
            "agent_successful_execution_rate",
            "agent_tool_selection_accuracy",
            "agent_schema_valid_call_rate",
            "agent_visible_oracle_hit_rate",
            "agent_tool_description_reduction",
        ];
        let lower = [
            "agent_invalid_call_rate",
            "agent_wrong_tool_execution_rate",
            "agent_prompt_tokens_per_attempt",
        ];
        let definitions = higher
            .into_iter()
            .map(|key| MetricDefinition::new(key, MetricDirection::HigherIsBetter))
            .chain(
                lower
                    .into_iter()
                    .map(|key| MetricDefinition::new(key, MetricDirection::LowerIsBetter)),
            )
            .collect::<Result<Vec<_>, _>>()
            .map_err(adapter_error)?;
        let mut gates = Vec::new();
        for role in [EvidenceRole::Development, EvidenceRole::SealedAcceptance] {
            gates.push(
                MetricGate::new(
                    "mrr",
                    role,
                    MetricGateCondition::MinimumImprovement { value: 0.0005 },
                )
                .map_err(adapter_error)?,
            );
            for key in ["recall_at_1", "recall_at_2", "recall_at_3"] {
                gates.push(
                    MetricGate::new(
                        key,
                        role,
                        MetricGateCondition::MaximumRegression { value: 0.0 },
                    )
                    .map_err(adapter_error)?,
                );
            }
            gates.push(
                MetricGate::new(
                    "mean_positive_margin",
                    role,
                    MetricGateCondition::MaximumRegression { value: 0.005 },
                )
                .map_err(adapter_error)?,
            );
            for key in [
                "agent_success_rate",
                "agent_mean_completed_stage_rate",
                "agent_successful_execution_rate",
                "agent_tool_selection_accuracy",
                "agent_schema_valid_call_rate",
                "agent_visible_oracle_hit_rate",
                "agent_invalid_call_rate",
                "agent_wrong_tool_execution_rate",
            ] {
                gates.push(
                    MetricGate::new(
                        key,
                        role,
                        MetricGateCondition::MaximumRegression { value: 0.0 },
                    )
                    .map_err(adapter_error)?,
                );
            }
        }
        Ok(NomosBenchmarkPlan {
            metric_contract: MetricContract::create(definitions, "mrr", gates)
                .map_err(adapter_error)?,
            development_suite_keys,
            sealed_suite_key: sealed[0].clone(),
            maximum_evaluation_seconds: INITIAL_BENCHMARK_MAXIMUM_EVALUATION_SECONDS,
        })
    }

    /// Verify that the current bound runtime can execute this shared benchmark
    /// without reading suite contents or requiring its original baseline run.
    pub fn verify_benchmark_definition(
        project: &ExternalProjectSnapshot,
        definition: &BenchmarkDefinition,
    ) -> Result<(), EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        definition.validate_integrity().map_err(adapter_error)?;
        if project.task != definition.task
            || project.backend != definition.backend
            || project.source_revision != definition.source_revision
        {
            return Err(adapter_error(
                "The shared benchmark uses another task adapter or runtime revision",
            ));
        }
        let configuration = Self::task_configuration(project)?;
        configuration.validate()?;
        let mut suites = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        for selected in &definition.suites {
            let suite = configuration
                .suites
                .get(&selected.key)
                .ok_or_else(|| adapter_error("Shared benchmark suite is missing"))?;
            if suite.fingerprint != selected.fingerprint || suite.role != selected.role {
                return Err(adapter_error(
                    "Shared benchmark suite differs from the current runtime",
                ));
            }
            let input = project
                .inputs
                .iter()
                .find(|input| input.key == suite.path && input.role == suite.role)
                .ok_or_else(|| {
                    adapter_error("Shared benchmark membership is not pinned to its evidence role")
                })?;
            inputs.insert(input.key.clone(), input);
            suites.insert(
                selected.key.clone(),
                json!({"fingerprint":suite.fingerprint,
                    "retrieval":suite.retrieval_fingerprint,"agent":suite.agent_fingerprint}),
            );
        }
        let context = artifact_core::fingerprint(&json!({
            "schema":"nomos-benchmark-context-v1", "retrieval_evaluator":RETRIEVAL_EVALUATOR_VERSION,
            "agent_evaluator":AGENT_EVALUATOR_VERSION, "dense_text":DENSE_TEXT_VERSION,
            "agent_evaluation":project.task_configuration.get("agent_evaluation"),
            "inputs":inputs, "suites":suites,
        }))
        .map_err(adapter_error)?;
        if context != definition.evaluation_configuration_fingerprint {
            return Err(adapter_error(
                "Shared benchmark evaluation configuration changed",
            ));
        }
        Ok(())
    }

    /// The scientific store verifies the recorded project/protocol first. This
    /// extracts their evaluator identity; it does not assert current file health
    /// or grant permission to execute a benchmark.
    pub fn recorded_benchmark(
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
    ) -> Result<BenchmarkDefinition, EncoderTaskAdapterError> {
        protocol
            .validate_integrity(project)
            .map_err(adapter_error)?;
        if project.backend.name != ADAPTER_NAME
            || project.backend.protocol_version != ADAPTER_PROTOCOL_VERSION
        {
            return Err(adapter_error(
                "The benchmark uses another evaluation adapter",
            ));
        }
        let configuration = Self::task_configuration(project)?;
        configuration.validate()?;
        let mut suites = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        for report in protocol
            .baseline_development_reports()
            .into_iter()
            .chain(std::iter::once(&protocol.baseline_sealed_report))
        {
            let suite = configuration
                .suites
                .get(&report.suite_key)
                .ok_or_else(|| adapter_error("Recorded benchmark suite is missing"))?;
            if suite.fingerprint != report.suite_fingerprint || suite.role != report.evidence_role {
                return Err(adapter_error(
                    "Recorded benchmark report differs from its suite",
                ));
            }
            let input = project
                .inputs
                .iter()
                .find(|input| input.key == suite.path && input.role == suite.role)
                .ok_or_else(|| {
                    adapter_error("Benchmark membership is not pinned to the correct role")
                })?;
            inputs.insert(input.key.clone(), input);
            suites.insert(
                report.suite_key.clone(),
                json!({"fingerprint":suite.fingerprint,
                "retrieval":suite.retrieval_fingerprint,"agent":suite.agent_fingerprint}),
            );
        }
        let context = artifact_core::fingerprint(&json!({
            "schema":"nomos-benchmark-context-v1", "retrieval_evaluator":RETRIEVAL_EVALUATOR_VERSION,
            "agent_evaluator":AGENT_EVALUATOR_VERSION, "dense_text":DENSE_TEXT_VERSION,
            "agent_evaluation":project.task_configuration.get("agent_evaluation"),
            "inputs":inputs, "suites":suites,
        })).map_err(adapter_error)?;
        BenchmarkDefinition::from_protocol(project, protocol, context).map_err(adapter_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use encoder_experiment_core::domain::{
        BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, ModelArtifactIdentity,
    };
    use serde_json::json;

    fn fp(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project() -> ExternalProjectSnapshot {
        let backend =
            BackendIdentity::new(ADAPTER_NAME, ADAPTER_PROTOCOL_VERSION, fp('b')).unwrap();
        let inputs = [
            ("train.jsonl", EvidenceRole::Training),
            ("generic.jsonl", EvidenceRole::Development),
            ("regression.jsonl", EvidenceRole::Development),
            ("holdout.jsonl", EvidenceRole::SealedAcceptance),
        ]
        .into_iter()
        .map(|(key, role)| ExternalArtifactIdentity::new(key, role, 1, fp('c')).unwrap())
        .collect();
        ExternalProjectSnapshot::create(
            "Fixture",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            fp('a'),
            backend,
            inputs,
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, fp('d')).unwrap(),
            json!({
                "adapter_protocol":ADAPTER_PROTOCOL_VERSION,"source_reference":{},"baseline_evidence":{},"training_inputs":["train.jsonl"],
                "reference_models":{"reference":{"path":"reference","format":"sentence-transformers","bytes":1,"fingerprint":fp('e'),"provenance":{}}},
                "agent_evaluation":{"backend":"onnx","chat_model":{"path":"chat","format":"onnxruntime-genai","bytes":1,"fingerprint":fp('f'),"source":{}},
                    "selector_strategy":"multiview","candidate_strategy":"multiview","nomos_top_k":1,"max_attempts":1,
                    "development":{"suite":"development","sessions":5,"pairing":"cycle","condition":"nomos"},
                    "sealed":{"suite":"promotion","sessions":5,"pairing":"cycle","condition":"nomos"}},
                "suites":{
                    "generic":{"path":"generic.jsonl","role":"development","fingerprint":fp('1'),"retrieval_fingerprint":fp('2'),"agent_fingerprint":fp('3')},
                    "regression":{"path":"regression.jsonl","role":"development","fingerprint":fp('4'),"retrieval_fingerprint":fp('5'),"agent_fingerprint":fp('6')},
                    "holdout":{"path":"holdout.jsonl","role":"sealed_acceptance","fingerprint":fp('7'),"retrieval_fingerprint":fp('8'),"agent_fingerprint":fp('9')}
                }
            }),
            Utc::now(),
        )
        .unwrap()
    }

    #[test]
    fn initial_plan_is_complete_model_independent_and_uses_named_suites() {
        let project = project();
        let plan = NomosBackend::initial_benchmark_plan(&project).unwrap();
        assert_eq!(plan.development_suite_keys, ["generic", "regression"]);
        assert_eq!(plan.sealed_suite_key, "holdout");
        assert_eq!(plan.metric_contract.primary_metric, "mrr");
        assert_eq!(plan.metric_contract.definitions.len(), 15);
        assert_eq!(plan.metric_contract.gates.len(), 26);
        assert_eq!(plan.maximum_evaluation_seconds, 1_800);
        assert!(plan.metric_contract.definitions.iter().any(|definition| {
            definition.key == "agent_prompt_tokens_per_attempt"
                && definition.direction == MetricDirection::LowerIsBetter
        }));

        let mut changed_model = project.clone();
        changed_model.baseline_model.fingerprint = fp('0');
        changed_model.fingerprint = changed_model.reproduce_fingerprint().unwrap();
        assert_eq!(
            NomosBackend::initial_benchmark_plan(&changed_model).unwrap(),
            plan
        );
    }

    #[test]
    fn initial_plan_requires_development_and_exactly_one_final_holdout() {
        let mut project = project();
        project.task_configuration["suites"]["holdout"]["role"] = json!("development");
        project.fingerprint = project.reproduce_fingerprint().unwrap();
        assert!(NomosBackend::initial_benchmark_plan(&project).is_err());
    }
}
