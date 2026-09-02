//! Compiled adapter for the isolated Nomos retrieval-ranking production pilot.

use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chrono::Utc;
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, ParameterValue, TrainingCandidate,
    },
    metrics::{EvaluationReport, MetricContract},
    ports::{
        AdapterInspection, BoxFuture, EncoderTaskAdapterError, EncoderTaskBackend, TrainOutput,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;

const ADAPTER_NAME: &str = "nomos";
const ADAPTER_PROTOCOL_VERSION: &str = "nomos-ranking-v1";
const EXPERIMENT_MANIFEST_NAME: &str = "encoder-gym-experiment.json";
const TREE_HASH_ALGORITHM: &str = "sha256-ordinal-path-size-content-sha256-v1";

#[derive(Debug, Clone)]
pub struct NomosBackend {
    root: PathBuf,
    python: PathBuf,
    manifest: NomosExperimentManifest,
    identity: BackendIdentity,
}

impl NomosBackend {
    pub fn open(
        root: impl AsRef<Path>,
        python: impl Into<PathBuf>,
    ) -> Result<Self, EncoderTaskAdapterError> {
        let root = root.as_ref().canonicalize().map_err(|error| {
            adapter_error(format!("could not resolve Nomos experiment root: {error}"))
        })?;
        if !root.join("ENCODER_GYM_EXPERIMENT.md").is_file() {
            return Err(adapter_error(
                "Nomos adapter requires the isolated ENCODER_GYM_EXPERIMENT.md marker",
            ));
        }
        let raw = fs::read(root.join(EXPERIMENT_MANIFEST_NAME)).map_err(|error| {
            adapter_error(format!("could not read Nomos experiment manifest: {error}"))
        })?;
        let manifest: NomosExperimentManifest = serde_json::from_slice(&raw).map_err(|error| {
            adapter_error(format!("Nomos experiment manifest is invalid: {error}"))
        })?;
        manifest.validate()?;
        let configuration_fingerprint = artifact_core::fingerprint(&json!({
            "adapter": ADAPTER_NAME,
            "protocol_version": ADAPTER_PROTOCOL_VERSION,
            "manifest_schema_version": manifest.schema_version,
            "tree_hash_algorithm": manifest.tree_hash_algorithm,
        }))
        .map_err(adapter_error)?;
        let identity = BackendIdentity::new(
            ADAPTER_NAME,
            ADAPTER_PROTOCOL_VERSION,
            configuration_fingerprint,
        )
        .map_err(adapter_error)?;
        Ok(Self {
            root,
            python: python.into(),
            manifest,
            identity,
        })
    }

    pub fn project_snapshot(&self) -> Result<ExternalProjectSnapshot, EncoderTaskAdapterError> {
        self.verify_no_remote()?;
        self.verify_clean_worktree()?;
        let revision = self.git_output(["rev-parse", "HEAD"])?;
        let manifest_fingerprint = sha256_file(&self.root.join(EXPERIMENT_MANIFEST_NAME))?;
        let source_fingerprint = artifact_core::fingerprint(&json!({
            "experiment_revision": revision,
            "manifest_sha256": manifest_fingerprint,
            "source_commit": self.manifest.source.commit,
        }))
        .map_err(adapter_error)?;

        let mut inputs = Vec::with_capacity(self.manifest.datasets.len());
        for dataset in &self.manifest.datasets {
            let path = self.resolve_existing(&dataset.path)?;
            verify_file(&path, dataset.bytes, &dataset.sha256)?;
            inputs.push(
                ExternalArtifactIdentity::new(
                    dataset.path.clone(),
                    dataset.role.into_domain()?,
                    dataset.bytes,
                    prefixed(&dataset.sha256),
                )
                .map_err(adapter_error)?,
            );
        }
        let baseline_path = self.resolve_existing(&self.manifest.baseline.pytorch_path)?;
        let (baseline_bytes, baseline_digest) = tree_identity(&baseline_path)?;
        if baseline_digest != self.manifest.baseline.pytorch_tree_sha256 {
            return Err(adapter_error(format!(
                "Nomos baseline PyTorch tree no longer matches the experiment manifest: expected {}, observed {baseline_digest}",
                self.manifest.baseline.pytorch_tree_sha256
            )));
        }
        let onnx_path = self.resolve_existing(&self.manifest.baseline.onnx_path)?;
        let (onnx_bytes, onnx_digest) = tree_identity(&onnx_path)?;
        if onnx_digest != self.manifest.baseline.onnx_tree_sha256 {
            return Err(adapter_error(format!(
                "Nomos baseline ONNX tree no longer matches the experiment manifest: expected {}, observed {onnx_digest}",
                self.manifest.baseline.onnx_tree_sha256
            )));
        }
        let baseline_model = ModelArtifactIdentity::new(
            self.manifest.baseline.pytorch_path.clone(),
            "sentence-transformers",
            baseline_bytes,
            prefixed(&baseline_digest),
        )
        .map_err(adapter_error)?;
        let training_inputs = self
            .manifest
            .datasets
            .iter()
            .filter(|value| value.role == NativeEvidenceRole::Training)
            .map(|value| value.path.clone())
            .collect::<Vec<_>>();
        let development = self
            .manifest
            .dataset_for(NativeEvidenceRole::DevelopmentHoldout)?;
        let sealed = self
            .manifest
            .dataset_for(NativeEvidenceRole::SealedHoldout)?;
        ExternalProjectSnapshot::create(
            self.manifest.experiment.clone(),
            EncoderTaskKind::RetrievalRanking,
            revision,
            source_fingerprint,
            self.identity.clone(),
            inputs,
            baseline_model,
            json!({
                "adapter_protocol": ADAPTER_PROTOCOL_VERSION,
                "source_reference": {
                    "repository": self.manifest.source.repository,
                    "commit": self.manifest.source.commit,
                    "snapshot_date": self.manifest.source.snapshot_date,
                    "access": self.manifest.source.access,
                },
                "baseline_evidence": {
                    "onnx": {
                        "path": self.manifest.baseline.onnx_path,
                        "bytes": onnx_bytes,
                        "fingerprint": prefixed(&onnx_digest),
                    },
                    "weak_agent_raw": self.manifest.baseline.weak_agent_raw_completed,
                    "weak_agent_complete_coprocessor": self.manifest.baseline.weak_agent_complete_coprocessor_completed,
                    "historical_evaluation_runs_fingerprint": prefixed(&self.manifest.evaluation_runs_tree_sha256),
                },
                "training_inputs": training_inputs,
                "suites": {
                    "development": {
                        "path": development.path,
                        "role": "development",
                        "fingerprint": prefixed(&development.sha256),
                    },
                    "sealed": {
                        "path": sealed.path,
                        "role": "sealed_acceptance",
                        "fingerprint": prefixed(&sealed.sha256),
                    }
                }
            }),
            Utc::now(),
        )
        .map_err(adapter_error)
    }

    fn verify_no_remote(&self) -> Result<(), EncoderTaskAdapterError> {
        if !self.git_output(["remote"])?.is_empty() {
            return Err(adapter_error(
                "isolated Nomos experiment must not have a Git remote",
            ));
        }
        Ok(())
    }

    fn verify_clean_worktree(&self) -> Result<(), EncoderTaskAdapterError> {
        if !self.git_output(["status", "--porcelain"])?.is_empty() {
            return Err(adapter_error(
                "isolated Nomos experiment must be committed and clean before registration",
            ));
        }
        Ok(())
    }

    fn git_output<const N: usize>(
        &self,
        arguments: [&str; N],
    ) -> Result<String, EncoderTaskAdapterError> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(arguments)
            .output()
            .map_err(|error| {
                adapter_error(format!("could not inspect isolated Git state: {error}"))
            })?;
        if !output.status.success() {
            return Err(adapter_error("could not inspect isolated Nomos Git state"));
        }
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_owned())
            .map_err(|_| adapter_error("Git inspection returned non-UTF-8 output"))
    }

    fn resolve_existing(&self, relative: &str) -> Result<PathBuf, EncoderTaskAdapterError> {
        validate_relative(relative)?;
        let resolved = self.root.join(relative).canonicalize().map_err(|error| {
            adapter_error(format!(
                "could not resolve Nomos artifact {relative}: {error}"
            ))
        })?;
        if !resolved.starts_with(&self.root) {
            return Err(adapter_error(
                "Nomos artifact escaped the isolated workspace",
            ));
        }
        Ok(resolved)
    }

    fn task_configuration(
        project: &ExternalProjectSnapshot,
    ) -> Result<TaskConfiguration, EncoderTaskAdapterError> {
        serde_json::from_value(project.task_configuration.clone())
            .map_err(|error| adapter_error(format!("Nomos task configuration is invalid: {error}")))
    }

    async fn run_bounded(
        &self,
        arguments: &[String],
        maximum_seconds: u64,
    ) -> Result<Value, EncoderTaskAdapterError> {
        if maximum_seconds == 0 {
            return Err(adapter_error(
                "Nomos process requires a positive time limit",
            ));
        }
        let mut command = Command::new(&self.python);
        command
            .args(arguments)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(maximum_seconds), command.output())
            .await
            .map_err(|_| adapter_error("Nomos process exceeded its finite time limit"))?
            .map_err(|error| adapter_error(format!("could not start Nomos process: {error}")))?;
        if !output.status.success() {
            let stderr = bounded_text(&output.stderr, 2_000);
            return Err(adapter_error(format!(
                "Nomos process failed with status {}: {stderr}",
                output.status
            )));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| adapter_error(format!("Nomos process output was not JSON: {error}")))
    }

    fn candidate_output(&self, candidate: &TrainingCandidate) -> PathBuf {
        self.root
            .join("artifacts")
            .join("encoder-gym-candidates")
            .join(candidate.id.to_string())
    }

    fn model_path(
        &self,
        model: &ModelArtifactIdentity,
    ) -> Result<PathBuf, EncoderTaskAdapterError> {
        let path = self.resolve_existing(&model.key)?;
        let (bytes, digest) = tree_identity(&path)?;
        if bytes != model.bytes || prefixed(&digest) != model.fingerprint {
            return Err(adapter_error(
                "Nomos model tree does not match its immutable model identity",
            ));
        }
        Ok(path)
    }
}

