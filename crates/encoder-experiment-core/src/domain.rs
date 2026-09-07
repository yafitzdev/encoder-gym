use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{EncoderExperimentError, canonical_sha256, fingerprint, required};

pub const PROJECT_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const OPTIMIZATION_PROTOCOL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderTaskKind {
    RetrievalRanking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    Training,
    Development,
    Calibration,
    SealedAcceptance,
}

impl EvidenceRole {
    pub const fn adaptation_eligible(self) -> bool {
        matches!(self, Self::Training | Self::Development | Self::Calibration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendIdentity {
    pub name: String,
    pub protocol_version: String,
    pub configuration_fingerprint: String,
}

impl BackendIdentity {
    pub fn new(
        name: impl Into<String>,
        protocol_version: impl Into<String>,
        configuration_fingerprint: impl Into<String>,
    ) -> Result<Self, EncoderExperimentError> {
        let value = Self {
            name: required(name, "backend name")?,
            protocol_version: required(protocol_version, "backend protocol version")?,
            configuration_fingerprint: configuration_fingerprint.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), EncoderExperimentError> {
        if self.name.trim() != self.name
            || self.name.is_empty()
            || self.protocol_version.trim() != self.protocol_version
            || self.protocol_version.is_empty()
            || !canonical_sha256(&self.configuration_fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "backend identity is not canonical".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalArtifactIdentity {
    pub key: String,
    pub role: EvidenceRole,
    pub bytes: u64,
    pub fingerprint: String,
}

impl ExternalArtifactIdentity {
    pub fn new(
        key: impl Into<String>,
        role: EvidenceRole,
        bytes: u64,
        fingerprint: impl Into<String>,
    ) -> Result<Self, EncoderExperimentError> {
        let value = Self {
            key: required(key, "external artifact key")?,
            role,
            bytes,
            fingerprint: fingerprint.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), EncoderExperimentError> {
        if self.key.trim() != self.key
            || self.key.is_empty()
            || self.bytes == 0
            || !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(format!(
                "external artifact {} is not canonical",
                self.key
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelArtifactIdentity {
    pub id: Uuid,
    pub key: String,
    pub format: String,
    pub bytes: u64,
    pub fingerprint: String,
}

impl ModelArtifactIdentity {
    pub fn new(
        key: impl Into<String>,
        format: impl Into<String>,
        bytes: u64,
        fingerprint: impl Into<String>,
    ) -> Result<Self, EncoderExperimentError> {
        let value = Self {
            id: Uuid::new_v4(),
            key: required(key, "model artifact key")?,
            format: required(format, "model artifact format")?,
            bytes,
            fingerprint: fingerprint.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), EncoderExperimentError> {
        if self.key.trim() != self.key
            || self.key.is_empty()
            || self.format.trim() != self.format
            || self.format.is_empty()
            || self.bytes == 0
            || !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "model artifact identity is not canonical".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalProjectSnapshot {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub task: EncoderTaskKind,
    pub source_revision: String,
    pub source_fingerprint: String,
    pub backend: BackendIdentity,
    pub inputs: Vec<ExternalArtifactIdentity>,
    pub baseline_model: ModelArtifactIdentity,
    pub task_configuration: Value,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ExternalProjectSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        name: impl Into<String>,
        task: EncoderTaskKind,
        source_revision: impl Into<String>,
        source_fingerprint: impl Into<String>,
        backend: BackendIdentity,
        mut inputs: Vec<ExternalArtifactIdentity>,
        baseline_model: ModelArtifactIdentity,
        task_configuration: Value,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        inputs.sort_by(|left, right| left.key.cmp(&right.key));
        let mut value = Self {
            schema_version: PROJECT_SNAPSHOT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: required(name, "external project name")?,
            task,
            source_revision: required(source_revision, "source revision")?,
            source_fingerprint: source_fingerprint.into(),
            backend,
            inputs,
            baseline_model,
            task_configuration,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderExperimentError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderExperimentError::Integrity(format!(
                "external project snapshot {} fingerprint changed",
                self.id
            )));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "name": self.name,
            "task": self.task,
            "source_revision": self.source_revision,
            "source_fingerprint": self.source_fingerprint,
            "backend": self.backend,
            "inputs": self.inputs,
            "baseline_model": self.baseline_model,
            "task_configuration": self.task_configuration,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderExperimentError> {
        if self.schema_version != PROJECT_SNAPSHOT_SCHEMA_VERSION
            || self.name.trim() != self.name
            || self.name.is_empty()
            || self.source_revision.trim() != self.source_revision
            || self.source_revision.is_empty()
            || !canonical_sha256(&self.source_fingerprint)
            || self.task_configuration.is_null()
            || !canonical_sha256(&self.fingerprint) && !self.fingerprint.is_empty()
        {
            return Err(EncoderExperimentError::Validation(
                "external project snapshot fields are not canonical".into(),
            ));
        }
        self.backend.validate()?;
        self.baseline_model.validate()?;
        if self.inputs.is_empty() {
            return Err(EncoderExperimentError::Validation(
                "external project snapshot requires immutable inputs".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for input in &self.inputs {
            input.validate()?;
            if !keys.insert(input.key.as_str()) {
                return Err(EncoderExperimentError::Validation(
                    "external project input keys must be unique".into(),
                ));
            }
            roles.insert(input.role);
        }
        if !roles.contains(&EvidenceRole::Training)
            || !roles.contains(&EvidenceRole::Development)
            || !roles.contains(&EvidenceRole::SealedAcceptance)
        {
            return Err(EncoderExperimentError::Validation(
                "external project requires training, development, and sealed evidence".into(),
            ));
        }
        if self
            .inputs
            .windows(2)
            .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(EncoderExperimentError::Validation(
                "external project inputs must use canonical key order".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Text(String),
}

impl ParameterValue {
    fn validate(&self) -> Result<(), EncoderExperimentError> {
        match self {
            Self::Number(value) if !value.is_finite() => Err(EncoderExperimentError::Validation(
                "candidate parameter numbers must be finite".into(),
            )),
            Self::Text(value) if value.trim() != value || value.is_empty() => {
                Err(EncoderExperimentError::Validation(
                    "candidate parameter text must be non-empty and canonical".into(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingCandidate {
    pub id: Uuid,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub sequence: u32,
    pub maximum_training_seconds: u64,
    pub parameters: BTreeMap<String, ParameterValue>,
    pub fingerprint: String,
}

impl TrainingCandidate {
    pub fn create(
        project: &ExternalProjectSnapshot,
        sequence: u32,
        maximum_training_seconds: u64,
        parameters: BTreeMap<String, ParameterValue>,
    ) -> Result<Self, EncoderExperimentError> {
        Self::create_identified(
            Uuid::new_v4(),
            project,
            sequence,
            maximum_training_seconds,
            parameters,
        )
    }

    /// Construct an exact candidate identity reserved by a durable parent workflow.
    pub fn create_identified(
        id: Uuid,
        project: &ExternalProjectSnapshot,
        sequence: u32,
        maximum_training_seconds: u64,
        parameters: BTreeMap<String, ParameterValue>,
    ) -> Result<Self, EncoderExperimentError> {
        project.validate_integrity()?;
        if id.is_nil() || sequence == 0 || maximum_training_seconds == 0 || parameters.is_empty() {
            return Err(EncoderExperimentError::Validation(
                "training candidate requires a positive sequence, time limit, and parameters"
                    .into(),
            ));
        }
        for (key, value) in &parameters {
            if key.trim() != key || key.is_empty() {
                return Err(EncoderExperimentError::Validation(
                    "candidate parameter keys must be non-empty and canonical".into(),
                ));
            }
            value.validate()?;
        }
        let mut candidate = Self {
            id,
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            sequence,
            maximum_training_seconds,
            parameters,
            fingerprint: String::new(),
        };
        candidate.fingerprint = candidate.reproduce_fingerprint()?;
        Ok(candidate)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "id": self.id,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "sequence": self.sequence,
            "maximum_training_seconds": self.maximum_training_seconds,
            "parameters": self.parameters,
        }))
    }

    pub fn validate_integrity(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<(), EncoderExperimentError> {
        if self.project_snapshot_id != project.id
            || self.project_snapshot_fingerprint != project.fingerprint
            || self.sequence == 0
            || self.maximum_training_seconds == 0
            || self.parameters.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(EncoderExperimentError::Integrity(
                "training candidate does not match its project snapshot".into(),
            ));
        }
        for (key, value) in &self.parameters {
            if key.trim() != key || key.is_empty() {
                return Err(EncoderExperimentError::Integrity(
                    "training candidate parameter key changed".into(),
                ));
            }
            value.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationBudget {
    pub maximum_candidates: u32,
    pub maximum_training_seconds: u64,
    pub maximum_development_evaluations: u32,
    pub maximum_sealed_evaluations: u32,
}

impl OptimizationBudget {
    pub fn validate(&self) -> Result<(), EncoderExperimentError> {
        if self.maximum_candidates == 0
            || self.maximum_training_seconds == 0
            || self.maximum_development_evaluations < self.maximum_candidates
            || self.maximum_sealed_evaluations != 1
        {
            return Err(EncoderExperimentError::Validation(
                "optimization budget must be finite, cover every candidate development evaluation, and permit exactly one sealed evaluation".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos pilot",
            EncoderTaskKind::RetrievalRanking,
            "14e0a16",
            digest('a'),
            BackendIdentity::new("nomos", "nomos-ranking-v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 10, digest('c'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "development",
                    EvidenceRole::Development,
                    10,
                    digest('d'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    10,
                    digest('e'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 10, digest('f'))
                .unwrap(),
            json!({"query_protocol":"dense-text.v3"}),
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn project_snapshot_requires_all_evidence_roles_and_replays_integrity() {
        let project = project();
        project.validate_integrity().unwrap();
        let mut tampered = project.clone();
        tampered.source_revision.push_str("-changed");
        assert!(matches!(
            tampered.validate_integrity(),
            Err(EncoderExperimentError::Integrity(_))
        ));
    }

    #[test]
    fn candidate_is_bound_to_the_exact_project() {
        let snapshot = project();
        let candidate = TrainingCandidate::create(
            &snapshot,
            1,
            600,
            BTreeMap::from([
                ("learning_rate".into(), ParameterValue::Number(0.000003)),
                ("loss".into(), ParameterValue::Text("triplet".into())),
            ]),
        )
        .unwrap();
        candidate.validate_integrity(&snapshot).unwrap();
        let other = project();
        assert!(candidate.validate_integrity(&other).is_err());
    }
}
