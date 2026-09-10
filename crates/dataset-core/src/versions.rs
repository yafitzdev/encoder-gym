//! Schema-neutral, immutable dataset branches and row-level version history.
//! Membership is not qualification or permission to train; those remain the
//! responsibility of the existing curation and trainer-input contracts.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{DatasetError, SnapshotSplit};

type Result<T> = std::result::Result<T, DatasetError>;

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(DatasetError::SnapshotIntegrity(message.into()))
    }
}

fn hash<T: Serialize>(value: &T) -> Result<String> {
    artifact_core::fingerprint(value).map_err(|error| DatasetError::Fingerprint(error.to_string()))
}

fn valid_hash(value: &str) -> Result<()> {
    require(
        value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }),
        "Invalid dataset content fingerprint",
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRecord {
    pub import_id: Uuid,
    pub artifact_fingerprint: String,
    /// One-based nonempty JSONL record position in the immutable source.
    pub record: u64,
}

impl SourceRecord {
    fn validate(&self) -> Result<()> {
        require(
            !self.import_id.is_nil() && self.record > 0,
            "Invalid source record identity",
        )?;
        valid_hash(&self.artifact_fingerprint)
    }

    fn row_id(&self) -> Result<String> {
        self.validate()?;
        hash(&(
            "dataset-source-row-v1",
            &self.artifact_fingerprint,
            self.record,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetMember {
    pub id: String,
    pub source: SourceRecord,
    pub content_fingerprint: String,
    pub split: SnapshotSplit,
}

impl DatasetMember {
    pub fn imported(
        source: SourceRecord,
        content_fingerprint: String,
        split: SnapshotSplit,
    ) -> Result<Self> {
        let member = Self {
            id: source.row_id()?,
            source,
            content_fingerprint,
            split,
        };
        member.validate()?;
        Ok(member)
    }

    pub fn validate(&self) -> Result<()> {
        valid_hash(&self.id)?;
        valid_hash(&self.content_fingerprint)?;
        self.source.validate()
    }

    fn validate_new(&self) -> Result<()> {
        self.validate()?;
        require(
            self.id == self.source.row_id()?,
            "A new row must retain its source-derived identity",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetVersionRef {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub project_id: Uuid,
    pub number: u64,
    pub fingerprint: String,
}

impl DatasetVersionRef {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.id.is_nil()
                && !self.dataset_id.is_nil()
                && !self.project_id.is_nil()
                && self.number > 0,
            "Invalid dataset version reference",
        )?;
        valid_hash(&self.fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetBranch {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub origin: Option<DatasetVersionRef>,
    pub created_at: DateTime<Utc>,
}

impl DatasetBranch {
    pub fn new(
        id: Uuid,
        project_id: Uuid,
        name: String,
        origin: Option<DatasetVersionRef>,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        let branch = Self {
            id,
            project_id,
            name: name.trim().into(),
            origin,
            created_at,
        };
        branch.validate()?;
        Ok(branch)
    }

    pub fn validate(&self) -> Result<()> {
        require(
            !self.id.is_nil() && !self.project_id.is_nil(),
            "Invalid dataset identity",
        )?;
        require(
            !self.name.is_empty()
                && self.name == self.name.trim()
                && self.name.chars().count() <= 120
                && !self.name.chars().any(char::is_control),
            "Dataset name must contain 1–120 printable characters",
        )?;
        if let Some(origin) = &self.origin {
            origin.validate()?;
            require(
                origin.project_id == self.project_id && origin.dataset_id != self.id,
                "A variant must fork another dataset in this project",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetChanges {
    pub added: Vec<DatasetMember>,
    pub removed: Vec<String>,
    pub replaced: Vec<DatasetMember>,
}

impl DatasetChanges {
    fn validate(&self) -> Result<()> {
        let mut touched = BTreeSet::new();
        for member in &self.added {
            member.validate_new()?;
            require(touched.insert(&member.id), "A row has multiple changes")?;
        }
        for id in &self.removed {
            valid_hash(id)?;
            require(touched.insert(id), "A row has multiple changes")?;
        }
        for member in &self.replaced {
            member.validate()?;
            require(touched.insert(&member.id), "A row has multiple changes")?;
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.replaced.is_empty()
    }

    pub fn apply(&self, previous: &[DatasetMember]) -> Result<Vec<DatasetMember>> {
        self.validate()?;
        validate_members(previous)?;
        let original: BTreeMap<_, _> = previous.iter().map(|member| (&member.id, member)).collect();
        let removed: BTreeSet<_> = self.removed.iter().collect();
        let replacements: BTreeMap<_, _> = self
            .replaced
            .iter()
            .map(|member| (&member.id, member))
            .collect();
        for id in &removed {
            require(
                original.contains_key(id),
                "Cannot remove a row absent from the parent version",
            )?;
        }
        for (id, replacement) in &replacements {
            let original = original.get(id).ok_or_else(|| {
                DatasetError::SnapshotIntegrity(
                    "Cannot replace a row absent from the parent version".into(),
                )
            })?;
            require(
                original != replacement,
                "Replacement does not change the row",
            )?;
        }
        for addition in &self.added {
            require(
                !original.contains_key(&addition.id),
                "An added row already exists in the parent version",
            )?;
        }
        let mut members = Vec::new();
        for member in previous {
            if !removed.contains(&member.id) {
                members.push(
                    replacements
                        .get(&member.id)
                        .copied()
                        .unwrap_or(member)
                        .clone(),
                );
            }
        }
        members.extend(self.added.iter().cloned());
        require(
            !members.is_empty(),
            "A dataset version must contain at least one row",
        )?;
        validate_members(&members)?;
        Ok(members)
    }
}

fn validate_members(members: &[DatasetMember]) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for member in members {
        member.validate()?;
        require(
            ids.insert(&member.id),
            "Dataset row identities must be unique",
        )?;
        require(
            sources.insert((&member.source.artifact_fingerprint, member.source.record)),
            "A source record can appear only once in a version",
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetVersion {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub project_id: Uuid,
    pub number: u64,
    pub parent: Option<DatasetVersionRef>,
    pub members: Vec<DatasetMember>,
    pub changes: DatasetChanges,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DatasetVersion {
    pub fn initial(
        id: Uuid,
        dataset: &DatasetBranch,
        members: Vec<DatasetMember>,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        dataset.validate()?;
        require(
            dataset.origin.is_none(),
            "A variant must start from its declared parent",
        )?;
        let changes = DatasetChanges {
            added: members.clone(),
            ..Default::default()
        };
        let members = changes.apply(&[])?;
        Self::seal(id, dataset, 1, None, members, changes, created_at)
    }

    pub fn fork(
        id: Uuid,
        dataset: &DatasetBranch,
        parent: &Self,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        dataset.validate()?;
        parent.validate_integrity()?;
        require(
            dataset.origin.as_ref() == Some(&parent.reference()),
            "Variant origin differs from the selected version",
        )?;
        require(
            created_at >= parent.created_at && dataset.created_at >= parent.created_at,
            "A variant cannot precede its source version",
        )?;
        Self::seal(
            id,
            dataset,
            1,
            Some(parent.reference()),
            parent.members.clone(),
            DatasetChanges::default(),
            created_at,
        )
    }

    pub fn revise(
        id: Uuid,
        dataset: &DatasetBranch,
        parent: &Self,
        changes: DatasetChanges,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        dataset.validate()?;
        parent.validate_integrity()?;
        require(
            parent.dataset_id == dataset.id && parent.project_id == dataset.project_id,
            "A new version must belong to its parent dataset",
        )?;
        require(
            created_at >= parent.created_at,
            "A new version cannot precede its parent",
        )?;
        require(!changes.is_empty(), "No dataset changes were supplied")?;
        let members = changes.apply(&parent.members)?;
        let number = parent.number.checked_add(1).ok_or_else(|| {
            DatasetError::SnapshotIntegrity("Dataset version number overflow".into())
        })?;
        Self::seal(
            id,
            dataset,
            number,
            Some(parent.reference()),
            members,
            changes,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn seal(
        id: Uuid,
        dataset: &DatasetBranch,
        number: u64,
        parent: Option<DatasetVersionRef>,
        members: Vec<DatasetMember>,
        changes: DatasetChanges,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        require(
            created_at >= dataset.created_at,
            "A version cannot precede its dataset",
        )?;
        let mut version = Self {
            id,
            dataset_id: dataset.id,
            project_id: dataset.project_id,
            number,
            parent,
            members,
            changes,
            created_at,
            fingerprint: String::new(),
        };
        version.fingerprint = version.compute_fingerprint()?;
        version.validate_integrity()?;
        Ok(version)
    }

    fn compute_fingerprint(&self) -> Result<String> {
        hash(&(
            "dataset-version-v1",
            self.id,
            self.dataset_id,
            self.project_id,
            self.number,
            &self.parent,
            &self.members,
            &self.changes,
            self.created_at,
        ))
    }

    pub fn reference(&self) -> DatasetVersionRef {
        DatasetVersionRef {
            id: self.id,
            dataset_id: self.dataset_id,
            project_id: self.project_id,
            number: self.number,
            fingerprint: self.fingerprint.clone(),
        }
    }

    /// Storage-envelope validation. `verify` additionally reproduces the version
    /// from its parent; persistence must validate the whole linked ancestry.
    pub fn validate_integrity(&self) -> Result<()> {
        self.reference().validate()?;
        require(
            !self.members.is_empty(),
            "A dataset version must contain at least one row",
        )?;
        validate_members(&self.members)?;
        self.changes.validate()?;
        match &self.parent {
            None => require(
                self.number == 1 && self.changes.apply(&[])? == self.members,
                "Invalid initial dataset membership",
            )?,
            Some(parent) => {
                parent.validate()?;
                require(
                    parent.id != self.id && parent.project_id == self.project_id,
                    "Invalid dataset version parent",
                )?;
                if parent.dataset_id == self.dataset_id {
                    require(
                        parent.number.checked_add(1) == Some(self.number)
                            && !self.changes.is_empty(),
                        "Invalid dataset version sequence",
                    )?;
                } else {
                    require(
                        self.number == 1 && self.changes.is_empty(),
                        "A variant starts as an unchanged copy of its origin",
                    )?;
                }
            }
        }
        require(
            self.compute_fingerprint()? == self.fingerprint,
            "Dataset version fingerprint does not reproduce",
        )
    }

    pub fn verify(&self, dataset: &DatasetBranch, parent: Option<&Self>) -> Result<()> {
        // Reconstruct through the validating constructors, then compare the
        // entire artifact (including its fingerprint). Validating this same
        // membership before reconstruction would repeat the complete work.
        require(
            self.dataset_id == dataset.id && self.project_id == dataset.project_id,
            "Version belongs to another dataset",
        )?;
        require(
            self.parent == parent.map(Self::reference),
            "Version parent identity differs from the supplied parent",
        )?;
        let reproduced = match parent {
            None => Self::initial(self.id, dataset, self.members.clone(), self.created_at)?,
            Some(parent) if self.number == 1 => {
                Self::fork(self.id, dataset, parent, self.created_at)?
            }
            Some(parent) => Self::revise(
                self.id,
                dataset,
                parent,
                self.changes.clone(),
                self.created_at,
            )?,
        };
        require(
            reproduced == *self,
            "Dataset changes do not reproduce the recorded version",
        )
    }
}
