//! One normalization path for dispatch and strictly read-only completed-result recovery.
use super::*;

impl NomosBackend {
    pub(super) async fn evaluate_native(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
        maximum_seconds: Option<u64>,
    ) -> Result<Option<EvaluationReport>, EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        contract.validate_integrity().map_err(adapter_error)?;
        model.validate().map_err(adapter_error)?;
        self.require_current_project(&project)?;
        self.inspect(project.clone()).await?;
        let configuration = Self::task_configuration(&project)?;
        configuration.validate()?;
        let suite = configuration
            .suites
            .get(&suite_key)
            .ok_or_else(|| adapter_error(format!("unknown Nomos suite {suite_key}")))?;
        let input = self.resolve_existing(&suite.path)?;
        let model_path = self.model_path(&model)?;
        let model_root = evaluation_model_root(&self.root, &model)?;
        let retrieval_root =
            evaluation_component_root(&model_root, "retrieval", &suite.retrieval_fingerprint)?;
        let retrieval_output = retrieval_root.join(format!("{suite_key}.json"));
        let retrieval_output_relative = retrieval_output
            .strip_prefix(&self.root)
            .map_err(adapter_error)?
            .to_string_lossy()
            .replace('\\', "/");
        let model_relative = model_path
            .strip_prefix(&self.root)
            .map_err(adapter_error)?
            .to_string_lossy()
            .replace('\\', "/");
        let input_relative = input
            .strip_prefix(&self.root)
            .map_err(adapter_error)?
            .to_string_lossy()
            .replace('\\', "/");
        let arguments = vec![
            "-m".into(),
            "tools.evaluate_dense_router".into(),
            "--model".into(),
            model_relative.clone(),
            "--input".into(),
            input_relative,
            "--limit".into(),
            "10000".into(),
            "--device".into(),
            "cpu".into(),
            "--output".into(),
            retrieval_output_relative,
        ];
        let raw = if retrieval_output.exists() {
            read_json(&retrieval_output)?
        } else {
            let Some(maximum_seconds) = maximum_seconds else {
                return Ok(None);
            };
            self.observe_subject(NativePhase::EvaluatingRetrieval, &suite_key);
            self.run_bounded(&arguments, maximum_seconds).await?;
            read_json(&retrieval_output)?
        };
        let native: NativeEvaluation = serde_json::from_value(raw)
            .map_err(|error| adapter_error(format!("Nomos evaluation JSON is invalid: {error}")))?;
        let first = native
            .inputs
            .values()
            .next()
            .ok_or_else(|| adapter_error("Nomos evaluation returned no input metrics"))?;
        if native.inputs.len() != 1 {
            return Err(adapter_error(
                "Nomos evaluation must normalize exactly one suite",
            ));
        }
        let mut available = BTreeMap::from([
            ("recall_at_1".to_owned(), first.metrics.recall_at_1),
            ("recall_at_2".to_owned(), first.metrics.recall_at_2),
            ("recall_at_3".to_owned(), first.metrics.recall_at_3),
            ("mrr".to_owned(), first.metrics.mrr),
            (
                "mean_positive_margin".to_owned(),
                first.metrics.mean_positive_margin,
            ),
        ]);
        let mut support = first.metrics.states;
        let requires_agent = contract
            .definitions
            .iter()
            .any(|definition| definition.key.starts_with("agent_"));
        if requires_agent {
            let agent = &configuration.agent_evaluation;
            let agent_suite = agent.suite(suite.role);
            let chat_model = self.resolve_existing(&agent.chat_model.path)?;
            let (chat_bytes, chat_digest) = tree_identity(&chat_model)?;
            if chat_bytes != agent.chat_model.bytes
                || prefixed(&chat_digest) != agent.chat_model.fingerprint
            {
                return Err(adapter_error(
                    "Nomos agent chat model changed after project registration",
                ));
            }
            let chat_model_relative = chat_model
                .strip_prefix(&self.root)
                .map_err(adapter_error)?
                .to_string_lossy()
                .replace('\\', "/");
            let agent_root =
                evaluation_component_root(&model_root, "agent", &suite.agent_fingerprint)?;
            let agent_output = agent_root.join(format!("{suite_key}.json"));
            let agent_trace = agent_root.join(format!("{suite_key}.trace.jsonl"));
            let agent_output_relative = workspace_relative(&self.root, &agent_output)?;
            let agent_trace_relative = workspace_relative(&self.root, &agent_trace)?;
            let agent_arguments = vec![
                "-m".into(),
                "tools.evaluate_real_agent_sessions".into(),
                "--backend".into(),
                agent.backend.clone(),
                "--onnx-model".into(),
                chat_model_relative,
                "--nomos-model".into(),
                model_relative,
                "--selector-strategy".into(),
                agent.selector_strategy.clone(),
                "--candidate-strategy".into(),
                agent.candidate_strategy.clone(),
                "--sessions".into(),
                agent_suite.sessions.to_string(),
                "--pairing".into(),
                agent_suite.pairing.clone(),
                "--suite".into(),
                agent_suite.suite.clone(),
                "--max-attempts".into(),
                agent.max_attempts.to_string(),
                "--nomos-top-k".into(),
                agent.nomos_top_k.to_string(),
                "--condition".into(),
                agent_suite.condition.clone(),
                "--output".into(),
                agent_output_relative,
                "--trace-output".into(),
                agent_trace_relative,
            ];
            let raw_agent = if agent_output.exists() {
                if !agent_trace.is_file() {
                    if maximum_seconds.is_none() {
                        return Ok(None);
                    }
                    return Err(adapter_error(
                        "Nomos agent evaluation report exists without its trace evidence",
                    ));
                }
                read_json(&agent_output)?
            } else {
                let Some(maximum_seconds) = maximum_seconds else {
                    return Ok(None);
                };
                self.observe_subject(NativePhase::EvaluatingAgent, &suite_key);
                self.run_bounded(&agent_arguments, maximum_seconds).await?;
                if !agent_trace.is_file() {
                    return Err(adapter_error(
                        "Nomos agent evaluation did not produce trace evidence",
                    ));
                }
                read_json(&agent_output)?
            };
            let native_agent: NativeAgentEvaluation =
                serde_json::from_value(raw_agent).map_err(|error| {
                    adapter_error(format!("Nomos agent evaluation JSON is invalid: {error}"))
                })?;
            let summary = native_agent.validate_and_summary(agent, agent_suite)?;
            available.extend(summary.normalized_metrics()?);
            support = support.min(summary.sessions);
        }
        let mut normalized = BTreeMap::new();
        for definition in &contract.definitions {
            normalized.insert(
                definition.key.clone(),
                *available.get(&definition.key).ok_or_else(|| {
                    adapter_error(format!(
                        "Nomos ranking adapter cannot produce metric {}",
                        definition.key
                    ))
                })?,
            );
        }
        EvaluationReport::create(
            &project,
            model,
            suite.role,
            suite_key,
            suite.fingerprint.clone(),
            &contract,
            normalized,
            support,
            Utc::now(),
        )
        .map(Some)
        .map_err(adapter_error)
    }
}
