//! Adapter-owned admission and immutable materialization of managed rows.

use super::*;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
};

const MATERIALIZATION_SCHEMA_VERSION: u32 = 1;
const MEMBER_FINGERPRINT_PROTOCOL: &str = "nomos-managed-training-members-v1";

/// Row-free identity of one managed dataset rendered into Nomos-native JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NomosTrainingDataset {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub dataset_version_id: Uuid,
    pub dataset_version_fingerprint: String,
    pub rows: u64,
    pub member_fingerprint: String,
    pub artifact: ExternalArtifactIdentity,
    pub fingerprint: String,
}

impl NomosTrainingDataset {
    fn create(
        run_id: Uuid,
        dataset_version_id: Uuid,
        dataset_version_fingerprint: String,
        rows: u64,
        member_fingerprint: String,
        artifact: ExternalArtifactIdentity,
    ) -> Result<Self, EncoderTaskAdapterError> {
        let mut value = Self {
            schema_version: MATERIALIZATION_SCHEMA_VERSION,
            run_id,
            dataset_version_id,
            dataset_version_fingerprint,
            rows,
            member_fingerprint,
            artifact,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate()?;
        Ok(value)
    }

    fn reproduce(&self) -> Result<String, EncoderTaskAdapterError> {
        artifact_core::fingerprint(&json!({
            "schemaVersion":self.schema_version,
            "runId":self.run_id,
            "datasetVersionId":self.dataset_version_id,
            "datasetVersionFingerprint":self.dataset_version_fingerprint,
            "rows":self.rows,
            "memberFingerprint":self.member_fingerprint,
            "artifact":self.artifact,
        }))
        .map_err(adapter_error)
    }

    pub(super) fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        self.artifact.validate().map_err(adapter_error)?;
        let expected_key = artifact_key(
            self.run_id,
            self.dataset_version_id,
            &self.dataset_version_fingerprint,
        )?;
        if self.schema_version != MATERIALIZATION_SCHEMA_VERSION
            || self.run_id.is_nil()
            || self.dataset_version_id.is_nil()
            || !canonical_fingerprint(&self.dataset_version_fingerprint)
            || self.rows == 0
            || !canonical_fingerprint(&self.member_fingerprint)
            || self.artifact.role != EvidenceRole::Training
            || self.artifact.key != expected_key
            || self.reproduce()? != self.fingerprint
        {
            return Err(adapter_error(
                "Nomos managed training dataset identity is invalid",
            ));
        }
        Ok(())
    }

    pub(super) fn verify_in(&self, root: &Path) -> Result<(), EncoderTaskAdapterError> {
        self.validate()?;
        let artifact = contained_existing(root, &self.artifact.key)?;
        verify_file(
            &artifact,
            self.artifact.bytes,
            self.artifact
                .fingerprint
                .strip_prefix("sha256:")
                .ok_or_else(|| adapter_error("Nomos training fingerprint is malformed"))?,
        )?;
        let receipt = contained_existing(root, &receipt_key(self)?)?;
        let stored: Self = serde_json::from_slice(&fs::read(receipt).map_err(|error| {
            adapter_error(format!("could not read Nomos training receipt: {error}"))
        })?)
        .map_err(|error| adapter_error(format!("Nomos training receipt is invalid: {error}")))?;
        if stored != *self {
            return Err(adapter_error(
                "Nomos training receipt differs from its immutable identity",
            ));
        }
        Ok(())
    }
}

/// Streaming writer. The application supplies schema-neutral managed rows;
/// this adapter alone decides whether they are valid Nomos training states.
pub struct NomosTrainingDatasetWriter {
    root: PathBuf,
    final_directory: PathBuf,
    scratch_parent: PathBuf,
    scratch_directory: Option<PathBuf>,
    writer: Option<BufWriter<File>>,
    run_id: Uuid,
    dataset_version_id: Uuid,
    dataset_version_fingerprint: String,
    expected_rows: u64,
    rows: u64,
    member_digest: Sha256,
    member_ids: BTreeSet<String>,
    decision_ids: BTreeSet<String>,
}

