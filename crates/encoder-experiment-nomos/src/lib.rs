//! Compiled adapter for the isolated Nomos retrieval-ranking production pilot.

mod agent_training;
mod benchmark;
mod dataset_landscape;
mod development_evidence;
mod evaluation;
mod generated_training;
mod managed_training;
mod native_assessment;
mod native_inventory;
mod progress;
mod repair_delta;
mod repair_evidence;
mod repair_outcome;
mod training_clearance;
pub use agent_training::NomosFineTuneSettings;
mod training_accounting;
mod training_data;
pub use benchmark::NomosBenchmarkPlan;
pub use dataset_landscape::{
    NomosDatasetInvestigation, NomosDatasetInvestigationPage, NomosDatasetLandscape,
};
pub use development_evidence::{
    NomosAgentPopulation, NomosDevelopmentCluster, NomosDevelopmentEvidence,
};
pub use generated_training::NomosGenerationTemplates;
mod generation_preview;
pub use generation_preview::{NomosGenerationPreviewRow, project_generation_preview};
mod development_comparison;
pub use development_comparison::NomosDevelopmentComparison;
pub use managed_training::{NomosTrainingDataset, NomosTrainingDatasetWriter};
pub use native_assessment::{
    NOMOS_LABEL_POLICY_VERSION, NomosNativeAssessmentBinding,
    target_brief as nomos_native_target_brief,
};
pub use native_inventory::{NomosNativeInventory, NomosNativeInventoryMember};
pub use progress::{
    NativePhase, NativeProgress, ProgressObserver, TrainingMetrics, with_file_progress,
    with_stop_probe,
};
pub use repair_evidence::{NomosRepairEvidence, project_repair_evidence};
pub use repair_outcome::{NomosRepairMetricPoint, project_repair_metric_points};
pub use training_clearance::NomosTrainingClearance;
pub use training_data::{VerifiedTrainingData, VerifiedTrainingInput};

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
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
use encoder_repair_core::{
    collection::{CollectedDevelopmentObservations, DevelopmentObservationRequest},
    observation::DevelopmentObservation,
    ports::{
        BoxFuture as RepairBoxFuture, DevelopmentObservationBackend,
        DevelopmentObservationBackendError,
    },
    training::{
        REPAIR_BASE_ROWS_PARAMETER, REPAIR_COMBINED_MEMBERSHIP_PARAMETER,
        REPAIR_DELTA_BYTES_PARAMETER, REPAIR_DELTA_FINGERPRINT_PARAMETER,
        REPAIR_DELTA_KEY_PARAMETER, REPAIR_DELTA_ROWS_PARAMETER,
        REPAIR_INPUTS_FINGERPRINT_PARAMETER, REPAIR_SELECTION_FINGERPRINT_PARAMETER,
        REPAIR_SELECTION_ID_PARAMETER, REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER,
        REPAIR_SNAPSHOT_ID_PARAMETER, REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER,
        REPAIR_TOTAL_ROWS_PARAMETER,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;
use workflow_core::benchmark_generation::{
    BenchmarkFreshnessAuthority, BenchmarkGeneration, DevelopmentSuiteAuthority,
    ExternalBenchmarkAuthority,
};

const ADAPTER_NAME: &str = "nomos";
const ADAPTER_PROTOCOL_VERSION: &str = "nomos-ranking-v3";
const RETRIEVAL_EVALUATOR_VERSION: &str = "nomos-dense-router-evaluation-v1";
const AGENT_EVALUATOR_VERSION: &str = "nomos-real-agent-sessions-v1";
const EXPERIMENT_MANIFEST_NAME: &str = "encoder-gym-experiment.json";
const TREE_HASH_ALGORITHM: &str = "sha256-ordinal-path-size-content-sha256-v1";
const DEVELOPMENT_OBSERVER_NAME: &str = "nomos-development-observer";
const DEVELOPMENT_OBSERVER_PROTOCOL: &str = "nomos-development-observations-v1";
const DEVELOPMENT_OBSERVER_SCHEMA_VERSION: u32 = 1;
const DENSE_TEXT_VERSION: &str = "dense-text.v3";
const REPAIR_TRAINING_RECEIPT_SCHEMA: &str = "encoder-gym-nomos-repair-training-receipt.v1";
const REPAIR_TRAINING_RECEIPT_NAME: &str = "encoder_gym_repair_training_receipt.json";
const REPAIR_DELTA_ADAPTER_NAME: &str = "nomos-native-repair-delta";
const REPAIR_DELTA_ADAPTER_PROTOCOL: &str = "nomos-native-repair-delta-v1";
const NATIVE_SPAWN_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(100), Duration::from_millis(250)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativePackage {
    Nomos,
    /// Existing isolated runtimes may predate the source-package rename. They
    /// remain immutable, valid execution inputs for already-authorized runs.
    PreRenameFitzTool,
}

impl NativePackage {
    fn detect(root: &Path) -> Result<Self, EncoderTaskAdapterError> {
        if root.join("nomos").is_dir() {
            Ok(Self::Nomos)
        } else if root.join("fitz_tool").is_dir() {
            Ok(Self::PreRenameFitzTool)
        } else {
            Err(adapter_error("Nomos native package is missing: nomos"))
        }
    }

    const fn module(self) -> &'static str {
        match self {
            Self::Nomos => "nomos",
            Self::PreRenameFitzTool => "fitz_tool",
        }
    }
}

fn development_observer_sources(package: NativePackage) -> Vec<String> {
    vec![
        "tools/collect_encoder_gym_development_observations.py".into(),
        "tools/evaluate_dense_router.py".into(),
        format!("{}/dense_router.py", package.module()),
        format!("{}/embedding_backend.py", package.module()),
        format!("{}/onnx_encoder.py", package.module()),
    ]
}

fn repair_delta_sources(package: NativePackage) -> Vec<String> {
    vec![
        "tools/generate_encoder_gym_repair_delta_v1.py".into(),
        format!("{}/encoder_gym_repair_delta_v1.py", package.module()),
        format!("{}/dense_router.py", package.module()),
        format!("{}/generic_contracts.py", package.module()),
        format!("{}/router_v2.py", package.module()),
        format!("{}/scaling_matrix_v1.py", package.module()),
    ]
}

#[derive(Debug, Clone)]
pub struct NomosBackend {
    root: PathBuf,
    python: PathBuf,
    native_package: NativePackage,
    manifest: NomosExperimentManifest,
    baseline_override: Option<ModelArtifactIdentity>,
    training_override: Option<NomosTrainingDataset>,
    identity: BackendIdentity,
    observer_identity: BackendIdentity,
    repair_delta_identity: BackendIdentity,
    progress: Option<Arc<dyn ProgressObserver>>,
    training_accounting:
        Option<Arc<dyn encoder_experiment_core::training_budget::TrainingAccounting>>,
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
        let native_package = NativePackage::detect(&root)?;
        let observer_source_revision = git_output_at(&root, ["rev-parse", "HEAD"])?;
        let mut observer_sources = BTreeMap::new();
        for relative in development_observer_sources(native_package) {
            let path = root.join(&relative);
            if !path.is_file() {
                return Err(adapter_error(format!(
                    "Nomos development observer source is missing: {relative}"
                )));
            }
            observer_sources.insert(relative, prefixed(&sha256_file(&path)?));
        }
        let observer_configuration_fingerprint = artifact_core::fingerprint(&json!({
            "observer": DEVELOPMENT_OBSERVER_NAME,
            "protocol": DEVELOPMENT_OBSERVER_PROTOCOL,
            "source_revision": observer_source_revision,
            "sources": observer_sources,
            "text_version": DENSE_TEXT_VERSION,
        }))
        .map_err(adapter_error)?;
        let observer_identity = BackendIdentity::new(
            DEVELOPMENT_OBSERVER_NAME,
            DEVELOPMENT_OBSERVER_PROTOCOL,
            observer_configuration_fingerprint,
        )
        .map_err(adapter_error)?;
        let mut repair_delta_source_hashes = BTreeMap::new();
        for relative in repair_delta_sources(native_package) {
            let path = root.join(&relative);
            if !path.is_file() {
                return Err(adapter_error(format!(
                    "Nomos repair-delta adapter source is missing: {relative}"
                )));
            }
            repair_delta_source_hashes.insert(relative, prefixed(&sha256_file(&path)?));
        }
        let repair_delta_configuration_fingerprint = artifact_core::fingerprint(&json!({
            "adapter": REPAIR_DELTA_ADAPTER_NAME,
            "protocol": REPAIR_DELTA_ADAPTER_PROTOCOL,
            "source_revision": observer_source_revision,
            "sources": repair_delta_source_hashes,
        }))
        .map_err(adapter_error)?;
        let repair_delta_identity = BackendIdentity::new(
            REPAIR_DELTA_ADAPTER_NAME,
            REPAIR_DELTA_ADAPTER_PROTOCOL,
            repair_delta_configuration_fingerprint,
        )
        .map_err(adapter_error)?;
        Ok(Self {
            root,
            python: python.into(),
            native_package,
            manifest,
            baseline_override: None,
            training_override: None,
            identity,
            observer_identity,
            repair_delta_identity,
            progress: None,
            training_accounting: None,
        })
    }

    /// Bind the immutable task runtime to a promoted checkpoint that already
    /// exists inside its content-addressed candidate outputs. Runtime source
    /// code and the checked-in experiment manifest remain unchanged.
    pub fn with_baseline_model(
        mut self,
        model: ModelArtifactIdentity,
    ) -> Result<Self, EncoderTaskAdapterError> {
        model.validate().map_err(adapter_error)?;
        if model.format != "sentence-transformers" {
            return Err(adapter_error(
                "Nomos promoted baselines must use sentence-transformers format",
            ));
        }
        if model.key == self.manifest.baseline.pytorch_path {
            return Ok(self);
        }
        self.model_path(&model)?;
        self.baseline_override = Some(model);
        Ok(self)
    }

    /// Bind execution to a project-owned dataset already admitted and
    /// materialized by this adapter. The checked-in experiment stays intact.
    pub fn with_training_dataset(
        mut self,
        dataset: NomosTrainingDataset,
    ) -> Result<Self, EncoderTaskAdapterError> {
        dataset.verify_in(&self.root)?;
        self.training_override = Some(dataset);
        Ok(self)
    }

    /// Compile the adapter's conservative first candidate. The application
    /// chooses only the identity and finite time ceiling; native knobs remain
    /// explicit and owned by this task adapter.
    pub async fn initial_training_candidate(
        &self,
        id: Uuid,
        project: &ExternalProjectSnapshot,
        maximum_training_seconds: u64,
    ) -> Result<TrainingCandidate, EncoderTaskAdapterError> {
        self.verify_current_snapshot(project.clone()).await?;
        Self::initial_training_candidate_definition(id, project, maximum_training_seconds)
    }

    /// Build the adapter-owned candidate definition without touching native
    /// files. Execution still re-verifies the complete current project before
    /// any trainer receives it.
    pub fn initial_training_candidate_definition(
        id: Uuid,
        project: &ExternalProjectSnapshot,
        maximum_training_seconds: u64,
    ) -> Result<TrainingCandidate, EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        let parameters = BTreeMap::from([
            ("strategy".into(), ParameterValue::Text("fine_tune".into())),
            ("loss".into(), ParameterValue::Text("triplet".into())),
            ("epochs".into(), ParameterValue::Number(1.0)),
            ("batch_size".into(), ParameterValue::Integer(64)),
            ("mining_batch_size".into(), ParameterValue::Integer(256)),
            ("learning_rate".into(), ParameterValue::Number(0.000_003)),
            ("margin".into(), ParameterValue::Number(0.1)),
            ("query_strategy".into(), ParameterValue::Text("full".into())),
            (
                "positive_strategy".into(),
                ParameterValue::Text("best".into()),
            ),
            ("seed".into(), ParameterValue::Integer(20_260_902)),
            ("device".into(), ParameterValue::Text("cuda".into())),
        ]);
        NativeCandidateStrategy::parse(&parameters)?;
        TrainingCandidate::create_identified(id, project, 1, maximum_training_seconds, parameters)
            .map_err(adapter_error)
    }

    pub fn project_snapshot(&self) -> Result<ExternalProjectSnapshot, EncoderTaskAdapterError> {
        self.verify_no_remote()?;
        self.verify_clean_worktree()?;
        let revision = self.git_output(["rev-parse", "HEAD"])?;
        let runtime_source_fingerprint = self.runtime_source_fingerprint(&revision)?;

        let mut inputs = Vec::with_capacity(self.manifest.datasets.len());
        for dataset in &self.manifest.datasets {
            if self.training_override.is_some() && dataset.role == NativeEvidenceRole::Training {
                continue;
            }
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
        if let Some(dataset) = &self.training_override {
            inputs.push(dataset.artifact.clone());
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
        let manifest_baseline = ModelArtifactIdentity::new(
            self.manifest.baseline.pytorch_path.clone(),
            "sentence-transformers",
            baseline_bytes,
            prefixed(&baseline_digest),
        )
        .map_err(adapter_error)?;
        let baseline_model = self
            .baseline_override
            .as_ref()
            .unwrap_or(&manifest_baseline)
            .clone();
        if self.baseline_override.is_some() {
            self.model_path(&baseline_model)?;
        }
        let source_fingerprint = bound_source_fingerprint(
            runtime_source_fingerprint,
            self.baseline_override.as_ref(),
            self.training_override.as_ref(),
        )?;
        let mut reference_models = BTreeMap::new();
        for reference in &self.manifest.reference_models {
            let (bytes, digest) = self.verify_tree_artifact(
                &reference.path,
                reference.bytes,
                &reference.tree_sha256,
                "reference model",
            )?;
            reference_models.insert(
                reference.key.clone(),
                json!({
                    "path": reference.path,
                    "format": reference.format,
                    "bytes": bytes,
                    "fingerprint": prefixed(&digest),
                    "provenance": reference.provenance,
                }),
            );
        }
        let (chat_model_bytes, chat_model_digest) = self.verify_tree_artifact(
            &self.manifest.agent_evaluation.chat_model_path,
            self.manifest.agent_evaluation.chat_model_bytes,
            &self.manifest.agent_evaluation.chat_model_tree_sha256,
            "agent chat model",
        )?;
        if let Some(authority) = &self.manifest.benchmark_authority {
            let path = self.resolve_existing(&authority.path)?;
            verify_file(&path, authority.bytes, &authority.sha256)?;
        }
        let training_inputs = self.training_override.as_ref().map_or_else(
            || {
                self.manifest
                    .datasets
                    .iter()
                    .filter(|value| value.role == NativeEvidenceRole::Training)
                    .map(|value| value.path.clone())
                    .collect::<Vec<_>>()
            },
            |dataset| vec![dataset.artifact.key.clone()],
        );
        let mut suites = serde_json::Map::new();
        for suite in self.manifest.evaluation_suites()? {
            let retrieval_fingerprint = artifact_core::fingerprint(&json!({
                "evaluator": RETRIEVAL_EVALUATOR_VERSION,
                "dataset": {
                    "path": suite.dataset.path,
                    "fingerprint": prefixed(&suite.dataset.sha256),
                },
            }))
            .map_err(adapter_error)?;
            let agent_fingerprint = artifact_core::fingerprint(&json!({
                "evaluator": AGENT_EVALUATOR_VERSION,
                "agent_evaluation": {
                    "chat_model_fingerprint": prefixed(&chat_model_digest),
                    "selector_strategy": self.manifest.agent_evaluation.selector_strategy,
                    "candidate_strategy": self.manifest.agent_evaluation.candidate_strategy,
                    "nomos_top_k": self.manifest.agent_evaluation.nomos_top_k,
                    "max_attempts": self.manifest.agent_evaluation.max_attempts,
                    "suite": suite.agent,
                },
            }))
            .map_err(adapter_error)?;
            let fingerprint = artifact_core::fingerprint(&json!({
                "retrieval_fingerprint": retrieval_fingerprint,
                "agent_fingerprint": agent_fingerprint,
            }))
            .map_err(adapter_error)?;
            suites.insert(
                suite.key,
                json!({
                    "path": suite.dataset.path,
                    "role": suite.role,
                    "fingerprint": fingerprint,
                    "retrieval_fingerprint": retrieval_fingerprint,
                    "agent_fingerprint": agent_fingerprint,
                }),
            );
        }
        let mut task_configuration = json!({
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
            "reference_models": reference_models,
            "agent_evaluation": {
                "backend": self.manifest.agent_evaluation.backend,
                "chat_model": {
                    "path": self.manifest.agent_evaluation.chat_model_path,
                    "format": self.manifest.agent_evaluation.chat_model_format,
                    "bytes": chat_model_bytes,
                    "fingerprint": prefixed(&chat_model_digest),
                    "source": self.manifest.agent_evaluation.source,
                },
                "selector_strategy": self.manifest.agent_evaluation.selector_strategy,
                "candidate_strategy": self.manifest.agent_evaluation.candidate_strategy,
                "nomos_top_k": self.manifest.agent_evaluation.nomos_top_k,
                "max_attempts": self.manifest.agent_evaluation.max_attempts,
                "development": self.manifest.agent_evaluation.development,
                "sealed": self.manifest.agent_evaluation.sealed,
            },
            "suites": suites,
        });
        if let Some(authority) = &self.manifest.benchmark_authority {
            task_configuration
                .as_object_mut()
                .expect("task configuration is an object")
                .insert(
                    "benchmark_authority".into(),
                    json!({
                        "path": authority.path,
                        "bytes": authority.bytes,
                        "fingerprint": prefixed(&authority.sha256),
                    }),
                );
        }
        if let Some(dataset) = &self.training_override {
            task_configuration
                .as_object_mut()
                .expect("task configuration is an object")
                .insert("managed_training_dataset".into(), json!(dataset));
        }
        ExternalProjectSnapshot::create(
            self.manifest.experiment.clone(),
            EncoderTaskKind::RetrievalRanking,
            revision,
            source_fingerprint,
            self.identity.clone(),
            inputs,
            baseline_model,
            task_configuration,
            Utc::now(),
        )
        .map_err(adapter_error)
    }

    /// Reverify a persisted scientific project against the current isolated runtime.
    /// Snapshot IDs and creation times remain store-owned; currentness is content-based.
    pub async fn verify_current_snapshot(
        &self,
        project: ExternalProjectSnapshot,
    ) -> Result<AdapterInspection, EncoderTaskAdapterError> {
        let inspection = self.inspect(project.clone()).await?;
        if !self.project_matches_current(&project)? {
            return Err(adapter_error(
                "The persisted Nomos project no longer matches the current isolated runtime",
            ));
        }
        Ok(inspection)
    }

    /// Compile the current schema-v4, row-free Nomos authority evidence into
    /// the provider-neutral renewable benchmark contract.
    pub fn benchmark_generation(
        &self,
        predecessor: Option<&BenchmarkGeneration>,
    ) -> Result<BenchmarkGeneration, EncoderTaskAdapterError> {
        let pin =
            self.manifest.benchmark_authority.as_ref().ok_or_else(|| {
                adapter_error("Nomos manifest has no benchmark-generation authority")
            })?;
        let path = self.resolve_existing(&pin.path)?;
        verify_file(&path, pin.bytes, &pin.sha256)?;
        let raw = fs::read(&path).map_err(adapter_error)?;
        let evidence: NomosGenerationAuthorityFile =
            serde_json::from_slice(&raw).map_err(|error| {
                adapter_error(format!("Nomos benchmark authority is invalid: {error}"))
            })?;
        evidence.validate()?;
        let project = self.project_snapshot()?;
        let configuration = Self::task_configuration(&project)?;
        configuration.validate()?;
        let development = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::Development)
            .collect::<BTreeMap<_, _>>();
        let sealed = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::SealedAcceptance)
            .collect::<Vec<_>>();
        if sealed.len() != 1
            || development.len() != evidence.development_suites.len()
            || development
                .keys()
                .map(|key| key.as_str())
                .collect::<Vec<_>>()
                != evidence
                    .development_suites
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
        {
            return Err(adapter_error(
                "Nomos authority suite set does not match the current project snapshot",
            ));
        }
        let (sealed_key, sealed_suite) = sealed[0];
        let mut authorities = Vec::with_capacity(development.len());
        for (suite_key, suite) in development {
            let authority = evidence
                .development_suites
                .get(suite_key)
                .expect("suite sets were checked");
            if authority.development_suite_fingerprint != suite.fingerprint
                || authority.sealed_suite_fingerprint != sealed_suite.fingerprint
                || evidence.sealed_suite_key != *sealed_key
            {
                return Err(adapter_error(
                    "Nomos authority fingerprints do not match the current evaluation suites",
                ));
            }
            authorities.push(
                DevelopmentSuiteAuthority::from_external_evidence(
                    suite_key.clone(),
                    authority.clone(),
                )
                .map_err(adapter_error)?,
            );
        }
        if evidence.freshness.sealed_suite_id
            != authorities[0]
                .bundle
                .sealed_suite_id
                .expect("external authority requires sealed suite")
            || evidence.freshness.sealed_suite_fingerprint != sealed_suite.fingerprint
        {
            return Err(adapter_error(
                "Nomos freshness evidence does not bind the successor sealed suite",
            ));
        }
        BenchmarkGeneration::create(
            predecessor,
            authorities,
            evidence.freshness,
            evidence.created_at,
        )
        .map_err(adapter_error)
    }

    /// Convert the isolated cohort's row-free audit into the strict external
    /// authority envelope consumed by renewable benchmark generations.
    ///
    /// This does not inspect benchmark rows or execute a model. The audit is
    /// accepted only when it proves a frozen population, zero overlap under
    /// every required identity, sufficient per-slice support, no prior model
    /// evaluation, and an independent reviewer.
    #[allow(clippy::too_many_arguments)]
    pub fn build_benchmark_authority(
        &self,
        qualification_report: impl AsRef<Path>,
        acquired_by: impl Into<String>,
        reviewed_by: impl Into<String>,
        rationale: impl Into<String>,
        reviewed_at: DateTime<Utc>,
        valid_until: DateTime<Utc>,
    ) -> Result<Vec<u8>, EncoderTaskAdapterError> {
        let report_path = qualification_report.as_ref();
        let report_path = if report_path.is_absolute() {
            let canonical = report_path.canonicalize().map_err(adapter_error)?;
            if !canonical.starts_with(&self.root) {
                return Err(adapter_error(
                    "Nomos qualification report must be inside the isolated workspace",
                ));
            }
            canonical
        } else {
            let report_path = report_path.to_str().ok_or_else(|| {
                adapter_error("Nomos qualification report path is not valid UTF-8")
            })?;
            self.resolve_existing(report_path)?
        };
        let raw = fs::read(&report_path).map_err(adapter_error)?;
        let report: NomosExternalQualificationReport =
            serde_json::from_slice(&raw).map_err(|error| {
                adapter_error(format!("Nomos qualification report is invalid: {error}"))
            })?;
        report.validate()?;

        let acquired_by = acquired_by.into();
        let reviewed_by = reviewed_by.into();
        let rationale = rationale.into();
        if acquired_by.trim().is_empty()
            || acquired_by.trim() != acquired_by
            || reviewed_by.trim().is_empty()
            || reviewed_by.trim() != reviewed_by
            || acquired_by == reviewed_by
            || rationale.trim().is_empty()
            || rationale.trim() != rationale
            || reviewed_at >= valid_until
        {
            return Err(adapter_error(
                "Nomos benchmark authority review fields are invalid or not independent",
            ));
        }

        let project = self.project_snapshot()?;
        let configuration = Self::task_configuration(&project)?;
        configuration.validate()?;
        let development = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::Development)
            .collect::<BTreeMap<_, _>>();
        let sealed = configuration
            .suites
            .iter()
            .filter(|(_, suite)| suite.role == EvidenceRole::SealedAcceptance)
            .collect::<Vec<_>>();
        if development.is_empty() || sealed.len() != 1 {
            return Err(adapter_error(
                "Nomos authority requires development suites and exactly one sealed suite",
            ));
        }
        let (sealed_suite_key, sealed_suite) = sealed[0];
        let sealed_suite_id = Uuid::new_v4();
        let population_fingerprint = prefixed(&report.population_fingerprint);
        let source_manifest_fingerprint = prefixed(&report.source_manifest_fingerprint);
        let acquisition_spec_fingerprint = artifact_core::fingerprint(&json!({
            "protocol": report.protocol,
            "policy": report.policy,
            "source_manifest_fingerprint": source_manifest_fingerprint,
        }))
        .map_err(adapter_error)?;

        let mut development_suites = BTreeMap::new();
        for (suite_key, suite) in development {
            let mut contamination =
                workflow_core::benchmark_generation::ExternalContaminationEvidence {
                    id: Uuid::new_v4(),
                    population_fingerprint: population_fingerprint.clone(),
                    policy: report.policy.clone(),
                    overlap_counts: report.overlap_counts.clone(),
                    checked_at: reviewed_at,
                    fingerprint: String::new(),
                };
            contamination.fingerprint = contamination
                .reproduce_fingerprint()
                .map_err(adapter_error)?;
            let mut qualification =
                workflow_core::benchmark_generation::ExternalQualificationEvidence {
                    id: Uuid::new_v4(),
                    population_fingerprint: population_fingerprint.clone(),
                    minimum_overall_support: report.minimum_overall_support,
                    observed_overall_support: report.observed_overall_support,
                    minimum_slice_support: report.minimum_slice_support.clone(),
                    observed_slice_support: report.observed_slice_support.clone(),
                    ready: report.ready,
                    assessed_at: reviewed_at,
                    fingerprint: String::new(),
                };
            qualification.fingerprint = qualification
                .reproduce_fingerprint()
                .map_err(adapter_error)?;
            let mut approval = workflow_core::benchmark_generation::ExternalQualificationApproval {
                id: Uuid::new_v4(),
                qualification_id: qualification.id,
                qualification_fingerprint: qualification.fingerprint.clone(),
                approved: true,
                reviewed_by: reviewed_by.clone(),
                independent: true,
                rationale: rationale.clone(),
                reviewed_at,
                fingerprint: String::new(),
            };
            approval.fingerprint = approval.reproduce_fingerprint().map_err(adapter_error)?;
            let mut authority = ExternalBenchmarkAuthority {
                schema_version:
                    workflow_core::benchmark_generation::EXTERNAL_BENCHMARK_AUTHORITY_SCHEMA_VERSION,
                id: Uuid::new_v4(),
                bundle_id: Uuid::new_v4(),
                development_suite_id: Uuid::new_v4(),
                development_suite_fingerprint: suite.fingerprint.clone(),
                sealed_suite_id,
                sealed_suite_fingerprint: sealed_suite.fingerprint.clone(),
                acquisition_spec_fingerprint: acquisition_spec_fingerprint.clone(),
                source_manifest_fingerprint: source_manifest_fingerprint.clone(),
                population_fingerprint: population_fingerprint.clone(),
                acquired_by: acquired_by.clone(),
                contamination,
                qualification,
                approval,
                created_at: reviewed_at,
                fingerprint: String::new(),
            };
            authority.fingerprint = authority.reproduce_fingerprint().map_err(adapter_error)?;
            authority.validate_integrity().map_err(adapter_error)?;
            development_suites.insert(suite_key.clone(), authority);
        }

        let freshness = BenchmarkFreshnessAuthority::create(
            sealed_suite_id,
            sealed_suite.fingerprint.clone(),
            population_fingerprint,
            source_manifest_fingerprint,
            reviewed_at,
            reviewed_at,
            valid_until,
            reviewed_by,
            rationale,
        )
        .map_err(adapter_error)?;
        let envelope = NomosGenerationAuthorityFile {
            schema_version: 1,
            sealed_suite_key: sealed_suite_key.clone(),
            development_suites,
            freshness,
            created_at: reviewed_at,
        };
        envelope.validate()?;
        serde_json::to_vec_pretty(&envelope).map_err(adapter_error)
    }

    fn verify_no_remote(&self) -> Result<(), EncoderTaskAdapterError> {
        if !self.git_output(["remote"])?.is_empty() {
            return Err(adapter_error(
                "isolated Nomos experiment must not have a Git remote",
            ));
        }
        Ok(())
    }

    fn runtime_source_fingerprint(
        &self,
        revision: &str,
    ) -> Result<String, EncoderTaskAdapterError> {
        // Callers first verify a clean, isolated checkout. Use the committed
        // manifest consistently: checkout line endings are not a code change.
        artifact_core::fingerprint(&json!({
            "experiment_revision": revision,
            "manifest_sha256": git_blob_sha256_at(&self.root, EXPERIMENT_MANIFEST_NAME)?,
            "source_commit": self.manifest.source.commit,
        }))
        .map_err(adapter_error)
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
        git_output_at(&self.root, arguments)
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

    fn verify_tree_artifact(
        &self,
        relative: &str,
        expected_bytes: u64,
        expected_digest: &str,
        kind: &str,
    ) -> Result<(u64, String), EncoderTaskAdapterError> {
        let path = self.resolve_existing(relative)?;
        let (bytes, digest) = tree_identity(&path)?;
        if bytes != expected_bytes || digest != expected_digest {
            return Err(adapter_error(format!(
                "Nomos {kind} {relative} no longer matches its immutable manifest identity"
            )));
        }
        Ok((bytes, digest))
    }

    fn task_configuration(
        project: &ExternalProjectSnapshot,
    ) -> Result<TaskConfiguration, EncoderTaskAdapterError> {
        serde_json::from_value(project.task_configuration.clone())
            .map_err(|error| adapter_error(format!("Nomos task configuration is invalid: {error}")))
    }

    fn supports_identity(&self, identity: &BackendIdentity) -> bool {
        if identity.name != ADAPTER_NAME || identity.protocol_version != ADAPTER_PROTOCOL_VERSION {
            return false;
        }
        [3_u32, 4_u32].into_iter().any(|schema_version| {
            artifact_core::fingerprint(&json!({
                "adapter": ADAPTER_NAME,
                "protocol_version": ADAPTER_PROTOCOL_VERSION,
                "manifest_schema_version": schema_version,
                "tree_hash_algorithm": TREE_HASH_ALGORITHM,
            }))
            .is_ok_and(|fingerprint| fingerprint == identity.configuration_fingerprint)
        })
    }

    fn project_matches_current(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<bool, EncoderTaskAdapterError> {
        let current = self.project_snapshot()?;
        Ok(current.source_revision == project.source_revision
            && current.source_fingerprint == project.source_fingerprint
            && current.backend == project.backend
            && current.inputs == project.inputs
            && same_model_content(&current.baseline_model, &project.baseline_model)
            && current.task_configuration == project.task_configuration)
    }

    fn require_current_project(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<(), EncoderTaskAdapterError> {
        if !self.project_matches_current(project)? {
            return Err(adapter_error(
                "Nomos execution requires the exact current isolated project revision",
            ));
        }
        Ok(())
    }

    fn verify_pinned_project_artifacts(
        &self,
        project: &ExternalProjectSnapshot,
        configuration: &TaskConfiguration,
    ) -> Result<(), EncoderTaskAdapterError> {
        let revision_object = format!("{}^{{commit}}", project.source_revision);
        self.git_output(["cat-file", "-e", revision_object.as_str()])?;
        for artifact in &project.inputs {
            let path = self.resolve_existing(&artifact.key)?;
            verify_file(
                &path,
                artifact.bytes,
                artifact
                    .fingerprint
                    .strip_prefix("sha256:")
                    .ok_or_else(|| adapter_error("Nomos input fingerprint is malformed"))?,
            )?;
        }
        let project_training = project
            .inputs
            .iter()
            .filter(|artifact| artifact.role == EvidenceRole::Training)
            .collect::<Vec<_>>();
        if configuration
            .managed_training_dataset
            .as_ref()
            .is_some_and(|dataset| {
                project_training.as_slice() != [&dataset.artifact]
                    || self.training_override.as_ref() != Some(dataset)
            })
        {
            return Err(adapter_error(
                "Nomos managed training data does not match the bound project",
            ));
        }
        let baseline = self.resolve_existing(&project.baseline_model.key)?;
        let (bytes, digest) = tree_identity(&baseline)?;
        if bytes != project.baseline_model.bytes
            || prefixed(&digest) != project.baseline_model.fingerprint
        {
            return Err(adapter_error(
                "Nomos historical baseline no longer matches its immutable identity",
            ));
        }
        for reference in configuration.reference_models.values() {
            let path = self.resolve_existing(&reference.path)?;
            let (bytes, digest) = tree_identity(&path)?;
            if bytes != reference.bytes || prefixed(&digest) != reference.fingerprint {
                return Err(adapter_error(
                    "Nomos historical reference model no longer matches its immutable identity",
                ));
            }
        }
        let chat = &configuration.agent_evaluation.chat_model;
        let path = self.resolve_existing(&chat.path)?;
        let (bytes, digest) = tree_identity(&path)?;
        if bytes != chat.bytes || prefixed(&digest) != chat.fingerprint {
            return Err(adapter_error(
                "Nomos historical agent model no longer matches its immutable identity",
            ));
        }
        if let Some(authority) = &configuration.benchmark_authority {
            let path = self.resolve_existing(&authority.path)?;
            verify_file(
                &path,
                authority.bytes,
                authority
                    .fingerprint
                    .strip_prefix("sha256:")
                    .ok_or_else(|| adapter_error("Nomos authority fingerprint is malformed"))?,
            )?;
        }
        Ok(())
    }

    pub fn with_progress_observer(mut self, observer: Arc<dyn ProgressObserver>) -> Self {
        self.progress = Some(observer);
        self
    }

    fn observe(&self, phase: NativePhase) {
        if let Some(observer) = &self.progress {
            observer.observe(NativeProgress::phase(phase));
        }
    }

    fn observe_subject(&self, phase: NativePhase, subject: &str) {
        if let Some(observer) = &self.progress {
            let mut progress = NativeProgress::phase(phase);
            progress.subject = Some(subject.into());
            observer.observe(progress);
        }
    }

    fn observe_saved_training(&self, manifest: &Value) {
        if let (Some(observer), Some(progress)) =
            (&self.progress, progress::saved_training_metrics(manifest))
        {
            observer.observe(progress);
        }
    }

    async fn run_bounded(
        &self,
        arguments: &[String],
        maximum_seconds: u64,
    ) -> Result<(), EncoderTaskAdapterError> {
        self.run_with_deadline(arguments, Duration::from_secs(maximum_seconds))
            .await
            .map_err(|error| match error {
                EncoderTaskAdapterError::TimeLimitExceeded => {
                    adapter_error("Nomos process exceeded its finite time limit")
                }
                error => error,
            })
    }

    async fn run_with_deadline(
        &self,
        arguments: &[String],
        maximum_duration: Duration,
    ) -> Result<(), EncoderTaskAdapterError> {
        progress::check_stop()?;
        if maximum_duration.is_zero() {
            return Err(adapter_error(
                "Nomos process requires a positive time limit",
            ));
        }
        use tokio::io::AsyncReadExt;
        let training = arguments.get(1).is_some_and(|module| {
            matches!(
                module.as_str(),
                "tools.train_dense_triplet_router" | "tools.train_dense_router"
            )
        });
        let mut child = self.spawn_native_process(arguments).await?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let drain = async {
            let mut buffer = [0_u8; 4096];
            let mut line = Vec::new();
            let mut tail = Vec::new();
            let mut overflow = false;
            loop {
                let count = stderr.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > 2000 {
                    tail.drain(..tail.len() - 2000);
                }
                for &byte in &buffer[..count] {
                    if byte == b'\r' || byte == b'\n' {
                        if training && !overflow {
                            if let Some(value) =
                                progress::training_counter(&String::from_utf8_lossy(&line))
                            {
                                if let Some(observer) = &self.progress {
                                    observer.observe(value);
                                }
                            }
                        }
                        line.clear();
                        overflow = false;
                    } else if line.len() < 8192 {
                        line.push(byte);
                    } else {
                        overflow = true;
                    }
                }
            }
            Ok::<_, std::io::Error>(tail)
        };
        let work = tokio::time::timeout(maximum_duration, async {
            let mut sink = tokio::io::sink();
            tokio::try_join!(child.wait(), tokio::io::copy(&mut stdout, &mut sink), drain)
        });
        let result = tokio::select! {
            result = work => Some(result),
            () = progress::wait_for_stop() => None,
        };
        if result.as_ref().is_none_or(|result| result.is_err()) {
            // Await confirmed termination before the parent may acknowledge Stop
            // or release its lease for a replacement execution.
            progress::terminate_child(&mut child).await;
            return Err(if result.is_none() {
                adapter_error("Optimization stopped")
            } else {
                EncoderTaskAdapterError::TimeLimitExceeded
            });
        }
        let output = match result.expect("present result").expect("deadline handled") {
            Ok(output) => output,
            Err(error) => {
                progress::terminate_child(&mut child).await;
                return Err(adapter_error(format!("Nomos process I/O failed: {error}")));
            }
        };
        if !output.0.success() {
            let stderr = bounded_text(&output.2, 2_000);
            return Err(adapter_error(format!(
                "Nomos process failed with status {}: {stderr}",
                output.0
            )));
        }
        Ok(())
    }

    async fn spawn_native_process(
        &self,
        arguments: &[String],
    ) -> Result<tokio::process::Child, EncoderTaskAdapterError> {
        for (attempt, retry_delay) in NATIVE_SPAWN_RETRY_DELAYS
            .iter()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let mut command = Command::new(&self.python);
            command
                .args(arguments)
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            match command.spawn() {
                Ok(child) => return Ok(child),
                Err(error)
                    if retry_delay.is_some()
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::Interrupted
                        ) =>
                {
                    tokio::time::sleep(*retry_delay.expect("guarded retry delay")).await;
                }
                Err(error) => {
                    return Err(adapter_error(format!(
                        "could not start Nomos process after {} attempt(s): {error}; executable={} ({}); working directory={} ({})",
                        attempt + 1,
                        self.python.display(),
                        path_state(&self.python, false),
                        self.root.display(),
                        path_state(&self.root, true),
                    )));
                }
            }
        }
        unreachable!("native process launch loop always returns")
    }

    fn candidate_output(&self, candidate: &TrainingCandidate) -> PathBuf {
        self.root
            .join("artifacts")
            .join("encoder-gym-candidates")
            .join(candidate.id.to_string())
    }

    fn candidate_staging_output(&self, candidate: &TrainingCandidate) -> PathBuf {
        self.root
            .join("artifacts")
            .join("encoder-gym-candidates")
            .join(format!(".{}-staging", candidate.id))
    }

    fn candidate_staging_checkpoints(&self, candidate: &TrainingCandidate) -> PathBuf {
        self.root
            .join("artifacts")
            .join("encoder-gym-candidates")
            .join(format!("..{}-staging-checkpoints", candidate.id))
    }

    fn repair_training_output_root(&self) -> PathBuf {
        self.root.join("artifacts").join("encoder-gym-candidates")
    }

    fn clear_repair_training_scratch(
        &self,
        candidate: &TrainingCandidate,
    ) -> Result<(), EncoderTaskAdapterError> {
        remove_exact_scratch_directory(
            &self.candidate_staging_output(candidate),
            &self.repair_training_output_root(),
        )?;
        self.clear_repair_training_checkpoints(candidate)
    }

    fn clear_repair_training_checkpoints(
        &self,
        candidate: &TrainingCandidate,
    ) -> Result<(), EncoderTaskAdapterError> {
        remove_exact_scratch_directory(
            &self.candidate_staging_checkpoints(candidate),
            &self.repair_training_output_root(),
        )
    }

    fn finalize_repair_staging(
        &self,
        candidate: &TrainingCandidate,
        staging: &Path,
        output: &Path,
    ) -> Result<(), EncoderTaskAdapterError> {
        if staging != self.candidate_staging_output(candidate)
            || output != self.candidate_output(candidate)
            || staging.parent() != Some(self.repair_training_output_root().as_path())
            || output.parent() != Some(self.repair_training_output_root().as_path())
            || output.exists()
        {
            return Err(adapter_error(
                "Nomos repair training staging paths are not the exact candidate targets",
            ));
        }
        let metadata = staging.symlink_metadata().map_err(adapter_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(adapter_error(
                "Nomos repair training staging output is not a plain directory",
            ));
        }
        fs::rename(staging, output).map_err(|error| {
            adapter_error(format!(
                "could not atomically publish Nomos repair candidate: {error}"
            ))
        })?;
        self.clear_repair_training_checkpoints(candidate)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_or_validate_repair_training_receipt(
        &self,
        output: &Path,
        logical_output_relative: &str,
        native_output_relative: &str,
        project: &ExternalProjectSnapshot,
        candidate: &TrainingCandidate,
        strategy: &NativeCandidateStrategy,
        arguments: &[String],
        native_manifest: &Value,
    ) -> Result<Option<Value>, EncoderTaskAdapterError> {
        let Some(binding) = strategy.repair_binding() else {
            return Ok(None);
        };
        let expected = NativeRepairTrainingReceipt::create(
            project,
            candidate,
            binding.clone(),
            logical_output_relative,
            native_output_relative,
            arguments,
            native_manifest,
        )?;
        let path = output.join(REPAIR_TRAINING_RECEIPT_NAME);
        if path.exists() {
            let actual: NativeRepairTrainingReceipt =
                serde_json::from_value(read_json(&path)?).map_err(adapter_error)?;
            actual.validate_against(&expected)?;
            return serde_json::to_value(actual)
                .map(Some)
                .map_err(adapter_error);
        }
        let temporary = output.join(format!(".{REPAIR_TRAINING_RECEIPT_NAME}.tmp"));
        remove_exact_scratch_file(&temporary, output)?;
        let mut bytes = serde_json::to_vec_pretty(&expected).map_err(adapter_error)?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes).map_err(adapter_error)?;
        fs::rename(&temporary, &path).map_err(|error| {
            adapter_error(format!(
                "could not atomically persist Nomos repair training receipt: {error}"
            ))
        })?;
        serde_json::to_value(expected)
            .map(Some)
            .map_err(adapter_error)
    }

    #[allow(clippy::too_many_arguments)]
    fn read_and_validate_repair_training_receipt(
        &self,
        output: &Path,
        logical_output_relative: &str,
        native_output_relative: &str,
        project: &ExternalProjectSnapshot,
        candidate: &TrainingCandidate,
        strategy: &NativeCandidateStrategy,
        arguments: &[String],
        native_manifest: &Value,
    ) -> Result<Option<Value>, EncoderTaskAdapterError> {
        let Some(binding) = strategy.repair_binding() else {
            return Ok(None);
        };
        let expected = NativeRepairTrainingReceipt::create(
            project,
            candidate,
            binding.clone(),
            logical_output_relative,
            native_output_relative,
            arguments,
            native_manifest,
        )?;
        let actual: NativeRepairTrainingReceipt =
            serde_json::from_value(read_json(&output.join(REPAIR_TRAINING_RECEIPT_NAME))?)
                .map_err(adapter_error)?;
        actual.validate_against(&expected)?;
        serde_json::to_value(actual)
            .map(Some)
            .map_err(adapter_error)
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

    /// Resolve an immutable model identity to its verified native checkpoint.
    ///
    /// This is intentionally adapter-specific: callers may transfer an
    /// accepted checkpoint into managed custody only after the native adapter
    /// has reproduced the scientific tree identity.
    pub fn verified_model_path(
        &self,
        model: &ModelArtifactIdentity,
    ) -> Result<PathBuf, EncoderTaskAdapterError> {
        self.model_path(model)
    }
}

impl DevelopmentObservationBackend for NomosBackend {
    fn observer_identity(&self) -> BackendIdentity {
        self.observer_identity.clone()
    }

    fn collect_development_observations(
        &self,
        project: ExternalProjectSnapshot,
        request: DevelopmentObservationRequest,
    ) -> RepairBoxFuture<
        '_,
        Result<CollectedDevelopmentObservations, DevelopmentObservationBackendError>,
    > {
        Box::pin(async move {
            request
                .validate_for_project(&project)
                .map_err(observation_error)?;
            self.inspect(project.clone())
                .await
                .map_err(observation_error)?;
            let configuration = Self::task_configuration(&project).map_err(observation_error)?;
            configuration.validate().map_err(observation_error)?;
            let suite = configuration
                .suites
                .get(&request.suite_key)
                .ok_or_else(|| {
                    observation_error(format!(
                        "unknown Nomos development suite {}",
                        request.suite_key
                    ))
                })?;
            if suite.role != EvidenceRole::Development
                || suite.fingerprint != request.suite_fingerprint
            {
                return Err(observation_error(
                    "Nomos observation request does not bind an exact development suite",
                ));
            }
            let source_artifact = project
                .inputs
                .iter()
                .find(|artifact| artifact.key == suite.path)
                .ok_or_else(|| {
                    observation_error(
                        "Nomos development suite has no pinned source artifact identity",
                    )
                })?;
            if source_artifact.role != EvidenceRole::Development {
                return Err(observation_error(
                    "Nomos development suite resolved to non-development evidence",
                ));
            }
            let model_path = self.model_path(&request.model).map_err(observation_error)?;
            let model_relative =
                workspace_relative(&self.root, &model_path).map_err(observation_error)?;
            let output =
                development_observation_output(&self.root, &self.observer_identity, &request)
                    .map_err(observation_error)?;
            let output_relative =
                workspace_relative(&self.root, &output).map_err(observation_error)?;
            let mut arguments = vec![
                "-m".into(),
                "tools.collect_encoder_gym_development_observations".into(),
                "--manifest".into(),
                EXPERIMENT_MANIFEST_NAME.into(),
                "--model".into(),
                model_relative,
                "--suite-key".into(),
                request.suite_key.clone(),
                "--request-fingerprint".into(),
                request.fingerprint.clone(),
                "--device".into(),
                "cpu".into(),
                "--output".into(),
                output_relative.clone(),
            ];
            for dimension in &request.slice_dimensions {
                arguments.push("--dimension".into());
                arguments.push(dimension.clone());
            }
            if !output.exists() {
                self.run_bounded(&arguments, request.maximum_seconds)
                    .await
                    .map_err(observation_error)?;
            }
            let raw = fs::read(&output).map_err(observation_error)?;
            let native: NativeDevelopmentObservationFile =
                serde_json::from_slice(&raw).map_err(observation_error)?;
            native
                .validate(&request, source_artifact)
                .map_err(observation_error)?;
            let artifact = ExternalArtifactIdentity::new(
                output_relative,
                EvidenceRole::Development,
                raw.len() as u64,
                prefixed(&sha256_file(&output).map_err(observation_error)?),
            )
            .map_err(observation_error)?;
            CollectedDevelopmentObservations::create(
                &request,
                self.observer_identity.clone(),
                artifact,
                native.source_row_count,
                native.metric_eligible_row_count,
                native.observations,
            )
            .map_err(observation_error)
        })
    }
}

fn same_model_content(left: &ModelArtifactIdentity, right: &ModelArtifactIdentity) -> bool {
    left.key == right.key
        && left.format == right.format
        && left.bytes == right.bytes
        && left.fingerprint == right.fingerprint
}

fn bound_source_fingerprint(
    runtime_source_fingerprint: String,
    baseline_override: Option<&ModelArtifactIdentity>,
    training_override: Option<&NomosTrainingDataset>,
) -> Result<String, EncoderTaskAdapterError> {
    match (baseline_override, training_override) {
        (None, None) => return Ok(runtime_source_fingerprint),
        (Some(active_baseline), None) => {
            // Preserve the already shipped promoted-baseline identity.
            return artifact_core::fingerprint(&json!({
                "runtime_source_fingerprint": runtime_source_fingerprint,
                "active_baseline": active_baseline,
            }))
            .map_err(adapter_error);
        }
        _ => {}
    }
    artifact_core::fingerprint(&json!({
        "runtime_source_fingerprint": runtime_source_fingerprint,
        "active_baseline": baseline_override,
        "managed_training_dataset": training_override,
    }))
    .map_err(adapter_error)
}

impl EncoderTaskBackend for NomosBackend {
    fn stop_requested(&self) -> bool {
        progress::stopped()
    }

    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }

    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>> {
        Box::pin(async move {
            self.observe(NativePhase::CheckingFiles);
            project.validate_integrity().map_err(adapter_error)?;
            if !self.supports_identity(&project.backend) {
                return Err(adapter_error(
                    "Nomos project pins a different adapter identity",
                ));
            }
            let configuration = Self::task_configuration(&project)?;
            configuration.validate()?;
            self.verify_pinned_project_artifacts(&project, &configuration)?;
            let current_revision_match = self.project_matches_current(&project)?;
            let mut verified_artifact_keys = project
                .inputs
                .iter()
                .map(|value| value.key.clone())
                .collect::<Vec<_>>();
            verified_artifact_keys.extend(
                configuration
                    .reference_models
                    .values()
                    .map(|value| value.path.clone()),
            );
            verified_artifact_keys.push(configuration.agent_evaluation.chat_model.path);
            if let Some(authority) = &configuration.benchmark_authority {
                verified_artifact_keys.push(authority.path.clone());
            }
            verified_artifact_keys.sort();
            Ok(AdapterInspection {
                source_fingerprint: project.source_fingerprint,
                verified_artifact_keys,
                metadata: json!({
                    "adapter": ADAPTER_NAME,
                    "protocol_version": ADAPTER_PROTOCOL_VERSION,
                    "original_repository_access": self.manifest.source.access,
                    "git_remote_present": false,
                    "current_revision_match": current_revision_match,
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
            self.require_current_project(&project)?;
            self.inspect(project.clone()).await?;
            let configuration = Self::task_configuration(&project)?;
            configuration.validate()?;
            let strategy = NativeCandidateStrategy::parse(&candidate.parameters)?;
            self.observe(NativePhase::CheckingTrainingData);
            strategy.verify_training_inputs(self, &project, &configuration)?;
            let output = self.candidate_output(&candidate);
            let output_relative = output
                .strip_prefix(&self.root)
                .map_err(adapter_error)?
                .to_string_lossy()
                .replace('\\', "/");
            let staging = strategy
                .repair_binding()
                .map(|_| self.candidate_staging_output(&candidate));
            let native_output = staging.as_ref().unwrap_or(&output);
            let native_output_relative = workspace_relative(&self.root, native_output)?;
            let arguments =
                strategy.arguments(&project, &configuration, &native_output_relative)?;
            if output.exists() {
                let native_manifest = strategy.read_and_validate_manifest(
                    &output,
                    &native_output_relative,
                    &project,
                    &configuration,
                )?;
                let repair_receipt = self.read_and_validate_repair_training_receipt(
                    &output,
                    &output_relative,
                    &native_output_relative,
                    &project,
                    &candidate,
                    &strategy,
                    &arguments,
                    &native_manifest,
                )?;
                let (bytes, digest) = tree_identity(&output)?;
                self.observe_saved_training(&native_manifest);
                return Ok(TrainOutput {
                    model: ModelArtifactIdentity::new(
                        output_relative,
                        "sentence-transformers",
                        bytes,
                        prefixed(&digest),
                    )
                    .map_err(adapter_error)?,
                    // Recovery accounts the full reserved duration because the exact elapsed time
                    // may have been lost between process completion and journal append.
                    duration_seconds: candidate.maximum_training_seconds,
                    metadata: json!({
                        "recovered_completed_output": true,
                        "strategy": strategy.name(),
                        "native_manifest": native_manifest,
                        "repair_receipt": repair_receipt,
                    }),
                });
            }
            if let Some(staging) = &staging {
                if staging.exists() {
                    match strategy.read_and_validate_manifest(
                        staging,
                        &native_output_relative,
                        &project,
                        &configuration,
                    ) {
                        Ok(native_manifest) => {
                            self.write_or_validate_repair_training_receipt(
                                staging,
                                &output_relative,
                                &native_output_relative,
                                &project,
                                &candidate,
                                &strategy,
                                &arguments,
                                &native_manifest,
                            )?;
                            self.finalize_repair_staging(&candidate, staging, &output)?;
                            let receipt = self.read_and_validate_repair_training_receipt(
                                &output,
                                &output_relative,
                                &native_output_relative,
                                &project,
                                &candidate,
                                &strategy,
                                &arguments,
                                &native_manifest,
                            )?;
                            let (bytes, digest) = tree_identity(&output)?;
                            self.observe_saved_training(&native_manifest);
                            return Ok(TrainOutput {
                                model: ModelArtifactIdentity::new(
                                    output_relative,
                                    "sentence-transformers",
                                    bytes,
                                    prefixed(&digest),
                                )
                                .map_err(adapter_error)?,
                                duration_seconds: candidate.maximum_training_seconds,
                                metadata: json!({
                                    "recovered_completed_staging": true,
                                    "strategy": strategy.name(),
                                    "native_manifest": native_manifest,
                                    "repair_receipt": receipt,
                                }),
                            });
                        }
                        Err(_) => self.clear_repair_training_scratch(&candidate)?,
                    }
                } else {
                    self.clear_repair_training_checkpoints(&candidate)?;
                }
            }
            let started = std::time::Instant::now();
            self.observe(NativePhase::LoadingModel);
            self.run_accounted_training(&arguments, &candidate).await?;
            self.observe(NativePhase::SavingCheckpoint);
            let native_manifest = strategy.read_and_validate_manifest(
                native_output,
                &native_output_relative,
                &project,
                &configuration,
            )?;
            let repair_receipt = if let Some(staging) = &staging {
                let receipt = self.write_or_validate_repair_training_receipt(
                    staging,
                    &output_relative,
                    &native_output_relative,
                    &project,
                    &candidate,
                    &strategy,
                    &arguments,
                    &native_manifest,
                )?;
                self.finalize_repair_staging(&candidate, staging, &output)?;
                receipt
            } else {
                None
            };
            let (bytes, digest) = tree_identity(&output)?;
            let model = ModelArtifactIdentity::new(
                output_relative,
                "sentence-transformers",
                bytes,
                prefixed(&digest),
            )
            .map_err(adapter_error)?;
            self.observe_saved_training(&native_manifest);
            Ok(TrainOutput {
                model,
                duration_seconds: started.elapsed().as_secs(),
                metadata: json!({
                    "strategy": strategy.name(),
                    "native_manifest": native_manifest,
                    "repair_receipt": repair_receipt,
                }),
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
            self.evaluate_native(project, model, contract, suite_key, Some(maximum_seconds))
                .await?
                .ok_or_else(|| adapter_error("Dispatched evaluation produced no report"))
        })
    }

    fn recover_evaluation(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
    ) -> BoxFuture<'_, Result<Option<EvaluationReport>, EncoderTaskAdapterError>> {
        Box::pin(self.evaluate_native(project, model, contract, suite_key, None))
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
    reference_models: Vec<ReferenceModelManifest>,
    agent_evaluation: AgentEvaluationManifest,
    datasets: Vec<DatasetManifest>,
    #[serde(default)]
    evaluation_suites: Vec<NativeEvaluationSuiteManifest>,
    #[serde(default)]
    benchmark_authority: Option<PinnedFileManifest>,
    evaluation_runs_tree_sha256: String,
}

impl NomosExperimentManifest {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        if !matches!(self.schema_version, 3 | 4)
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
            || self.reference_models.is_empty()
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
        let mut reference_keys = std::collections::BTreeSet::new();
        let mut reference_paths = std::collections::BTreeSet::new();
        for reference in &self.reference_models {
            reference.validate()?;
            if !reference_keys.insert(reference.key.as_str())
                || !reference_paths.insert(reference.path.as_str())
            {
                return Err(adapter_error(
                    "Nomos reference model keys and paths must be unique",
                ));
            }
        }
        self.agent_evaluation.validate()?;
        self.evaluation_suites()?;
        match (self.schema_version, &self.benchmark_authority) {
            (3 | 4, None) => {}
            (4, Some(authority)) => authority.validate("benchmark authority")?,
            _ => {
                return Err(adapter_error(
                    "only successor Nomos manifests may pin benchmark authority evidence",
                ));
            }
        }
        if reference_paths.contains(self.baseline.pytorch_path.as_str())
            || reference_paths.contains(self.baseline.onnx_path.as_str())
            || reference_paths.contains(self.agent_evaluation.chat_model_path.as_str())
        {
            return Err(adapter_error(
                "Nomos support artifacts must use distinct paths",
            ));
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

    fn dataset_by_path(&self, path: &str) -> Result<&DatasetManifest, EncoderTaskAdapterError> {
        let matching = self
            .datasets
            .iter()
            .filter(|value| value.path == path)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(adapter_error(format!(
                "Nomos evaluation suite dataset {path} is not an exact manifest artifact"
            )));
        }
        Ok(matching[0])
    }

    fn evaluation_suites(
        &self,
    ) -> Result<Vec<ResolvedEvaluationSuite<'_>>, EncoderTaskAdapterError> {
        if self.schema_version == 3 {
            if !self.evaluation_suites.is_empty() {
                return Err(adapter_error(
                    "legacy Nomos manifests cannot declare dynamic evaluation suites",
                ));
            }
            return Ok(vec![
                ResolvedEvaluationSuite {
                    key: "development".into(),
                    dataset: self.dataset_for(NativeEvidenceRole::DevelopmentHoldout)?,
                    role: EvidenceRole::Development,
                    agent: &self.agent_evaluation.development,
                },
                ResolvedEvaluationSuite {
                    key: "sealed".into(),
                    dataset: self.dataset_for(NativeEvidenceRole::SealedHoldout)?,
                    role: EvidenceRole::SealedAcceptance,
                    agent: &self.agent_evaluation.sealed,
                },
            ]);
        }
        if self.evaluation_suites.len() < 2 {
            return Err(adapter_error(
                "Nomos successor manifest requires named development and sealed suites",
            ));
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        let mut sealed_count = 0_u32;
        let mut development_count = 0_u32;
        let mut resolved = Vec::with_capacity(self.evaluation_suites.len());
        for suite in &self.evaluation_suites {
            validate_canonical_key(&suite.key, "evaluation suite key")?;
            validate_relative(&suite.dataset_path)?;
            if !keys.insert(suite.key.as_str()) || !paths.insert(suite.dataset_path.as_str()) {
                return Err(adapter_error(
                    "Nomos evaluation suite keys and dataset paths must be unique",
                ));
            }
            let dataset = self.dataset_by_path(&suite.dataset_path)?;
            let (role, expected_native_role, agent) = match suite.role {
                NativeEvaluationSuiteRole::Development => {
                    development_count += 1;
                    (
                        EvidenceRole::Development,
                        NativeEvidenceRole::DevelopmentHoldout,
                        &self.agent_evaluation.development,
                    )
                }
                NativeEvaluationSuiteRole::SealedAcceptance => {
                    sealed_count += 1;
                    (
                        EvidenceRole::SealedAcceptance,
                        NativeEvidenceRole::SealedHoldout,
                        &self.agent_evaluation.sealed,
                    )
                }
            };
            if dataset.role != expected_native_role {
                return Err(adapter_error(format!(
                    "Nomos suite {} role disagrees with dataset {}",
                    suite.key, suite.dataset_path
                )));
            }
            resolved.push(ResolvedEvaluationSuite {
                key: suite.key.clone(),
                dataset,
                role,
                agent,
            });
        }
        if development_count == 0 || sealed_count != 1 {
            return Err(adapter_error(
                "Nomos successor manifest requires at least one development suite and exactly one sealed suite",
            ));
        }
        resolved.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(resolved)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeEvaluationSuiteManifest {
    key: String,
    dataset_path: String,
    role: NativeEvaluationSuiteRole,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativeEvaluationSuiteRole {
    Development,
    SealedAcceptance,
}

struct ResolvedEvaluationSuite<'a> {
    key: String,
    dataset: &'a DatasetManifest,
    role: EvidenceRole,
    agent: &'a AgentSuiteManifest,
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
struct ReferenceModelManifest {
    key: String,
    path: String,
    format: String,
    bytes: u64,
    tree_sha256: String,
    provenance: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedFileManifest {
    path: String,
    bytes: u64,
    sha256: String,
}

impl PinnedFileManifest {
    fn validate(&self, kind: &str) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.path)?;
        if self.bytes == 0 || !raw_sha256(&self.sha256) {
            return Err(adapter_error(format!("Nomos {kind} pin is invalid")));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomosGenerationAuthorityFile {
    schema_version: u32,
    sealed_suite_key: String,
    development_suites: BTreeMap<String, ExternalBenchmarkAuthority>,
    freshness: BenchmarkFreshnessAuthority,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomosExternalQualificationReport {
    schema_version: u32,
    protocol: String,
    policy: String,
    source_manifest: Value,
    source_manifest_fingerprint: String,
    population_fingerprint: String,
    overlap_counts: BTreeMap<String, u64>,
    overlap_by_artifact: BTreeMap<String, BTreeMap<String, u64>>,
    minimum_overall_support: u64,
    observed_overall_support: u64,
    minimum_slice_support: BTreeMap<String, u64>,
    observed_slice_support: BTreeMap<String, u64>,
    ready: bool,
    rows_disclosed: bool,
    model_evaluations_performed: u64,
    review_status: String,
}

impl NomosExternalQualificationReport {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        let expected_overlap = [
            "exact_content",
            "group_identity",
            "normalized_content",
            "source_identity",
        ]
        .into_iter()
        .map(|key| (key.to_owned(), 0_u64))
        .collect::<BTreeMap<_, _>>();
        let reproduced_manifest = artifact_core::fingerprint(&self.source_manifest)
            .map_err(adapter_error)?
            .strip_prefix("sha256:")
            .expect("artifact fingerprints are prefixed")
            .to_owned();
        if self.schema_version != 1
            || self.protocol != "nomos-successor-qualification-v1"
            || self.policy != "zero_tolerance_all_pairwise_v1"
            || !self.source_manifest.is_object()
            || !raw_sha256(&self.source_manifest_fingerprint)
            || reproduced_manifest != self.source_manifest_fingerprint
            || !raw_sha256(&self.population_fingerprint)
            || self.overlap_counts != expected_overlap
            || self.overlap_by_artifact.is_empty()
            || self
                .overlap_by_artifact
                .values()
                .any(|counts| *counts != expected_overlap)
            || self.minimum_overall_support == 0
            || self.observed_overall_support < self.minimum_overall_support
            || self.minimum_slice_support.is_empty()
            || self.minimum_slice_support.keys().collect::<Vec<_>>()
                != self.observed_slice_support.keys().collect::<Vec<_>>()
            || self.minimum_slice_support.iter().any(|(key, minimum)| {
                key.trim().is_empty()
                    || key.trim() != key
                    || *minimum == 0
                    || self.observed_slice_support[key] < *minimum
            })
            || !self.ready
            || self.rows_disclosed
            || self.model_evaluations_performed != 0
            || self.review_status != "awaiting_independent_approval"
        {
            return Err(adapter_error(
                "Nomos qualification report does not satisfy strict successor authority",
            ));
        }
        Ok(())
    }
}

impl NomosGenerationAuthorityFile {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        if self.schema_version != 1
            || self.sealed_suite_key.trim() != self.sealed_suite_key
            || self.sealed_suite_key.is_empty()
            || self.development_suites.is_empty()
        {
            return Err(adapter_error(
                "Nomos benchmark authority envelope is invalid",
            ));
        }
        for (key, authority) in &self.development_suites {
            validate_canonical_key(key, "benchmark development suite key")?;
            authority.validate_integrity().map_err(adapter_error)?;
            if self.created_at < authority.created_at {
                return Err(adapter_error(
                    "Nomos benchmark authority envelope predates its evidence",
                ));
            }
        }
        self.freshness.validate_integrity().map_err(adapter_error)?;
        if self.created_at < self.freshness.frozen_at {
            return Err(adapter_error(
                "Nomos benchmark authority envelope predates freshness review",
            ));
        }
        Ok(())
    }
}

impl ReferenceModelManifest {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        validate_canonical_key(&self.key, "reference model key")?;
        validate_relative(&self.path)?;
        if self.format.trim() != self.format
            || self.format.is_empty()
            || self.bytes == 0
            || !raw_sha256(&self.tree_sha256)
            || !self.provenance.is_object()
        {
            return Err(adapter_error("Nomos reference model identity is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentEvaluationManifest {
    backend: String,
    chat_model_path: String,
    chat_model_format: String,
    chat_model_bytes: u64,
    chat_model_tree_sha256: String,
    source: Value,
    selector_strategy: String,
    candidate_strategy: String,
    nomos_top_k: u64,
    max_attempts: u64,
    development: AgentSuiteManifest,
    sealed: AgentSuiteManifest,
}

impl AgentEvaluationManifest {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.chat_model_path)?;
        if self.backend != "onnx"
            || self.chat_model_format != "onnxruntime-genai"
            || self.chat_model_bytes == 0
            || !raw_sha256(&self.chat_model_tree_sha256)
            || !self.source.is_object()
            || self.selector_strategy != "multiview"
            || self.candidate_strategy != "multiview"
            || !(1..=3).contains(&self.nomos_top_k)
            || !(1..=5).contains(&self.max_attempts)
        {
            return Err(adapter_error(
                "Nomos agent evaluation configuration is invalid",
            ));
        }
        self.development.validate("development")?;
        self.sealed.validate("promotion")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSuiteManifest {
    suite: String,
    sessions: u64,
    pairing: String,
    condition: String,
}

impl AgentSuiteManifest {
    fn validate(&self, expected_suite: &str) -> Result<(), EncoderTaskAdapterError> {
        if self.suite != expected_suite
            || !(1..=256).contains(&self.sessions)
            || !["cycle", "cross-product"].contains(&self.pairing.as_str())
            || self.condition != "nomos"
        {
            return Err(adapter_error("Nomos agent suite configuration is invalid"));
        }
        Ok(())
    }
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
    source_reference: Value,
    baseline_evidence: Value,
    training_inputs: Vec<String>,
    #[serde(default)]
    managed_training_dataset: Option<NomosTrainingDataset>,
    reference_models: BTreeMap<String, PinnedTreeConfiguration>,
    agent_evaluation: AgentEvaluationConfiguration,
    suites: BTreeMap<String, SuiteConfiguration>,
    #[serde(default)]
    benchmark_authority: Option<PinnedFileConfiguration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeDevelopmentObservationFile {
    schema_version: u32,
    protocol: String,
    request_fingerprint: String,
    suite_key: String,
    dataset_fingerprint: String,
    text_version: String,
    slice_dimensions: Vec<String>,
    source_row_count: u64,
    metric_eligible_row_count: u64,
    observations: Vec<DevelopmentObservation>,
}

impl NativeDevelopmentObservationFile {
    fn validate(
        &self,
        request: &DevelopmentObservationRequest,
        source_artifact: &ExternalArtifactIdentity,
    ) -> Result<(), EncoderTaskAdapterError> {
        if self.schema_version != DEVELOPMENT_OBSERVER_SCHEMA_VERSION
            || self.protocol != DEVELOPMENT_OBSERVER_PROTOCOL
            || self.request_fingerprint != request.fingerprint
            || self.suite_key != request.suite_key
            || self.dataset_fingerprint != source_artifact.fingerprint
            || self.text_version != DENSE_TEXT_VERSION
            || self.slice_dimensions != request.slice_dimensions
            || self.source_row_count == 0
            || self.metric_eligible_row_count < request.report_support
            || self.metric_eligible_row_count > self.source_row_count
            || self.observations.len() as u64 != self.source_row_count
        {
            return Err(adapter_error(
                "Nomos native observations do not match the exact development request",
            ));
        }
        for observation in &self.observations {
            observation.validate().map_err(adapter_error)?;
            if observation
                .slices
                .keys()
                .map(String::as_str)
                .ne(request.slice_dimensions.iter().map(String::as_str))
            {
                return Err(adapter_error(
                    "Nomos native observation slices do not match the requested dimensions",
                ));
            }
        }
        Ok(())
    }
}

impl TaskConfiguration {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        if self.adapter_protocol != ADAPTER_PROTOCOL_VERSION
            || self.source_reference.is_null()
            || self.baseline_evidence.is_null()
            || self.training_inputs.is_empty()
            || self.reference_models.is_empty()
        {
            return Err(adapter_error(
                "Nomos project task configuration does not match this adapter",
            ));
        }
        for input in &self.training_inputs {
            validate_relative(input)?;
        }
        if let Some(dataset) = &self.managed_training_dataset {
            dataset.validate()?;
            if self.training_inputs.as_slice() != [dataset.artifact.key.as_str()] {
                return Err(adapter_error(
                    "Nomos managed dataset does not match its native training input",
                ));
            }
        }
        for (key, reference) in &self.reference_models {
            validate_canonical_key(key, "reference model key")?;
            reference.validate("reference model")?;
        }
        self.agent_evaluation.validate()?;
        if let Some(authority) = &self.benchmark_authority {
            authority.validate()?;
        }
        if self.suites.is_empty() {
            return Err(adapter_error("Nomos project defines no evaluation suites"));
        }
        for (key, suite) in &self.suites {
            if key.trim() != key || key.is_empty() {
                return Err(adapter_error("Nomos evaluation suite key is not canonical"));
            }
            validate_relative(&suite.path)?;
            for fingerprint in [
                &suite.fingerprint,
                &suite.retrieval_fingerprint,
                &suite.agent_fingerprint,
            ] {
                if !fingerprint.strip_prefix("sha256:").is_some_and(raw_sha256) {
                    return Err(adapter_error(
                        "Nomos evaluation suite fingerprint is not canonical",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedFileConfiguration {
    path: String,
    bytes: u64,
    fingerprint: String,
}

impl PinnedFileConfiguration {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.path)?;
        if self.bytes == 0
            || !self
                .fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
        {
            return Err(adapter_error(
                "Nomos pinned authority configuration is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedTreeConfiguration {
    path: String,
    format: String,
    bytes: u64,
    fingerprint: String,
    provenance: Value,
}

impl PinnedTreeConfiguration {
    fn validate(&self, kind: &str) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.path)?;
        if self.format.trim() != self.format
            || self.format.is_empty()
            || self.bytes == 0
            || !self
                .fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || !self.provenance.is_object()
        {
            return Err(adapter_error(format!(
                "Nomos {kind} configuration is invalid"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentEvaluationConfiguration {
    backend: String,
    chat_model: AgentChatModelConfiguration,
    selector_strategy: String,
    candidate_strategy: String,
    nomos_top_k: u64,
    max_attempts: u64,
    development: AgentSuiteManifest,
    sealed: AgentSuiteManifest,
}

impl AgentEvaluationConfiguration {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        self.chat_model.validate()?;
        if self.backend != "onnx"
            || self.selector_strategy != "multiview"
            || self.candidate_strategy != "multiview"
            || !(1..=3).contains(&self.nomos_top_k)
            || !(1..=5).contains(&self.max_attempts)
        {
            return Err(adapter_error(
                "Nomos agent evaluation task configuration is invalid",
            ));
        }
        self.development.validate("development")?;
        self.sealed.validate("promotion")?;
        Ok(())
    }

    fn suite(&self, role: EvidenceRole) -> &AgentSuiteManifest {
        match role {
            EvidenceRole::Development => &self.development,
            EvidenceRole::SealedAcceptance => &self.sealed,
            _ => unreachable!("task suites are validated as development or sealed"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentChatModelConfiguration {
    path: String,
    format: String,
    bytes: u64,
    fingerprint: String,
    source: Value,
}

impl AgentChatModelConfiguration {
    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.path)?;
        if self.format != "onnxruntime-genai"
            || self.bytes == 0
            || !self
                .fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || !self.source.is_object()
        {
            return Err(adapter_error("Nomos agent chat model identity is invalid"));
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
    retrieval_fingerprint: String,
    agent_fingerprint: String,
}

#[derive(Debug)]
enum NativeCandidateStrategy {
    FineTune(Box<NativeTrainingParameters>),
    LinearInterpolation {
        reference_model: String,
        specialist_weight: f64,
    },
}

impl NativeCandidateStrategy {
    fn parse(values: &BTreeMap<String, ParameterValue>) -> Result<Self, EncoderTaskAdapterError> {
        match text_parameter(values, "strategy", "fine_tune")?.as_str() {
            "fine_tune" => Ok(Self::FineTune(Box::new(NativeTrainingParameters::parse(
                values,
            )?))),
            "linear_interpolation" => {
                let allowed = ["strategy", "reference_model", "specialist_weight"];
                if values.keys().any(|key| !allowed.contains(&key.as_str())) {
                    return Err(adapter_error(
                        "Nomos interpolation candidate contains an unknown parameter",
                    ));
                }
                let reference_model = text_parameter(values, "reference_model", "")?;
                let specialist_weight = number_parameter(values, "specialist_weight", 0.0)?;
                validate_canonical_key(&reference_model, "reference model key")?;
                if !(0.0 < specialist_weight && specialist_weight <= 0.5) {
                    return Err(adapter_error(
                        "Nomos specialist weight must be greater than zero and at most 0.5",
                    ));
                }
                Ok(Self::LinearInterpolation {
                    reference_model,
                    specialist_weight,
                })
            }
            _ => Err(adapter_error("unsupported Nomos candidate strategy")),
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::FineTune(_) => "fine_tune",
            Self::LinearInterpolation { .. } => "linear_interpolation",
        }
    }

    fn repair_binding(&self) -> Option<&NativeRepairTrainingBinding> {
        match self {
            Self::FineTune(parameters) => parameters.repair.as_ref(),
            Self::LinearInterpolation { .. } => None,
        }
    }

    fn verify_training_inputs(
        &self,
        backend: &NomosBackend,
        project: &ExternalProjectSnapshot,
        configuration: &TaskConfiguration,
    ) -> Result<(), EncoderTaskAdapterError> {
        let Some(repair) = self.repair_binding() else {
            return Ok(());
        };
        repair.validate()?;
        let project_training = project
            .inputs
            .iter()
            .filter(|input| input.role == EvidenceRole::Training)
            .map(|input| input.key.as_str())
            .collect::<BTreeSet<_>>();
        if configuration.training_inputs.len() != project_training.len()
            || configuration
                .training_inputs
                .iter()
                .any(|key| !project_training.contains(key.as_str()))
            || project
                .inputs
                .iter()
                .any(|input| input.key == repair.delta_key)
        {
            return Err(adapter_error(
                "Nomos repair training inputs do not extend the exact project training population",
            ));
        }
        let delta = backend.resolve_existing(&repair.delta_key)?;
        verify_file(
            &delta,
            repair.delta_bytes,
            repair
                .delta_fingerprint
                .strip_prefix("sha256:")
                .ok_or_else(|| adapter_error("Nomos repair delta fingerprint is malformed"))?,
        )
    }

    fn arguments(
        &self,
        project: &ExternalProjectSnapshot,
        configuration: &TaskConfiguration,
        output_relative: &str,
    ) -> Result<Vec<String>, EncoderTaskAdapterError> {
        match self {
            Self::FineTune(parameters) => {
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
                if let Some(repair) = &parameters.repair {
                    arguments.push("--input".into());
                    arguments.push(repair.delta_key.clone());
                }
                arguments.extend([
                    "--output".into(),
                    output_relative.into(),
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
                Ok(arguments)
            }
            Self::LinearInterpolation {
                reference_model,
                specialist_weight,
            } => {
                let reference = configuration
                    .reference_models
                    .get(reference_model)
                    .ok_or_else(|| adapter_error("unknown pinned Nomos reference model"))?;
                Ok(vec![
                    "-m".into(),
                    "tools.interpolate_dense_models".into(),
                    "--base".into(),
                    project.baseline_model.key.clone(),
                    "--specialist".into(),
                    reference.path.clone(),
                    "--specialist-weight".into(),
                    specialist_weight.to_string(),
                    "--output".into(),
                    output_relative.into(),
                ])
            }
        }
    }

    fn read_and_validate_manifest(
        &self,
        output: &Path,
        output_relative: &str,
        project: &ExternalProjectSnapshot,
        configuration: &TaskConfiguration,
    ) -> Result<Value, EncoderTaskAdapterError> {
        let (path, expected_method) = match self {
            Self::FineTune(_) => (output.join("nomos_training_manifest.json"), None),
            Self::LinearInterpolation { .. } => (
                output.join("nomos_interpolation_manifest.json"),
                Some("linear_weight_interpolation"),
            ),
        };
        let native_manifest = read_json(&path)?;
        let native_output = native_manifest
            .get("output")
            .and_then(Value::as_str)
            .map(normalized_native_path);
        if native_output.as_deref() != Some(output_relative) {
            return Err(adapter_error(
                "existing Nomos candidate output does not belong to this immutable candidate",
            ));
        }
        match self {
            Self::FineTune(parameters) => {
                let native_base = native_manifest
                    .get("base_model")
                    .and_then(Value::as_str)
                    .map(normalized_native_path);
                if native_base.as_deref() != Some(project.baseline_model.key.as_str()) {
                    return Err(adapter_error(
                        "Nomos fine-tune manifest references the wrong baseline",
                    ));
                }
                if let Some(repair) = &parameters.repair {
                    validate_repair_training_manifest(
                        &native_manifest,
                        repair,
                        parameters,
                        configuration,
                    )?;
                }
                if parameters.receipt_protocol == "managed-settings-v1" {
                    NomosBackend::verify_configured_training_manifest(
                        configuration,
                        parameters,
                        &native_manifest,
                    )?;
                }
            }
            Self::LinearInterpolation {
                reference_model,
                specialist_weight,
            } => {
                let reference = configuration
                    .reference_models
                    .get(reference_model)
                    .ok_or_else(|| adapter_error("unknown pinned Nomos reference model"))?;
                let method = native_manifest.get("method").and_then(Value::as_str);
                let base = native_manifest
                    .get("base")
                    .and_then(Value::as_str)
                    .map(normalized_native_path);
                let specialist = native_manifest
                    .get("specialist")
                    .and_then(Value::as_str)
                    .map(normalized_native_path);
                let observed_weight = native_manifest
                    .get("specialist_weight")
                    .and_then(Value::as_f64);
                if method != expected_method
                    || base.as_deref() != Some(project.baseline_model.key.as_str())
                    || specialist.as_deref() != Some(reference.path.as_str())
                    || observed_weight != Some(*specialist_weight)
                {
                    return Err(adapter_error(
                        "Nomos interpolation manifest does not match its immutable candidate",
                    ));
                }
            }
        }
        Ok(native_manifest)
    }
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
    receipt_protocol: String,
    repair: Option<NativeRepairTrainingBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRepairTrainingBinding {
    snapshot_id: Uuid,
    snapshot_fingerprint: String,
    snapshot_specification_fingerprint: String,
    combined_membership_fingerprint: String,
    inputs_fingerprint: String,
    selection_id: Uuid,
    selection_fingerprint: String,
    delta_key: String,
    delta_bytes: u64,
    delta_fingerprint: String,
    base_rows: u64,
    delta_rows: u64,
    total_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRepairTrainingReceipt {
    schema_version: String,
    project_id: Uuid,
    project_fingerprint: String,
    candidate_id: Uuid,
    candidate_fingerprint: String,
    binding: NativeRepairTrainingBinding,
    logical_output: String,
    native_output: String,
    arguments_fingerprint: String,
    native_manifest_fingerprint: String,
    fingerprint: String,
}

impl NativeRepairTrainingReceipt {
    #[allow(clippy::too_many_arguments)]
    fn create(
        project: &ExternalProjectSnapshot,
        candidate: &TrainingCandidate,
        binding: NativeRepairTrainingBinding,
        logical_output: &str,
        native_output: &str,
        arguments: &[String],
        native_manifest: &Value,
    ) -> Result<Self, EncoderTaskAdapterError> {
        project.validate_integrity().map_err(adapter_error)?;
        candidate
            .validate_integrity(project)
            .map_err(adapter_error)?;
        binding.validate()?;
        validate_relative(logical_output)?;
        validate_relative(native_output)?;
        let mut value = Self {
            schema_version: REPAIR_TRAINING_RECEIPT_SCHEMA.into(),
            project_id: project.id,
            project_fingerprint: project.fingerprint.clone(),
            candidate_id: candidate.id,
            candidate_fingerprint: candidate.fingerprint.clone(),
            binding,
            logical_output: logical_output.into(),
            native_output: native_output.into(),
            arguments_fingerprint: artifact_core::fingerprint(&arguments).map_err(adapter_error)?,
            native_manifest_fingerprint: artifact_core::fingerprint(native_manifest)
                .map_err(adapter_error)?,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    fn validate_against(&self, expected: &Self) -> Result<(), EncoderTaskAdapterError> {
        if self.schema_version != REPAIR_TRAINING_RECEIPT_SCHEMA
            || !self
                .project_fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || !self
                .candidate_fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || !self
                .arguments_fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || !self
                .native_manifest_fingerprint
                .strip_prefix("sha256:")
                .is_some_and(raw_sha256)
            || self.reproduce_fingerprint()? != self.fingerprint
            || self != expected
        {
            return Err(adapter_error(
                "Nomos repair training receipt is stale, foreign, or changed",
            ));
        }
        Ok(())
    }

    fn reproduce_fingerprint(&self) -> Result<String, EncoderTaskAdapterError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        artifact_core::fingerprint(&value).map_err(adapter_error)
    }
}

impl NativeRepairTrainingBinding {
    const PARAMETER_KEYS: [&'static str; 13] = [
        REPAIR_SNAPSHOT_ID_PARAMETER,
        REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER,
        REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER,
        REPAIR_COMBINED_MEMBERSHIP_PARAMETER,
        REPAIR_INPUTS_FINGERPRINT_PARAMETER,
        REPAIR_SELECTION_ID_PARAMETER,
        REPAIR_SELECTION_FINGERPRINT_PARAMETER,
        REPAIR_DELTA_KEY_PARAMETER,
        REPAIR_DELTA_BYTES_PARAMETER,
        REPAIR_DELTA_FINGERPRINT_PARAMETER,
        REPAIR_BASE_ROWS_PARAMETER,
        REPAIR_DELTA_ROWS_PARAMETER,
        REPAIR_TOTAL_ROWS_PARAMETER,
    ];

    fn parse(
        values: &BTreeMap<String, ParameterValue>,
    ) -> Result<Option<Self>, EncoderTaskAdapterError> {
        let present = Self::PARAMETER_KEYS
            .iter()
            .filter(|key| values.contains_key(**key))
            .count();
        if present == 0 {
            return Ok(None);
        }
        if present != Self::PARAMETER_KEYS.len() {
            return Err(adapter_error(
                "Nomos repair training binding must be complete or absent",
            ));
        }
        let value = Self {
            snapshot_id: parse_uuid_parameter(values, REPAIR_SNAPSHOT_ID_PARAMETER)?,
            snapshot_fingerprint: text_parameter(
                values,
                REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER,
                "",
            )?,
            snapshot_specification_fingerprint: text_parameter(
                values,
                REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER,
                "",
            )?,
            combined_membership_fingerprint: text_parameter(
                values,
                REPAIR_COMBINED_MEMBERSHIP_PARAMETER,
                "",
            )?,
            inputs_fingerprint: text_parameter(values, REPAIR_INPUTS_FINGERPRINT_PARAMETER, "")?,
            selection_id: parse_uuid_parameter(values, REPAIR_SELECTION_ID_PARAMETER)?,
            selection_fingerprint: text_parameter(
                values,
                REPAIR_SELECTION_FINGERPRINT_PARAMETER,
                "",
            )?,
            delta_key: text_parameter(values, REPAIR_DELTA_KEY_PARAMETER, "")?,
            delta_bytes: integer_parameter(values, REPAIR_DELTA_BYTES_PARAMETER, 0)?,
            delta_fingerprint: text_parameter(values, REPAIR_DELTA_FINGERPRINT_PARAMETER, "")?,
            base_rows: integer_parameter(values, REPAIR_BASE_ROWS_PARAMETER, 0)?,
            delta_rows: integer_parameter(values, REPAIR_DELTA_ROWS_PARAMETER, 0)?,
            total_rows: integer_parameter(values, REPAIR_TOTAL_ROWS_PARAMETER, 0)?,
        };
        value.validate()?;
        Ok(Some(value))
    }

    fn validate(&self) -> Result<(), EncoderTaskAdapterError> {
        validate_relative(&self.delta_key)?;
        if self.snapshot_id.is_nil()
            || self.selection_id.is_nil()
            || [
                &self.snapshot_fingerprint,
                &self.snapshot_specification_fingerprint,
                &self.combined_membership_fingerprint,
                &self.inputs_fingerprint,
                &self.selection_fingerprint,
                &self.delta_fingerprint,
            ]
            .iter()
            .any(|value| !value.strip_prefix("sha256:").is_some_and(raw_sha256))
            || self.delta_bytes == 0
            || self.base_rows == 0
            || self.delta_rows == 0
            || self.total_rows != self.base_rows.checked_add(self.delta_rows).unwrap_or(0)
        {
            return Err(adapter_error(
                "Nomos repair training binding is incomplete or contradictory",
            ));
        }
        Ok(())
    }
}

impl NativeTrainingParameters {
    fn parse(values: &BTreeMap<String, ParameterValue>) -> Result<Self, EncoderTaskAdapterError> {
        let allowed = [
            "strategy",
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
            "receipt_protocol",
            REPAIR_SNAPSHOT_ID_PARAMETER,
            REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER,
            REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER,
            REPAIR_COMBINED_MEMBERSHIP_PARAMETER,
            REPAIR_INPUTS_FINGERPRINT_PARAMETER,
            REPAIR_SELECTION_ID_PARAMETER,
            REPAIR_SELECTION_FINGERPRINT_PARAMETER,
            REPAIR_DELTA_KEY_PARAMETER,
            REPAIR_DELTA_BYTES_PARAMETER,
            REPAIR_DELTA_FINGERPRINT_PARAMETER,
            REPAIR_BASE_ROWS_PARAMETER,
            REPAIR_DELTA_ROWS_PARAMETER,
            REPAIR_TOTAL_ROWS_PARAMETER,
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
            receipt_protocol: text_parameter(values, "receipt_protocol", "legacy")?,
            repair: NativeRepairTrainingBinding::parse(values)?,
        };
        if text_parameter(values, "strategy", "fine_tune")? != "fine_tune"
            || !["triplet", "mnrl", "cached-mnrl"].contains(&result.loss.as_str())
            || !(0.0 < result.epochs && result.epochs <= 10.0)
            || !(1..=4_096).contains(&result.batch_size)
            || !(1..=100_000).contains(&result.mining_batch_size)
            || !(0.0 < result.learning_rate && result.learning_rate <= 0.001)
            || !(0.0 < result.margin && result.margin <= 1.0)
            || !["full", "question"].contains(&result.query_strategy.as_str())
            || !["all", "best"].contains(&result.positive_strategy.as_str())
            || !["cpu", "cuda"].contains(&result.device.as_str())
            || !["legacy", "managed-settings-v1"].contains(&result.receipt_protocol.as_str())
            || result.receipt_protocol == "managed-settings-v1"
                && (result.loss != "triplet" || result.repair.is_some())
            || result.repair.is_some() && result.loss != "triplet"
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

fn validate_repair_training_manifest(
    manifest: &Value,
    repair: &NativeRepairTrainingBinding,
    parameters: &NativeTrainingParameters,
    configuration: &TaskConfiguration,
) -> Result<(), EncoderTaskAdapterError> {
    let expected_inputs = configuration
        .training_inputs
        .iter()
        .cloned()
        .chain(std::iter::once(repair.delta_key.clone()))
        .map(|value| normalized_native_path(&value))
        .collect::<Vec<_>>();
    let observed_inputs = manifest
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or_else(|| adapter_error("Nomos repair manifest omitted its exact inputs"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(normalized_native_path)
                .ok_or_else(|| adapter_error("Nomos repair manifest input is not text"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let input_rows = normalized_count_map(manifest, "input_row_counts")?;
    let trainable_rows = normalized_count_map(manifest, "trainable_row_counts")?;
    let input_total = input_rows.values().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| adapter_error("Nomos repair manifest input count overflowed"))
    })?;
    let trainable_total = trainable_rows.values().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| adapter_error("Nomos repair manifest trainable count overflowed"))
    })?;
    let delta_key = normalized_native_path(&repair.delta_key);
    let expected_input_set = expected_inputs.iter().cloned().collect::<BTreeSet<_>>();
    let checkpoint = manifest
        .get("checkpoint_sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let training_duration = manifest
        .get("training_duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let training_loss = manifest
        .get("training_loss")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    if observed_inputs != expected_inputs
        || input_rows.keys().cloned().collect::<BTreeSet<_>>() != expected_input_set
        || trainable_rows.keys().cloned().collect::<BTreeSet<_>>() != expected_input_set
        || input_rows.get(&delta_key) != Some(&repair.delta_rows)
        || trainable_rows.get(&delta_key) != Some(&repair.delta_rows)
        || input_total != repair.total_rows
        || trainable_total != repair.total_rows
        || manifest
            .get("unique_trainable_rows")
            .and_then(Value::as_u64)
            != Some(repair.total_rows)
        || manifest.get("training_triplets").and_then(Value::as_u64) != Some(repair.total_rows)
        || manifest.get("epochs").and_then(Value::as_f64) != Some(parameters.epochs)
        || manifest.get("batch_size").and_then(Value::as_u64) != Some(parameters.batch_size)
        || manifest.get("learning_rate").and_then(Value::as_f64) != Some(parameters.learning_rate)
        || manifest.get("margin").and_then(Value::as_f64) != Some(parameters.margin)
        || manifest.get("query_strategy").and_then(Value::as_str)
            != Some(parameters.query_strategy.as_str())
        || manifest.get("positive_strategy").and_then(Value::as_str)
            != Some(parameters.positive_strategy.as_str())
        || manifest.get("seed").and_then(Value::as_u64) != Some(parameters.seed)
        || manifest.get("device").and_then(Value::as_str) != Some(parameters.device.as_str())
        || manifest.get("loss").and_then(Value::as_str) != Some("TripletLoss(COSINE)")
        || manifest.get("training_script").and_then(Value::as_str)
            != Some("tools.train_dense_triplet_router.v2")
        || !raw_sha256(checkpoint)
        || !training_duration.is_finite()
        || training_duration < 0.0
        || !training_loss.is_finite()
    {
        return Err(adapter_error(
            "Nomos repair training manifest does not match the immutable candidate request",
        ));
    }
    Ok(())
}

fn normalized_count_map(
    manifest: &Value,
    key: &str,
) -> Result<BTreeMap<String, u64>, EncoderTaskAdapterError> {
    manifest
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| adapter_error(format!("Nomos repair manifest omitted {key}")))?
        .iter()
        .map(|(path, count)| {
            count
                .as_u64()
                .map(|count| (normalized_native_path(path), count))
                .ok_or_else(|| adapter_error(format!("Nomos repair manifest {key} is invalid")))
        })
        .collect()
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeAgentEvaluation {
    backend: String,
    suite: String,
    suite_version: String,
    sessions_per_condition: u64,
    pairing: String,
    max_attempts: u64,
    nomos_top_k: u64,
    summaries: BTreeMap<String, NativeAgentSummary>,
}

impl NativeAgentEvaluation {
    fn validate_and_summary<'a>(
        &'a self,
        configuration: &AgentEvaluationConfiguration,
        suite: &AgentSuiteManifest,
    ) -> Result<&'a NativeAgentSummary, EncoderTaskAdapterError> {
        if self.backend != configuration.backend
            || self.suite != suite.suite
            || self.suite_version.trim() != self.suite_version
            || self.suite_version.is_empty()
            || self.sessions_per_condition != suite.sessions
            || self.pairing != suite.pairing
            || self.max_attempts != configuration.max_attempts
            || self.nomos_top_k != configuration.nomos_top_k
            || self.summaries.len() != 1
        {
            return Err(adapter_error(
                "Nomos agent report does not match the pinned evaluation configuration",
            ));
        }
        let summary = self
            .summaries
            .get(&suite.condition)
            .ok_or_else(|| adapter_error("Nomos agent report omitted its pinned condition"))?;
        summary.validate(suite.sessions)?;
        Ok(summary)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeAgentSummary {
    sessions: u64,
    success_rate: f64,
    mean_completed_stage_rate: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
    tool_call_attempts: u64,
    prompt_tokens_per_attempt: f64,
    successful_execution_rate: f64,
    tool_selection_accuracy: f64,
    schema_valid_call_rate: f64,
    wrong_tool_executions: u64,
    invalid_calls: u64,
    visible_oracle_hit_rate: f64,
    tool_description_reduction: f64,
}

impl NativeAgentSummary {
    fn validate(&self, expected_sessions: u64) -> Result<(), EncoderTaskAdapterError> {
        let rates = [
            self.success_rate,
            self.mean_completed_stage_rate,
            self.successful_execution_rate,
            self.tool_selection_accuracy,
            self.schema_valid_call_rate,
            self.visible_oracle_hit_rate,
            self.tool_description_reduction,
        ];
        if self.sessions != expected_sessions
            || self.tool_call_attempts == 0
            || self.invalid_calls > self.tool_call_attempts
            || self.wrong_tool_executions > self.tool_call_attempts - self.invalid_calls
            || self.prompt_tokens == 0
            || self.completion_tokens == 0
            || !self.prompt_tokens_per_attempt.is_finite()
            || self.prompt_tokens_per_attempt <= 0.0
            || rates
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(adapter_error("Nomos agent summary is invalid"));
        }
        let observed = self.prompt_tokens as f64 / self.tool_call_attempts as f64;
        if (observed - self.prompt_tokens_per_attempt).abs() > 1e-9 {
            return Err(adapter_error(
                "Nomos agent prompt-token accounting is inconsistent",
            ));
        }
        Ok(())
    }

    fn normalized_metrics(&self) -> Result<BTreeMap<String, f64>, EncoderTaskAdapterError> {
        let valid_executions = self
            .tool_call_attempts
            .checked_sub(self.invalid_calls)
            .ok_or_else(|| adapter_error("Nomos agent call accounting underflowed"))?;
        if valid_executions == 0 {
            return Err(adapter_error(
                "Nomos agent report contains no valid execution attempts",
            ));
        }
        Ok(BTreeMap::from([
            ("agent_success_rate".into(), self.success_rate),
            (
                "agent_mean_completed_stage_rate".into(),
                self.mean_completed_stage_rate,
            ),
            (
                "agent_successful_execution_rate".into(),
                self.successful_execution_rate,
            ),
            (
                "agent_tool_selection_accuracy".into(),
                self.tool_selection_accuracy,
            ),
            (
                "agent_schema_valid_call_rate".into(),
                self.schema_valid_call_rate,
            ),
            (
                "agent_visible_oracle_hit_rate".into(),
                self.visible_oracle_hit_rate,
            ),
            (
                "agent_invalid_call_rate".into(),
                self.invalid_calls as f64 / self.tool_call_attempts as f64,
            ),
            (
                "agent_wrong_tool_execution_rate".into(),
                self.wrong_tool_executions as f64 / valid_executions as f64,
            ),
            (
                "agent_prompt_tokens_per_attempt".into(),
                self.prompt_tokens_per_attempt,
            ),
            (
                "agent_tool_description_reduction".into(),
                self.tool_description_reduction,
            ),
        ]))
    }
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

fn parse_uuid_parameter(
    values: &BTreeMap<String, ParameterValue>,
    key: &str,
) -> Result<Uuid, EncoderTaskAdapterError> {
    let value = text_parameter(values, key, "")?;
    Uuid::parse_str(&value)
        .map_err(|_| adapter_error(format!("Nomos parameter {key} must be a canonical UUID")))
        .and_then(|parsed| {
            if parsed.is_nil() || parsed.to_string() != value {
                Err(adapter_error(format!(
                    "Nomos parameter {key} must be a canonical UUID"
                )))
            } else {
                Ok(parsed)
            }
        })
}

fn validate_canonical_key(value: &str, kind: &str) -> Result<(), EncoderTaskAdapterError> {
    if value.trim() != value
        || value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(adapter_error(format!("Nomos {kind} is not canonical")));
    }
    Ok(())
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

fn normalized_native_path(value: &str) -> String {
    value.replace('\\', "/")
}

fn workspace_relative(root: &Path, path: &Path) -> Result<String, EncoderTaskAdapterError> {
    path.strip_prefix(root)
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map_err(adapter_error)
}

fn development_observation_output(
    root: &Path,
    observer: &BackendIdentity,
    request: &DevelopmentObservationRequest,
) -> Result<PathBuf, EncoderTaskAdapterError> {
    let observer_digest = observer
        .configuration_fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos observer fingerprint is not canonical"))?;
    let model_digest = request
        .model
        .fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos observation model fingerprint is not canonical"))?;
    let suite_digest = request
        .suite_fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos observation suite fingerprint is not canonical"))?;
    let request_digest = request
        .fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos observation request fingerprint is not canonical"))?;
    Ok(root
        .join("runs")
        .join("encoder-gym-repair")
        .join("observations")
        .join("by-content")
        .join(observer_digest)
        .join(model_digest)
        .join(suite_digest)
        .join(format!("{request_digest}.json")))
}

fn git_output_at<const N: usize>(
    root: &Path,
    arguments: [&str; N],
) -> Result<String, EncoderTaskAdapterError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| adapter_error(format!("could not inspect isolated Git state: {error}")))?;
    if !output.status.success() {
        return Err(adapter_error("could not inspect isolated Nomos Git state"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| adapter_error("Git inspection returned non-UTF-8 output"))
}

fn git_blob_sha256_at(root: &Path, relative: &str) -> Result<String, EncoderTaskAdapterError> {
    validate_relative(relative)?;
    let object = format!("HEAD:{}", relative.replace('\\', "/"));
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "blob", object.as_str()])
        .output()
        .map_err(|error| adapter_error(format!("could not inspect isolated Git blob: {error}")))?;
    if !output.status.success() {
        return Err(adapter_error(
            "could not inspect committed isolated Nomos manifest",
        ));
    }
    Ok(format!("{:x}", Sha256::digest(output.stdout)))
}

fn evaluation_model_root(
    root: &Path,
    model: &ModelArtifactIdentity,
) -> Result<PathBuf, EncoderTaskAdapterError> {
    let model_digest = model
        .fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos model fingerprint is not canonical"))?;
    Ok(root
        .join("runs")
        .join("encoder-gym-evaluations")
        .join("by-content")
        .join(model_digest))
}

fn evaluation_component_root(
    model_root: &Path,
    component: &str,
    fingerprint: &str,
) -> Result<PathBuf, EncoderTaskAdapterError> {
    if !["retrieval", "agent"].contains(&component) {
        return Err(adapter_error("unknown Nomos evaluation component"));
    }
    let component_digest = fingerprint
        .strip_prefix("sha256:")
        .filter(|value| raw_sha256(value))
        .ok_or_else(|| adapter_error("Nomos component fingerprint is not canonical"))?;
    Ok(model_root.join(component).join(component_digest))
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

fn remove_exact_scratch_directory(
    path: &Path,
    expected_parent: &Path,
) -> Result<(), EncoderTaskAdapterError> {
    if path.parent() != Some(expected_parent) {
        return Err(adapter_error(
            "Nomos scratch directory escaped the exact candidate output root",
        ));
    }
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(adapter_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(adapter_error(
            "Nomos scratch target is not a plain directory",
        ));
    }
    fs::remove_dir_all(path).map_err(|error| {
        adapter_error(format!(
            "could not clear exact incomplete Nomos repair scratch directory: {error}"
        ))
    })
}

fn remove_exact_scratch_file(
    path: &Path,
    expected_parent: &Path,
) -> Result<(), EncoderTaskAdapterError> {
    if path.parent() != Some(expected_parent) {
        return Err(adapter_error(
            "Nomos scratch file escaped the exact candidate staging directory",
        ));
    }
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(adapter_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(adapter_error("Nomos scratch target is not a plain file"));
    }
    fs::remove_file(path).map_err(|error| {
        adapter_error(format!(
            "could not clear exact incomplete Nomos repair scratch file: {error}"
        ))
    })
}

fn sha256_file(path: &Path) -> Result<String, EncoderTaskAdapterError> {
    let file = fs::File::open(path)
        .map_err(|error| adapter_error(format!("could not hash Nomos file: {error}")))?;
    let total = file.metadata().map_err(adapter_error)?.len();
    let mut completed = 0;
    progress::file_progress(path, 0, total)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        progress::check_stop()?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| adapter_error(format!("could not hash Nomos file: {error}")))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        completed += read as u64;
        if completed % (8 * 1024 * 1024) == 0 {
            progress::file_progress(path, completed, total)?;
        }
    }
    progress::file_progress(path, completed, total)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn read_json(path: &Path) -> Result<Value, EncoderTaskAdapterError> {
    let bytes = fs::read(path)
        .map_err(|error| adapter_error(format!("could not read Nomos JSON evidence: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| adapter_error(format!("Nomos JSON evidence is invalid: {error}")))
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

fn path_state(path: &Path, expect_directory: bool) -> String {
    match fs::metadata(path) {
        Ok(metadata) if expect_directory && metadata.is_dir() => "directory exists".into(),
        Ok(metadata) if !expect_directory && metadata.is_file() => "file exists".into(),
        Ok(_) if expect_directory => "exists but is not a directory".into(),
        Ok(_) => "exists but is not a file".into(),
        Err(error) if !path.is_absolute() && path.components().count() == 1 => {
            format!("PATH lookup required; direct check failed: {error}")
        }
        Err(error) => format!("unavailable: {error}"),
    }
}

fn adapter_error(error: impl std::fmt::Display) -> EncoderTaskAdapterError {
    EncoderTaskAdapterError::Failure(error.to_string())
}

fn observation_error(error: impl std::fmt::Display) -> DevelopmentObservationBackendError {
    DevelopmentObservationBackendError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use encoder_experiment_core::domain::ParameterValue;

    use super::*;

    pub(super) fn manifest_with_named_suites() -> NomosExperimentManifest {
        NomosExperimentManifest {
            schema_version: 4,
            experiment: "successor".into(),
            tree_hash_algorithm: TREE_HASH_ALGORITHM.into(),
            source: SourceManifest {
                repository: "source".into(),
                access: "read_only_reference".into(),
                commit: "revision".into(),
                snapshot_date: "2026-09-02".into(),
            },
            isolation: IsolationManifest {
                git_remote_allowed: false,
                hard_links_allowed: false,
                symlinks_allowed: false,
                outputs_must_remain_below_experiment_root: true,
            },
            baseline: BaselineManifest {
                pytorch_path: "baseline".into(),
                pytorch_tree_sha256: "1".repeat(64),
                onnx_path: "onnx".into(),
                onnx_tree_sha256: "2".repeat(64),
                weak_agent_raw_completed: CountMetric {
                    completed: 1,
                    total: 1,
                },
                weak_agent_complete_coprocessor_completed: CountMetric {
                    completed: 1,
                    total: 1,
                },
            },
            reference_models: Vec::new(),
            agent_evaluation: AgentEvaluationManifest {
                backend: "onnx".into(),
                chat_model_path: "chat".into(),
                chat_model_format: "onnxruntime-genai".into(),
                chat_model_bytes: 1,
                chat_model_tree_sha256: "3".repeat(64),
                source: json!({}),
                selector_strategy: "multiview".into(),
                candidate_strategy: "multiview".into(),
                nomos_top_k: 1,
                max_attempts: 2,
                development: AgentSuiteManifest {
                    suite: "development".into(),
                    sessions: 16,
                    pairing: "cross-product".into(),
                    condition: "nomos".into(),
                },
                sealed: AgentSuiteManifest {
                    suite: "promotion".into(),
                    sessions: 32,
                    pairing: "cross-product".into(),
                    condition: "nomos".into(),
                },
            },
            datasets: vec![
                DatasetManifest {
                    path: "train.jsonl".into(),
                    role: NativeEvidenceRole::Training,
                    bytes: 1,
                    sha256: "4".repeat(64),
                },
                DatasetManifest {
                    path: "generic.jsonl".into(),
                    role: NativeEvidenceRole::DevelopmentHoldout,
                    bytes: 1,
                    sha256: "5".repeat(64),
                },
                DatasetManifest {
                    path: "retired.jsonl".into(),
                    role: NativeEvidenceRole::DevelopmentHoldout,
                    bytes: 1,
                    sha256: "6".repeat(64),
                },
                DatasetManifest {
                    path: "successor.jsonl".into(),
                    role: NativeEvidenceRole::SealedHoldout,
                    bytes: 1,
                    sha256: "7".repeat(64),
                },
            ],
            evaluation_suites: vec![
                NativeEvaluationSuiteManifest {
                    key: "retired_post_scaling".into(),
                    dataset_path: "retired.jsonl".into(),
                    role: NativeEvaluationSuiteRole::Development,
                },
                NativeEvaluationSuiteManifest {
                    key: "generic".into(),
                    dataset_path: "generic.jsonl".into(),
                    role: NativeEvaluationSuiteRole::Development,
                },
                NativeEvaluationSuiteManifest {
                    key: "successor_sealed".into(),
                    dataset_path: "successor.jsonl".into(),
                    role: NativeEvaluationSuiteRole::SealedAcceptance,
                },
            ],
            benchmark_authority: Some(PinnedFileManifest {
                path: "authority.json".into(),
                bytes: 1,
                sha256: "9".repeat(64),
            }),
            evaluation_runs_tree_sha256: "8".repeat(64),
        }
    }

    #[test]
    fn successor_manifest_keeps_named_development_suites_independent() {
        let manifest = manifest_with_named_suites();
        let suites = manifest.evaluation_suites().unwrap();
        assert_eq!(
            suites
                .iter()
                .map(|suite| suite.key.as_str())
                .collect::<Vec<_>>(),
            vec!["generic", "retired_post_scaling", "successor_sealed"]
        );
        assert_eq!(
            suites
                .iter()
                .filter(|suite| suite.role == EvidenceRole::Development)
                .count(),
            2
        );
        assert_eq!(
            suites
                .iter()
                .filter(|suite| suite.role == EvidenceRole::SealedAcceptance)
                .count(),
            1
        );

        let mut invalid = manifest;
        invalid.evaluation_suites[0].role = NativeEvaluationSuiteRole::SealedAcceptance;
        assert!(invalid.evaluation_suites().is_err());
    }

    #[test]
    fn candidate_parameter_envelope_is_strict() {
        let valid = NativeTrainingParameters::parse(&BTreeMap::from([
            ("loss".into(), ParameterValue::Text("triplet".into())),
            ("learning_rate".into(), ParameterValue::Number(0.000003)),
            ("device".into(), ParameterValue::Text("cuda".into())),
        ]))
        .unwrap();
        assert_eq!(valid.loss, "triplet");

        let mut repair = BTreeMap::from([
            ("loss".into(), ParameterValue::Text("triplet".into())),
            (
                REPAIR_SNAPSHOT_ID_PARAMETER.into(),
                ParameterValue::Text(Uuid::new_v4().to_string()),
            ),
            (
                REPAIR_SNAPSHOT_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"1".repeat(64))),
            ),
            (
                REPAIR_SNAPSHOT_SPECIFICATION_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"2".repeat(64))),
            ),
            (
                REPAIR_COMBINED_MEMBERSHIP_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"3".repeat(64))),
            ),
            (
                REPAIR_INPUTS_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"4".repeat(64))),
            ),
            (
                REPAIR_SELECTION_ID_PARAMETER.into(),
                ParameterValue::Text(Uuid::new_v4().to_string()),
            ),
            (
                REPAIR_SELECTION_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"5".repeat(64))),
            ),
            (
                REPAIR_DELTA_KEY_PARAMETER.into(),
                ParameterValue::Text("runs/repair/delta.jsonl".into()),
            ),
            (
                REPAIR_DELTA_BYTES_PARAMETER.into(),
                ParameterValue::Integer(12),
            ),
            (
                REPAIR_DELTA_FINGERPRINT_PARAMETER.into(),
                ParameterValue::Text(prefixed(&"6".repeat(64))),
            ),
            (
                REPAIR_BASE_ROWS_PARAMETER.into(),
                ParameterValue::Integer(10),
            ),
            (
                REPAIR_DELTA_ROWS_PARAMETER.into(),
                ParameterValue::Integer(2),
            ),
            (
                REPAIR_TOTAL_ROWS_PARAMETER.into(),
                ParameterValue::Integer(12),
            ),
        ]);
        let parsed_repair = NativeTrainingParameters::parse(&repair).unwrap();
        let binding = parsed_repair.repair.as_ref().unwrap();
        let configuration = TaskConfiguration {
            adapter_protocol: ADAPTER_PROTOCOL_VERSION.into(),
            source_reference: json!({}),
            baseline_evidence: json!({}),
            training_inputs: vec!["base-a.jsonl".into(), "base-b.jsonl".into()],
            managed_training_dataset: None,
            reference_models: BTreeMap::new(),
            agent_evaluation: AgentEvaluationConfiguration {
                backend: "onnx".into(),
                chat_model: AgentChatModelConfiguration {
                    path: "chat".into(),
                    format: "onnxruntime-genai".into(),
                    bytes: 1,
                    fingerprint: prefixed(&"7".repeat(64)),
                    source: json!({}),
                },
                selector_strategy: "multiview".into(),
                candidate_strategy: "multiview".into(),
                nomos_top_k: 1,
                max_attempts: 1,
                development: AgentSuiteManifest {
                    suite: "development".into(),
                    sessions: 1,
                    pairing: "cycle".into(),
                    condition: "nomos".into(),
                },
                sealed: AgentSuiteManifest {
                    suite: "promotion".into(),
                    sessions: 1,
                    pairing: "cycle".into(),
                    condition: "nomos".into(),
                },
            },
            suites: BTreeMap::new(),
            benchmark_authority: None,
        };
        let manifest = json!({
            "inputs": ["base-a.jsonl", "base-b.jsonl", "runs/repair/delta.jsonl"],
            "input_row_counts": {
                "base-a.jsonl": 5,
                "base-b.jsonl": 5,
                "runs/repair/delta.jsonl": 2,
            },
            "trainable_row_counts": {
                "base-a.jsonl": 5,
                "base-b.jsonl": 5,
                "runs/repair/delta.jsonl": 2,
            },
            "unique_trainable_rows": 12,
            "training_triplets": 12,
            "epochs": 1.0,
            "batch_size": 64,
            "learning_rate": 0.000003,
            "margin": 0.1,
            "query_strategy": "full",
            "positive_strategy": "best",
            "seed": 20260902,
            "device": "cuda",
            "loss": "TripletLoss(COSINE)",
            "training_script": "tools.train_dense_triplet_router.v2",
            "checkpoint_sha256": "8".repeat(64),
            "training_duration_seconds": 1.0,
            "training_loss": 0.5,
        });
        validate_repair_training_manifest(&manifest, binding, &parsed_repair, &configuration)
            .unwrap();
        let mut changed_manifest = manifest.clone();
        changed_manifest["training_triplets"] = json!(11);
        assert!(
            validate_repair_training_manifest(
                &changed_manifest,
                binding,
                &parsed_repair,
                &configuration,
            )
            .is_err()
        );
        super::training_data::tests::verify_readonly_handoff(repair.clone(), manifest);
        repair.remove(REPAIR_DELTA_FINGERPRINT_PARAMETER);
        assert!(NativeTrainingParameters::parse(&repair).is_err());

        assert!(
            NativeTrainingParameters::parse(&BTreeMap::from([(
                "shell_command".into(),
                ParameterValue::Text("anything".into()),
            )]))
            .is_err()
        );

        let interpolation = NativeCandidateStrategy::parse(&BTreeMap::from([
            (
                "strategy".into(),
                ParameterValue::Text("linear_interpolation".into()),
            ),
            (
                "reference_model".into(),
                ParameterValue::Text("balanced_fullreplay_mnrl_v1".into()),
            ),
            ("specialist_weight".into(), ParameterValue::Number(0.1)),
        ]))
        .unwrap();
        assert_eq!(interpolation.name(), "linear_interpolation");
        assert!(
            NativeCandidateStrategy::parse(&BTreeMap::from([
                (
                    "strategy".into(),
                    ParameterValue::Text("linear_interpolation".into()),
                ),
                (
                    "reference_model".into(),
                    ParameterValue::Text("balanced_fullreplay_mnrl_v1".into()),
                ),
                ("specialist_weight".into(), ParameterValue::Number(0.75)),
            ]))
            .is_err()
        );
    }

    #[test]
    fn scratch_cleanup_is_fenced_to_one_exact_candidate_parent() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("candidates");
        fs::create_dir(&parent).unwrap();
        let scratch = parent.join(".candidate-staging");
        fs::create_dir(&scratch).unwrap();
        fs::write(scratch.join("partial"), b"partial").unwrap();
        remove_exact_scratch_directory(&scratch, &parent).unwrap();
        assert!(!scratch.exists());

        let outside = directory.path().join("outside");
        fs::create_dir(&outside).unwrap();
        assert!(remove_exact_scratch_directory(&outside, &parent).is_err());
        assert!(outside.exists());
    }

    #[test]
    fn current_project_matching_uses_model_content_not_ephemeral_identity() {
        let left =
            ModelArtifactIdentity::new("model", "format", 42, prefixed(&"a".repeat(64))).unwrap();
        let right =
            ModelArtifactIdentity::new("model", "format", 42, prefixed(&"a".repeat(64))).unwrap();
        assert_ne!(left.id, right.id);
        assert!(same_model_content(&left, &right));

        let changed =
            ModelArtifactIdentity::new("model", "format", 42, prefixed(&"b".repeat(64))).unwrap();
        assert!(!same_model_content(&left, &changed));
    }

    #[test]
    fn promoted_baseline_gets_a_stable_successor_source_identity() {
        let runtime = prefixed(&"1".repeat(64));
        let promoted = ModelArtifactIdentity::new(
            "runs/encoder-gym/candidates/promoted",
            "sentence-transformers",
            42,
            prefixed(&"a".repeat(64)),
        )
        .unwrap();
        assert_eq!(
            bound_source_fingerprint(runtime.clone(), None, None).unwrap(),
            runtime
        );
        let first = bound_source_fingerprint(runtime.clone(), Some(&promoted), None).unwrap();
        let legacy = artifact_core::fingerprint(&json!({
            "runtime_source_fingerprint":runtime.clone(),
            "active_baseline":promoted.clone(),
        }))
        .unwrap();
        let second = bound_source_fingerprint(runtime, Some(&promoted), None).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, legacy);
        assert_ne!(first, promoted.fingerprint);
    }

    #[test]
    fn paths_cannot_escape_the_isolated_workspace() {
        assert!(validate_relative("data/generated/train.jsonl").is_ok());
        assert!(validate_relative("../nomos/data.jsonl").is_err());
        assert!(validate_relative("C:\\Users\\source.jsonl").is_err());
    }

    #[test]
    fn nomos_is_canonical_while_pre_rename_runtimes_remain_openable() {
        let canonical = tempfile::tempdir().unwrap();
        fs::create_dir(canonical.path().join("nomos")).unwrap();
        fs::create_dir(canonical.path().join("fitz_tool")).unwrap();
        let package = NativePackage::detect(canonical.path()).unwrap();
        assert_eq!(package, NativePackage::Nomos);
        assert!(development_observer_sources(package).contains(&"nomos/dense_router.py".into()));

        let pre_rename = tempfile::tempdir().unwrap();
        fs::create_dir(pre_rename.path().join("fitz_tool")).unwrap();
        let package = NativePackage::detect(pre_rename.path()).unwrap();
        assert_eq!(package, NativePackage::PreRenameFitzTool);
        assert!(
            repair_delta_sources(package)
                .contains(&"fitz_tool/encoder_gym_repair_delta_v1.py".into())
        );
    }

    #[test]
    fn committed_manifest_identity_ignores_checkout_line_endings() {
        let repository = tempfile::tempdir().unwrap();
        let git = |arguments: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(repository.path())
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "Encoder Gym Test"]);
        git(&["config", "user.email", "encoder-gym@example.invalid"]);
        git(&["config", "core.autocrlf", "true"]);
        let manifest = repository.path().join(EXPERIMENT_MANIFEST_NAME);
        let committed = b"{\n  \"schema_version\": 4\n}\n";
        fs::write(
            repository.path().join(".gitattributes"),
            "*.json text eol=crlf\n",
        )
        .unwrap();
        fs::write(&manifest, committed).unwrap();
        git(&["add", ".gitattributes", EXPERIMENT_MANIFEST_NAME]);
        git(&["commit", "--quiet", "-m", "fixture"]);

        fs::remove_file(&manifest).unwrap();
        git(&["checkout-index", "--force", "--", EXPERIMENT_MANIFEST_NAME]);
        assert!(
            fs::read(&manifest)
                .unwrap()
                .windows(2)
                .any(|pair| pair == b"\r\n")
        );
        git(&["diff", "--quiet", "--", EXPERIMENT_MANIFEST_NAME]);
        assert_ne!(
            sha256_file(&manifest).unwrap(),
            git_blob_sha256_at(repository.path(), EXPERIMENT_MANIFEST_NAME).unwrap()
        );
        assert_eq!(
            git_blob_sha256_at(repository.path(), EXPERIMENT_MANIFEST_NAME).unwrap(),
            format!("{:x}", Sha256::digest(committed))
        );
    }

    #[test]
    fn evaluation_paths_are_content_addressed_not_uuid_addressed() {
        let suite = SuiteConfiguration {
            path: "data/dev.jsonl".into(),
            role: EvidenceRole::Development,
            fingerprint: prefixed(&"b".repeat(64)),
            retrieval_fingerprint: prefixed(&"c".repeat(64)),
            agent_fingerprint: prefixed(&"d".repeat(64)),
        };
        let first = ModelArtifactIdentity::new(
            "artifacts/first",
            "sentence-transformers",
            10,
            prefixed(&"a".repeat(64)),
        )
        .unwrap();
        let second = ModelArtifactIdentity::new(
            "artifacts/second",
            "sentence-transformers",
            10,
            prefixed(&"a".repeat(64)),
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            evaluation_model_root(Path::new("workspace"), &first).unwrap(),
            evaluation_model_root(Path::new("workspace"), &second).unwrap()
        );
        assert_eq!(
            evaluation_component_root(
                &evaluation_model_root(Path::new("workspace"), &first).unwrap(),
                "retrieval",
                &suite.retrieval_fingerprint,
            )
            .unwrap(),
            evaluation_component_root(
                &evaluation_model_root(Path::new("workspace"), &second).unwrap(),
                "retrieval",
                &suite.retrieval_fingerprint,
            )
            .unwrap()
        );
    }

    #[test]
    fn native_development_observations_require_exact_request_and_dimensions() {
        let request = DevelopmentObservationRequest {
            schema_version: 1,
            project_snapshot_id: Uuid::new_v4(),
            project_snapshot_fingerprint: prefixed(&"1".repeat(64)),
            evaluation_report_id: Uuid::new_v4(),
            evaluation_report_fingerprint: prefixed(&"2".repeat(64)),
            evidence_role: EvidenceRole::Development,
            model: ModelArtifactIdentity::new(
                "model",
                "sentence-transformers",
                1,
                prefixed(&"3".repeat(64)),
            )
            .unwrap(),
            suite_key: "generic".into(),
            suite_fingerprint: prefixed(&"4".repeat(64)),
            report_support: 1,
            report_metrics: BTreeMap::from([("mrr".into(), 1.0), ("recall_at_1".into(), 1.0)]),
            slice_dimensions: vec!["workflow".into()],
            maximum_seconds: 60,
            fingerprint: prefixed(&"5".repeat(64)),
        };
        let source = ExternalArtifactIdentity::new(
            "generic.jsonl",
            EvidenceRole::Development,
            1,
            prefixed(&"6".repeat(64)),
        )
        .unwrap();
        let observation = DevelopmentObservation::create(
            prefixed(&"7".repeat(64)),
            prefixed(&"8".repeat(64)),
            BTreeMap::from([("workflow".into(), "lookup".into())]),
            false,
            Some(1),
            1.0,
            Some(0.1),
            Some(prefixed(&"9".repeat(64))),
        )
        .unwrap();
        let mut native = NativeDevelopmentObservationFile {
            schema_version: DEVELOPMENT_OBSERVER_SCHEMA_VERSION,
            protocol: DEVELOPMENT_OBSERVER_PROTOCOL.into(),
            request_fingerprint: request.fingerprint.clone(),
            suite_key: request.suite_key.clone(),
            dataset_fingerprint: source.fingerprint.clone(),
            text_version: DENSE_TEXT_VERSION.into(),
            slice_dimensions: request.slice_dimensions.clone(),
            source_row_count: 1,
            metric_eligible_row_count: 1,
            observations: vec![observation],
        };
        native.validate(&request, &source).unwrap();

        native.slice_dimensions = vec!["question".into()];
        assert!(native.validate(&request, &source).is_err());
    }

    #[test]
    fn development_observation_paths_are_content_addressed() {
        let request = DevelopmentObservationRequest {
            schema_version: 1,
            project_snapshot_id: Uuid::new_v4(),
            project_snapshot_fingerprint: prefixed(&"1".repeat(64)),
            evaluation_report_id: Uuid::new_v4(),
            evaluation_report_fingerprint: prefixed(&"2".repeat(64)),
            evidence_role: EvidenceRole::Development,
            model: ModelArtifactIdentity::new(
                "model",
                "sentence-transformers",
                1,
                prefixed(&"3".repeat(64)),
            )
            .unwrap(),
            suite_key: "generic".into(),
            suite_fingerprint: prefixed(&"4".repeat(64)),
            report_support: 1,
            report_metrics: BTreeMap::from([("mrr".into(), 1.0)]),
            slice_dimensions: vec!["workflow".into()],
            maximum_seconds: 60,
            fingerprint: prefixed(&"5".repeat(64)),
        };
        let observer = BackendIdentity::new("observer", "v1", prefixed(&"0".repeat(64))).unwrap();
        let path =
            development_observation_output(Path::new("workspace"), &observer, &request).unwrap();
        assert!(path.ends_with(Path::new(&format!(
            "runs/encoder-gym-repair/observations/by-content/{}/{}/{}/{}.json",
            "0".repeat(64),
            "3".repeat(64),
            "4".repeat(64),
            "5".repeat(64)
        ))));
    }

    #[test]
    fn agent_report_normalization_preserves_production_outcomes() {
        let report: NativeAgentEvaluation = serde_json::from_value(json!({
            "backend": "onnx",
            "suite": "development",
            "suite_version": "development.v1",
            "sessions_per_condition": 16,
            "pairing": "cross-product",
            "max_attempts": 2,
            "nomos_top_k": 3,
            "summaries": {
                "nomos": {
                    "sessions": 16,
                    "success_rate": 0.75,
                    "mean_completed_stage_rate": 0.8,
                    "prompt_tokens": 3200,
                    "completion_tokens": 800,
                    "tool_call_attempts": 40,
                    "prompt_tokens_per_attempt": 80.0,
                    "successful_execution_rate": 0.9,
                    "tool_selection_accuracy": 0.875,
                    "schema_valid_call_rate": 0.95,
                    "wrong_tool_executions": 2,
                    "invalid_calls": 2,
                    "visible_oracle_hit_rate": 1.0,
                    "tool_description_reduction": 0.91
                }
            }
        }))
        .unwrap();
        let configuration = AgentEvaluationConfiguration {
            backend: "onnx".into(),
            chat_model: AgentChatModelConfiguration {
                path: "artifacts/chat".into(),
                format: "onnxruntime-genai".into(),
                bytes: 1,
                fingerprint: prefixed(&"a".repeat(64)),
                source: json!({"revision":"pinned"}),
            },
            selector_strategy: "multiview".into(),
            candidate_strategy: "multiview".into(),
            nomos_top_k: 3,
            max_attempts: 2,
            development: AgentSuiteManifest {
                suite: "development".into(),
                sessions: 16,
                pairing: "cross-product".into(),
                condition: "nomos".into(),
            },
            sealed: AgentSuiteManifest {
                suite: "promotion".into(),
                sessions: 32,
                pairing: "cross-product".into(),
                condition: "nomos".into(),
            },
        };
        let metrics = report
            .validate_and_summary(&configuration, &configuration.development)
            .unwrap()
            .normalized_metrics()
            .unwrap();
        assert_eq!(metrics["agent_success_rate"], 0.75);
        assert_eq!(metrics["agent_invalid_call_rate"], 0.05);
        assert_eq!(metrics["agent_wrong_tool_execution_rate"], 2.0 / 38.0);
    }
}
