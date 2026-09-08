//! Explicit binding from a managed project to slice-owned scientific authority.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{BoundIdentity, Invalid, require, validate_hash, validate_name, validate_relative};

fn safe_identifier(value: &str, label: &str) -> Result<(), Invalid> {
    require(
        !value.trim().is_empty()
            && value.chars().count() <= 240
            && !value.chars().any(char::is_control),
        &format!("{label} is invalid."),
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterBinding {
    pub key: String,
    pub protocol: String,
    pub configuration_fingerprint: String,
}

impl AdapterBinding {
    pub fn validate(&self) -> Result<(), Invalid> {
        safe_identifier(&self.key, "Adapter key")?;
        safe_identifier(&self.protocol, "Adapter protocol")?;
        validate_hash(&self.configuration_fingerprint)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    Managed,
    ExternalIsolated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBinding {
    pub kind: RuntimeKind,
    /// Managed paths are project-relative. External paths are local execution
    /// dependencies and must be revalidated by the owning adapter.
    pub location: String,
    pub project_snapshot: BoundIdentity,
}

impl RuntimeBinding {
    pub fn validate(&self) -> Result<(), Invalid> {
        match self.kind {
            RuntimeKind::Managed => validate_relative(&self.location)?,
            RuntimeKind::ExternalIsolated => {
                safe_identifier(&self.location, "External runtime location")?
            }
        }
        self.project_snapshot.validate("Runtime project snapshot")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScientificStoreBinding {
    /// Always relative to the managed project. The scientific adapter owns its
    /// schema and contents; project.sqlite stores only this reference.
    pub database_path: String,
    pub schema: BoundIdentity,
}

impl ScientificStoreBinding {
    pub fn validate(&self) -> Result<(), Invalid> {
        validate_relative(&self.database_path)?;
        require(
            self.database_path.starts_with("runs/") && self.database_path.ends_with(".sqlite"),
            "Scientific databases must use a contained runs/*.sqlite path.",
        )?;
        self.schema.validate("Scientific store schema")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScientificBinding {
    pub id: Uuid,
    pub project_id: Uuid,
    pub baseline_revision_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_binding_id: Option<Uuid>,
    pub adapter: AdapterBinding,
    pub runtime: RuntimeBinding,
    pub store: ScientificStoreBinding,
    pub actor: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub specification_fingerprint: String,
    pub fingerprint: String,
}

impl ScientificBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        project_id: Uuid,
        baseline_revision_id: Uuid,
        previous_binding_id: Option<Uuid>,
        adapter: AdapterBinding,
        runtime: RuntimeBinding,
        store: ScientificStoreBinding,
        actor: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            project_id,
            baseline_revision_id,
            previous_binding_id,
            adapter,
            runtime,
            store,
            actor: actor.into(),
            reason: reason.into(),
            created_at,
            specification_fingerprint: String::new(),
            fingerprint: String::new(),
        };
        value.specification_fingerprint = value.reproduce_specification_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, Invalid> {
        fingerprint(&serde_json::json!({
            "projectId": self.project_id,
            "baselineRevisionId": self.baseline_revision_id,
            "adapter": self.adapter,
            "runtime": self.runtime,
            "store": self.store,
        }))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, Invalid> {
        fingerprint(&serde_json::json!({
            "id": self.id,
            "projectId": self.project_id,
            "baselineRevisionId": self.baseline_revision_id,
            "previousBindingId": self.previous_binding_id,
            "adapter": self.adapter,
            "runtime": self.runtime,
            "store": self.store,
            "actor": self.actor,
            "reason": self.reason,
            "createdAt": self.created_at,
            "specificationFingerprint": self.specification_fingerprint,
        }))
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.id.is_nil()
                && !self.project_id.is_nil()
                && !self.baseline_revision_id.is_nil()
                && self.previous_binding_id != Some(self.id),
            "Scientific binding identities are invalid.",
        )?;
        self.adapter.validate()?;
        self.runtime.validate()?;
        self.store.validate()?;
        validate_name(&self.actor)?;
        validate_name(&self.reason)?;
        validate_hash(&self.specification_fingerprint)?;
        validate_hash(&self.fingerprint)?;
        require(
            self.reproduce_specification_fingerprint()? == self.specification_fingerprint,
            "Scientific binding specification fingerprint does not reproduce.",
        )?;
        require(
            self.reproduce_fingerprint()? == self.fingerprint,
            "Scientific binding fingerprint does not reproduce.",
        )
    }
}

fn fingerprint(value: &serde_json::Value) -> Result<String, Invalid> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Invalid(format!("Could not fingerprint scientific binding: {error}")))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn binding() -> ScientificBinding {
        ScientificBinding::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            AdapterBinding {
                key: "nomos".into(),
                protocol: "nomos-production-v1".into(),
                configuration_fingerprint: digest('1'),
            },
            RuntimeBinding {
                kind: RuntimeKind::ExternalIsolated,
                location: "C:/isolated/nomos".into(),
                project_snapshot: BoundIdentity {
                    id: "revision".into(),
                    fingerprint: digest('2'),
                },
            },
            ScientificStoreBinding {
                database_path: "runs/scientific.sqlite".into(),
                schema: BoundIdentity {
                    id: "production-schema-v1".into(),
                    fingerprint: digest('3'),
                },
            },
            "operator",
            "configure Nomos runtime",
            Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn binding_is_content_bound_to_baseline_runtime_and_store() {
        let value = binding();
        assert_eq!(
            value.reproduce_specification_fingerprint().unwrap(),
            value.specification_fingerprint
        );
        assert_eq!(value.reproduce_fingerprint().unwrap(), value.fingerprint);
        let mut changed = value.clone();
        changed.baseline_revision_id = Uuid::new_v4();
        assert!(changed.validate().is_err());
    }

    #[test]
    fn store_cannot_escape_or_replace_the_custody_database() {
        let mut value = binding();
        value.store.database_path = "project.sqlite".into();
        value.specification_fingerprint = value.reproduce_specification_fingerprint().unwrap();
        value.fingerprint = value.reproduce_fingerprint().unwrap();
        assert!(value.validate().is_err());
        value.store.database_path = "../outside.sqlite".into();
        value.specification_fingerprint = value.reproduce_specification_fingerprint().unwrap();
        value.fingerprint = value.reproduce_fingerprint().unwrap();
        assert!(value.validate().is_err());
    }
}
