//! Portable project custody contracts. No filesystem, database, or trainer policy.
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

mod activity;
mod benchmark;
pub mod benchmark_results;
mod models;
mod optimization_launch;
mod optimization_run;
mod optimization_setup;
mod providers;
mod readiness;
mod scientific;
mod training_data;
pub use benchmark::{BenchmarkSource, ProjectBenchmarkVersion};
pub use optimization_launch::{
    FinalEvaluationAuthorization, OptimizationExecutionLimits, OptimizationLaunchAuthorization,
    OptimizationLaunchScope,
};
pub use optimization_run::{
    ProjectOptimizationEvent, ProjectOptimizationEventKind, ProjectOptimizationRun,
    ProjectOptimizationRunState, ProjectOptimizationRunView, replay_project_optimization,
};
pub use optimization_setup::{OptimizationInputs, OptimizationSetup};
pub use training_data::{ModelDatasetLink, ModelTrainingEvidence, TrainingDatasetInput};

pub use activity::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, ProjectAction,
    ProjectActivityEvent, ProjectActivityLog,
};
pub use models::{
    BaselineChange, BaselineRevision, BoundIdentity, ModelArtifact, ModelCatalog, ModelOrigin,
};
pub use providers::{
    ProviderAuthentication, ProviderCatalog, ProviderConfiguration, ProviderKind, ProviderLimits,
    ProviderRole, SecretReference,
};
pub use readiness::{
    ReadinessAction, ReadinessCategory, ReadinessCheck, ReadinessReport, ReadinessState,
};
pub use scientific::{
    AdapterBinding, RuntimeBinding, RuntimeKind, ScientificBinding, ScientificStoreBinding,
};

pub const MANIFEST: &str = "encoder-gym.json";
pub const DATABASE: &str = "project.sqlite";
pub const BASELINE: &str = "models/baseline";
pub const DIRECTORIES: &[&str] = &[
    BASELINE,
    "models/candidates",
    "datasets/imports",
    "datasets/snapshots",
    "runs",
    "evaluations",
];

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Invalid(pub String);

pub(crate) fn require(ok: bool, message: &str) -> Result<(), Invalid> {
    if ok {
        Ok(())
    } else {
        Err(Invalid(message.into()))
    }
}

pub fn validate_name(name: &str) -> Result<(), Invalid> {
    require(
        !name.trim().is_empty()
            && name.chars().count() <= 120
            && !name.chars().any(char::is_control),
        "Name must contain 1–120 printable characters.",
    )
}

pub fn validate_relative(path: &str) -> Result<(), Invalid> {
    require(
        !path.is_empty()
            && !path.contains(['\\', ':'])
            && !path.chars().any(char::is_control)
            && path.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.ends_with(['.', ' '])
                    && !part.contains(['<', '>', '"', '|', '?', '*'])
                    && ![
                        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                        "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
                        "LPT7", "LPT8", "LPT9",
                    ]
                    .contains(
                        &part
                            .split('.')
                            .next()
                            .unwrap_or("")
                            .to_ascii_uppercase()
                            .as_str(),
                    )
            }),
        "Artifact paths must be portable, contained relative paths.",
    )
}

pub fn validate_hash(value: &str) -> Result<(), Invalid> {
    require(
        value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        }),
        "Expected a lowercase SHA-256 content identity.",
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIdentity {
    pub path: String,
    pub bytes: u64,
    pub fingerprint: String,
}

