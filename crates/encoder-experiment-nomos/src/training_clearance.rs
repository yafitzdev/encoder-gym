//! Complete native population admission. Separate from publication and model
//! evaluation; only a bound, clean receipt may authorize the Agent handoff.
use super::*;
use encoder_experiment_core::benchmark::BenchmarkDefinition;
use std::io::Write;

const PROGRAM: &str = include_str!("qualify_training.py");
const PROTOCOL: &str = "nomos-training-clearance-v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NomosTrainingClearance {
    pub protocol: String,
    pub request_fingerprint: String,
    pub training_rows: u64,
    pub benchmark_rows: u64,
    pub invalid_rows: u64,
    pub duplicate_rows: u64,
    pub overlap_rows: u64,
    /// Optional native provenance is checked wherever declared; absence is an
    /// explicit limitation, not evidence of complete grouping information.
    pub missing_group_rows: u64,
    pub missing_lineage_rows: u64,
    #[serde(default)]
    pub fingerprint: String,
}

impl NomosTrainingClearance {
    pub fn training_allowed(&self) -> bool {
        self.training_rows > 0
            && self.benchmark_rows > 0
            && self.invalid_rows == 0
            && self.duplicate_rows == 0
            && self.overlap_rows == 0
            && self
                .reproduce()
                .is_ok_and(|value| value == self.fingerprint)
    }

    fn reproduce(&self) -> Result<String, EncoderTaskAdapterError> {
        let mut value = serde_json::to_value(self).map_err(adapter_error)?;
        value
            .as_object_mut()
            .expect("clearance is an object")
            .remove("fingerprint");
        artifact_core::fingerprint(&value).map_err(adapter_error)
    }

    fn validate(&self, request: &Value) -> Result<(), EncoderTaskAdapterError> {
        let maximum = self.training_rows.saturating_add(self.benchmark_rows);
        if self.protocol != PROTOCOL
            || self.request_fingerprint != request["fingerprint"].as_str().unwrap_or_default()
            || Some(self.training_rows) != request["rows"].as_u64()
            || self.training_rows == 0
            || self.benchmark_rows == 0
            || self.invalid_rows > self.training_rows
            || self.duplicate_rows > self.training_rows
            || self.overlap_rows > self.training_rows
            || self.missing_group_rows > maximum
            || self.missing_lineage_rows > maximum
            || self.reproduce()? != self.fingerprint
        {
            return Err(adapter_error(
                "Native training clearance does not match its exact inputs",
            ));
        }
        Ok(())
    }
}

impl NomosBackend {
    /// Verify all derived rows with the pinned native schema and compare their
    /// native model input/source/group/lineage against all benchmark members.
    /// Protected payloads stay inside the fixed local audit process. There are
    /// no model/provider calls or evaluation scores in this operation.
    pub async fn qualify_training_dataset(
        &self,
        project: &ExternalProjectSnapshot,
        benchmark: &BenchmarkDefinition,
        dataset: &NomosTrainingDataset,
    ) -> Result<NomosTrainingClearance, EncoderTaskAdapterError> {
        Self::verify_benchmark_definition(project, benchmark)?;
        // Check code/manifest authority without rehashing baseline and reference
        // model weights: this operation reads rows, never loads a model.
        self.verify_no_remote()?;
        self.verify_clean_worktree()?;
        let revision = self.git_output(["rev-parse", "HEAD"])?;
        let runtime = self.runtime_source_fingerprint(&revision)?;
        if revision != project.source_revision
            || bound_source_fingerprint(
                runtime,
                self.baseline_override.as_ref(),
                self.training_override.as_ref(),
            )? != project.source_fingerprint
        {
            return Err(adapter_error("Native qualification runtime changed"));
        }
        dataset.verify_in(&self.root)?;
        let configuration = Self::task_configuration(project)?;
        let mut inputs = BTreeMap::new();
        for suite in &benchmark.suites {
            let native = &configuration.suites[&suite.key];
            let input = project
                .inputs
                .iter()
                .find(|input| input.key == native.path && input.role == suite.role)
                .ok_or_else(|| adapter_error("Benchmark membership is missing"))?;
            // Also rechecked by the audit on the same bytes it consumes.
            let path = self.resolve_existing(&input.key)?;
            verify_file(
                &path,
                input.bytes,
                input
                    .fingerprint
                    .strip_prefix("sha256:")
                    .ok_or_else(|| adapter_error("Invalid benchmark checksum"))?,
            )?;
            inputs.insert(input.key.clone(), input.clone());
        }
        let mut sources = BTreeMap::new();
        for relative in repair_delta_sources(self.native_package) {
            let path = self.resolve_existing(&relative)?;
            sources.insert(relative, prefixed(&sha256_file(&path)?));
        }
        let mut request = json!({
            "protocol":PROTOCOL, "programFingerprint":prefixed(&format!("{:x}", Sha256::digest(PROGRAM.as_bytes()))),
            "projectFingerprint":project.fingerprint, "benchmarkFingerprint":benchmark.fingerprint,
            "dataset":dataset, "rows":dataset.rows, "training":dataset.artifact,
            "benchmarkInputs":inputs.into_values().collect::<Vec<_>>(), "sources":sources,
            "nativePackage":self.native_package.module(),
        });
        let fingerprint = artifact_core::fingerprint(&request).map_err(adapter_error)?;
        request["fingerprint"] = fingerprint.clone().into();
        let parent = managed_training::ensure_output_directory(
            &self.root,
            &[
                "runs",
                "encoder-gym-project-runs",
                &dataset.run_id.to_string(),
                "qualification",
            ],
        )?;
        let destination = parent.join(&fingerprint[7..]);
        if destination.exists() {
            return read_clearance(&self.root, &destination, &request);
        }
        let scratch = parent.join(format!(".tmp-{}", Uuid::new_v4()));
        fs::create_dir(&scratch).map_err(adapter_error)?;
        let result = async {
            let request_path = scratch.join("request.json");
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&request_path)
                .map_err(adapter_error)?;
            file.write_all(&serde_json::to_vec(&request).map_err(adapter_error)?)
                .map_err(adapter_error)?;
            file.sync_all().map_err(adapter_error)?;
            drop(file);
            self.observe(NativePhase::CheckingTrainingData);
            self.run_bounded(
                &[
                    "-c".into(),
                    PROGRAM.into(),
                    workspace_relative(&self.root, &request_path)?,
                ],
                120,
            )
            .await?;
            let raw_path = scratch.join("result.json");
            if fs::metadata(&raw_path).map_err(adapter_error)?.len() > 131072 {
                return Err(adapter_error(
                    "Native clearance output exceeds its size bound",
                ));
            }
            let mut clearance: NomosTrainingClearance =
                serde_json::from_slice(&fs::read(raw_path).map_err(adapter_error)?)
                    .map_err(adapter_error)?;
            clearance.fingerprint = clearance.reproduce()?;
            clearance.validate(&request)?;
            let mut receipt = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(scratch.join("clearance.json"))
                .map_err(adapter_error)?;
            receipt
                .write_all(&serde_json::to_vec(&clearance).map_err(adapter_error)?)
                .map_err(adapter_error)?;
            receipt.sync_all().map_err(adapter_error)?;
            drop(receipt);
            match fs::rename(&scratch, &destination) {
                Ok(()) => Ok(clearance),
                Err(_) if destination.exists() => {
                    read_clearance(&self.root, &destination, &request)
                }
                Err(error) => Err(adapter_error(error)),
            }
        }
        .await;
        // Remove only our exact generated scratch within the checked parent.
        if scratch.exists() {
            remove_exact_scratch_directory(&scratch, &parent)?;
        }
        result
    }
}