impl std::fmt::Debug for NomosTrainingDatasetWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NomosTrainingDatasetWriter")
            .field("run_id", &self.run_id)
            .field("dataset_version_id", &self.dataset_version_id)
            .field("expected_rows", &self.expected_rows)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl NomosBackend {
    /// Reopen an already-published managed training artifact from its exact
    /// project-run identity. This never scans for a substitute dataset.
    pub fn load_training_dataset(
        &self,
        run_id: Uuid,
        dataset_version_id: Uuid,
        dataset_version_fingerprint: &str,
    ) -> Result<NomosTrainingDataset, EncoderTaskAdapterError> {
        let artifact = artifact_key(run_id, dataset_version_id, dataset_version_fingerprint)?;
        let receipt = Path::new(&artifact)
            .parent()
            .ok_or_else(|| adapter_error("Nomos training artifact has no parent"))?
            .join("materialization.json")
            .to_string_lossy()
            .replace('\\', "/");
        let receipt = contained_existing(&self.root, &receipt)?;
        let dataset: NomosTrainingDataset =
            serde_json::from_slice(&fs::read(receipt).map_err(adapter_error)?)
                .map_err(adapter_error)?;
        if dataset.run_id != run_id
            || dataset.dataset_version_id != dataset_version_id
            || dataset.dataset_version_fingerprint != dataset_version_fingerprint
            || dataset.artifact.key != artifact
        {
            return Err(adapter_error(
                "Nomos training receipt belongs to another project run",
            ));
        }
        dataset.verify_in(&self.root)?;
        Ok(dataset)
    }

    /// Start an isolated rendering attempt. Existing completed content is never
    /// overwritten; a retry must reproduce it byte for byte.
    pub fn materialize_training_dataset(
        &self,
        run_id: Uuid,
        dataset_version_id: Uuid,
        dataset_version_fingerprint: impl Into<String>,
        expected_rows: u64,
    ) -> Result<NomosTrainingDatasetWriter, EncoderTaskAdapterError> {
        let dataset_version_fingerprint = dataset_version_fingerprint.into();
        if run_id.is_nil()
            || dataset_version_id.is_nil()
            || expected_rows == 0
            || !canonical_fingerprint(&dataset_version_fingerprint)
        {
            return Err(adapter_error(
                "Nomos training materialization request is invalid",
            ));
        }
        let digest = dataset_version_fingerprint
            .strip_prefix("sha256:")
            .expect("canonical fingerprint was checked");
        let scratch_parent = ensure_output_directory(
            &self.root,
            &[
                "runs",
                "encoder-gym-project-runs",
                &run_id.to_string(),
                "training",
                &dataset_version_id.to_string(),
            ],
        )?;
        let final_directory = scratch_parent.join(digest);
        let scratch_directory = scratch_parent.join(format!(".{digest}.tmp-{}", Uuid::new_v4()));
        fs::create_dir(&scratch_directory).map_err(|error| {
            adapter_error(format!("could not create Nomos training scratch: {error}"))
        })?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(scratch_directory.join("dataset.jsonl"))
            .map_err(|error| {
                adapter_error(format!("could not create Nomos training data: {error}"))
            })?;
        let mut member_digest = Sha256::new();
        member_digest.update(MEMBER_FINGERPRINT_PROTOCOL.as_bytes());
        Ok(NomosTrainingDatasetWriter {
            root: self.root.clone(),
            final_directory,
            scratch_parent,
            scratch_directory: Some(scratch_directory),
            writer: Some(BufWriter::new(file)),
            run_id,
            dataset_version_id,
            dataset_version_fingerprint,
            expected_rows,
            rows: 0,
            member_digest,
            member_ids: BTreeSet::new(),
            decision_ids: BTreeSet::new(),
        })
    }
}

