use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderExperimentError, canonical_sha256,
    domain::{EvidenceRole, ExternalProjectSnapshot, OptimizationBudget, TrainingCandidate},
    fingerprint,
    metrics::{EvaluationReport, MetricContract},
    required,
};

pub const LEGACY_EXPERIMENT_PROTOCOL_SCHEMA_VERSION: u32 = 1;
pub const EXPERIMENT_PROTOCOL_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentSelectionRule {
    MaximizeWorstSuiteThenMean,
}

/// Immutable authority and evidence pinned before an adaptive candidate run starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentProtocol {
    pub schema_version: u32,
    pub id: Uuid,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub metric_contract: MetricContract,
    pub baseline_development_report: EvaluationReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_baseline_development_reports: Vec<EvaluationReport>,
    pub baseline_sealed_report: EvaluationReport,
    pub budget: OptimizationBudget,
    pub maximum_evaluation_seconds: u64,
    pub development_suite_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development_selection_rule: Option<DevelopmentSelectionRule>,
    pub sealed_suite_key: String,
    pub candidates: Vec<TrainingCandidate>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ExperimentProtocol {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project: &ExternalProjectSnapshot,
        metric_contract: MetricContract,
        baseline_development_report: EvaluationReport,
        baseline_sealed_report: EvaluationReport,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        development_suite_key: impl Into<String>,
        sealed_suite_key: impl Into<String>,
        mut candidates: Vec<TrainingCandidate>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        candidates.sort_by_key(|candidate| candidate.sequence);
        let mut value = Self {
            schema_version: LEGACY_EXPERIMENT_PROTOCOL_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            metric_contract,
            baseline_development_report,
            additional_baseline_development_reports: vec![],
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            development_suite_key: required(development_suite_key, "development suite key")?,
            development_selection_rule: None,
            sealed_suite_key: required(sealed_suite_key, "sealed suite key")?,
            candidates,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields(project)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_multi(
        project: &ExternalProjectSnapshot,
        metric_contract: MetricContract,
        baseline_development_reports: Vec<EvaluationReport>,
        baseline_sealed_report: EvaluationReport,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        sealed_suite_key: impl Into<String>,
        candidates: Vec<TrainingCandidate>,
        selection_rule: DevelopmentSelectionRule,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        Self::create_multi_identified(
            Uuid::new_v4(),
            project,
            metric_contract,
            baseline_development_reports,
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            sealed_suite_key,
            candidates,
            selection_rule,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_multi_identified(
        id: Uuid,
        project: &ExternalProjectSnapshot,
        metric_contract: MetricContract,
        mut baseline_development_reports: Vec<EvaluationReport>,
        baseline_sealed_report: EvaluationReport,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        sealed_suite_key: impl Into<String>,
        candidates: Vec<TrainingCandidate>,
        selection_rule: DevelopmentSelectionRule,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        baseline_development_reports.sort_by(|left, right| left.suite_key.cmp(&right.suite_key));
        let baseline_development_report = baseline_development_reports
            .first()
            .cloned()
            .ok_or_else(|| {
                EncoderExperimentError::Validation(
                    "multi-suite protocol requires development evidence".into(),
                )
            })?;
        let additional_baseline_development_reports =
            baseline_development_reports.into_iter().skip(1).collect();
        let mut value = Self {
            schema_version: EXPERIMENT_PROTOCOL_SCHEMA_VERSION,
            id,
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            metric_contract,
            development_suite_key: baseline_development_report.suite_key.clone(),
            baseline_development_report,
            additional_baseline_development_reports,
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            development_selection_rule: Some(selection_rule),
            sealed_suite_key: required(sealed_suite_key, "sealed suite key")?,
            candidates,
            created_at,
            fingerprint: String::new(),
        };
        value.candidates.sort_by_key(|candidate| candidate.sequence);
        value.validate_fields(project)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<(), EncoderExperimentError> {
        self.validate_fields(project)?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderExperimentError::Integrity(
                "experiment protocol fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        if self.schema_version == LEGACY_EXPERIMENT_PROTOCOL_SCHEMA_VERSION {
            return fingerprint(&serde_json::json!({
                "schema_version": self.schema_version,
                "id": self.id,
                "project_snapshot_id": self.project_snapshot_id,
                "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
                "metric_contract": self.metric_contract,
                "baseline_development_report": self.baseline_development_report,
                "baseline_sealed_report": self.baseline_sealed_report,
                "budget": self.budget,
                "maximum_evaluation_seconds": self.maximum_evaluation_seconds,
                "development_suite_key": self.development_suite_key,
                "sealed_suite_key": self.sealed_suite_key,
                "candidates": self.candidates,
                "created_at": self.created_at,
            }));
        }
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "metric_contract": self.metric_contract,
            "baseline_development_report": self.baseline_development_report,
            "additional_baseline_development_reports": self.additional_baseline_development_reports,
            "baseline_sealed_report": self.baseline_sealed_report,
            "budget": self.budget,
            "maximum_evaluation_seconds": self.maximum_evaluation_seconds,
            "development_suite_key": self.development_suite_key,
            "development_selection_rule": self.development_selection_rule,
            "sealed_suite_key": self.sealed_suite_key,
            "candidates": self.candidates,
            "created_at": self.created_at,
        }))
    }

    pub fn candidate(&self, id: Uuid) -> Option<&TrainingCandidate> {
        self.candidates.iter().find(|candidate| candidate.id == id)
    }

    pub fn baseline_development_reports(&self) -> Vec<&EvaluationReport> {
        std::iter::once(&self.baseline_development_report)
            .chain(&self.additional_baseline_development_reports)
            .collect()
    }

    pub fn development_suite_keys(&self) -> Vec<String> {
        self.baseline_development_reports()
            .into_iter()
            .map(|report| report.suite_key.clone())
            .collect()
    }

    pub fn baseline_development_report_for(&self, suite_key: &str) -> Option<&EvaluationReport> {
        self.baseline_development_reports()
            .into_iter()
            .find(|report| report.suite_key == suite_key)
    }

    fn validate_fields(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<(), EncoderExperimentError> {
        project.validate_integrity()?;
        self.metric_contract.validate_integrity()?;
        self.budget.validate()?;
        if !matches!(
            self.schema_version,
            LEGACY_EXPERIMENT_PROTOCOL_SCHEMA_VERSION | EXPERIMENT_PROTOCOL_SCHEMA_VERSION
        ) || self.project_snapshot_id != project.id
            || self.project_snapshot_fingerprint != project.fingerprint
            || self.maximum_evaluation_seconds == 0
            || self.development_suite_key.trim() != self.development_suite_key
            || self.development_suite_key.is_empty()
            || self.sealed_suite_key.trim() != self.sealed_suite_key
            || self.sealed_suite_key.is_empty()
            || self.development_suite_key == self.sealed_suite_key
            || self.candidates.is_empty()
            || self.candidates.len() > self.budget.maximum_candidates as usize
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "experiment protocol fields or finite budgets are invalid".into(),
            ));
        }

        let baseline_development_reports = self.baseline_development_reports();
        let development_evaluation_count = self
            .candidates
            .len()
            .checked_mul(baseline_development_reports.len())
            .ok_or_else(|| {
                EncoderExperimentError::Validation(
                    "candidate development evaluation budget overflowed".into(),
                )
            })?;
        if development_evaluation_count > self.budget.maximum_development_evaluations as usize
            || self.schema_version == LEGACY_EXPERIMENT_PROTOCOL_SCHEMA_VERSION
                && (!self.additional_baseline_development_reports.is_empty()
                    || self.development_selection_rule.is_some())
            || self.schema_version == EXPERIMENT_PROTOCOL_SCHEMA_VERSION
                && self.development_selection_rule.is_none()
        {
            return Err(EncoderExperimentError::Validation(
                "protocol schema and development evaluation budget are inconsistent".into(),
            ));
        }
        let mut development_keys = BTreeSet::new();
        for report in &baseline_development_reports {
            report.validate_integrity(project, &self.metric_contract)?;
            if report.evidence_role != EvidenceRole::Development
                || report.model != project.baseline_model
                || report.suite_key == self.sealed_suite_key
                || !development_keys.insert(report.suite_key.as_str())
            {
                return Err(EncoderExperimentError::Validation(
                    "development baseline reports must bind unique named suites and the baseline model"
                        .into(),
                ));
            }
        }
        self.baseline_sealed_report
            .validate_integrity(project, &self.metric_contract)?;
        if self.baseline_development_report.evidence_role != EvidenceRole::Development
            || self.baseline_development_report.suite_key != self.development_suite_key
            || self.baseline_sealed_report.evidence_role != EvidenceRole::SealedAcceptance
            || self.baseline_sealed_report.suite_key != self.sealed_suite_key
            || self.baseline_development_report.model != project.baseline_model
            || self.baseline_sealed_report.model != project.baseline_model
        {
            return Err(EncoderExperimentError::Validation(
                "experiment protocol requires exact baseline reports on its declared suites".into(),
            ));
        }
        if baseline_development_reports
            .windows(2)
            .any(|pair| pair[0].suite_key >= pair[1].suite_key)
        {
            return Err(EncoderExperimentError::Validation(
                "development suites must use canonical key order".into(),
            ));
        }
        for gate in &self.metric_contract.gates {
            if let Some(suite_key) = gate.suite_key.as_deref() {
                let known = match gate.role {
                    EvidenceRole::Development => development_keys.contains(suite_key),
                    EvidenceRole::SealedAcceptance => suite_key == self.sealed_suite_key,
                    _ => false,
                };
                if !known {
                    return Err(EncoderExperimentError::Validation(format!(
                        "metric gate references unknown suite {suite_key}"
                    )));
                }
            }
        }

        let mut candidate_ids = BTreeSet::new();
        let mut maximum_training_seconds = 0_u64;
        for (index, candidate) in self.candidates.iter().enumerate() {
            candidate.validate_integrity(project)?;
            if candidate.sequence
                != u32::try_from(index + 1).map_err(|_| {
                    EncoderExperimentError::Validation("candidate sequence overflowed".into())
                })?
                || !candidate_ids.insert(candidate.id)
            {
                return Err(EncoderExperimentError::Validation(
                    "experiment candidates must have unique contiguous sequences".into(),
                ));
            }
            maximum_training_seconds = maximum_training_seconds
                .checked_add(candidate.maximum_training_seconds)
                .ok_or_else(|| {
                    EncoderExperimentError::Validation(
                        "candidate training authority overflowed".into(),
                    )
                })?;
        }
        if maximum_training_seconds > self.budget.maximum_training_seconds {
            return Err(EncoderExperimentError::Validation(
                "candidate training authority exceeds the protocol budget".into(),
            ));
        }
        Ok(())
    }
}
