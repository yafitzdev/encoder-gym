//! Normalize recorded evaluation context without reading native or sealed files.
use super::*;
use encoder_experiment_core::{benchmark::BenchmarkDefinition, protocol::ExperimentProtocol};

impl NomosBackend {
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