impl NomosTrainingDatasetWriter {
    pub fn append(
        &mut self,
        member_id: &str,
        content_fingerprint: &str,
        value: &Value,
    ) -> Result<(), EncoderTaskAdapterError> {
        if self.rows >= self.expected_rows
            || !canonical_fingerprint(member_id)
            || !canonical_fingerprint(content_fingerprint)
            || artifact_core::fingerprint(value).map_err(adapter_error)? != content_fingerprint
            || !self.member_ids.insert(member_id.to_owned())
        {
            return Err(adapter_error(
                "Managed row identity changed during Nomos materialization",
            ));
        }
        let decision_id = validate_native_training_row(value)?;
        if !self.decision_ids.insert(decision_id) {
            return Err(adapter_error(
                "Nomos training dataset contains a duplicate decision state",
            ));
        }
        update_member_digest(&mut self.member_digest, member_id);
        update_member_digest(&mut self.member_digest, content_fingerprint);
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| adapter_error("Nomos training writer is already closed"))?;
        serde_json::to_writer(&mut *writer, value).map_err(adapter_error)?;
        writer.write_all(b"\n").map_err(adapter_error)?;
        self.rows += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<NomosTrainingDataset, EncoderTaskAdapterError> {
        if self.rows != self.expected_rows {
            return Err(adapter_error(format!(
                "Nomos training dataset expected {} rows but received {}",
                self.expected_rows, self.rows
            )));
        }
        let mut writer = self
            .writer
            .take()
            .ok_or_else(|| adapter_error("Nomos training writer is already closed"))?;
        writer.flush().map_err(adapter_error)?;
        writer.get_ref().sync_all().map_err(adapter_error)?;
        drop(writer);
        let scratch = self
            .scratch_directory
            .as_ref()
            .expect("live writer retains its scratch directory");
        let scratch_data = scratch.join("dataset.jsonl");
        let bytes = fs::metadata(&scratch_data).map_err(adapter_error)?.len();
        let digest = sha256_file(&scratch_data)?;
        let artifact = ExternalArtifactIdentity::new(
            artifact_key(
                self.run_id,
                self.dataset_version_id,
                &self.dataset_version_fingerprint,
            )?,
            EvidenceRole::Training,
            bytes,
            prefixed(&digest),
        )
        .map_err(adapter_error)?;
        let member_fingerprint = prefixed(&format!("{:x}", self.member_digest.clone().finalize()));
        let receipt = NomosTrainingDataset::create(
            self.run_id,
            self.dataset_version_id,
            self.dataset_version_fingerprint.clone(),
            self.rows,
            member_fingerprint,
            artifact,
        )?;
        let receipt_bytes = serde_json::to_vec_pretty(&receipt).map_err(adapter_error)?;
        let mut receipt_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(scratch.join("materialization.json"))
            .map_err(adapter_error)?;
        receipt_file
            .write_all(&receipt_bytes)
            .map_err(adapter_error)?;
        receipt_file.sync_all().map_err(adapter_error)?;
        drop(receipt_file);

        if self.final_directory.exists() {
            verify_existing_materialization(&self.root, &self.final_directory, &receipt)?;
            remove_exact_scratch_directory(scratch, &self.scratch_parent)?;
            self.scratch_directory = None;
            return Ok(receipt);
        }
        match fs::rename(scratch, &self.final_directory) {
            Ok(()) => self.scratch_directory = None,
            Err(_error) if self.final_directory.exists() => {
                verify_existing_materialization(&self.root, &self.final_directory, &receipt)?;
                remove_exact_scratch_directory(scratch, &self.scratch_parent)?;
                self.scratch_directory = None;
            }
            Err(error) => {
                return Err(adapter_error(format!(
                    "could not publish Nomos training dataset: {error}"
                )));
            }
        }
        receipt.verify_in(&self.root)?;
        Ok(receipt)
    }
}

impl Drop for NomosTrainingDatasetWriter {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(scratch) = self.scratch_directory.take() {
            let _ = remove_exact_scratch_directory(&scratch, &self.scratch_parent);
        }
    }
}

fn verify_existing_materialization(
    root: &Path,
    directory: &Path,
    expected: &NomosTrainingDataset,
) -> Result<(), EncoderTaskAdapterError> {
    let relative = directory.strip_prefix(root).map_err(adapter_error)?;
    let metadata = directory.symlink_metadata().map_err(adapter_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(adapter_error(
            "Nomos training materialization target is not a plain directory",
        ));
    }
    let stored: NomosTrainingDataset = serde_json::from_slice(
        &fs::read(directory.join("materialization.json")).map_err(adapter_error)?,
    )
    .map_err(adapter_error)?;
    if stored != *expected || Path::new(&stored.artifact.key).parent() != Some(relative) {
        return Err(adapter_error(
            "Existing Nomos training materialization has different content",
        ));
    }
    stored.verify_in(root)
}