fn read_clearance(
    root: &Path,
    directory: &Path,
    request: &Value,
) -> Result<NomosTrainingClearance, EncoderTaskAdapterError> {
    let read = |name: &str| -> Result<Vec<u8>, EncoderTaskAdapterError> {
        let key = workspace_relative(root, &directory.join(name))?;
        let path = managed_training::contained_existing(root, &key)?;
        if fs::metadata(&path).map_err(adapter_error)?.len() > 131072 {
            return Err(adapter_error(
                "Native clearance record exceeds its size bound",
            ));
        }
        fs::read(path).map_err(adapter_error)
    };
    let stored: Value = serde_json::from_slice(&read("request.json")?).map_err(adapter_error)?;
    if &stored != request {
        return Err(adapter_error("Native clearance request changed"));
    }
    let clearance: NomosTrainingClearance =
        serde_json::from_slice(&read("clearance.json")?).map_err(adapter_error)?;
    clearance.validate(request)?;
    Ok(clearance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clearance() -> (NomosTrainingClearance, Value) {
        let request = json!({"rows":2,"fingerprint":prefixed(&"1".repeat(64))});
        let mut result = NomosTrainingClearance {
            protocol: PROTOCOL.into(),
            request_fingerprint: request["fingerprint"].as_str().unwrap().into(),
            training_rows: 2,
            benchmark_rows: 3,
            invalid_rows: 0,
            duplicate_rows: 0,
            overlap_rows: 0,
            missing_group_rows: 5,
            missing_lineage_rows: 5,
            fingerprint: String::new(),
        };
        result.fingerprint = result.reproduce().unwrap();
        (result, request)
    }

    #[test]
    fn clearance_requires_exact_complete_inputs_and_untampered_facts() {
        let (result, request) = clearance();
        result.validate(&request).unwrap();
        assert!(result.training_allowed());
        let mut foreign = request.clone();
        foreign["fingerprint"] = prefixed(&"2".repeat(64)).into();
        assert!(result.validate(&foreign).is_err());
        foreign = request.clone();
        foreign["rows"] = 1.into();
        assert!(result.validate(&foreign).is_err());
        let mut tampered = result.clone();
        tampered.missing_group_rows = 0;
        assert!(tampered.validate(&request).is_err());
        assert!(!tampered.training_allowed());
        for field in ["invalidRows", "duplicateRows", "overlapRows"] {
            let mut value = serde_json::to_value(&result).unwrap();
            value[field] = 1.into();
            let mut blocked: NomosTrainingClearance = serde_json::from_value(value).unwrap();
            blocked.fingerprint = blocked.reproduce().unwrap();
            blocked.validate(&request).unwrap();
            assert!(!blocked.training_allowed());
        }
    }
}
