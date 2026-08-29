use analysis_core::protocol::AnalysisProtocol;
use dataset_core::domain::SnapshotSplit;
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
}
