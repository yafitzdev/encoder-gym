//! Project catalog identity, not benchmark execution or acceptance policy.
use crate::{BoundIdentity, Invalid, require};
use chrono::{DateTime, Utc};
use encoder_experiment_core::benchmark::BenchmarkDefinition;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BenchmarkSource {
    pub scientific_binding: BoundIdentity,
    pub project_snapshot: BoundIdentity,
    pub protocol: BoundIdentity,
}

impl BenchmarkSource {
    pub fn validate(&self) -> Result<(), Invalid> {
        for reference in [
            &self.scientific_binding,
            &self.project_snapshot,
            &self.protocol,
        ] {
            reference.validate("Benchmark source")?;
            require(
                Uuid::parse_str(&reference.id).is_ok_and(|id| !id.is_nil()),
                "Benchmark sources require exact artifact UUIDs.",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectBenchmarkVersion {
    pub id: Uuid,
    pub project_id: Uuid,
    pub number: u64,
    pub parent: Option<BoundIdentity>,
    pub definition: BenchmarkDefinition,
    pub source: BenchmarkSource,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectBenchmarkVersion {
    pub fn create(
        id: Uuid,
        project_id: Uuid,
        parent: Option<&Self>,
        definition: BenchmarkDefinition,
        source: BenchmarkSource,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let number = parent.map_or(Ok(1), |parent| {
            parent
                .number
                .checked_add(1)
                .ok_or_else(|| Invalid("Benchmark version overflow.".into()))
        })?;
        let mut value = Self {
            id,
            project_id,
            number,
            parent: parent.map(|parent| BoundIdentity {
                id: parent.id.to_string(),
                fingerprint: parent.fingerprint.clone(),
            }),
            definition,
            source,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate(parent)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(
            &serde_json::json!({ "id":self.id,"projectId":self.project_id,
            "number":self.number,"parent":self.parent,"definition":self.definition,
            "source":self.source,"createdAt":self.created_at }),
        )
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate(&self, parent: Option<&Self>) -> Result<(), Invalid> {
        self.definition
            .validate_integrity()
            .map_err(|error| Invalid(error.to_string()))?;
        self.source.validate()?;
        require(
            !self.id.is_nil()
                && !self.project_id.is_nil()
                && self.number > 0
                && self.reproduce()? == self.fingerprint,
            "Benchmark version identity changed.",
        )?;
        match (parent, &self.parent) {
            (None, None) => require(
                self.number == 1,
                "Benchmark history must start at version 1.",
            ),
            (Some(parent), Some(reference)) => {
                parent
                    .definition
                    .validate_integrity()
                    .map_err(|error| Invalid(error.to_string()))?;
                parent.source.validate()?;
                reference.validate("Parent benchmark version")?;
                require(
                    !parent.id.is_nil()
                        && parent.reproduce()? == parent.fingerprint
                        && parent.project_id == self.project_id
                        && parent.id != self.id
                        && reference.id == parent.id.to_string()
                        && reference.fingerprint == parent.fingerprint
                        && parent.number.checked_add(1) == Some(self.number)
                        && parent.created_at <= self.created_at
                        && parent.definition.fingerprint != self.definition.fingerprint,
                    "Benchmark history has a stale, foreign, unchanged or invalid parent.",
                )
            }
            _ => Err(Invalid(
                "Benchmark predecessor is missing or unexpected.".into(),
            )),
        }
    }
}