fn validate_native_training_row(value: &Value) -> Result<String, EncoderTaskAdapterError> {
    let row = value
        .as_object()
        .ok_or_else(|| adapter_error("Nomos training row must be a JSON object"))?;
    if row.get("schema_version").and_then(Value::as_str) != Some("decision-state.v2")
        || row.get("evaluation_partition").and_then(Value::as_str) != Some("train")
        || row.get("accepted").and_then(Value::as_bool) != Some(true)
        || row.get("task_kind").and_then(Value::as_str) == Some("verify")
    {
        return Err(adapter_error(
            "Nomos training row is not an accepted train-partition decision state",
        ));
    }
    let decision_id = required_text(row.get("decision_state_id"), "decision_state_id")?;
    required_text(row.get("question"), "question")?;
    let legal = string_set(row.get("legal_candidate_ids"), "legal_candidate_ids", true)?;
    let previous = string_set(
        row.get("previous_candidate_ids"),
        "previous_candidate_ids",
        false,
    )?;
    let registry = row
        .get("tool_registry")
        .and_then(Value::as_object)
        .ok_or_else(|| adapter_error("Nomos training row has no tool registry"))?;
    required_text(registry.get("registry_id"), "tool_registry.registry_id")?;
    required_text(
        registry.get("registry_fingerprint"),
        "tool_registry.registry_fingerprint",
    )?;
    let tools = registry
        .get("tools")
        .and_then(Value::as_array)
        .filter(|tools| !tools.is_empty())
        .ok_or_else(|| adapter_error("Nomos tool registry has no tools"))?;
    let mut tool_ids = BTreeSet::new();
    for tool in tools {
        let tool = tool
            .as_object()
            .ok_or_else(|| adapter_error("Nomos tool registry entry is not an object"))?;
        let id = required_text(tool.get("tool_id"), "tool_registry.tools.tool_id")?;
        if !tool_ids.insert(id) || !tool_shape_is_usable(tool) {
            return Err(adapter_error("Nomos tool registry entry is invalid"));
        }
    }
    if !legal.is_subset(&tool_ids) {
        return Err(adapter_error(
            "Nomos legal candidates are absent from the tool registry",
        ));
    }
    if !previous.is_subset(&tool_ids) {
        return Err(adapter_error(
            "Nomos previous candidates are absent from the tool registry",
        ));
    }
    let label = row
        .get("label")
        .and_then(Value::as_object)
        .ok_or_else(|| adapter_error("Nomos training row has no label"))?;
    let positives = string_set(
        label.get("acceptable_tools"),
        "label.acceptable_tools",
        false,
    )?;
    let negatives = string_set(
        label.get("hard_negative_tools"),
        "label.hard_negative_tools",
        false,
    )?;
    if !positives.is_subset(&legal)
        || !negatives.is_subset(&legal)
        || !positives.is_disjoint(&negatives)
    {
        return Err(adapter_error(
            "Nomos training label does not match its legal candidates",
        ));
    }
    let eligible = if row.get("task_kind").and_then(Value::as_str) == Some("recover") {
        legal
            .difference(&previous)
            .cloned()
            .collect::<BTreeSet<_>>()
    } else {
        legal.clone()
    };
    let usable_positives = positives.intersection(&eligible).count();
    let usable_negatives = if negatives.is_empty() {
        eligible.difference(&positives).count()
    } else {
        negatives.intersection(&eligible).count()
    };
    let control = label.get("no_tool_needed").and_then(Value::as_bool) == Some(true)
        || label.get("router_should_abstain").and_then(Value::as_bool) == Some(true);
    if usable_negatives == 0 || !control && usable_positives == 0 {
        return Err(adapter_error(
            "Nomos training row cannot produce a positive/negative training example",
        ));
    }
    Ok(decision_id)
}