/// A deterministic inventory identity, independent of source and destination paths.
pub fn inventory_fingerprint(files: &[FileIdentity]) -> String {
    let bytes = serde_json::to_vec(files).expect("file identities serialize");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalModel {
    pub source: String,
    pub format: String,
    pub architecture: String,
    pub files: Vec<FileIdentity>,
    pub bytes: u64,
    pub fingerprint: String,
    /// Custody does not authorize execution or prove trainer compatibility.
    pub execution: String,
}

impl LocalModel {
    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.files.is_empty() && self.files.len() <= 512,
            "Invalid model inventory size.",
        )?;
        require(
            matches!(
                self.format.as_str(),
                "safetensors-encoder" | "sentence-transformers"
            ),
            "Unsupported model format.",
        )?;
        require(
            self.execution == "not-configured",
            "Workspace custody cannot authorize model execution.",
        )?;
        require(
            !self.source.is_empty() && !self.architecture.is_empty(),
            "Missing model source or architecture.",
        )?;
        let mut names = BTreeSet::new();
        let mut previous = "";
        let mut bytes = 0_u64;
        for file in &self.files {
            validate_relative(&file.path)?;
            validate_hash(&file.fingerprint)?;
            require(
                file.path.as_str() > previous && names.insert(file.path.to_lowercase()),
                "Model paths must be sorted and unique, including case-insensitive filesystems.",
            )?;
            previous = &file.path;
            bytes = bytes
                .checked_add(file.bytes)
                .ok_or_else(|| Invalid("Model size overflow.".into()))?;
        }
        for required in ["config.json", "tokenizer.json", "model.safetensors"] {
            require(
                names.contains(required),
                "Model requires config.json, tokenizer.json, and model.safetensors.",
            )?;
        }
        require(
            bytes == self.bytes && inventory_fingerprint(&self.files) == self.fingerprint,
            "Model inventory identity does not match.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectManifest {
    pub version: u32,
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub task: Option<String>,
    pub baseline: LocalModel,
}

impl ProjectManifest {
    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            self.version == 1,
            "Unsupported Encoder Gym workspace version.",
        )?;
        require(!self.id.is_nil(), "Project identity cannot be empty.")?;
        validate_name(&self.name)?;
        if let Some(task) = &self.task {
            validate_name(task)?;
        }
        self.baseline.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatasetPurpose {
    Unassigned,
    Training,
    Development,
    Sealed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingSource {
    pub baseline_fingerprint: String,
    pub manifest_fingerprint: String,
    pub input: String,
    pub declared_rows: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetImport {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub source: String,
    pub purpose: DatasetPurpose,
    pub format: String,
    pub artifact: FileIdentity,
    pub rows: u64,
    pub training_source: Option<TrainingSource>,
}

impl DatasetImport {
    pub fn validate(&self) -> Result<(), Invalid> {
        validate_name(&self.name)?;
        validate_hash(&self.artifact.fingerprint)?;
        let hex = &self.artifact.fingerprint[7..];
        require(
            self.artifact.path == format!("datasets/imports/{hex}/data.jsonl"),
            "Dataset path does not match its content identity.",
        )?;
        require(
            !self.id.is_nil()
                && !self.source.is_empty()
                && self.rows > 0
                && self.artifact.bytes > 0
                && self.format == "jsonl",
            "Invalid imported dataset metadata.",
        )?;
        if let Some(provenance) = &self.training_source {
            validate_hash(&provenance.baseline_fingerprint)?;
            validate_hash(&provenance.manifest_fingerprint)?;
            validate_relative(&provenance.input)?;
            require(
                self.purpose == DatasetPurpose::Training && self.rows == provenance.declared_rows,
                "Training provenance must match the imported row count and purpose.",
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_portable_and_contained() {
        for bad in [
            "", "/x", "../x", "C:/x", "a\\b", "a//b", "a/../b", "a.", "a ", "x:stream",
        ] {
            assert!(validate_relative(bad).is_err(), "{bad}");
        }
        validate_relative("1_Pooling/config.json").unwrap();
    }

    #[test]
    fn identity_depends_on_paths_and_bytes_not_source_location() {
        let files = vec![FileIdentity {
            path: "x".into(),
            bytes: 4,
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        }];
        let mut changed = files.clone();
        changed[0].path = "y".into();
        assert_ne!(
            inventory_fingerprint(&files),
            inventory_fingerprint(&changed)
        );
        assert_eq!(
            inventory_fingerprint(&files),
            inventory_fingerprint(&files.clone())
        );
    }
}
