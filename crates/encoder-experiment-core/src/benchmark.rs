//! Model-independent identity for a shared evaluation benchmark.
//! This is not qualification, exposure, or permission to execute a test.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    EncoderExperimentError, canonical_sha256,
    domain::{BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalProjectSnapshot},
    fingerprint,
    metrics::{EvaluationReport, MetricContract},
    protocol::ExperimentProtocol,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSuite {
    pub key: String,
    pub role: EvidenceRole,
    /// Complete membership AND evaluation-procedure identity, owned by the adapter.
    pub fingerprint: String,
    pub support: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDefinition {
    pub schema_version: u32,
    pub task: EncoderTaskKind,
    pub backend: BackendIdentity,
    pub source_revision: String,
    pub evaluation_configuration_fingerprint: String,
    pub metric_contract: MetricContract,
    pub suites: Vec<BenchmarkSuite>,
    pub fingerprint: String,
}

impl BenchmarkDefinition {
    pub fn create(
        project: &ExternalProjectSnapshot,
        evaluation_configuration_fingerprint: String,
        metric_contract: MetricContract,
        mut suites: Vec<BenchmarkSuite>,
    ) -> Result<Self, EncoderExperimentError> {
        project.validate_integrity()?;
        suites.sort_by(|a, b| a.key.cmp(&b.key));
        let mut value = Self {
            schema_version: 1,
            task: project.task,
            backend: project.backend.clone(),
            source_revision: project.source_revision.clone(),
            evaluation_configuration_fingerprint,
            metric_contract,
            suites,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_integrity()?;
        Ok(value)
    }

    pub fn from_protocol(
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        evaluation_configuration_fingerprint: String,
    ) -> Result<Self, EncoderExperimentError> {
        protocol.validate_integrity(project)?;
        let suites = protocol
            .baseline_development_reports()
            .into_iter()
            .chain(std::iter::once(&protocol.baseline_sealed_report))
            .map(|report| BenchmarkSuite {
                key: report.suite_key.clone(),
                role: report.evidence_role,
                fingerprint: report.suite_fingerprint.clone(),
                support: report.support,
            })
            .collect();
        Self::create(
            project,
            evaluation_configuration_fingerprint,
            protocol.metric_contract.clone(),
            suites,
        )
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version, "task": self.task,
            "backend": self.backend, "source_revision": self.source_revision,
            "evaluation_configuration_fingerprint": self.evaluation_configuration_fingerprint,
            "metric_contract": self.metric_contract, "suites": self.suites,
        }))
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderExperimentError> {
        self.backend.validate()?;
        self.metric_contract.validate_integrity()?;
        if self.schema_version != 1
            || self.source_revision.is_empty()
            || self.source_revision.trim() != self.source_revision
            || !canonical_sha256(&self.evaluation_configuration_fingerprint)
            || self.suites.is_empty()
            || !self
                .suites
                .iter()
                .any(|s| s.role == EvidenceRole::Development)
            || self.suites.iter().any(|s| {
                s.key.is_empty()
                    || s.key.trim() != s.key
                    || s.support == 0
                    || !canonical_sha256(&s.fingerprint)
                    || !matches!(
                        s.role,
                        EvidenceRole::Development | EvidenceRole::SealedAcceptance
                    )
            })
            || self
                .suites
                .windows(2)
                .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(EncoderExperimentError::Validation(
                "Benchmark definition is not canonical.".into(),
            ));
        }
        for gate in &self.metric_contract.gates {
            if !self.suites.iter().any(|s| {
                s.role == gate.role && gate.suite_key.as_ref().is_none_or(|key| *key == s.key)
            }) {
                return Err(EncoderExperimentError::Validation(
                    "Metric gate has no matching benchmark suite.".into(),
                ));
            }
        }
        if self.fingerprint != self.reproduce_fingerprint()? {
            return Err(EncoderExperimentError::Integrity(
                "Benchmark definition changed.".into(),
            ));
        }
        Ok(())
    }

    /// The adapter must derive the configuration fingerprint from this report's
    /// recorded project, not from the current project or presentation defaults.
    pub fn validate_development_report(
        &self,
        project: &ExternalProjectSnapshot,
        evaluation_configuration_fingerprint: &str,
        report: &EvaluationReport,
    ) -> Result<(), EncoderExperimentError> {
        self.validate_integrity()?;
        project.validate_integrity()?;
        report.validate_integrity(project, &self.metric_contract)?;
        if project.task != self.task
            || project.backend != self.backend
            || project.source_revision != self.source_revision
            || evaluation_configuration_fingerprint != self.evaluation_configuration_fingerprint
            || report.evidence_role != EvidenceRole::Development
            || !self.suites.iter().any(|s| {
                s.role == EvidenceRole::Development
                    && s.key == report.suite_key
                    && s.fingerprint == report.suite_fingerprint
                    && s.support == report.support
            })
        {
            return Err(EncoderExperimentError::Validation(
                "Report does not match this development benchmark.".into(),
            ));
        }
        Ok(())
    }
}

/// Report provenance for a comparison; raw and sealed evidence are excluded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BenchmarkResult {
    pub model: crate::domain::ModelArtifactIdentity,
    pub report_id: uuid::Uuid,
    pub report_fingerprint: String,
    pub suite_key: String,
    pub metrics: std::collections::BTreeMap<String, f64>,
    pub support: u64,
    pub created_at: DateTime<Utc>,
}

impl BenchmarkResult {
    pub fn from_development(
        definition: &BenchmarkDefinition,
        project: &ExternalProjectSnapshot,
        configuration_fingerprint: &str,
        report: &EvaluationReport,
    ) -> Result<Self, EncoderExperimentError> {
        definition.validate_development_report(project, configuration_fingerprint, report)?;
        Ok(Self {
            model: report.model.clone(),
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            suite_key: report.suite_key.clone(),
            metrics: report.metrics.clone(),
            support: report.support,
            created_at: report.created_at,
        })
    }
}
