use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dataset_core::domain::{DatasetSnapshot, SnapshotMember};
use generation_core::domain::{
    BackendConfiguration, DatasetDefinition, GenerationCell, GenerationPlan,
};
use project_config::PersistedProjectConfiguration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::{
    benchmark::BenchmarkSuite,
    contamination::{ContaminationKind, ContaminationReport, ContaminationStatus},
    governance::{CohortRoleDecision, EvaluationCohort},
    workflow::{WorkflowDefinition, WorkflowStage},
};

#[derive(Debug, Clone, PartialEq)]
pub struct CohortEvidence {
    pub snapshot: DatasetSnapshot,
    pub source_dataset: DatasetDefinition,
    pub members: Vec<SnapshotMember>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreparationEvidence {
    pub cohorts: BTreeMap<Uuid, CohortEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTargetPreview {
    pub cell: GenerationCell,
    pub target: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContaminationPreview {
    pub scope: String,
    pub status: ContaminationStatus,
    pub counts: BTreeMap<ContaminationKind, u64>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparationPreview {
    pub manifest_fingerprint: String,
    pub project_name: String,
    pub dataset_name: String,
    pub generation_backend: String,
    pub generation_model: String,
    pub training_backend: String,
    pub requested_total_rows: u32,
    pub initial_target_rows: u32,
    pub reserved_rows: u32,
    pub estimated_initial_requests: u64,
    pub cells: Vec<CellTargetPreview>,
    pub development_cohorts: usize,
    pub sealed_cohorts: usize,
    pub contamination: Vec<ContaminationPreview>,
    pub stages: Vec<WorkflowStage>,
    pub governance_mode: String,
    pub issues: Vec<PreparationIssue>,
    pub eligible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparationBundle {
    pub dataset: DatasetDefinition,
    pub default_generation_plan: GenerationPlan,
    pub backend_configuration: Option<BackendConfiguration>,
    pub project_configuration: PersistedProjectConfiguration,
    pub cohorts: Vec<EvaluationCohort>,
    pub role_decisions: Vec<CohortRoleDecision>,
    pub contamination_reports: Vec<ContaminationReport>,
    pub development_suite: BenchmarkSuite,
    pub sealed_suite: Option<BenchmarkSuite>,
    pub workflow_definition: WorkflowDefinition,
    pub preparation: PreparedProject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedProject {
    pub id: Uuid,
    pub name: String,
    pub manifest_fingerprint: String,
    pub dataset_id: Uuid,
    pub project_configuration_id: Uuid,
    pub development_suite_id: Uuid,
    pub sealed_suite_id: Option<Uuid>,
    pub workflow_definition_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl PreparedProject {
    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        prepared_fingerprint(self)
    }
}

pub(crate) fn prepared_fingerprint(
    value: &PreparedProject,
) -> Result<String, artifact_core::FingerprintError> {
    artifact_core::fingerprint(&serde_json::json!({
        "id": value.id,
        "name": value.name,
        "manifest_fingerprint": value.manifest_fingerprint,
        "dataset_id": value.dataset_id,
        "project_configuration_id": value.project_configuration_id,
        "development_suite_id": value.development_suite_id,
        "sealed_suite_id": value.sealed_suite_id,
        "workflow_definition_id": value.workflow_definition_id,
        "created_at": value.created_at,
    }))
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("project preparation persistence failed: {0}")]
pub struct PreparationStoreError(pub String);
