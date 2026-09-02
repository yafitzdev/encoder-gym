use std::collections::BTreeSet;

use encoder_experiment_core::{
    domain::{
        BackendIdentity, EvidenceRole, ExternalArtifactIdentity, ExternalProjectSnapshot,
        ModelArtifactIdentity,
    },
    metrics::{EvaluationReport, MetricContract},
};
use serde::{Deserialize, Serialize};

use crate::{
    EncoderRepairError, canonical_sha256, fingerprint, observation::DevelopmentObservation,
};

pub const DEVELOPMENT_OBSERVATION_REQUEST_SCHEMA_VERSION: u32 = 1;
pub const COLLECTED_DEVELOPMENT_OBSERVATIONS_SCHEMA_VERSION: u32 = 1;

/// Exact development-report authority presented to a native observation adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentObservationRequest {
    pub schema_version: u32,
    pub project_snapshot_id: uuid::Uuid,
    pub project_snapshot_fingerprint: String,
    pub evaluation_report_id: uuid::Uuid,
    pub evaluation_report_fingerprint: String,
    pub evidence_role: EvidenceRole,
    pub model: ModelArtifactIdentity,
    pub suite_key: String,
    pub suite_fingerprint: String,
    pub metric_support: u64,
    pub slice_dimensions: Vec<String>,
    pub maximum_seconds: u64,
    pub fingerprint: String,
}

impl DevelopmentObservationRequest {
    pub fn create(
        project: &ExternalProjectSnapshot,
        report: &EvaluationReport,
        contract: &MetricContract,
        mut slice_dimensions: Vec<String>,
        maximum_seconds: u64,
    ) -> Result<Self, EncoderRepairError> {
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        report
            .validate_integrity(project, contract)
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if report.evidence_role != EvidenceRole::Development {
            return Err(EncoderRepairError::Validation(
                "development observations cannot be requested from sealed evidence".into(),
            ));
        }
        slice_dimensions.sort();
        slice_dimensions.dedup();
        let mut value = Self {
            schema_version: DEVELOPMENT_OBSERVATION_REQUEST_SCHEMA_VERSION,
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            evaluation_report_id: report.id,
            evaluation_report_fingerprint: report.fingerprint.clone(),
            evidence_role: report.evidence_role,
            model: report.model.clone(),
            suite_key: report.suite_key.clone(),
            suite_fingerprint: report.suite_fingerprint.clone(),
            metric_support: report.support,
            slice_dimensions,
            maximum_seconds,
            fingerprint: String::new(),
        };
        value.validate_for_project(project)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_for_project(
        &self,
        project: &ExternalProjectSnapshot,
    ) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        project
            .validate_integrity()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if self.project_snapshot_id != project.id
            || self.project_snapshot_fingerprint != project.fingerprint
        {
            return Err(EncoderRepairError::Integrity(
                "development observation request project binding changed".into(),
            ));
        }
        if !self.fingerprint.is_empty() && self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "development observation request fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderRepairError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "development observation request fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "evaluation_report_id": self.evaluation_report_id,
            "evaluation_report_fingerprint": self.evaluation_report_fingerprint,
            "evidence_role": self.evidence_role,
            "model": self.model,
            "suite_key": self.suite_key,
            "suite_fingerprint": self.suite_fingerprint,
            "metric_support": self.metric_support,
            "slice_dimensions": self.slice_dimensions,
            "maximum_seconds": self.maximum_seconds,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        self.model
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if self.schema_version != DEVELOPMENT_OBSERVATION_REQUEST_SCHEMA_VERSION
            || self.project_snapshot_id.is_nil()
            || self.evaluation_report_id.is_nil()
            || self.evidence_role != EvidenceRole::Development
            || !canonical_sha256(&self.project_snapshot_fingerprint)
            || !canonical_sha256(&self.evaluation_report_fingerprint)
            || self.suite_key.is_empty()
            || self.suite_key.trim() != self.suite_key
            || !canonical_sha256(&self.suite_fingerprint)
            || self.metric_support == 0
            || self.slice_dimensions.is_empty()
            || self.maximum_seconds == 0
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "development observation request is incomplete or invalid".into(),
            ));
        }
        if self
            .slice_dimensions
            .iter()
            .any(|value| value.is_empty() || value.trim() != value)
            || self
                .slice_dimensions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(EncoderRepairError::Validation(
                "development observation dimensions must be canonical and unique".into(),
            ));
        }
        Ok(())
    }
}

/// Complete normalized output from a native development-only observer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectedDevelopmentObservations {
    pub schema_version: u32,
    pub request_fingerprint: String,
    pub observer: BackendIdentity,
    pub observation_artifact: ExternalArtifactIdentity,
    pub source_row_count: u64,
    pub metric_eligible_row_count: u64,
    pub observations: Vec<DevelopmentObservation>,
    pub fingerprint: String,
}

