//! Adapter-owned translation of finite user settings, with resolved hardware.
use super::*;

pub struct NomosFineTuneSettings {
    pub epochs: u32,
    pub batch_size: u32,
    pub learning_rate_nanos: u32,
    pub device: String,
}

impl NomosBackend {
    /// The old fixed-training adapter accepted a minimal legacy manifest. New
    /// Agent iterations require proof of the exact input and effective settings.
    pub(super) fn verify_configured_training_manifest(
        configuration: &TaskConfiguration,
        parameters: &NativeTrainingParameters,
        manifest: &Value,
    ) -> Result<(), EncoderTaskAdapterError> {
        let dataset = configuration
            .managed_training_dataset
            .as_ref()
            .ok_or_else(|| adapter_error("Managed trainer population missing"))?;
        let expected_inputs = vec![dataset.artifact.key.clone()];
        let inputs: Vec<String> = manifest["inputs"]
            .as_array()
            .ok_or_else(|| adapter_error("Training manifest omitted inputs"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(normalized_native_path)
                    .ok_or_else(|| adapter_error("Invalid training input"))
            })
            .collect::<Result<_, _>>()?;
        let counts = BTreeMap::from([(dataset.artifact.key.clone(), dataset.rows)]);
        if inputs != expected_inputs
            || normalized_count_map(manifest, "input_row_counts")? != counts
            || normalized_count_map(manifest, "trainable_row_counts")? != counts
            || manifest["unique_trainable_rows"].as_u64() != Some(dataset.rows)
            || manifest["training_triplets"].as_u64() != Some(dataset.rows)
            || manifest["epochs"].as_f64() != Some(parameters.epochs)
            || manifest["batch_size"].as_u64() != Some(parameters.batch_size)
            || manifest["learning_rate"].as_f64() != Some(parameters.learning_rate)
            || manifest["device"].as_str() != Some(parameters.device.as_str())
            || manifest["seed"].as_u64() != Some(parameters.seed)
            || manifest["query_strategy"].as_str() != Some(parameters.query_strategy.as_str())
            || manifest["positive_strategy"].as_str() != Some(parameters.positive_strategy.as_str())
            || manifest["margin"].as_f64() != Some(parameters.margin)
            || manifest["training_script"].as_str() != Some("tools.train_dense_triplet_router.v2")
        {
            return Err(adapter_error(
                "Training output does not reproduce its exact dataset and settings",
            ));
        }
        Ok(())
    }
    /// Probe only for Auto; an explicit CUDA request must fail rather than
    /// silently changing hardware. The resolved choice is frozen in the candidate.
    pub async fn resolve_training_device(
        &self,
        requested: &str,
    ) -> Result<String, EncoderTaskAdapterError> {
        if ["cpu", "cuda"].contains(&requested) {
            return Ok(requested.into());
        }
        if requested != "auto" {
            return Err(adapter_error("Unsupported training device"));
        }
        let mut command = Command::new(&self.python);
        command
            .args([
                "-c",
                "import torch; print('cuda' if torch.cuda.is_available() else 'cpu')",
            ])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Fixed program emits one four-byte enum, never a model or row payload.
        let output = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .map_err(|_| adapter_error("Training device detection timed out"))?
            .map_err(|_| adapter_error("Training device detection could not start"))?;
        let device = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || !["cpu", "cuda"].contains(&device.as_str()) {
            return Err(adapter_error("Training device detection failed"));
        }
        Ok(device)
    }

    pub fn configured_training_candidate(
        id: Uuid,
        project: &ExternalProjectSnapshot,
        maximum_seconds: u64,
        settings: NomosFineTuneSettings,
    ) -> Result<TrainingCandidate, EncoderTaskAdapterError> {
        if !(1..=10).contains(&settings.epochs)
            || !(1..=256).contains(&settings.batch_size)
            || !(1..=1_000_000).contains(&settings.learning_rate_nanos)
            || !["cpu", "cuda"].contains(&settings.device.as_str())
        {
            return Err(adapter_error(
                "Training settings exceed the supported envelope",
            ));
        }
        let mut parameters =
            Self::initial_training_candidate_definition(id, project, maximum_seconds)?.parameters;
        parameters.insert(
            "epochs".into(),
            ParameterValue::Number(f64::from(settings.epochs)),
        );
        parameters.insert(
            "batch_size".into(),
            ParameterValue::Integer(i64::from(settings.batch_size)),
        );
        parameters.insert(
            "learning_rate".into(),
            ParameterValue::Number(f64::from(settings.learning_rate_nanos) / 1_000_000_000.0),
        );
        parameters.insert("device".into(), ParameterValue::Text(settings.device));
        parameters.insert(
            "receipt_protocol".into(),
            ParameterValue::Text("managed-settings-v1".into()),
        );
        NativeCandidateStrategy::parse(&parameters)?;
        TrainingCandidate::create_identified(id, project, 1, maximum_seconds, parameters)
            .map_err(adapter_error)
    }
}