fn tool_shape_is_usable(tool: &serde_json::Map<String, Value>) -> bool {
    ["tool_family", "description", "side_effect_class"]
        .into_iter()
        .all(|key| {
            tool.get(key)
                .and_then(Value::as_str)
                .is_some_and(|v| !v.trim().is_empty())
        })
        && [
            "capabilities",
            "input_modalities",
            "output_modalities",
            "evidence_roles",
        ]
        .into_iter()
        .all(|key| {
            tool.get(key)
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().all(|value| value.as_str().is_some()))
        })
        && tool.get("argument_schema").is_some_and(Value::is_object)
}

fn required_text(value: Option<&Value>, field: &str) -> Result<String, EncoderTaskAdapterError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.trim() == *value)
        .map(str::to_owned)
        .ok_or_else(|| adapter_error(format!("Nomos training {field} is invalid")))
}

fn string_set(
    value: Option<&Value>,
    field: &str,
    require_nonempty: bool,
) -> Result<BTreeSet<String>, EncoderTaskAdapterError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| adapter_error(format!("Nomos training {field} must be an array")))?;
    let set = values
        .iter()
        .map(|value| required_text(Some(value), field))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if set.len() != values.len() || require_nonempty && set.is_empty() {
        return Err(adapter_error(format!(
            "Nomos training {field} is empty or contains duplicates"
        )));
    }
    Ok(set)
}