impl CollectedDevelopmentObservations {
    pub fn create(
        request: &DevelopmentObservationRequest,
        observer: BackendIdentity,
        observation_artifact: ExternalArtifactIdentity,
        source_row_count: u64,
        metric_eligible_row_count: u64,
        mut observations: Vec<DevelopmentObservation>,
    ) -> Result<Self, EncoderRepairError> {
        request.validate_integrity()?;
        observations.sort_by(|left, right| left.source_row_id.cmp(&right.source_row_id));
        let mut value = Self {
            schema_version: COLLECTED_DEVELOPMENT_OBSERVATIONS_SCHEMA_VERSION,
            request_fingerprint: request.fingerprint.clone(),
            observer,
            observation_artifact,
            source_row_count,
            metric_eligible_row_count,
            observations,
            fingerprint: String::new(),
        };
        value.validate_for_request(request)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_for_request(
        &self,
        request: &DevelopmentObservationRequest,
    ) -> Result<(), EncoderRepairError> {
        request.validate_integrity()?;
        self.validate_fields()?;
        if self.request_fingerprint != request.fingerprint
            || self.metric_eligible_row_count != request.metric_support
        {
            return Err(EncoderRepairError::Integrity(
                "collected observations do not match the exact report request".into(),
            ));
        }
        let abstentions = self
            .observations
            .iter()
            .filter(|value| value.expected_abstention)
            .count() as u64;
        if self.source_row_count - self.metric_eligible_row_count != abstentions {
            return Err(EncoderRepairError::Integrity(
                "collected observation eligibility does not explain the complete source set".into(),
            ));
        }
        if !self.fingerprint.is_empty() && self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderRepairError::Integrity(
                "collected development observation fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderRepairError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "request_fingerprint": self.request_fingerprint,
            "observer": self.observer,
            "observation_artifact": self.observation_artifact,
            "source_row_count": self.source_row_count,
            "metric_eligible_row_count": self.metric_eligible_row_count,
            "observations": self.observations,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderRepairError> {
        self.observer
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        self.observation_artifact
            .validate()
            .map_err(|error| EncoderRepairError::Experiment(error.to_string()))?;
        if self.schema_version != COLLECTED_DEVELOPMENT_OBSERVATIONS_SCHEMA_VERSION
            || !canonical_sha256(&self.request_fingerprint)
            || self.observation_artifact.role != EvidenceRole::Development
            || self.source_row_count == 0
            || self.metric_eligible_row_count == 0
            || self.metric_eligible_row_count > self.source_row_count
            || self.observations.len() as u64 != self.source_row_count
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderRepairError::Validation(
                "collected development observations are incomplete or invalid".into(),
            ));
        }
        let mut identities = BTreeSet::new();
        for observation in &self.observations {
            observation.validate()?;
            if !identities.insert(observation.source_row_id.as_str()) {
                return Err(EncoderRepairError::Validation(
                    "collected development row identities must be unique".into(),
                ));
            }
        }
        if self
            .observations
            .windows(2)
            .any(|pair| pair[0].source_row_id >= pair[1].source_row_id)
        {
            return Err(EncoderRepairError::Validation(
                "collected development observations must be canonically ordered".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use encoder_experiment_core::{
        domain::{EncoderTaskKind, ExternalProjectSnapshot},
        metrics::{MetricDefinition, MetricDirection, MetricGate, MetricGateCondition},
    };
    use serde_json::json;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "repair collection",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            digest('a'),
            BackendIdentity::new("fake", "v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('1'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "development",
                    EvidenceRole::Development,
                    1,
                    digest('2'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('3'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "fake", 1, digest('4')).unwrap(),
            json!({"task":"ranking"}),
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
        )
        .unwrap()
    }

    fn contract() -> MetricContract {
        MetricContract::create(
            vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MinimumImprovement { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "mrr",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn report(project: &ExternalProjectSnapshot, role: EvidenceRole) -> EvaluationReport {
        EvaluationReport::create(
            project,
            project.baseline_model.clone(),
            role,
            if role == EvidenceRole::Development {
                "development"
            } else {
                "sealed"
            },
            if role == EvidenceRole::Development {
                digest('5')
            } else {
                digest('6')
            },
            &contract(),
            BTreeMap::from([("mrr".into(), 0.8)]),
            1,
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 1).unwrap(),
        )
        .unwrap()
    }

    fn observation(id: &str, abstention: bool) -> DevelopmentObservation {
        DevelopmentObservation::create(
            id,
            digest(if abstention { '7' } else { '8' }),
            BTreeMap::from([("workflow".into(), "lookup".into())]),
            abstention,
            if abstention { None } else { Some(1) },
            if abstention { 0.0 } else { 1.0 },
            if abstention { None } else { Some(0.1) },
            Some(digest('9')),
        )
        .unwrap()
    }

    #[test]
    fn collection_is_complete_reproducible_and_explains_abstentions() {
        let project = project();
        let request = DevelopmentObservationRequest::create(
            &project,
            &report(&project, EvidenceRole::Development),
            &contract(),
            vec!["workflow".into()],
            60,
        )
        .unwrap();
        let collected = CollectedDevelopmentObservations::create(
            &request,
            BackendIdentity::new("observer", "v1", digest('a')).unwrap(),
            ExternalArtifactIdentity::new(
                "observations",
                EvidenceRole::Development,
                1,
                digest('b'),
            )
            .unwrap(),
            2,
            1,
            vec![observation("row-2", true), observation("row-1", false)],
        )
        .unwrap();
        collected.validate_for_request(&request).unwrap();
        assert_eq!(collected.observations[0].source_row_id, "row-1");

        let mut tampered = collected;
        tampered.metric_eligible_row_count = 2;
        assert!(tampered.validate_for_request(&request).is_err());
    }

    #[test]
    fn sealed_report_cannot_create_a_collection_request() {
        let project = project();
        assert!(
            DevelopmentObservationRequest::create(
                &project,
                &report(&project, EvidenceRole::SealedAcceptance),
                &contract(),
                vec!["workflow".into()],
                60,
            )
            .is_err()
        );
    }
}