impl EncoderTaskBackend for NomosBackend {
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }

    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>> {
        Box::pin(async move {
            project.validate_integrity().map_err(adapter_error)?;
            if project.backend != self.identity {
                return Err(adapter_error(
                    "Nomos project pins a different adapter identity",
                ));
            }
            let current = self.project_snapshot()?;
            if current.source_revision != project.source_revision
                || current.source_fingerprint != project.source_fingerprint
                || current.inputs != project.inputs
                || current.baseline_model.fingerprint != project.baseline_model.fingerprint
            {
                return Err(adapter_error(
                    "isolated Nomos workspace changed after project snapshot creation",
                ));
            }
            Ok(AdapterInspection {
                source_fingerprint: project.source_fingerprint,
                verified_artifact_keys: project
                    .inputs
                    .iter()
                    .map(|value| value.key.clone())
                    .collect(),
                metadata: json!({
                    "adapter": ADAPTER_NAME,
                    "protocol_version": ADAPTER_PROTOCOL_VERSION,
                    "original_repository_access": self.manifest.source.access,
                    "git_remote_present": false,
                }),
            })
        })
    }

    fn train(
        &self,
        project: ExternalProjectSnapshot,
        candidate: TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainOutput, EncoderTaskAdapterError>> {
        Box::pin(async move {
            project.validate_integrity().map_err(adapter_error)?;
            candidate
                .validate_integrity(&project)
                .map_err(adapter_error)?;
            self.inspect(project.clone()).await?;
            let configuration = Self::task_configuration(&project)?;
            configuration.validate()?;
            let parameters = NativeTrainingParameters::parse(&candidate.parameters)?;
            let output = self.candidate_output(&candidate);
            if output.exists() {
                return Err(adapter_error(
                    "Nomos candidate output already exists; immutable candidates are not overwritten",
                ));
            }
            let output_relative = output
                .strip_prefix(&self.root)
                .map_err(adapter_error)?
                .to_string_lossy()
                .replace('\\', "/");
            let module = match parameters.loss.as_str() {
                "triplet" => "tools.train_dense_triplet_router",
                "mnrl" | "cached-mnrl" => "tools.train_dense_router",
                _ => return Err(adapter_error("unsupported Nomos loss")),
            };
            let mut arguments = vec![
                "-m".into(),
                module.into(),
                "--base-model".into(),
                project.baseline_model.key.clone(),
            ];
            for input in &configuration.training_inputs {
                validate_relative(input)?;
                arguments.push("--input".into());
                arguments.push(input.clone());
            }
            arguments.extend([
                "--output".into(),
                output_relative.clone(),
                "--epochs".into(),
                parameters.epochs.to_string(),
                "--batch-size".into(),
                parameters.batch_size.to_string(),
                "--learning-rate".into(),
                parameters.learning_rate.to_string(),
                "--seed".into(),
                parameters.seed.to_string(),
                "--device".into(),
                parameters.device.clone(),
            ]);
            if module.ends_with("triplet_router") {
                arguments.extend([
                    "--margin".into(),
                    parameters.margin.to_string(),
                    "--mining-batch-size".into(),
                    parameters.mining_batch_size.to_string(),
                    "--query-strategy".into(),
                    parameters.query_strategy.clone(),
                    "--positive-strategy".into(),
                    parameters.positive_strategy.clone(),
                ]);
            } else {
                arguments.extend(["--loss".into(), parameters.loss.clone()]);
            }
            let started = std::time::Instant::now();
            let metadata = self
                .run_bounded(&arguments, candidate.maximum_training_seconds)
                .await?;
            let (bytes, digest) = tree_identity(&output)?;
            let model = ModelArtifactIdentity::new(
                output_relative,
                "sentence-transformers",
                bytes,
                prefixed(&digest),
            )
            .map_err(adapter_error)?;
            Ok(TrainOutput {
                model,
                duration_seconds: started.elapsed().as_secs(),
                metadata,
            })
        })
    }

    fn evaluate(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
        maximum_seconds: u64,
    ) -> BoxFuture<'_, Result<EvaluationReport, EncoderTaskAdapterError>> {
        Box::pin(async move {
            project.validate_integrity().map_err(adapter_error)?;
            contract.validate_integrity().map_err(adapter_error)?;
            model.validate().map_err(adapter_error)?;
            self.inspect(project.clone()).await?;
            let configuration = Self::task_configuration(&project)?;
            configuration.validate()?;
            let suite = configuration
                .suites
                .get(&suite_key)
                .ok_or_else(|| adapter_error(format!("unknown Nomos suite {suite_key}")))?;
            let input = self.resolve_existing(&suite.path)?;
            let model_path = self.model_path(&model)?;
            let output = self
                .root
                .join("runs")
                .join("encoder-gym-evaluations")
                .join(model.id.to_string())
                .join(format!("{suite_key}.json"));
            if output.exists() {
                return Err(adapter_error(
                    "Nomos evaluation evidence already exists and cannot be overwritten",
                ));
            }
            let output_relative = output
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
                model_relative,
                "--input".into(),
                input_relative,
                "--limit".into(),
                "10000".into(),
                "--device".into(),
                "cpu".into(),
                "--output".into(),
                output_relative,
            ];
            let raw = self.run_bounded(&arguments, maximum_seconds).await?;
            let native: NativeEvaluation = serde_json::from_value(raw).map_err(|error| {
                adapter_error(format!("Nomos evaluation JSON is invalid: {error}"))
            })?;
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
            let available = BTreeMap::from([
                ("recall_at_1".to_owned(), first.metrics.recall_at_1),
                ("recall_at_2".to_owned(), first.metrics.recall_at_2),
                ("recall_at_3".to_owned(), first.metrics.recall_at_3),
                ("mrr".to_owned(), first.metrics.mrr),
                (
                    "mean_positive_margin".to_owned(),
                    first.metrics.mean_positive_margin,
                ),
            ]);
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
                first.metrics.states,
                Utc::now(),
            )
            .map_err(adapter_error)
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomosExperimentManifest {
    schema_version: u32,
    experiment: String,
    tree_hash_algorithm: String,
    source: SourceManifest,
    isolation: IsolationManifest,
    baseline: BaselineManifest,
    datasets: Vec<DatasetManifest>,
    evaluation_runs_tree_sha256: String,
}

impl NomosExperimentManifest {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        if self.schema_version != 2
            || self.experiment.trim() != self.experiment
            || self.experiment.is_empty()
            || self.tree_hash_algorithm != TREE_HASH_ALGORITHM
            || self.source.access != "read_only_reference"
            || self.source.repository.trim() != self.source.repository
            || self.source.repository.is_empty()
            || self.source.commit.trim().is_empty()
            || chrono::NaiveDate::parse_from_str(&self.source.snapshot_date, "%Y-%m-%d").is_err()
            || self.isolation.git_remote_allowed
            || self.isolation.hard_links_allowed
            || self.isolation.symlinks_allowed
            || !self.isolation.outputs_must_remain_below_experiment_root
            || !raw_sha256(&self.baseline.pytorch_tree_sha256)
            || !raw_sha256(&self.baseline.onnx_tree_sha256)
            || !raw_sha256(&self.evaluation_runs_tree_sha256)
            || !self.baseline.weak_agent_raw_completed.valid()
            || !self
                .baseline
                .weak_agent_complete_coprocessor_completed
                .valid()
        {
            return Err(adapter_error(
                "Nomos experiment isolation manifest is invalid",
            ));
        }
        let roles = self
            .datasets
            .iter()
            .map(|value| value.role)
            .collect::<std::collections::BTreeSet<_>>();
        if !roles.contains(&NativeEvidenceRole::Training)
            || !roles.contains(&NativeEvidenceRole::DevelopmentHoldout)
            || !roles.contains(&NativeEvidenceRole::SealedHoldout)
        {
            return Err(adapter_error(
                "Nomos experiment requires training, development, and sealed datasets",
            ));
        }
        for dataset in &self.datasets {
            validate_relative(&dataset.path)?;
            if dataset.bytes == 0 || !raw_sha256(&dataset.sha256) {
                return Err(adapter_error("Nomos dataset identity is invalid"));
            }
        }
        Ok(())
    }

    fn dataset_for(
        &self,
        role: NativeEvidenceRole,
    ) -> Result<&DatasetManifest, EncoderTaskAdapterError> {
        let matching = self
            .datasets
            .iter()
            .filter(|value| value.role == role)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(adapter_error(format!(
                "Nomos experiment requires exactly one {role:?} dataset"
            )));
        }
        Ok(matching[0])
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceManifest {
    repository: String,
    access: String,
    commit: String,
    snapshot_date: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct IsolationManifest {
    git_remote_allowed: bool,
    hard_links_allowed: bool,
    symlinks_allowed: bool,
    outputs_must_remain_below_experiment_root: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselineManifest {
    pytorch_path: String,
    pytorch_tree_sha256: String,
    onnx_path: String,
    onnx_tree_sha256: String,
    weak_agent_raw_completed: CountMetric,
    weak_agent_complete_coprocessor_completed: CountMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CountMetric {
    completed: u64,
    total: u64,
}

impl CountMetric {
    const fn valid(&self) -> bool {
        self.total > 0 && self.completed <= self.total
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetManifest {
    path: String,
    role: NativeEvidenceRole,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativeEvidenceRole {
    Training,
    DevelopmentHoldout,
    Calibration,
    SealedHoldout,
}

impl NativeEvidenceRole {
    fn into_domain(self) -> Result<EvidenceRole, EncoderTaskAdapterError> {
        Ok(match self {
            Self::Training => EvidenceRole::Training,
            Self::DevelopmentHoldout => EvidenceRole::Development,
            Self::Calibration => EvidenceRole::Calibration,
            Self::SealedHoldout => EvidenceRole::SealedAcceptance,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskConfiguration {
    adapter_protocol: String,
    training_inputs: Vec<String>,
    suites: BTreeMap<String, SuiteConfiguration>,
}

impl TaskConfiguration {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        if self.adapter_protocol != ADAPTER_PROTOCOL_VERSION || self.training_inputs.is_empty() {
            return Err(adapter_error(
                "Nomos project task configuration does not match this adapter",
            ));
        }
        for input in &self.training_inputs {
            validate_relative(input)?;
        }
        if self.suites.is_empty() {
            return Err(adapter_error("Nomos project defines no evaluation suites"));
        }
        for (key, suite) in &self.suites {
            if key.trim() != key || key.is_empty() {
                return Err(adapter_error("Nomos evaluation suite key is not canonical"));
            }
            validate_relative(&suite.path)?;
            if !suite
                .fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            {
                return Err(adapter_error(
                    "Nomos evaluation suite fingerprint is not canonical",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteConfiguration {
    path: String,
    role: EvidenceRole,
    fingerprint: String,
}

#[derive(Debug)]
struct NativeTrainingParameters {
    loss: String,
    epochs: f64,
    batch_size: u64,
    mining_batch_size: u64,
    learning_rate: f64,
    margin: f64,
    query_strategy: String,
    positive_strategy: String,
    seed: u64,
    device: String,
}

impl NativeTrainingParameters {
    fn parse(values: &BTreeMap<String, ParameterValue>) -> Result<Self, EncoderTaskAdapterError> {
        let allowed = [
            "loss",
            "epochs",
            "batch_size",
            "mining_batch_size",
            "learning_rate",
            "margin",
            "query_strategy",
            "positive_strategy",
            "seed",
            "device",
        ];
        if values.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(adapter_error(
                "Nomos candidate contains an unknown parameter",
            ));
        }
        let result = Self {
            loss: text_parameter(values, "loss", "triplet")?,
            epochs: number_parameter(values, "epochs", 1.0)?,
            batch_size: integer_parameter(values, "batch_size", 64)?,
            mining_batch_size: integer_parameter(values, "mining_batch_size", 256)?,
            learning_rate: number_parameter(values, "learning_rate", 0.000003)?,
            margin: number_parameter(values, "margin", 0.1)?,
            query_strategy: text_parameter(values, "query_strategy", "full")?,
            positive_strategy: text_parameter(values, "positive_strategy", "best")?,
            seed: integer_parameter(values, "seed", 20_260_902)?,
            device: text_parameter(values, "device", "cuda")?,
        };
        if !["triplet", "mnrl", "cached-mnrl"].contains(&result.loss.as_str())
            || !(0.0 < result.epochs && result.epochs <= 10.0)
            || !(1..=4_096).contains(&result.batch_size)
            || !(1..=100_000).contains(&result.mining_batch_size)
            || !(0.0 < result.learning_rate && result.learning_rate <= 0.001)
            || !(0.0 < result.margin && result.margin <= 1.0)
            || !["full", "question"].contains(&result.query_strategy.as_str())
            || !["all", "best"].contains(&result.positive_strategy.as_str())
            || !["cpu", "cuda"].contains(&result.device.as_str())
        {
            return Err(adapter_error(
                "Nomos candidate parameters are outside the safe adapter envelope",
            ));
        }
        let triplet_only = [
            "mining_batch_size",
            "margin",
            "query_strategy",
            "positive_strategy",
        ];
        if result.loss != "triplet"
            && values
                .keys()
                .any(|key| triplet_only.contains(&key.as_str()))
        {
            return Err(adapter_error(
                "Nomos listwise candidates cannot carry ignored triplet parameters",
            ));
        }
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct NativeEvaluation {
    inputs: BTreeMap<String, NativeInputEvaluation>,
}

#[derive(Debug, Deserialize)]
struct NativeInputEvaluation {
    metrics: NativeRankingMetrics,
}

#[derive(Debug, Deserialize)]
struct NativeRankingMetrics {
    states: u64,
    recall_at_1: f64,
    recall_at_2: f64,
    recall_at_3: f64,
    mrr: f64,
    mean_positive_margin: f64,
}

fn text_parameter(
    values: &BTreeMap<String, ParameterValue>,
    key: &str,
    default: &str,
) -> Result<String, EncoderTaskAdapterError> {
    match values.get(key) {
        None => Ok(default.into()),
        Some(ParameterValue::Text(value)) => Ok(value.clone()),
        _ => Err(adapter_error(format!("Nomos parameter {key} must be text"))),
    }
}

fn number_parameter(
    values: &BTreeMap<String, ParameterValue>,
    key: &str,
    default: f64,
) -> Result<f64, EncoderTaskAdapterError> {
    match values.get(key) {
        None => Ok(default),
        Some(ParameterValue::Number(value)) => Ok(*value),
        Some(ParameterValue::Integer(value)) => Ok(*value as f64),
        _ => Err(adapter_error(format!(
            "Nomos parameter {key} must be numeric"
        ))),
    }
}

fn integer_parameter(
    values: &BTreeMap<String, ParameterValue>,
    key: &str,
    default: u64,
) -> Result<u64, EncoderTaskAdapterError> {
    match values.get(key) {
        None => Ok(default),
        Some(ParameterValue::Integer(value)) => u64::try_from(*value)
            .map_err(|_| adapter_error(format!("Nomos parameter {key} must be non-negative"))),
        _ => Err(adapter_error(format!(
            "Nomos parameter {key} must be an integer"
        ))),
    }
}

fn validate_relative(value: &str) -> Result<(), EncoderTaskAdapterError> {
    let path = Path::new(value);
    if value.trim() != value
        || value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(adapter_error(
            "Nomos artifact path must remain relative to the isolated workspace",
        ));
    }
    Ok(())
}

fn verify_file(
    path: &Path,
    bytes: u64,
    expected_hash: &str,
) -> Result<(), EncoderTaskAdapterError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| adapter_error(format!("could not inspect Nomos file: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != bytes {
        return Err(adapter_error(
            "Nomos file size/type does not match its immutable identity",
        ));
    }
    if sha256_file(path)? != expected_hash {
        return Err(adapter_error(
            "Nomos file hash does not match its immutable identity",
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, EncoderTaskAdapterError> {
    let file = fs::File::open(path)
        .map_err(|error| adapter_error(format!("could not hash Nomos file: {error}")))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| adapter_error(format!("could not hash Nomos file: {error}")))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn tree_identity(path: &Path) -> Result<(u64, String), EncoderTaskAdapterError> {
    if !path.is_dir() {
        return Err(adapter_error("Nomos model artifact must be a directory"));
    }
    let mut files = collect_files(path)?;
    files.sort_by_cached_key(|file| {
        file.strip_prefix(path)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/")
    });
    let mut total = 0_u64;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let metadata = file.symlink_metadata().map_err(|error| {
            adapter_error(format!("could not inspect Nomos model file: {error}"))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(adapter_error(
                "Nomos model tree cannot contain links or non-files",
            ));
        }
        total = total
            .checked_add(metadata.len())
            .ok_or_else(|| adapter_error("Nomos model byte count overflowed"))?;
        let relative = file
            .strip_prefix(path)
            .map_err(adapter_error)?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(format!(
            "{relative}\t{}\t{}",
            metadata.len(),
            sha256_file(&file)?
        ));
    }
    if total == 0 {
        return Err(adapter_error("Nomos model tree is empty"));
    }
    let identity = entries.join("\n");
    Ok((total, format!("{:x}", Sha256::digest(identity.as_bytes()))))
}

fn collect_files(path: &Path) -> Result<Vec<PathBuf>, EncoderTaskAdapterError> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| adapter_error(format!("could not read Nomos model tree: {error}")))?
    {
        let entry = entry.map_err(adapter_error)?;
        let file_type = entry.file_type().map_err(adapter_error)?;
        if file_type.is_symlink() {
            return Err(adapter_error("Nomos model tree cannot contain symlinks"));
        }
        if file_type.is_dir() {
            result.extend(collect_files(&entry.path())?);
        } else if file_type.is_file() {
            result.push(entry.path());
        } else {
            return Err(adapter_error(
                "Nomos model tree contains an unsupported entry",
            ));
        }
    }
    Ok(result)
}

fn raw_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn prefixed(value: &str) -> String {
    format!("sha256:{value}")
}

fn bounded_text(bytes: &[u8], maximum: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(maximum)]).into_owned()
}

fn adapter_error(error: impl std::fmt::Display) -> EncoderTaskAdapterError {
    EncoderTaskAdapterError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use encoder_experiment_core::domain::ParameterValue;

    use super::*;

    #[test]
    fn candidate_parameter_envelope_is_strict() {
        let valid = NativeTrainingParameters::parse(&BTreeMap::from([
            ("loss".into(), ParameterValue::Text("triplet".into())),
            ("learning_rate".into(), ParameterValue::Number(0.000003)),
            ("device".into(), ParameterValue::Text("cuda".into())),
        ]))
        .unwrap();
        assert_eq!(valid.loss, "triplet");

        assert!(
            NativeTrainingParameters::parse(&BTreeMap::from([(
                "shell_command".into(),
                ParameterValue::Text("anything".into()),
            )]))
            .is_err()
        );
    }

    #[test]
    fn paths_cannot_escape_the_isolated_workspace() {
        assert!(validate_relative("data/generated/train.jsonl").is_ok());
        assert!(validate_relative("../fitz-tool/data.jsonl").is_err());
        assert!(validate_relative("C:\\Users\\source.jsonl").is_err());
    }
}
