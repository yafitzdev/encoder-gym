use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    domain::{BackendIdentity, EvidenceRole, ExternalArtifactIdentity, ExternalProjectSnapshot},
    metrics::{EvaluationReport, MetricContract},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{EncoderRepairError, canonical_sha256, fingerprint, required};

pub const DEVELOPMENT_OBSERVATION_SET_SCHEMA_VERSION: u32 = 1;

/// A task-neutral, text-free observation for one development row.
///
/// Native labels and tool identifiers remain adapter-owned. Slice values and hashed candidate
/// identities are sufficient for deterministic comparative diagnosis without copying row text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentObservation {
    pub source_row_id: String,
    pub source_row_fingerprint: String,
    pub slices: BTreeMap<String, String>,
    pub expected_abstention: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_relevant_rank: Option<u32>,
    pub reciprocal_rank: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positive_margin: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_candidate_fingerprint: Option<String>,
}

impl DevelopmentObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        source_row_id: impl Into<String>,
        source_row_fingerprint: impl Into<String>,
        slices: BTreeMap<String, String>,
        expected_abstention: bool,
        first_relevant_rank: Option<u32>,
        reciprocal_rank: f64,
        positive_margin: Option<f64>,
        top_candidate_fingerprint: Option<String>,
    ) -> Result<Self, EncoderRepairError> {
        let value = Self {
            source_row_id: required(source_row_id, "source row id")?,
            source_row_fingerprint: source_row_fingerprint.into(),
            slices,
            expected_abstention,
            first_relevant_rank,
            reciprocal_rank,
            positive_margin,
            top_candidate_fingerprint,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), EncoderRepairError> {
        let expected_reciprocal = self
            .first_relevant_rank
            .map_or(0.0, |rank| 1.0 / f64::from(rank));
        if self.source_row_id.is_empty()
            || self.source_row_id.trim() != self.source_row_id
            || !canonical_sha256(&self.source_row_fingerprint)
            || self.slices.is_empty()
            || self.first_relevant_rank == Some(0)
            || !self.reciprocal_rank.is_finite()
            || (self.reciprocal_rank - expected_reciprocal).abs() > 1e-12
            || self.positive_margin.is_some_and(|value| !value.is_finite())
            || self
                .top_candidate_fingerprint
                .as_deref()
                .is_some_and(|value| !canonical_sha256(value))
            || self.expected_abstention
                && (self.first_relevant_rank.is_some()
                    || self.reciprocal_rank != 0.0
                    || self.positive_margin.is_some())
        {
            return Err(EncoderRepairError::Validation(
                "development observation is not canonical or internally consistent".into(),
            ));
        }
        for (key, value) in &self.slices {
            if key.is_empty() || key.trim() != key || value.is_empty() || value.trim() != value {
                return Err(EncoderRepairError::Validation(
                    "development observation slice is not canonical".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn top_one_correct(&self) -> bool {
        self.first_relevant_rank.is_some_and(|rank| rank == 1)
    }

    pub fn top_two_correct(&self) -> bool {
        self.first_relevant_rank.is_some_and(|rank| rank <= 2)
    }

    pub fn top_three_correct(&self) -> bool {
        self.first_relevant_rank.is_some_and(|rank| rank <= 3)
    }
}

/// Complete immutable development observations for one exact model/report/suite tuple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentObservationSet {
    pub schema_version: u32,
    pub id: Uuid,
    /// Stable identity of the exact source evidence, independent from record UUID/time.
    pub evidence_fingerprint: String,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub source_campaign_id: Uuid,
    pub source_experiment_run_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    pub evaluation_report_id: Uuid,
    pub evaluation_report_fingerprint: String,
    pub model_fingerprint: String,
    pub suite_key: String,
    pub suite_fingerprint: String,
    pub observer: BackendIdentity,
    pub observation_artifact: ExternalArtifactIdentity,
    pub observations: Vec<DevelopmentObservation>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl DevelopmentObservationSet {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project: &ExternalProjectSnapshot,
        source_campaign_id: Uuid,
        source_experiment_run_id: Uuid,
        candidate_id: Option<Uuid>,
        report: &EvaluationReport,
        contract: &MetricContract,
        observer: BackendIdentity,
        observation_artifact: ExternalArtifactIdentity,
        mut observations: Vec<DevelopmentObservation>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        report
            .validate_integrity(project, contract)
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if report.evidence_role != EvidenceRole::Development {
            return Err(EncoderRepairError::Validation(
                "only development reports can produce repair observations".into(),
            ));
        }
        observations.sort_by(|left, right| left.source_row_id.cmp(&right.source_row_id));
        let mut value = Self {
            schema_version: DEVELOPMENT_OBSERVATION_SET_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            evidence_fingerprint: String::new(),
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            source_campaign_id,
            source_experiment_run_id,
            candidate_id,
            evaluation_report_id: report.id,
            evaluation_report_fingerprint: report.fingerprint.clone(),
            model_fingerprint: report.model.fingerprint.clone(),
            suite_key: report.suite_key.clone(),
            suite_fingerprint: report.suite_fingerprint.clone(),
            observer,
            observation_artifact,
            observations,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.evidence_fingerprint = value.reproduce_evidence_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_evidence_fingerprint()? != self.evidence_fingerprint {
            return Err(EncoderRepairError::Integrity(
                "development observation source evidence fingerprint changed".into(),
            ));
        }
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "development observation set fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_evidence_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "source_campaign_id": self.source_campaign_id,
            "source_experiment_run_id": self.source_experiment_run_id,
            "candidate_id": self.candidate_id,
            "evaluation_report_id": self.evaluation_report_id,
            "evaluation_report_fingerprint": self.evaluation_report_fingerprint,
            "model_fingerprint": self.model_fingerprint,
            "suite_key": self.suite_key,
            "suite_fingerprint": self.suite_fingerprint,
            "observer": self.observer,
            "observation_artifact": self.observation_artifact,
            "observations": self.observations,
        }))
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "evidence_fingerprint": self.evidence_fingerprint,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "source_campaign_id": self.source_campaign_id,
            "source_experiment_run_id": self.source_experiment_run_id,
            "candidate_id": self.candidate_id,
            "evaluation_report_id": self.evaluation_report_id,
            "evaluation_report_fingerprint": self.evaluation_report_fingerprint,
            "model_fingerprint": self.model_fingerprint,
            "suite_key": self.suite_key,
            "suite_fingerprint": self.suite_fingerprint,
            "observer": self.observer,
            "observation_artifact": self.observation_artifact,
            "observations": self.observations,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        if self.schema_version != DEVELOPMENT_OBSERVATION_SET_SCHEMA_VERSION
            || self.id.is_nil()
            || !self.evidence_fingerprint.is_empty()
                && !canonical_sha256(&self.evidence_fingerprint)
            || self.source_campaign_id.is_nil()
            || self.source_experiment_run_id.is_nil()
            || self.candidate_id.is_some_and(|id| id.is_nil())
            || self.evaluation_report_id.is_nil()
            || !canonical_sha256(&self.project_snapshot_fingerprint)
            || !canonical_sha256(&self.evaluation_report_fingerprint)
            || !canonical_sha256(&self.model_fingerprint)
            || self.suite_key.is_empty()
            || self.suite_key.trim() != self.suite_key
            || !canonical_sha256(&self.suite_fingerprint)
            || self.observations.is_empty()
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "development observation set fields are invalid".into(),
            ));
        }
        self.observer
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        self.observation_artifact
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if self.observation_artifact.role != EvidenceRole::Development {
            return Err(EncoderRepairError::Validation(
                "observation artifact must be development evidence".into(),
            ));
        }
        let mut row_ids = BTreeSet::new();
        for observation in &self.observations {
            observation.validate()?;
            if !row_ids.insert(observation.source_row_id.as_str()) {
                return Err(EncoderRepairError::Validation(
                    "development observation row identities must be unique".into(),
                ));
            }
        }
        if self
            .observations
            .windows(2)
            .any(|pair| pair[0].source_row_id >= pair[1].source_row_id)
        {
            return Err(EncoderRepairError::Validation(
                "development observations must be canonically ordered".into(),
            ));
        }
        Ok(())
    }
}
