use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dataset_core::domain::{DatasetImport, DatasetSnapshot, ImportedRow, SnapshotMember};
use generation_core::domain::{
    BackendConfiguration, DatasetDefinition, GenerationCell, GenerationPlan,
};
use project_config::PersistedProjectConfiguration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::{
    benchmark::BenchmarkSuite,
    benchmark_bundle::BenchmarkBundle,
    benchmark_qualification::{
        BenchmarkQualification, BenchmarkQualificationPolicy, BenchmarkQualificationReview,
        BenchmarkReadiness, QualificationIssueSeverity,
    },
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

#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapSourceBundle {
    pub key: String,
    pub content_fingerprint: String,
    pub dataset: DatasetDefinition,
    pub dataset_import: DatasetImport,
    pub imported_rows: Vec<ImportedRow>,
    pub snapshot: DatasetSnapshot,
    pub members: Vec<SnapshotMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapSourcePreview {
    pub key: String,
    pub cohort_name: String,
    pub declared_path: String,
    pub content_fingerprint: String,
    pub processed_rows: u64,
    pub accepted_rows: u64,
    pub rejected_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BootstrapPreview {
    pub bootstrap_fingerprint: String,
    pub sources: Vec<BootstrapSourcePreview>,
    pub preparation: PreparationPreview,
    pub eligible: bool,
    pub issues: Vec<PreparationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapSourceSummary {
    pub key: String,
    pub content_fingerprint: String,
    pub dataset_id: Uuid,
    pub import_id: Uuid,
    pub snapshot_id: Uuid,
    pub accepted_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectBootstrap {
    pub id: Uuid,
    pub name: String,
    pub bootstrap_fingerprint: String,
    pub preparation_id: Uuid,
    pub sources: Vec<BootstrapSourceSummary>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectBootstrap {
    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        bootstrap_fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapBundle {
    pub sources: Vec<BootstrapSourceBundle>,
    pub preparation: PreparationBundle,
    pub bootstrap: ProjectBootstrap,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_supervision:
        Option<generation_supervisor_core::preset::ResolvedGenerationSupervision>,
    pub requested_total_rows: u32,
    pub initial_target_rows: u32,
    pub reserved_rows: u32,
    pub estimated_initial_requests: u64,
    pub cells: Vec<CellTargetPreview>,
    pub development_cohorts: usize,
    pub sealed_cohorts: usize,
    pub contamination: Vec<ContaminationPreview>,
    pub benchmark_qualification: Option<BenchmarkQualificationPreview>,
    pub stages: Vec<WorkflowStage>,
    pub governance_mode: String,
    pub issues: Vec<PreparationIssue>,
    pub eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkQualificationPreview {
    pub readiness: BenchmarkReadiness,
    pub policy: BenchmarkQualificationPolicy,
    pub cohorts: Vec<BenchmarkQualificationCohortPreview>,
    pub issues: Vec<BenchmarkQualificationIssuePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkQualificationCohortPreview {
    pub total_support: u64,
    pub label_support: BTreeMap<String, u64>,
    pub required_slice_support: BTreeMap<String, u64>,
    pub normalized_duplicate_rows: u64,
    pub distinct_producers: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkQualificationIssuePreview {
    pub severity: QualificationIssueSeverity,
    pub code: String,
    pub message: String,
    pub observed: Option<f64>,
    pub required: Option<f64>,
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
    pub benchmark_bundle: BenchmarkBundle,
    pub benchmark_qualification: BenchmarkQualification,
    pub benchmark_qualification_review: BenchmarkQualificationReview,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_bundle_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_bundle_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_qualification_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_qualification_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_qualification_review_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_qualification_review_fingerprint: Option<String>,
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
    let mut document = serde_json::json!({
        "id": value.id,
        "name": value.name,
        "manifest_fingerprint": value.manifest_fingerprint,
        "dataset_id": value.dataset_id,
        "project_configuration_id": value.project_configuration_id,
        "development_suite_id": value.development_suite_id,
        "sealed_suite_id": value.sealed_suite_id,
        "workflow_definition_id": value.workflow_definition_id,
        "created_at": value.created_at,
    });
    if value.benchmark_bundle_id.is_some() || value.benchmark_bundle_fingerprint.is_some() {
        let object = document
            .as_object_mut()
            .expect("prepared project fingerprint document");
        object.insert(
            "benchmark_bundle_id".into(),
            serde_json::json!(value.benchmark_bundle_id),
        );
        object.insert(
            "benchmark_bundle_fingerprint".into(),
            serde_json::json!(value.benchmark_bundle_fingerprint),
        );
    }
    if value.benchmark_qualification_id.is_some()
        || value.benchmark_qualification_fingerprint.is_some()
        || value.benchmark_qualification_review_id.is_some()
        || value.benchmark_qualification_review_fingerprint.is_some()
    {
        let object = document
            .as_object_mut()
            .expect("prepared project fingerprint document");
        object.insert(
            "benchmark_qualification_id".into(),
            serde_json::json!(value.benchmark_qualification_id),
        );
        object.insert(
            "benchmark_qualification_fingerprint".into(),
            serde_json::json!(value.benchmark_qualification_fingerprint),
        );
        object.insert(
            "benchmark_qualification_review_id".into(),
            serde_json::json!(value.benchmark_qualification_review_id),
        );
        object.insert(
            "benchmark_qualification_review_fingerprint".into(),
            serde_json::json!(value.benchmark_qualification_review_fingerprint),
        );
    }
    artifact_core::fingerprint(&document)
}

pub(crate) fn bootstrap_fingerprint(
    value: &ProjectBootstrap,
) -> Result<String, artifact_core::FingerprintError> {
    artifact_core::fingerprint(&serde_json::json!({
        "id": value.id,
        "name": value.name,
        "bootstrap_fingerprint": value.bootstrap_fingerprint,
        "preparation_id": value.preparation_id,
        "sources": value.sources,
        "created_at": value.created_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_preparation_omits_bundle_fields_and_reproduces_its_fingerprint() {
        let mut value = PreparedProject {
            id: Uuid::new_v4(),
            name: "legacy preparation".into(),
            manifest_fingerprint: "sha256:manifest".into(),
            dataset_id: Uuid::new_v4(),
            project_configuration_id: Uuid::new_v4(),
            development_suite_id: Uuid::new_v4(),
            sealed_suite_id: None,
            benchmark_bundle_id: None,
            benchmark_bundle_fingerprint: None,
            benchmark_qualification_id: None,
            benchmark_qualification_fingerprint: None,
            benchmark_qualification_review_id: None,
            benchmark_qualification_review_fingerprint: None,
            workflow_definition_id: Uuid::new_v4(),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = prepared_fingerprint(&value).expect("legacy fingerprint");

        let encoded = serde_json::to_value(&value).expect("legacy JSON");
        let object = encoded.as_object().expect("preparation object");
        assert!(!object.contains_key("benchmark_bundle_id"));
        assert!(!object.contains_key("benchmark_bundle_fingerprint"));
        assert!(!object.contains_key("benchmark_qualification_id"));
        let decoded: PreparedProject = serde_json::from_value(encoded).expect("legacy read");
        assert_eq!(
            decoded.reproduce_fingerprint().expect("reproduce"),
            decoded.fingerprint
        );
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("project preparation persistence failed: {0}")]
pub struct PreparationStoreError(pub String);
