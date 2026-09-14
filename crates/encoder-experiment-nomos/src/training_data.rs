//! Read-only custody handoff for an already completed native fine-tune.
use super::*;

#[derive(Debug, Clone)]
pub struct VerifiedTrainingInput {
    pub key: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub fingerprint: String,
    pub rows: u64,
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    // Reuse the strict native manifest/parameter fixture from the adapter tests.
    pub(crate) fn verify_readonly_handoff(
        mut parameters: BTreeMap<String, ParameterValue>,
        mut manifest: Value,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("runs/repair")).unwrap();
        for (key, rows) in [
            ("base-a.jsonl", 5),
            ("base-b.jsonl", 5),
            ("runs/repair/delta.jsonl", 2),
        ] {
            fs::write(root.join(key), "{}\n".repeat(rows)).unwrap();
        }
        parameters.insert(
            REPAIR_DELTA_BYTES_PARAMETER.into(),
            ParameterValue::Integer(6),
        );
        parameters.insert(
            REPAIR_DELTA_FINGERPRINT_PARAMETER.into(),
            ParameterValue::Text(prefixed(
                &sha256_file(&root.join("runs/repair/delta.jsonl")).unwrap(),
            )),
        );
        let fp = prefixed(&"7".repeat(64));
        let identity = BackendIdentity::new(ADAPTER_NAME, ADAPTER_PROTOCOL_VERSION, artifact_core::fingerprint(&json!({"adapter":ADAPTER_NAME,"protocol_version":ADAPTER_PROTOCOL_VERSION,"manifest_schema_version":4,"tree_hash_algorithm":TREE_HASH_ALGORITHM})).unwrap()).unwrap();
        let backend = NomosBackend {
            root: root.clone(),
            python: root.join("must-not-run-python"),
            manifest: crate::tests::manifest_with_named_suites(),
            baseline_override: None,
            training_override: None,
            identity: identity.clone(),
            observer_identity: identity.clone(),
            repair_delta_identity: identity.clone(),
            progress: None,
        };
        let configuration = json!({
            "adapter_protocol": ADAPTER_PROTOCOL_VERSION, "source_reference":{},"baseline_evidence":{},
            "training_inputs":["base-a.jsonl","base-b.jsonl"],
            "reference_models":{"reference":{"path":"reference","format":"sentence-transformers","bytes":1,"fingerprint":fp,"provenance":{}}},
            "agent_evaluation":{"backend":"onnx","chat_model":{"path":"chat","format":"onnxruntime-genai","bytes":1,"fingerprint":fp,"source":{}},"selector_strategy":"multiview","candidate_strategy":"multiview","nomos_top_k":1,"max_attempts":1,
                "development":{"suite":"development","sessions":1,"pairing":"cycle","condition":"nomos"},
                "sealed":{"suite":"promotion","sessions":1,"pairing":"cycle","condition":"nomos"}},
            "suites":{"development":{"path":"missing-development.jsonl","role":"development","fingerprint":fp,"retrieval_fingerprint":fp,"agent_fingerprint":fp}}
        });
        let mut inputs = Vec::new();
        for key in ["base-a.jsonl", "base-b.jsonl"] {
            inputs.push(
                ExternalArtifactIdentity::new(
                    key,
                    EvidenceRole::Training,
                    15,
                    prefixed(&sha256_file(&root.join(key)).unwrap()),
                )
                .unwrap(),
            );
        }
        // Neither holdout exists. Reading training provenance must not read them.
        inputs.push(
            ExternalArtifactIdentity::new(
                "missing-development.jsonl",
                EvidenceRole::Development,
                1,
                fp.clone(),
            )
            .unwrap(),
        );
        inputs.push(
            ExternalArtifactIdentity::new(
                "missing-sealed.jsonl",
                EvidenceRole::SealedAcceptance,
                1,
                fp.clone(),
            )
            .unwrap(),
        );
        let project = ExternalProjectSnapshot::create(
            "Fixture",
            EncoderTaskKind::RetrievalRanking,
            "fixture-revision",
            fp.clone(),
            identity,
            inputs,
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, fp).unwrap(),
            configuration,
            Utc::now(),
        )
        .unwrap();
        let candidate = TrainingCandidate::create(&project, 1, 60, parameters).unwrap();
        let strategy = NativeCandidateStrategy::parse(&candidate.parameters).unwrap();
        let configuration = NomosBackend::task_configuration(&project).unwrap();
        let output = backend.candidate_output(&candidate);
        fs::create_dir_all(&output).unwrap();
        let logical = workspace_relative(&root, &output).unwrap();
        let native =
            workspace_relative(&root, &backend.candidate_staging_output(&candidate)).unwrap();
        manifest["base_model"] = json!("baseline");
        manifest["output"] = json!(native);
        let manifest_path = output.join("nomos_training_manifest.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(output.join("model.safetensors"), b"fixture weights").unwrap();
        let args = strategy
            .arguments(&project, &configuration, &native)
            .unwrap();
        backend
            .write_or_validate_repair_training_receipt(
                &output, &logical, &native, &project, &candidate, &strategy, &args, &manifest,
            )
            .unwrap();
        let (bytes, digest) = tree_identity(&output).unwrap();
        let model =
            ModelArtifactIdentity::new(&logical, "sentence-transformers", bytes, prefixed(&digest))
                .unwrap();
        let data = backend
            .verified_training_data(&project, &candidate, &model)
            .unwrap();
        assert_eq!(
            data.inputs
                .iter()
                .map(|input| input.key.as_str())
                .collect::<Vec<_>>(),
            ["base-a.jsonl", "base-b.jsonl", "runs/repair/delta.jsonl"]
        );
        assert_eq!(data.inputs.iter().map(|input| input.rows).sum::<u64>(), 12);
        assert_eq!(
            data.snapshot_id,
            strategy.repair_binding().unwrap().snapshot_id
        );
        assert_eq!(tree_identity(&output).unwrap(), (bytes, digest));
        assert!(!backend.candidate_staging_output(&candidate).exists());
        fs::write(root.join("runs/repair/delta.jsonl"), "[]\n[]\n").unwrap();
        assert!(
            backend
                .verified_training_data(&project, &candidate, &model)
                .is_err()
        );
        fs::write(root.join("runs/repair/delta.jsonl"), "{}\n{}\n").unwrap();
        let receipt = output.join(REPAIR_TRAINING_RECEIPT_NAME);
        fs::remove_file(&receipt).unwrap();
        // Even a model identity recomputed after removal cannot bypass its receipt.
        let (bytes, digest) = tree_identity(&output).unwrap();
        let changed =
            ModelArtifactIdentity::new(&logical, "sentence-transformers", bytes, prefixed(&digest))
                .unwrap();
        assert!(
            backend
                .verified_training_data(&project, &candidate, &changed)
                .is_err()
        );
        assert!(!receipt.exists());
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedTrainingData {
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub manifest_bytes: u64,
    pub manifest_fingerprint: String,
    /// Exact native trainer order; dataset display order is independent.
    pub inputs: Vec<VerifiedTrainingInput>,
}

impl NomosBackend {
    /// Verify the published checkpoint, immutable candidate, native manifest,
    /// receipt and training files without running Python or inspecting holdouts.
    /// The application separately verifies the scientific completion journal.
    pub fn verified_training_data(
        &self,
        project: &ExternalProjectSnapshot,
        candidate: &TrainingCandidate,
        model: &ModelArtifactIdentity,
    ) -> Result<VerifiedTrainingData, EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        candidate
            .validate_integrity(project)
            .map_err(adapter_error)?;
        if !self.supports_identity(&project.backend) {
            return Err(adapter_error(
                "Training data uses another native adapter contract",
            ));
        }
        let configuration = Self::task_configuration(project)?;
        configuration.validate()?;
        let strategy = NativeCandidateStrategy::parse(&candidate.parameters)?;
        if configuration.managed_training_dataset.is_some() && strategy.repair_binding().is_none() {
            return self.verified_managed_training_data(
                project,
                candidate,
                model,
                &configuration,
                &strategy,
            );
        }
        let binding = strategy.repair_binding().ok_or_else(|| {
            adapter_error("This native model has no supported recorded training snapshot")
        })?;
        strategy.verify_training_inputs(self, project, &configuration)?;
        let output = self.model_path(model)?;
        let logical_output = workspace_relative(&self.root, &self.candidate_output(candidate))?;
        if model.key != logical_output {
            return Err(adapter_error("Checkpoint belongs to a different candidate"));
        }
        let native_output =
            workspace_relative(&self.root, &self.candidate_staging_output(candidate))?;
        let arguments = strategy.arguments(project, &configuration, &native_output)?;
        let manifest = strategy.read_and_validate_manifest(
            &output,
            &native_output,
            project,
            &configuration,
        )?;
        self.read_and_validate_repair_training_receipt(
            &output,
            &logical_output,
            &native_output,
            project,
            candidate,
            &strategy,
            &arguments,
            &manifest,
        )?;
        let counts = normalized_count_map(&manifest, "input_row_counts")?;
        let mut inputs = Vec::new();
        for key in &configuration.training_inputs {
            let input = project
                .inputs
                .iter()
                .find(|input| input.key == *key && input.role == EvidenceRole::Training)
                .ok_or_else(|| adapter_error("Recorded input is not pinned training data"))?;
            let path = self.resolve_existing(key)?;
            verify_file(
                &path,
                input.bytes,
                input
                    .fingerprint
                    .strip_prefix("sha256:")
                    .ok_or_else(|| adapter_error("Invalid input checksum"))?,
            )?;
            inputs.push(VerifiedTrainingInput {
                key: key.clone(),
                path,
                bytes: input.bytes,
                fingerprint: input.fingerprint.clone(),
                rows: *counts
                    .get(key)
                    .ok_or_else(|| adapter_error("Missing training input count"))?,
            });
        }
        inputs.push(VerifiedTrainingInput {
            key: binding.delta_key.clone(),
            path: self.resolve_existing(&binding.delta_key)?,
            bytes: binding.delta_bytes,
            fingerprint: binding.delta_fingerprint.clone(),
            rows: binding.delta_rows,
        });
        let manifest_path = output.join("nomos_training_manifest.json");
        Ok(VerifiedTrainingData {
            snapshot_id: binding.snapshot_id,
            snapshot_fingerprint: binding.snapshot_fingerprint.clone(),
            manifest_bytes: fs::metadata(&manifest_path).map_err(adapter_error)?.len(),
            manifest_fingerprint: prefixed(&sha256_file(&manifest_path)?),
            inputs,
        })
    }

    fn verified_managed_training_data(
        &self,
        project: &ExternalProjectSnapshot,
        candidate: &TrainingCandidate,
        model: &ModelArtifactIdentity,
        configuration: &TaskConfiguration,
        strategy: &NativeCandidateStrategy,
    ) -> Result<VerifiedTrainingData, EncoderTaskAdapterError> {
        let dataset = configuration
            .managed_training_dataset
            .as_ref()
            .expect("managed branch");
        if !matches!(strategy, NativeCandidateStrategy::FineTune(_))
            || self.training_override.as_ref() != Some(dataset)
            || project
                .inputs
                .iter()
                .filter(|input| input.role == EvidenceRole::Training)
                .collect::<Vec<_>>()
                != [&dataset.artifact]
        {
            return Err(adapter_error(
                "Managed checkpoint does not match its training dataset",
            ));
        }
        dataset.verify_in(&self.root)?;
        let output = self.model_path(model)?;
        let logical = workspace_relative(&self.root, &self.candidate_output(candidate))?;
        if model.key != logical {
            return Err(adapter_error("Checkpoint belongs to a different candidate"));
        }
        let manifest =
            strategy.read_and_validate_manifest(&output, &logical, project, configuration)?;
        let inputs = manifest
            .get("inputs")
            .and_then(Value::as_array)
            .ok_or_else(|| adapter_error("Managed training manifest omitted its inputs"))?;
        let counts = normalized_count_map(&manifest, "input_row_counts")?;
        if inputs.len() != 1
            || inputs[0].as_str().map(normalized_native_path).as_deref()
                != Some(dataset.artifact.key.as_str())
            || counts.len() != 1
            || counts.get(&dataset.artifact.key) != Some(&dataset.rows)
        {
            return Err(adapter_error(
                "Managed training manifest references a different population",
            ));
        }
        let manifest_path = output.join("nomos_training_manifest.json");
        Ok(VerifiedTrainingData {
            snapshot_id: dataset.dataset_version_id,
            snapshot_fingerprint: dataset.dataset_version_fingerprint.clone(),
            manifest_bytes: fs::metadata(&manifest_path).map_err(adapter_error)?.len(),
            manifest_fingerprint: prefixed(&sha256_file(&manifest_path)?),
            inputs: vec![VerifiedTrainingInput {
                key: dataset.artifact.key.clone(),
                path: self.resolve_existing(&dataset.artifact.key)?,
                bytes: dataset.artifact.bytes,
                fingerprint: dataset.artifact.fingerprint.clone(),
                rows: dataset.rows,
            }],
        })
    }
}