fn update_member_digest(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn artifact_key(
    run_id: Uuid,
    dataset_version_id: Uuid,
    dataset_version_fingerprint: &str,
) -> Result<String, EncoderTaskAdapterError> {
    let digest = dataset_version_fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Dataset version fingerprint is not canonical"))?;
    Ok(format!(
        "runs/encoder-gym-project-runs/{run_id}/training/{dataset_version_id}/{digest}/dataset.jsonl"
    ))
}

fn receipt_key(dataset: &NomosTrainingDataset) -> Result<String, EncoderTaskAdapterError> {
    let artifact = Path::new(&dataset.artifact.key);
    Ok(artifact
        .parent()
        .ok_or_else(|| adapter_error("Nomos training artifact has no parent"))?
        .join("materialization.json")
        .to_string_lossy()
        .replace('\\', "/"))
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(raw_sha256)
}

fn contained_existing(root: &Path, relative: &str) -> Result<PathBuf, EncoderTaskAdapterError> {
    validate_relative(relative)?;
    let path = root.join(relative).canonicalize().map_err(adapter_error)?;
    if !path.starts_with(root) {
        return Err(adapter_error(
            "Nomos managed training path escaped its root",
        ));
    }
    Ok(path)
}

fn ensure_output_directory(
    root: &Path,
    components: &[&str],
) -> Result<PathBuf, EncoderTaskAdapterError> {
    let mut directory = root.to_path_buf();
    for component in components {
        if component.is_empty()
            || Path::new(component).components().count() != 1
            || matches!(component, &"." | &"..")
        {
            return Err(adapter_error("Nomos output directory component is invalid"));
        }
        directory.push(component);
        match directory.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(adapter_error(
                    "Nomos output directory is not a plain directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory).map_err(adapter_error)?;
            }
            Err(error) => return Err(adapter_error(error)),
        }
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(decision_id: &str) -> Value {
        json!({
            "schema_version":"decision-state.v2",
            "decision_state_id":decision_id,
            "question":"Which tool should run?",
            "evaluation_partition":"train",
            "accepted":true,
            "task_kind":"route",
            "previous_candidate_ids":[],
            "legal_candidate_ids":["tool_a","tool_b"],
            "label":{
                "acceptable_tools":["tool_a"],
                "hard_negative_tools":["tool_b"]
            },
            "tool_registry":{
                "registry_id":"registry_a",
                "registry_fingerprint":"sha256:registry",
                "tools":[
                    {"tool_id":"tool_a","tool_family":"search","description":"Find the requested evidence","capabilities":["search"],"input_modalities":["text"],"output_modalities":["text"],"evidence_roles":["primary"],"side_effect_class":"none","argument_schema":{}},
                    {"tool_id":"tool_b","tool_family":"search","description":"Find unrelated evidence","capabilities":["search"],"input_modalities":["text"],"output_modalities":["text"],"evidence_roles":["primary"],"side_effect_class":"none","argument_schema":{}}
                ]
            }
        })
    }

    fn backend(root: &Path) -> NomosBackend {
        let identity = BackendIdentity::new(
            ADAPTER_NAME,
            ADAPTER_PROTOCOL_VERSION,
            prefixed(&"1".repeat(64)),
        )
        .unwrap();
        NomosBackend {
            root: root.to_path_buf(),
            python: root.join("python-must-not-run"),
            manifest: crate::tests::manifest_with_named_suites(),
            baseline_override: None,
            training_override: None,
            identity: identity.clone(),
            observer_identity: identity.clone(),
            repair_delta_identity: identity,
            progress: None,
        }
    }

    #[test]
    fn managed_rows_are_validated_streamed_and_published_immutably() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let backend = backend(&root);
        let run_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let version_fingerprint = prefixed(&"a".repeat(64));
        let first = row("decision-a");
        let second = row("decision-b");
        let first_fingerprint = artifact_core::fingerprint(&first).unwrap();
        let second_fingerprint = artifact_core::fingerprint(&second).unwrap();
        let mut writer = backend
            .materialize_training_dataset(run_id, version_id, &version_fingerprint, 2)
            .unwrap();
        writer
            .append(&prefixed(&"b".repeat(64)), &first_fingerprint, &first)
            .unwrap();
        writer
            .append(&prefixed(&"c".repeat(64)), &second_fingerprint, &second)
            .unwrap();
        let materialized = writer.finish().unwrap();
        materialized.verify_in(&root).unwrap();
        assert_eq!(
            backend
                .load_training_dataset(run_id, version_id, &version_fingerprint)
                .unwrap(),
            materialized
        );
        assert_eq!(materialized.rows, 2);
        assert_eq!(
            fs::read_to_string(root.join(&materialized.artifact.key))
                .unwrap()
                .lines()
                .count(),
            2
        );

        let mut retry = backend
            .materialize_training_dataset(run_id, version_id, &version_fingerprint, 2)
            .unwrap();
        retry
            .append(&prefixed(&"b".repeat(64)), &first_fingerprint, &first)
            .unwrap();
        retry
            .append(&prefixed(&"c".repeat(64)), &second_fingerprint, &second)
            .unwrap();
        assert_eq!(retry.finish().unwrap(), materialized);

        let mut invalid = row("decision-c");
        invalid["evaluation_partition"] = json!("test");
        let invalid_fingerprint = artifact_core::fingerprint(&invalid).unwrap();
        let mut rejected = backend
            .materialize_training_dataset(
                Uuid::new_v4(),
                Uuid::new_v4(),
                prefixed(&"d".repeat(64)),
                1,
            )
            .unwrap();
        assert!(
            rejected
                .append(&prefixed(&"e".repeat(64)), &invalid_fingerprint, &invalid)
                .is_err()
        );
    }

    #[test]
    fn recovery_rows_may_exclude_previous_candidates_from_the_legal_pool() {
        let mut recovery = row("decision-recovery");
        recovery["task_kind"] = json!("recover");
        recovery["previous_candidate_ids"] = json!(["tool_previous"]);
        recovery["tool_registry"]["tools"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "tool_id":"tool_previous",
                "tool_family":"search",
                "description":"A candidate rejected during the previous retrieval pass",
                "capabilities":["search"],
                "input_modalities":["text"],
                "output_modalities":["text"],
                "evidence_roles":["primary"],
                "side_effect_class":"none",
                "argument_schema":{}
            }));

        assert_eq!(
            validate_native_training_row(&recovery).unwrap(),
            "decision-recovery"
        );

        recovery["previous_candidate_ids"] = json!(["missing_tool"]);
        assert_eq!(
            validate_native_training_row(&recovery)
                .unwrap_err()
                .to_string(),
            "encoder task adapter failed: Nomos previous candidates are absent from the tool registry"
        );
    }

    #[test]
    fn managed_candidate_handoff_reopens_exact_dataset_without_repair_or_holdout_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut backend = backend(&root);
        backend.identity.configuration_fingerprint = artifact_core::fingerprint(&json!({
            "adapter":ADAPTER_NAME,"protocol_version":ADAPTER_PROTOCOL_VERSION,
            "manifest_schema_version":4,"tree_hash_algorithm":TREE_HASH_ALGORITHM
        }))
        .unwrap();
        let fp = prefixed(&"a".repeat(64));
        let value = row("managed-training-row");
        let mut writer = backend
            .materialize_training_dataset(Uuid::new_v4(), Uuid::new_v4(), &fp, 1)
            .unwrap();
        writer
            .append(&fp, &artifact_core::fingerprint(&value).unwrap(), &value)
            .unwrap();
        let dataset = writer.finish().unwrap();
        let configuration = json!({
            "adapter_protocol":ADAPTER_PROTOCOL_VERSION,"source_reference":{},"baseline_evidence":{},
            "training_inputs":[dataset.artifact.key], "managed_training_dataset":dataset,
            "reference_models":{"reference":{"path":"missing-reference","format":"sentence-transformers","bytes":1,"fingerprint":fp,"provenance":{}}},
            "agent_evaluation":{"backend":"onnx","chat_model":{"path":"missing-chat","format":"onnxruntime-genai","bytes":1,"fingerprint":fp,"source":{}},
                "selector_strategy":"multiview","candidate_strategy":"multiview","nomos_top_k":1,"max_attempts":1,
                "development":{"suite":"development","sessions":1,"pairing":"cycle","condition":"nomos"},
                "sealed":{"suite":"promotion","sessions":1,"pairing":"cycle","condition":"nomos"}},
            "suites":{"development":{"path":"missing-development.jsonl","role":"development","fingerprint":fp,"retrieval_fingerprint":fp,"agent_fingerprint":fp}}
        });
        let project = ExternalProjectSnapshot::create(
            "Managed",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            &fp,
            backend.identity.clone(),
            vec![
                dataset.artifact.clone(),
                ExternalArtifactIdentity::new(
                    "missing-development.jsonl",
                    EvidenceRole::Development,
                    1,
                    &fp,
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "missing-sealed.jsonl",
                    EvidenceRole::SealedAcceptance,
                    1,
                    &fp,
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, &fp).unwrap(),
            configuration,
            Utc::now(),
        )
        .unwrap();
        let candidate =
            NomosBackend::initial_training_candidate_definition(Uuid::new_v4(), &project, 60)
                .unwrap();
        let output = backend.candidate_output(&candidate);
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("model.safetensors"), b"immutable model").unwrap();
        let logical = workspace_relative(&root, &output).unwrap();
        let mut manifest = json!({"base_model":"baseline","output":logical,
            "inputs":[dataset.artifact.key],"input_row_counts":{dataset.artifact.key.clone():1}});
        let manifest_path = output.join("nomos_training_manifest.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let model = || {
            let (bytes, hash) = tree_identity(&output).unwrap();
            ModelArtifactIdentity::new(&logical, "sentence-transformers", bytes, prefixed(&hash))
                .unwrap()
        };
        // This was the registration bug: a freshly opened runtime had no override.
        assert!(
            backend
                .verified_training_data(&project, &candidate, &model())
                .is_err()
        );
        let reloaded = backend
            .load_training_dataset(
                dataset.run_id,
                dataset.dataset_version_id,
                &dataset.dataset_version_fingerprint,
            )
            .unwrap();
        let backend = backend.with_training_dataset(reloaded).unwrap();
        let result = backend
            .verified_training_data(&project, &candidate, &model())
            .unwrap();
        assert_eq!(result.snapshot_id, dataset.dataset_version_id);
        assert_eq!(
            result.snapshot_fingerprint,
            dataset.dataset_version_fingerprint
        );
        assert_eq!(result.inputs.len(), 1);
        assert_eq!(result.inputs[0].rows, 1);
        assert_eq!(result.inputs[0].fingerprint, dataset.artifact.fingerprint);
        // Even a rehashed model cannot claim a different input population.
        manifest["input_row_counts"][&dataset.artifact.key] = json!(2);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            backend
                .verified_training_data(&project, &candidate, &model())
                .is_err()
        );
    }
}
