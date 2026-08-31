use std::{collections::BTreeMap, path::PathBuf};

use analysis_core::protocol::AnalysisProtocol;
use dataset_core::domain::{ImportFormat, SnapshotSplit};
use evaluation_core::domain::EvaluationProtocol;
use optimization_core::protocol::OptimizationProtocol;
use project_config::ProjectConfig;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use workflow_core::{
    advisor::AdvisorConfiguration,
    allocation::InitialAllocationPolicy,
    benchmark::AcceptanceContract,
    contamination::ContaminationPolicy,
    governance::{CohortOrigin, CohortRole, DisclosureLevel},
    workflow::{IterationGovernance, TrainingIterationPolicy, WorkflowBudget, WorkflowPolicy},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationManifest {
    pub version: u32,
    pub name: String,
    pub project: ProjectConfig,
    #[serde(default)]
    pub contamination: ContaminationManifest,
    pub development: SuiteManifest,
    #[serde(default)]
    pub sealed: Option<SuiteManifest>,
    pub workflow: WorkflowManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapManifest {
    pub version: u32,
    pub name: String,
    pub project: ProjectConfig,
    #[serde(default)]
    pub contamination: ContaminationManifest,
    pub development: BootstrapSuiteManifest,
    #[serde(default)]
    pub sealed: Option<BootstrapSuiteManifest>,
    pub workflow: WorkflowManifest,
}

impl BootstrapManifest {
    pub fn parse_toml(source: &str) -> Result<Self, ManifestError> {
        toml::from_str(source).map_err(|error| ManifestError::Parse(error.to_string()))
    }

    pub fn parse_json(source: &str) -> Result<Self, ManifestError> {
        serde_json::from_str(source).map_err(|error| ManifestError::Parse(error.to_string()))
    }

    pub fn source_keys(&self) -> Vec<String> {
        self.development
            .cohorts
            .iter()
            .enumerate()
            .map(|(index, _)| format!("development:{index}"))
            .chain(
                self.sealed
                    .iter()
                    .flat_map(|suite| suite.cohorts.iter().enumerate())
                    .map(|(index, _)| format!("sealed:{index}")),
            )
            .collect()
    }

    pub fn fingerprint(
        &self,
        source_fingerprints: &BTreeMap<String, String>,
    ) -> Result<String, ManifestError> {
        let expected = self.source_keys();
        if expected.len() != source_fingerprints.len()
            || expected
                .iter()
                .any(|key| !source_fingerprints.contains_key(key))
        {
            return Err(ManifestError::BootstrapSources(
                "source fingerprints do not exactly match declared cohorts".into(),
            ));
        }
        artifact_core::fingerprint(&serde_json::json!({
            "manifest": self,
            "source_fingerprints": source_fingerprints,
        }))
        .map_err(|error| ManifestError::Fingerprint(error.to_string()))
    }

    pub fn resolve(
        &self,
        snapshot_ids: &BTreeMap<String, Uuid>,
    ) -> Result<PreparationManifest, ManifestError> {
        let expected = self.source_keys();
        if expected.len() != snapshot_ids.len()
            || expected.iter().any(|key| !snapshot_ids.contains_key(key))
        {
            return Err(ManifestError::BootstrapSources(
                "snapshot identities do not exactly match declared cohorts".into(),
            ));
        }
        Ok(PreparationManifest {
            version: self.version,
            name: self.name.clone(),
            project: self.project.clone(),
            contamination: self.contamination.clone(),
            development: self.development.resolve("development", snapshot_ids)?,
            sealed: self
                .sealed
                .as_ref()
                .map(|suite| suite.resolve("sealed", snapshot_ids))
                .transpose()?,
            workflow: self.workflow.clone(),
        })
    }
}

impl PreparationManifest {
    pub fn parse_toml(source: &str) -> Result<Self, ManifestError> {
        toml::from_str(source).map_err(|error| ManifestError::Parse(error.to_string()))
    }

    pub fn parse_json(source: &str) -> Result<Self, ManifestError> {
        serde_json::from_str(source).map_err(|error| ManifestError::Parse(error.to_string()))
    }

    pub fn fingerprint(&self) -> Result<String, ManifestError> {
        artifact_core::fingerprint(self)
            .map_err(|error| ManifestError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContaminationManifest {
    pub group_dimension: Option<String>,
    pub policy: ContaminationPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteManifest {
    pub name: String,
    #[serde(default)]
    pub required_model_formats: Vec<String>,
    pub cohorts: Vec<CohortManifest>,
    pub contract: AcceptanceContract,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSuiteManifest {
    pub name: String,
    #[serde(default)]
    pub required_model_formats: Vec<String>,
    pub cohorts: Vec<BootstrapCohortManifest>,
    pub contract: AcceptanceContract,
}

impl BootstrapSuiteManifest {
    fn resolve(
        &self,
        scope: &str,
        snapshot_ids: &BTreeMap<String, Uuid>,
    ) -> Result<SuiteManifest, ManifestError> {
        Ok(SuiteManifest {
            name: self.name.clone(),
            required_model_formats: self.required_model_formats.clone(),
            cohorts: self
                .cohorts
                .iter()
                .enumerate()
                .map(|(index, cohort)| {
                    let key = format!("{scope}:{index}");
                    Ok(CohortManifest {
                        name: cohort.name.clone(),
                        snapshot_id: *snapshot_ids.get(&key).ok_or_else(|| {
                            ManifestError::BootstrapSources(format!(
                                "missing snapshot identity for {key}"
                            ))
                        })?,
                        split: SnapshotSplit::Test,
                        origin: cohort.origin,
                        role: cohort.role,
                        protocol: cohort.protocol.clone(),
                        disclosure: cohort.disclosure,
                        adaptation_eligible: cohort.adaptation_eligible,
                    })
                })
                .collect::<Result<_, ManifestError>>()?,
            contract: self.contract.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapCohortManifest {
    pub name: String,
    pub source: LocalSourceManifest,
    pub origin: CohortOrigin,
    pub role: CohortRole,
    #[serde(default)]
    pub protocol: Option<EvaluationProtocol>,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSourceManifest {
    pub path: PathBuf,
    #[serde(default)]
    pub format: Option<ImportFormat>,
    #[serde(default = "default_text_field")]
    pub text_field: String,
    #[serde(default = "default_label_field")]
    pub label_field: String,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}

fn default_text_field() -> String {
    "text".into()
}

fn default_label_field() -> String {
    "label".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortManifest {
    pub name: String,
    pub snapshot_id: Uuid,
    pub split: SnapshotSplit,
    pub origin: CohortOrigin,
    pub role: CohortRole,
    #[serde(default)]
    pub protocol: Option<EvaluationProtocol>,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowManifest {
    pub name: String,
    pub total_rows: u64,
    #[serde(default)]
    pub reserved_rows: u64,
    pub allocation_policy: InitialAllocationPolicy,
    #[serde(default)]
    pub allocation_constraints: Vec<workflow_core::allocation::InitialCellConstraint>,
    #[serde(default)]
    pub analysis_protocol: Option<AnalysisProtocol>,
    #[serde(default)]
    pub optimization_protocol: Option<OptimizationProtocol>,
    #[serde(default)]
    pub advisor: Option<AdvisorConfiguration>,
    #[serde(default)]
    pub training_iteration_policy: Option<TrainingIterationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_gate: Option<workflow_core::workflow::WorkflowQualityGateRequest>,
    pub governance: IterationGovernance,
    pub budget: WorkflowBudget,
    pub policy: WorkflowPolicy,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ManifestError {
    #[error("invalid preparation manifest: {0}")]
    Parse(String),
    #[error("could not fingerprint preparation manifest: {0}")]
    Fingerprint(String),
    #[error("invalid bootstrap sources: {0}")]
    BootstrapSources(String),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dataset_core::domain::SnapshotSplit;
    use uuid::Uuid;

    use super::BootstrapManifest;

    #[test]
    fn bootstrap_manifest_is_strict_content_bound_and_resolves_without_uuid_plumbing() {
        let source = include_str!("../../../examples/pilot-support/project-bootstrap.toml");
        let manifest = BootstrapManifest::parse_toml(source).expect("pilot manifest");
        assert_eq!(manifest.source_keys(), vec!["development:0", "sealed:0"]);
        let fingerprints = BTreeMap::from([
            ("development:0".into(), "sha256:development".into()),
            ("sealed:0".into(), "sha256:sealed".into()),
        ]);
        let first = manifest
            .fingerprint(&fingerprints)
            .expect("bootstrap fingerprint");
        let mut changed = fingerprints.clone();
        changed.insert("sealed:0".into(), "sha256:changed".into());
        assert_ne!(
            first,
            manifest.fingerprint(&changed).expect("changed fingerprint")
        );
        let snapshots = BTreeMap::from([
            ("development:0".into(), Uuid::new_v4()),
            ("sealed:0".into(), Uuid::new_v4()),
        ]);
        let resolved = manifest.resolve(&snapshots).expect("resolved preparation");
        assert_eq!(resolved.development.cohorts[0].split, SnapshotSplit::Test);
        assert_eq!(
            resolved.sealed.expect("sealed suite").cohorts[0].snapshot_id,
            snapshots["sealed:0"]
        );

        let unknown = source.replace(
            "path = \"development.jsonl\"",
            "path = \"development.jsonl\"\nunknown_setting = true",
        );
        assert!(BootstrapManifest::parse_toml(&unknown).is_err());
    }
}
