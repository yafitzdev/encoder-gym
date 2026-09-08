//! Managed model catalog and active-baseline history.
//!
//! These records contain project-level references only. Training, evaluation,
//! acceptance, and promotion policy remain owned by their scientific slices.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Invalid, LocalModel, require, validate_hash, validate_name, validate_relative};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelOrigin {
    Imported,
    Trained,
    Transformed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundIdentity {
    pub id: String,
    pub fingerprint: String,
}

impl BoundIdentity {
    pub fn validate(&self, label: &str) -> Result<(), Invalid> {
        require(
            !self.id.trim().is_empty()
                && self.id.chars().count() <= 240
                && !self.id.chars().any(char::is_control),
            &format!("{label} identity is invalid."),
        )?;
        validate_hash(&self.fingerprint)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelArtifact {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub origin: ModelOrigin,
    pub path: String,
    pub format: String,
    pub bytes: u64,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_model_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producing_run: Option<BoundIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_snapshot: Option<BoundIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trainer: Option<BoundIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_configuration_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
}

impl ModelArtifact {
    pub fn imported_baseline(
        project_id: Uuid,
        id: Uuid,
        name: impl Into<String>,
        model: &LocalModel,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let value = Self {
            id,
            project_id,
            name: name.into(),
            created_at,
            origin: ModelOrigin::Imported,
            path: crate::BASELINE.into(),
            format: model.format.clone(),
            bytes: model.bytes,
            fingerprint: model.fingerprint.clone(),
            parent_model_id: None,
            producing_run: None,
            training_snapshot: None,
            trainer: None,
            effective_configuration_fingerprint: None,
            tokenizer_fingerprint: model
                .files
                .iter()
                .find(|file| file.path == "tokenizer.json")
                .map(|file| file.fingerprint.clone()),
            source_revision: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.id.is_nil() && !self.project_id.is_nil(),
            "Model and project identities cannot be empty.",
        )?;
        validate_name(&self.name)?;
        validate_relative(&self.path)?;
        require(
            self.path == crate::BASELINE
                || self.path.starts_with("models/candidates/")
                || self.path.starts_with("models/archive/"),
            "Model artifacts must remain in a managed model directory.",
        )?;
        require(
            !self.format.trim().is_empty()
                && self.format.chars().count() <= 120
                && !self.format.chars().any(char::is_control)
                && self.bytes > 0,
            "Model format or size is invalid.",
        )?;
        validate_hash(&self.fingerprint)?;
        if let Some(value) = &self.producing_run {
            value.validate("Producing run")?;
        }
        if let Some(value) = &self.training_snapshot {
            value.validate("Training snapshot")?;
        }
        if let Some(value) = &self.trainer {
            value.validate("Trainer")?;
        }
        for fingerprint in [
            self.effective_configuration_fingerprint.as_deref(),
            self.tokenizer_fingerprint.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_hash(fingerprint)?;
        }
        if let Some(revision) = &self.source_revision {
            require(
                !revision.trim().is_empty()
                    && revision.chars().count() <= 240
                    && !revision.chars().any(char::is_control),
                "Model source revision is invalid.",
            )?;
        }
        match self.origin {
            ModelOrigin::Imported => {}
            ModelOrigin::Trained => {
                require(
                    self.producing_run.is_some()
                        && self.training_snapshot.is_some()
                        && self.trainer.is_some()
                        && self.effective_configuration_fingerprint.is_some(),
                    "A trained model requires its run, snapshot, trainer, and effective configuration.",
                )?;
            }
            ModelOrigin::Transformed => {
                require(
                    self.parent_model_id.is_some()
                        && self.producing_run.is_some()
                        && self.trainer.is_some()
                        && self.effective_configuration_fingerprint.is_some(),
                    "A transformed model requires its parent, run, backend, and effective configuration.",
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BaselineChange {
    Initialization {
        source_fingerprint: String,
    },
    Promotion {
        decision_id: String,
        decision_fingerprint: String,
    },
    Restoration {
        target_revision_id: Uuid,
    },
}

impl BaselineChange {
    fn validate(&self) -> Result<(), Invalid> {
        match self {
            Self::Initialization { source_fingerprint } => validate_hash(source_fingerprint),
            Self::Promotion {
                decision_id,
                decision_fingerprint,
            } => {
                require(
                    !decision_id.trim().is_empty()
                        && decision_id.chars().count() <= 240
                        && !decision_id.chars().any(char::is_control),
                    "Promotion decision identity is invalid.",
                )?;
                validate_hash(decision_fingerprint)
            }
            Self::Restoration { target_revision_id } => require(
                !target_revision_id.is_nil(),
                "Restoration target revision cannot be empty.",
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaselineRevision {
    pub id: Uuid,
    pub project_id: Uuid,
    pub sequence: u64,
    pub model_artifact_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_revision_id: Option<Uuid>,
    pub change: BaselineChange,
    pub actor: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

struct BaselineRevisionSpec {
    id: Uuid,
    project_id: Uuid,
    sequence: u64,
    model_artifact_id: Uuid,
    previous_revision_id: Option<Uuid>,
    change: BaselineChange,
    actor: String,
    reason: String,
    created_at: DateTime<Utc>,
}

impl BaselineRevision {
    fn build(spec: BaselineRevisionSpec) -> Result<Self, Invalid> {
        let mut value = Self {
            id: spec.id,
            project_id: spec.project_id,
            sequence: spec.sequence,
            model_artifact_id: spec.model_artifact_id,
            previous_revision_id: spec.previous_revision_id,
            change: spec.change,
            actor: spec.actor,
            reason: spec.reason,
            created_at: spec.created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn initialize(
        id: Uuid,
        project_id: Uuid,
        model_artifact_id: Uuid,
        source_fingerprint: String,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::build(BaselineRevisionSpec {
            id,
            project_id,
            sequence: 1,
            model_artifact_id,
            previous_revision_id: None,
            change: BaselineChange::Initialization { source_fingerprint },
            actor: "encoder-gym".into(),
            reason: "Imported workspace baseline".into(),
            created_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn promotion(
        id: Uuid,
        project_id: Uuid,
        sequence: u64,
        model_artifact_id: Uuid,
        previous_revision_id: Uuid,
        decision_id: String,
        decision_fingerprint: String,
        actor: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::build(BaselineRevisionSpec {
            id,
            project_id,
            sequence,
            model_artifact_id,
            previous_revision_id: Some(previous_revision_id),
            change: BaselineChange::Promotion {
                decision_id,
                decision_fingerprint,
            },
            actor: actor.into(),
            reason: reason.into(),
            created_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restoration(
        id: Uuid,
        project_id: Uuid,
        sequence: u64,
        model_artifact_id: Uuid,
        previous_revision_id: Uuid,
        target_revision_id: Uuid,
        actor: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::build(BaselineRevisionSpec {
            id,
            project_id,
            sequence,
            model_artifact_id,
            previous_revision_id: Some(previous_revision_id),
            change: BaselineChange::Restoration { target_revision_id },
            actor: actor.into(),
            reason: reason.into(),
            created_at,
        })
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, Invalid> {
        let value = serde_json::json!({
            "id": self.id,
            "projectId": self.project_id,
            "sequence": self.sequence,
            "modelArtifactId": self.model_artifact_id,
            "previousRevisionId": self.previous_revision_id,
            "change": self.change,
            "actor": self.actor,
            "reason": self.reason,
            "createdAt": self.created_at,
        });
        let bytes = serde_json::to_vec(&value).map_err(|error| {
            Invalid(format!("Could not fingerprint baseline revision: {error}"))
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.id.is_nil() && !self.project_id.is_nil() && !self.model_artifact_id.is_nil(),
            "Baseline revision identities cannot be empty.",
        )?;
        require(
            self.sequence > 0,
            "Baseline revision sequence must be positive.",
        )?;
        validate_name(&self.actor)?;
        validate_name(&self.reason)?;
        self.change.validate()?;
        match self.change {
            BaselineChange::Initialization { .. } => require(
                self.sequence == 1 && self.previous_revision_id.is_none(),
                "Baseline initialization must be the first revision.",
            )?,
            BaselineChange::Promotion { .. } | BaselineChange::Restoration { .. } => require(
                self.sequence > 1 && self.previous_revision_id.is_some(),
                "A baseline change must follow a prior revision.",
            )?,
        }
        validate_hash(&self.fingerprint)?;
        require(
            self.reproduce_fingerprint()? == self.fingerprint,
            "Baseline revision fingerprint does not reproduce.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCatalog {
    pub project_id: Uuid,
    pub artifacts: Vec<ModelArtifact>,
    pub baseline_revisions: Vec<BaselineRevision>,
    pub active_baseline_revision_id: Uuid,
}

impl ModelCatalog {
    pub fn initialize(
        project_id: Uuid,
        artifact: ModelArtifact,
        revision_id: Uuid,
        source_fingerprint: String,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        require(
            artifact.project_id == project_id,
            "Initial model belongs to another project.",
        )?;
        let revision = BaselineRevision::initialize(
            revision_id,
            project_id,
            artifact.id,
            source_fingerprint,
            created_at,
        )?;
        let value = Self {
            project_id,
            artifacts: vec![artifact],
            baseline_revisions: vec![revision],
            active_baseline_revision_id: revision_id,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn active_revision(&self) -> &BaselineRevision {
        self.baseline_revisions
            .iter()
            .find(|revision| revision.id == self.active_baseline_revision_id)
            .expect("validated model catalog has an active revision")
    }

    pub fn active_model(&self) -> &ModelArtifact {
        let id = self.active_revision().model_artifact_id;
        self.artifacts
            .iter()
            .find(|artifact| artifact.id == id)
            .expect("validated model catalog has an active model")
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.project_id.is_nil(),
            "Project identity cannot be empty.",
        )?;
        require(
            !self.artifacts.is_empty() && !self.baseline_revisions.is_empty(),
            "A model catalog requires an artifact and baseline revision.",
        )?;
        let mut artifact_ids = BTreeSet::new();
        for artifact in &self.artifacts {
            artifact.validate()?;
            require(
                artifact.project_id == self.project_id && artifact_ids.insert(artifact.id),
                "Model artifacts must be unique and belong to this project.",
            )?;
            if let Some(parent) = artifact.parent_model_id {
                require(
                    artifact_ids.contains(&parent)
                        || self.artifacts.iter().any(|value| value.id == parent),
                    "Model parent is absent from the project catalog.",
                )?;
            }
        }
        let mut revision_ids = BTreeSet::new();
        let mut previous = None;
        for (index, revision) in self.baseline_revisions.iter().enumerate() {
            revision.validate()?;
            require(
                revision.project_id == self.project_id
                    && revision.sequence == index as u64 + 1
                    && revision.previous_revision_id == previous
                    && revision_ids.insert(revision.id)
                    && artifact_ids.contains(&revision.model_artifact_id),
                "Baseline revisions must form one ordered project-local chain.",
            )?;
            if let BaselineChange::Restoration { target_revision_id } = revision.change {
                require(
                    revision_ids.contains(&target_revision_id) && target_revision_id != revision.id,
                    "A restoration must reference an earlier baseline revision.",
                )?;
                let target = self
                    .baseline_revisions
                    .iter()
                    .find(|value| value.id == target_revision_id)
                    .expect("target was found in the preceding revision set");
                require(
                    target.model_artifact_id == revision.model_artifact_id,
                    "A restoration must restore the target revision's model.",
                )?;
            }
            previous = Some(revision.id);
        }
        require(
            previous == Some(self.active_baseline_revision_id),
            "The active baseline must be the latest revision.",
        )
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::FileIdentity;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn local_model() -> LocalModel {
        let mut files = vec![
            FileIdentity {
                path: "config.json".into(),
                bytes: 1,
                fingerprint: digest('1'),
            },
            FileIdentity {
                path: "model.safetensors".into(),
                bytes: 2,
                fingerprint: digest('2'),
            },
            FileIdentity {
                path: "tokenizer.json".into(),
                bytes: 3,
                fingerprint: digest('3'),
            },
        ];
        files.sort_by(|left, right| left.path.cmp(&right.path));
        LocalModel {
            source: "C:/source".into(),
            format: "safetensors-encoder".into(),
            architecture: "bert".into(),
            bytes: 6,
            fingerprint: crate::inventory_fingerprint(&files),
            files,
            execution: "not-configured".into(),
        }
    }

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap()
    }

    #[test]
    fn imported_baseline_initializes_one_active_revision() {
        let project_id = Uuid::new_v4();
        let artifact = ModelArtifact::imported_baseline(
            project_id,
            Uuid::new_v4(),
            "Imported baseline",
            &local_model(),
            at(),
        )
        .unwrap();
        let catalog = ModelCatalog::initialize(
            project_id,
            artifact.clone(),
            Uuid::new_v4(),
            artifact.fingerprint.clone(),
            at(),
        )
        .unwrap();
        assert_eq!(catalog.active_model(), &artifact);
        assert_eq!(catalog.baseline_revisions.len(), 1);
        assert!(matches!(
            catalog.active_revision().change,
            BaselineChange::Initialization { .. }
        ));
    }

    #[test]
    fn trained_and_transformed_artifacts_require_reproducibility_bindings() {
        let project_id = Uuid::new_v4();
        let mut artifact = ModelArtifact::imported_baseline(
            project_id,
            Uuid::new_v4(),
            "candidate",
            &local_model(),
            at(),
        )
        .unwrap();
        artifact.path = format!("models/candidates/{}/model", artifact.id);
        artifact.origin = ModelOrigin::Trained;
        assert!(artifact.validate().is_err());
        artifact.producing_run = Some(BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: digest('4'),
        });
        artifact.training_snapshot = Some(BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: digest('5'),
        });
        artifact.trainer = Some(BoundIdentity {
            id: "fake-trainer-v1".into(),
            fingerprint: digest('6'),
        });
        artifact.effective_configuration_fingerprint = Some(digest('7'));
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn catalog_rejects_stale_or_forked_baseline_chains() {
        let project_id = Uuid::new_v4();
        let artifact = ModelArtifact::imported_baseline(
            project_id,
            Uuid::new_v4(),
            "baseline",
            &local_model(),
            at(),
        )
        .unwrap();
        let mut catalog = ModelCatalog::initialize(
            project_id,
            artifact.clone(),
            Uuid::new_v4(),
            artifact.fingerprint.clone(),
            at(),
        )
        .unwrap();
        let stale = Uuid::new_v4();
        catalog.active_baseline_revision_id = stale;
        assert!(catalog.validate().is_err());
    }

    #[test]
    fn restoration_must_point_to_the_model_from_an_earlier_revision() {
        let project_id = Uuid::new_v4();
        let baseline = ModelArtifact::imported_baseline(
            project_id,
            Uuid::new_v4(),
            "baseline",
            &local_model(),
            at(),
        )
        .unwrap();
        let first_id = Uuid::new_v4();
        let mut catalog = ModelCatalog::initialize(
            project_id,
            baseline.clone(),
            first_id,
            baseline.fingerprint.clone(),
            at(),
        )
        .unwrap();
        let mut candidate = baseline.clone();
        candidate.id = Uuid::new_v4();
        candidate.name = "candidate".into();
        candidate.path = format!("models/candidates/{}/model", candidate.id);
        candidate.fingerprint = digest('8');
        let promoted_id = Uuid::new_v4();
        let promoted = BaselineRevision::promotion(
            promoted_id,
            project_id,
            2,
            candidate.id,
            first_id,
            "decision-1".into(),
            digest('9'),
            "operator",
            "accepted all gates",
            at(),
        )
        .unwrap();
        let restored_id = Uuid::new_v4();
        let restored = BaselineRevision::restoration(
            restored_id,
            project_id,
            3,
            baseline.id,
            promoted_id,
            first_id,
            "operator",
            "restore prior baseline",
            at(),
        )
        .unwrap();
        catalog.artifacts.push(candidate);
        catalog.baseline_revisions.extend([promoted, restored]);
        catalog.active_baseline_revision_id = restored_id;
        catalog.validate().unwrap();

        catalog.baseline_revisions[2].model_artifact_id = catalog.artifacts[1].id;
        catalog.baseline_revisions[2].fingerprint = catalog.baseline_revisions[2]
            .reproduce_fingerprint()
            .unwrap();
        assert!(catalog.validate().is_err());
    }
}
