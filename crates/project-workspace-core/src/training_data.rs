//! Immutable provenance edges, not permission to train or promote a model.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::{
    domain::SnapshotSplit,
    versions::{DatasetVersion, DatasetVersionRef},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    BoundIdentity, FileIdentity, Invalid, ModelArtifact, ModelOrigin, require, validate_hash,
    validate_relative,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingDatasetInput {
    /// Native input order belongs to this model, not to the dataset's display order.
    pub key: String,
    pub import_id: Uuid,
    pub fingerprint: String,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ModelTrainingEvidence {
    /// Recorded final-stage inputs, not proof about unknown ancestor pretraining.
    ImportedManifest { manifest: FileIdentity },
    CompletedTraining {
        manifest: FileIdentity,
        snapshot: BoundIdentity,
        run: BoundIdentity,
    },
    /// A verified rendering of an existing version, including selected subsets.
    /// Native file positions do not replace the version's stable row identities.
    MaterializedTraining {
        manifest: FileIdentity,
        snapshot: BoundIdentity,
        run: BoundIdentity,
        #[serde(rename = "orderedContentFingerprint")]
        ordered_content_fingerprint: String,
    },
}

impl ModelTrainingEvidence {
    pub fn manifest(&self) -> &FileIdentity {
        match self {
            Self::ImportedManifest { manifest }
            | Self::CompletedTraining { manifest, .. }
            | Self::MaterializedTraining { manifest, .. } => manifest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDatasetLink {
    pub project_id: Uuid,
    pub model_id: Uuid,
    pub model_fingerprint: String,
    pub version: DatasetVersionRef,
    pub inputs: Vec<TrainingDatasetInput>,
    pub evidence: ModelTrainingEvidence,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ModelDatasetLink {
    pub fn new(
        model: &ModelArtifact,
        version: &DatasetVersion,
        inputs: Vec<TrainingDatasetInput>,
        evidence: ModelTrainingEvidence,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut link = Self {
            project_id: model.project_id,
            model_id: model.id,
            model_fingerprint: model.fingerprint.clone(),
            version: version.reference(),
            inputs,
            evidence,
            created_at,
            fingerprint: String::new(),
        };
        link.fingerprint = link.reproduce()?;
        link.validate_for(model, version)?;
        Ok(link)
    }

    fn reproduce(&self) -> Result<String, Invalid> {
        let mut value = self.clone();
        value.fingerprint.clear();
        artifact_core::fingerprint(&value).map_err(|error| Invalid(error.to_string()))
    }

    /// Validate the metadata binding. This does not inspect row membership;
    /// recording or fully verifying a link must use `validate_for` as well.
    pub fn validate_reference(
        &self,
        model: &ModelArtifact,
        version: &DatasetVersionRef,
    ) -> Result<(), Invalid> {
        model.validate()?;
        version
            .validate()
            .map_err(|error| Invalid(error.to_string()))?;
        require(
            self.fingerprint == self.reproduce()?,
            "Model dataset link fingerprint changed.",
        )?;
        require(
            self.project_id == model.project_id
                && self.project_id == version.project_id
                && self.model_id == model.id
                && self.model_fingerprint == model.fingerprint
                && self.version == *version,
            "Model dataset link belongs to another artifact or version.",
        )?;
        let manifest = self.evidence.manifest();
        validate_relative(&manifest.path)?;
        validate_hash(&manifest.fingerprint)?;
        require(manifest.bytes > 0, "Training manifest is empty.")?;
        match &self.evidence {
            ModelTrainingEvidence::ImportedManifest { .. } => require(
                model.origin == ModelOrigin::Imported,
                "Imported evidence cannot replace a trained model's receipt.",
            )?,
            ModelTrainingEvidence::CompletedTraining { snapshot, run, .. }
            | ModelTrainingEvidence::MaterializedTraining { snapshot, run, .. } => {
                snapshot.validate("Training snapshot")?;
                run.validate("Training run")?;
                require(
                    model.origin == ModelOrigin::Trained
                        && model.training_snapshot.as_ref() == Some(snapshot)
                        && model.producing_run.as_ref() == Some(run),
                    "Training evidence does not match the model's recorded snapshot and run.",
                )?;
            }
        }
        if let ModelTrainingEvidence::MaterializedTraining {
            snapshot,
            ordered_content_fingerprint,
            ..
        } = &self.evidence
        {
            validate_hash(ordered_content_fingerprint)?;
            require(
                snapshot.id == version.id.to_string()
                    && snapshot.fingerprint == version.fingerprint,
                "Materialized training must identify the actual dataset version.",
            )?;
        }
        require(!self.inputs.is_empty(), "Training inputs are missing.")?;
        let mut keys = BTreeSet::new();
        let mut imports = BTreeMap::new();
        for input in &self.inputs {
            validate_relative(&input.key)?;
            validate_hash(&input.fingerprint)?;
            require(
                !input.import_id.is_nil()
                    && input.rows > 0
                    && keys.insert(&input.key)
                    && imports.insert(input.import_id, input).is_none(),
                "Training inputs contain missing or repeated sources.",
            )?;
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        model: &ModelArtifact,
        version: &DatasetVersion,
    ) -> Result<(), Invalid> {
        self.validate_reference(model, &version.reference())?;
        version
            .validate_integrity()
            .map_err(|error| Invalid(error.to_string()))?;
        if let ModelTrainingEvidence::MaterializedTraining {
            ordered_content_fingerprint,
            ..
        } = &self.evidence
        {
            let rows = self
                .inputs
                .iter()
                .try_fold(0_u64, |sum, input| sum.checked_add(input.rows));
            require(
                rows == Some(version.members.len() as u64)
                    && *ordered_content_fingerprint
                        == Self::materialized_content_fingerprint(version)?,
                "Rendered training content or order differs from the dataset version.",
            )?;
            return Ok(());
        }
        let imports = self
            .inputs
            .iter()
            .map(|input| (input.import_id, input))
            .collect::<BTreeMap<_, _>>();
        let mut observed: BTreeMap<Uuid, BTreeSet<u64>> = BTreeMap::new();
        for member in &version.members {
            let input = imports.get(&member.source.import_id).ok_or_else(|| {
                Invalid("Dataset contains an input absent from the training record.".into())
            })?;
            require(
                member.source.artifact_fingerprint == input.fingerprint
                    && member.source.record <= input.rows
                    && member.split == SnapshotSplit::Train,
                "Dataset membership does not match recorded training input.",
            )?;
            require(
                observed
                    .entry(input.import_id)
                    .or_default()
                    .insert(member.source.record),
                "Dataset repeats a training record.",
            )?;
        }
        for input in &self.inputs {
            require(
                observed
                    .get(&input.import_id)
                    .is_some_and(|records| records.len() as u64 == input.rows),
                "Dataset omits recorded training rows.",
            )?;
        }
        Ok(())
    }

    /// The local adapter must independently reproduce this from the native
    /// input records. This digest is not proof of file contents on its own.
    pub fn materialized_content_fingerprint(version: &DatasetVersion) -> Result<String, Invalid> {
        version
            .validate_integrity()
            .map_err(|error| Invalid(error.to_string()))?;
        require(
            !version.members.is_empty()
                && version
                    .members
                    .iter()
                    .all(|row| row.split == SnapshotSplit::Train),
            "Materialized training requires train-only membership.",
        )?;
        Self::ordered_content_fingerprint(
            version
                .members
                .iter()
                .map(|row| row.content_fingerprint.as_str()),
        )
    }

    pub fn ordered_content_fingerprint<'a>(
        contents: impl IntoIterator<Item = &'a str>,
    ) -> Result<String, Invalid> {
        let contents: Vec<_> = contents.into_iter().collect();
        for content in &contents {
            validate_hash(content)?;
        }
        artifact_core::fingerprint(&serde_json::json!({
            "protocol": "materialized-training-order-v1", "contents": contents
        }))
        .map_err(|error| Invalid(error.to_string()))
    }
}
