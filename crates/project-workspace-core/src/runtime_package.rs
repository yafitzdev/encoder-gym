//! Immutable identity for a project-contained adapter runtime package.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AdapterBinding, BoundIdentity, FileIdentity, Invalid, inventory_fingerprint, require,
    validate_hash, validate_relative,
};

pub const MANAGED_RUNTIME_PACKAGE_ID: &str = "encoder-gym-managed-runtime-package.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimePackageInventory {
    pub files: Vec<FileIdentity>,
    pub bytes: u64,
    pub fingerprint: String,
}

impl RuntimePackageInventory {
    pub fn new(mut files: Vec<FileIdentity>) -> Result<Self, Invalid> {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let bytes = files.iter().try_fold(0_u64, |sum, file| {
            sum.checked_add(file.bytes)
                .ok_or_else(|| Invalid("Runtime package size overflow.".into()))
        })?;
        let fingerprint = inventory_fingerprint(&files);
        let value = Self {
            files,
            bytes,
            fingerprint,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.files.is_empty() && self.files.len() <= 100_000,
            "Runtime package inventory must contain 1–100,000 files.",
        )?;
        let mut previous = "";
        let mut bytes = 0_u64;
        for file in &self.files {
            validate_relative(&file.path)?;
            validate_hash(&file.fingerprint)?;
            require(
                file.path.as_str() > previous,
                "Runtime package paths must be sorted and unique.",
            )?;
            previous = &file.path;
            bytes = bytes
                .checked_add(file.bytes)
                .ok_or_else(|| Invalid("Runtime package size overflow.".into()))?;
        }
        require(
            bytes == self.bytes && inventory_fingerprint(&self.files) == self.fingerprint,
            "Runtime package inventory identity does not reproduce.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedRuntimePackage {
    pub version: u32,
    pub adapter: AdapterBinding,
    pub project_snapshot: BoundIdentity,
    pub source_revision: String,
    pub source_fingerprint: String,
    /// Package-relative adapter working directory.
    pub workspace: String,
    /// Package-relative executable path.
    pub executable: String,
    pub runtime: RuntimePackageInventory,
    pub python: RuntimePackageInventory,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ManagedRuntimePackage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        adapter: AdapterBinding,
        project_snapshot: BoundIdentity,
        source_revision: impl Into<String>,
        source_fingerprint: impl Into<String>,
        workspace: impl Into<String>,
        executable: impl Into<String>,
        runtime: RuntimePackageInventory,
        python: RuntimePackageInventory,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            version: 1,
            adapter,
            project_snapshot,
            source_revision: source_revision.into(),
            source_fingerprint: source_fingerprint.into(),
            workspace: workspace.into(),
            executable: executable.into(),
            runtime,
            python,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn identity(&self) -> BoundIdentity {
        BoundIdentity {
            id: MANAGED_RUNTIME_PACKAGE_ID.into(),
            fingerprint: self.fingerprint.clone(),
        }
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, Invalid> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": self.version,
            "adapter": self.adapter,
            "projectSnapshot": self.project_snapshot,
            "sourceRevision": self.source_revision,
            "sourceFingerprint": self.source_fingerprint,
            "workspace": self.workspace,
            "executable": self.executable,
            "runtime": self.runtime,
            "python": self.python,
        }))
        .map_err(|error| Invalid(format!("Could not fingerprint runtime package: {error}")))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            self.version == 1,
            "Unsupported managed runtime package version.",
        )?;
        self.adapter.validate()?;
        self.project_snapshot
            .validate("Runtime package project snapshot")?;
        require(
            !self.source_revision.trim().is_empty()
                && self.source_revision.len() <= 240
                && !self.source_revision.chars().any(char::is_control),
            "Runtime package source revision is invalid.",
        )?;
        validate_hash(&self.source_fingerprint)?;
        validate_relative(&self.workspace)?;
        validate_relative(&self.executable)?;
        require(
            self.workspace == "workspace" && self.executable.starts_with("python/runtime/"),
            "Managed runtime package layout is not canonical.",
        )?;
        self.runtime.validate()?;
        self.python.validate()?;
        validate_hash(&self.fingerprint)?;
        require(
            self.reproduce_fingerprint()? == self.fingerprint,
            "Managed runtime package fingerprint does not reproduce.",
        )
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    #[test]
    fn package_identity_is_portable_and_content_bound() {
        let inventory = RuntimePackageInventory::new(vec![FileIdentity {
            path: "workspace/nomos/source.py".into(),
            bytes: 4,
            fingerprint: digest('1'),
        }])
        .unwrap();
        let package = ManagedRuntimePackage::new(
            AdapterBinding {
                key: "nomos".into(),
                protocol: "v1".into(),
                configuration_fingerprint: digest('2'),
            },
            BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('3'),
            },
            "abc123",
            digest('4'),
            "workspace",
            "python/runtime/python.exe",
            inventory.clone(),
            RuntimePackageInventory::new(vec![FileIdentity {
                path: "python/runtime/python.exe".into(),
                bytes: 8,
                fingerprint: digest('5'),
            }])
            .unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(package.identity().id, MANAGED_RUNTIME_PACKAGE_ID);
        assert_eq!(
            package.reproduce_fingerprint().unwrap(),
            package.fingerprint
        );
        let mut changed = package;
        changed.runtime.files[0].bytes += 1;
        assert!(changed.validate().is_err());
    }
}
