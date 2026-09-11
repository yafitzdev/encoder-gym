//! Normalize recorded evaluation context without reading native or sealed files.
use super::*;
use encoder_experiment_core::{benchmark::BenchmarkDefinition, protocol::ExperimentProtocol};

impl NomosBackend {
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
