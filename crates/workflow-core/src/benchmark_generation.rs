//! Renewable, one-use benchmark authority for production optimization campaigns.
//!
//! A generation composes existing benchmark bundles and their independently
//! approved qualification evidence. It does not replace bundle construction,
//! contamination checks, or qualification. Each development suite has its own
//! authority against the same sealed suite, which keeps suites distinct while
//! reusing the established governance contracts.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    benchmark_bundle::{BenchmarkBundle, BenchmarkBundleBinding},
    benchmark_qualification::{
        ApprovedBenchmarkQualificationBinding, BenchmarkQualification, BenchmarkQualificationReview,
    },
};

pub const BENCHMARK_GENERATION_SCHEMA_VERSION: u32 = 1;
pub const BENCHMARK_GENERATION_EVENT_SCHEMA_VERSION: u32 = 1;
pub const EXTERNAL_BENCHMARK_AUTHORITY_SCHEMA_VERSION: u32 = 1;
pub const HISTORICAL_BENCHMARK_CONSUMPTION_SCHEMA_VERSION: u32 = 1;

/// Full row-free evidence used when an external production task cannot use the
/// classification-specific benchmark builder. It retains the same scientific
/// requirements: exact population pins, zero-tolerance contamination, finite
/// support qualification, and an independent approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBenchmarkAuthority {
    pub schema_version: u32,
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub acquisition_spec_fingerprint: String,
    pub source_manifest_fingerprint: String,
    pub population_fingerprint: String,
    pub acquired_by: String,
    pub contamination: ExternalContaminationEvidence,
    pub qualification: ExternalQualificationEvidence,
    pub approval: ExternalQualificationApproval,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalContaminationEvidence {
    pub id: Uuid,
    pub population_fingerprint: String,
    pub policy: String,
    pub overlap_counts: BTreeMap<String, u64>,
    pub checked_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalQualificationEvidence {
    pub id: Uuid,
    pub population_fingerprint: String,
    pub minimum_overall_support: u64,
    pub observed_overall_support: u64,
    pub minimum_slice_support: BTreeMap<String, u64>,
    pub observed_slice_support: BTreeMap<String, u64>,
    pub ready: bool,
    pub assessed_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalQualificationApproval {
    pub id: Uuid,
    pub qualification_id: Uuid,
    pub qualification_fingerprint: String,
    pub approved: bool,
    pub reviewed_by: String,
    pub independent: bool,
    pub rationale: String,
    pub reviewed_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ExternalBenchmarkAuthority {
    pub fn validate_integrity(&self) -> Result<(), BenchmarkGenerationError> {
        if self.schema_version != EXTERNAL_BENCHMARK_AUTHORITY_SCHEMA_VERSION
            || [
                self.id,
                self.bundle_id,
                self.development_suite_id,
                self.sealed_suite_id,
            ]
            .contains(&Uuid::nil())
            || self.development_suite_id == self.sealed_suite_id
            || [
                self.development_suite_fingerprint.as_str(),
                self.sealed_suite_fingerprint.as_str(),
                self.acquisition_spec_fingerprint.as_str(),
                self.source_manifest_fingerprint.as_str(),
                self.population_fingerprint.as_str(),
            ]
            .iter()
            .any(|value| !canonical_fingerprint(value))
            || required_text(&self.acquired_by).is_none()
            || self.created_at < self.contamination.checked_at
            || self.created_at < self.qualification.assessed_at
            || self.created_at < self.approval.reviewed_at
        {
            return Err(BenchmarkGenerationError::InvalidExternalAuthority);
        }
        self.contamination.validate(&self.population_fingerprint)?;
        self.qualification.validate(&self.population_fingerprint)?;
        self.approval.validate(&self.qualification)?;
        if self.approval.reviewed_by == self.acquired_by
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(BenchmarkGenerationError::InvalidExternalAuthority);
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "bundle_id": self.bundle_id,
            "development_suite_id": self.development_suite_id,
            "development_suite_fingerprint": self.development_suite_fingerprint,
            "sealed_suite_id": self.sealed_suite_id,
            "sealed_suite_fingerprint": self.sealed_suite_fingerprint,
            "acquisition_spec_fingerprint": self.acquisition_spec_fingerprint,
            "source_manifest_fingerprint": self.source_manifest_fingerprint,
            "population_fingerprint": self.population_fingerprint,
            "acquired_by": self.acquired_by,
            "contamination": self.contamination,
            "qualification": self.qualification,
            "approval": self.approval,
            "created_at": self.created_at,
        }))
        .map_err(map_fingerprint)
    }
}

impl ExternalContaminationEvidence {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.id,
            self.population_fingerprint.as_str(),
            self.policy.as_str(),
            &self.overlap_counts,
            self.checked_at,
        ))
        .map_err(map_fingerprint)
    }

    fn validate(&self, population_fingerprint: &str) -> Result<(), BenchmarkGenerationError> {
        let expected = [
            "exact_content",
            "group_identity",
            "normalized_content",
            "source_identity",
        ]
        .into_iter()
        .map(|key| (key.to_owned(), 0_u64))
        .collect::<BTreeMap<_, _>>();
        if self.id.is_nil()
            || self.population_fingerprint != population_fingerprint
            || self.policy != "zero_tolerance_all_pairwise_v1"
            || self.overlap_counts != expected
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(BenchmarkGenerationError::InvalidExternalAuthority);
        }
        Ok(())
    }
}

impl ExternalQualificationEvidence {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.id,
            self.population_fingerprint.as_str(),
            self.minimum_overall_support,
            self.observed_overall_support,
            &self.minimum_slice_support,
            &self.observed_slice_support,
            self.ready,
            self.assessed_at,
        ))
        .map_err(map_fingerprint)
    }

    fn validate(&self, population_fingerprint: &str) -> Result<(), BenchmarkGenerationError> {
        if self.id.is_nil()
            || self.population_fingerprint != population_fingerprint
            || self.minimum_overall_support == 0
            || self.observed_overall_support < self.minimum_overall_support
            || self.minimum_slice_support.is_empty()
            || self.minimum_slice_support.keys().collect::<Vec<_>>()
                != self.observed_slice_support.keys().collect::<Vec<_>>()
            || self.minimum_slice_support.iter().any(|(key, minimum)| {
                required_text(key).is_none()
                    || *minimum == 0
                    || self.observed_slice_support[key] < *minimum
            })
            || !self.ready
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(BenchmarkGenerationError::InvalidExternalAuthority);
        }
        Ok(())
    }
}

impl ExternalQualificationApproval {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.id,
            self.qualification_id,
            self.qualification_fingerprint.as_str(),
            self.approved,
            self.reviewed_by.as_str(),
            self.independent,
            self.rationale.as_str(),
            self.reviewed_at,
        ))
        .map_err(map_fingerprint)
    }

    fn validate(
        &self,
        qualification: &ExternalQualificationEvidence,
    ) -> Result<(), BenchmarkGenerationError> {
        if self.id.is_nil()
            || self.qualification_id != qualification.id
            || self.qualification_fingerprint != qualification.fingerprint
            || !self.approved
            || !self.independent
            || required_text(&self.reviewed_by).is_none()
            || required_text(&self.rationale).is_none()
            || self.reviewed_at < qualification.assessed_at
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(BenchmarkGenerationError::InvalidExternalAuthority);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkFreshnessAuthority {
    pub id: Uuid,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub population_fingerprint: String,
    pub source_manifest_fingerprint: String,
    pub acquired_at: DateTime<Utc>,
    pub frozen_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub reviewed_by: String,
    pub rationale: String,
    pub fingerprint: String,
}

impl BenchmarkFreshnessAuthority {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        sealed_suite_id: Uuid,
        sealed_suite_fingerprint: impl Into<String>,
        population_fingerprint: impl Into<String>,
        source_manifest_fingerprint: impl Into<String>,
        acquired_at: DateTime<Utc>,
        frozen_at: DateTime<Utc>,
        valid_until: DateTime<Utc>,
        reviewed_by: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Result<Self, BenchmarkGenerationError> {
        let mut value = Self {
            id: Uuid::new_v4(),
            sealed_suite_id,
            sealed_suite_fingerprint: sealed_suite_fingerprint.into(),
            population_fingerprint: population_fingerprint.into(),
            source_manifest_fingerprint: source_manifest_fingerprint.into(),
            acquired_at,
            frozen_at,
            valid_until,
            reviewed_by: reviewed_by.into(),
            rationale: rationale.into(),
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), BenchmarkGenerationError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkGenerationError::FingerprintMismatch(
                "freshness authority",
            ));
        }
        Ok(())
    }

    pub fn validate_at(&self, at: DateTime<Utc>) -> Result<(), BenchmarkGenerationError> {
        self.validate_integrity()?;
        if at < self.frozen_at || at > self.valid_until {
            return Err(BenchmarkGenerationError::FreshnessExpired);
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.id,
            self.sealed_suite_id,
            self.sealed_suite_fingerprint.as_str(),
            self.population_fingerprint.as_str(),
            self.source_manifest_fingerprint.as_str(),
            self.acquired_at,
            self.frozen_at,
            self.valid_until,
            self.reviewed_by.as_str(),
            self.rationale.as_str(),
        ))
        .map_err(map_fingerprint)
    }

    fn validate_fields(&self) -> Result<(), BenchmarkGenerationError> {
        if self.id.is_nil()
            || self.sealed_suite_id.is_nil()
            || !canonical_fingerprint(&self.sealed_suite_fingerprint)
            || !canonical_fingerprint(&self.population_fingerprint)
            || !canonical_fingerprint(&self.source_manifest_fingerprint)
            || self.acquired_at > self.frozen_at
            || self.frozen_at >= self.valid_until
            || required_text(&self.reviewed_by).is_none()
            || required_text(&self.rationale).is_none()
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(BenchmarkGenerationError::InvalidFreshnessAuthority);
        }
        Ok(())
    }
}

/// Exact clean and approved authority for one named development suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentSuiteAuthority {
    pub suite_key: String,
    pub bundle: BenchmarkBundleBinding,
    pub qualification: ApprovedBenchmarkQualificationBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_evidence: Option<ExternalBenchmarkAuthority>,
}

impl DevelopmentSuiteAuthority {
    pub fn from_artifacts(
        suite_key: impl Into<String>,
        bundle: &BenchmarkBundle,
        qualification: &BenchmarkQualification,
        review: &BenchmarkQualificationReview,
    ) -> Result<Self, BenchmarkGenerationError> {
        let suite_key = suite_key.into();
        if required_text(&suite_key).is_none() {
            return Err(BenchmarkGenerationError::InvalidSuiteKey);
        }
        let bundle_binding = bundle
            .binding()
            .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?;
        let qualification_binding = review
            .approved_binding(qualification)
            .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?;
        if qualification_binding.benchmark_bundle_id != bundle.id
            || qualification_binding.benchmark_bundle_fingerprint != bundle.fingerprint
        {
            return Err(BenchmarkGenerationError::AuthorityMismatch);
        }
        let value = Self {
            suite_key,
            bundle: bundle_binding,
            qualification: qualification_binding,
            external_evidence: None,
        };
        value.validate_shape()?;
        Ok(value)
    }

    pub fn from_external_evidence(
        suite_key: impl Into<String>,
        evidence: ExternalBenchmarkAuthority,
    ) -> Result<Self, BenchmarkGenerationError> {
        evidence.validate_integrity()?;
        let suite_key = suite_key.into();
        if required_text(&suite_key).is_none() {
            return Err(BenchmarkGenerationError::InvalidSuiteKey);
        }
        let mut bundle = BenchmarkBundle {
            id: evidence.bundle_id,
            development_suite_id: evidence.development_suite_id,
            development_suite_fingerprint: evidence.development_suite_fingerprint.clone(),
            sealed_suite_id: Some(evidence.sealed_suite_id),
            sealed_suite_fingerprint: Some(evidence.sealed_suite_fingerprint.clone()),
            contamination_report_id: evidence.contamination.id,
            contamination_report_fingerprint: evidence.contamination.fingerprint.clone(),
            created_at: evidence.created_at,
            fingerprint: String::new(),
        };
        bundle.fingerprint = bundle
            .reproduce_fingerprint()
            .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?;
        let value = Self {
            suite_key,
            bundle: bundle
                .binding()
                .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?,
            qualification: ApprovedBenchmarkQualificationBinding {
                qualification_id: evidence.qualification.id,
                qualification_fingerprint: evidence.qualification.fingerprint.clone(),
                review_id: evidence.approval.id,
                review_fingerprint: evidence.approval.fingerprint.clone(),
                benchmark_bundle_id: evidence.bundle_id,
                benchmark_bundle_fingerprint: bundle.fingerprint,
            },
            external_evidence: Some(evidence),
        };
        Ok(value)
    }

    pub fn validate_shape(&self) -> Result<(), BenchmarkGenerationError> {
        if required_text(&self.suite_key).is_none() {
            return Err(BenchmarkGenerationError::InvalidSuiteKey);
        }
        self.bundle
            .validate_fingerprint()
            .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?;
        self.qualification
            .validate_shape()
            .map_err(|error| BenchmarkGenerationError::InvalidAuthority(error.to_string()))?;
        if self.qualification.benchmark_bundle_id != self.bundle.bundle_id
            || self.qualification.benchmark_bundle_fingerprint != self.bundle.bundle_fingerprint
        {
            return Err(BenchmarkGenerationError::AuthorityMismatch);
        }
        if let Some(evidence) = &self.external_evidence {
            evidence.validate_integrity()?;
            let expected = Self::from_external_evidence(self.suite_key.clone(), evidence.clone())?;
            if expected.bundle != self.bundle || expected.qualification != self.qualification {
                return Err(BenchmarkGenerationError::AuthorityMismatch);
            }
        }
        if self.bundle.sealed_suite_id.is_none() || self.bundle.sealed_suite_fingerprint.is_none() {
            return Err(BenchmarkGenerationError::SealedSuiteRequired);
        }
        Ok(())
    }
}

/// Immutable authority shared by every lifecycle event for one generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkGeneration {
    pub schema_version: u32,
    pub id: Uuid,
    pub predecessor_id: Option<Uuid>,
    pub predecessor_fingerprint: Option<String>,
    pub development_suites: Vec<DevelopmentSuiteAuthority>,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub freshness: Option<BenchmarkFreshnessAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_consumption: Option<HistoricalBenchmarkConsumption>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkGeneration {
    pub fn create(
        predecessor: Option<&BenchmarkGeneration>,
        mut development_suites: Vec<DevelopmentSuiteAuthority>,
        freshness: BenchmarkFreshnessAuthority,
        created_at: DateTime<Utc>,
    ) -> Result<Self, BenchmarkGenerationError> {
        if let Some(predecessor) = predecessor {
            predecessor.validate_integrity()?;
        }
        development_suites.sort_by(|left, right| left.suite_key.cmp(&right.suite_key));
        let sealed_suite_id = development_suites
            .first()
            .and_then(|authority| authority.bundle.sealed_suite_id)
            .ok_or(BenchmarkGenerationError::SealedSuiteRequired)?;
        let sealed_suite_fingerprint = development_suites
            .first()
            .and_then(|authority| authority.bundle.sealed_suite_fingerprint.clone())
            .ok_or(BenchmarkGenerationError::SealedSuiteRequired)?;
        let mut value = Self {
            schema_version: BENCHMARK_GENERATION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            predecessor_id: predecessor.map(|value| value.id),
            predecessor_fingerprint: predecessor.map(|value| value.fingerprint.clone()),
            development_suites,
            sealed_suite_id,
            sealed_suite_fingerprint,
            freshness: Some(freshness),
            historical_consumption: None,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), BenchmarkGenerationError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkGenerationError::FingerprintMismatch(
                "benchmark generation",
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        if let Some(history) = &self.historical_consumption {
            artifact_core::fingerprint(&serde_json::json!({
                "schema_version": self.schema_version,
                "id": self.id,
                "sealed_suite_id": self.sealed_suite_id,
                "sealed_suite_fingerprint": self.sealed_suite_fingerprint,
                "historical_consumption": history,
                "created_at": self.created_at,
            }))
            .map_err(map_fingerprint)
        } else {
            // Preserve the original schema-v1 fingerprint representation for
            // every ordinary qualified generation.
            artifact_core::fingerprint(&(
                self.schema_version,
                self.id,
                self.predecessor_id,
                self.predecessor_fingerprint.as_deref(),
                &self.development_suites,
                self.sealed_suite_id,
                self.sealed_suite_fingerprint.as_str(),
                self.freshness.as_ref(),
                self.created_at,
            ))
            .map_err(map_fingerprint)
        }
    }

    fn validate_fields(&self) -> Result<(), BenchmarkGenerationError> {
        if self.schema_version != BENCHMARK_GENERATION_SCHEMA_VERSION
            || self.id.is_nil()
            || self.predecessor_id.is_some() != self.predecessor_fingerprint.is_some()
            || self.predecessor_id == Some(self.id)
            || self
                .predecessor_fingerprint
                .as_deref()
                .is_some_and(|value| !canonical_fingerprint(value))
            || self.sealed_suite_id.is_nil()
            || !canonical_fingerprint(&self.sealed_suite_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(BenchmarkGenerationError::InvalidGeneration);
        }
        if let Some(history) = &self.historical_consumption {
            history.validate_integrity()?;
            if self.predecessor_id.is_some()
                || self.predecessor_fingerprint.is_some()
                || !self.development_suites.is_empty()
                || self.freshness.is_some()
                || history.sealed_suite_id != self.sealed_suite_id
                || history.sealed_suite_fingerprint != self.sealed_suite_fingerprint
                || self.created_at < history.consumed_at
            {
                return Err(BenchmarkGenerationError::InvalidHistoricalConsumption);
            }
            return Ok(());
        }
        let freshness = self
            .freshness
            .as_ref()
            .ok_or(BenchmarkGenerationError::InvalidFreshnessAuthority)?;
        if self.development_suites.is_empty() {
            return Err(BenchmarkGenerationError::InvalidGeneration);
        }
        freshness.validate_integrity()?;
        if freshness.sealed_suite_id != self.sealed_suite_id
            || freshness.sealed_suite_fingerprint != self.sealed_suite_fingerprint
        {
            return Err(BenchmarkGenerationError::FreshnessMismatch);
        }
        let mut keys = BTreeSet::new();
        let mut development_suite_ids = BTreeSet::new();
        for authority in &self.development_suites {
            authority.validate_shape()?;
            if !keys.insert(authority.suite_key.as_str())
                || !development_suite_ids.insert(authority.bundle.development_suite_id)
                || authority.bundle.sealed_suite_id != Some(self.sealed_suite_id)
                || authority.bundle.sealed_suite_fingerprint.as_deref()
                    != Some(self.sealed_suite_fingerprint.as_str())
            {
                return Err(BenchmarkGenerationError::SuiteAuthoritySet);
            }
        }
        if self
            .development_suites
            .windows(2)
            .any(|pair| pair[0].suite_key >= pair[1].suite_key)
        {
            return Err(BenchmarkGenerationError::SuiteAuthoritySet);
        }
        Ok(())
    }

    /// Record a sealed cohort that was already consumed before generation
    /// journals existed. Historical anchors are born exhausted, carry exact
    /// persisted run evidence, and can never become adaptive authority.
    pub fn historical_exhausted_anchor(
        history: HistoricalBenchmarkConsumption,
        created_at: DateTime<Utc>,
    ) -> Result<Self, BenchmarkGenerationError> {
        history.validate_integrity()?;
        let mut value = Self {
            schema_version: BENCHMARK_GENERATION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            predecessor_id: None,
            predecessor_fingerprint: None,
            development_suites: Vec::new(),
            sealed_suite_id: history.sealed_suite_id,
            sealed_suite_fingerprint: history.sealed_suite_fingerprint.clone(),
            freshness: None,
            historical_consumption: Some(history),
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }
}

/// Exact persisted evidence for a pre-journal sealed assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalBenchmarkConsumption {
    pub schema_version: u32,
    pub id: Uuid,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub experiment_run_id: Uuid,
    pub experiment_protocol_fingerprint: String,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub sealed_exposure_id: Uuid,
    pub sealed_exposure_fingerprint: String,
    pub final_decision_fingerprint: String,
    pub consumed_at: DateTime<Utc>,
    pub recorded_by: String,
    pub fingerprint: String,
}

impl HistoricalBenchmarkConsumption {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project_snapshot_id: Uuid,
        project_snapshot_fingerprint: impl Into<String>,
        experiment_run_id: Uuid,
        experiment_protocol_fingerprint: impl Into<String>,
        sealed_suite_fingerprint: impl Into<String>,
        sealed_exposure_id: Uuid,
        sealed_exposure_fingerprint: impl Into<String>,
        final_decision_fingerprint: impl Into<String>,
        consumed_at: DateTime<Utc>,
        recorded_by: impl Into<String>,
    ) -> Result<Self, BenchmarkGenerationError> {
        let mut value = Self {
            schema_version: HISTORICAL_BENCHMARK_CONSUMPTION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            project_snapshot_id,
            project_snapshot_fingerprint: project_snapshot_fingerprint.into(),
            experiment_run_id,
            experiment_protocol_fingerprint: experiment_protocol_fingerprint.into(),
            sealed_suite_id: Uuid::new_v4(),
            sealed_suite_fingerprint: sealed_suite_fingerprint.into(),
            sealed_exposure_id,
            sealed_exposure_fingerprint: sealed_exposure_fingerprint.into(),
            final_decision_fingerprint: final_decision_fingerprint.into(),
            consumed_at,
            recorded_by: recorded_by.into(),
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), BenchmarkGenerationError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkGenerationError::FingerprintMismatch(
                "historical benchmark consumption",
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.schema_version,
            self.id,
            self.project_snapshot_id,
            self.project_snapshot_fingerprint.as_str(),
            self.experiment_run_id,
            self.experiment_protocol_fingerprint.as_str(),
            self.sealed_suite_id,
            self.sealed_suite_fingerprint.as_str(),
            self.sealed_exposure_id,
            self.sealed_exposure_fingerprint.as_str(),
            self.final_decision_fingerprint.as_str(),
            self.consumed_at,
            self.recorded_by.as_str(),
        ))
        .map_err(map_fingerprint)
    }

    fn validate_fields(&self) -> Result<(), BenchmarkGenerationError> {
        if self.schema_version != HISTORICAL_BENCHMARK_CONSUMPTION_SCHEMA_VERSION
            || [
                self.id,
                self.project_snapshot_id,
                self.experiment_run_id,
                self.sealed_suite_id,
                self.sealed_exposure_id,
            ]
            .contains(&Uuid::nil())
            || [
                self.project_snapshot_fingerprint.as_str(),
                self.experiment_protocol_fingerprint.as_str(),
                self.sealed_suite_fingerprint.as_str(),
                self.sealed_exposure_fingerprint.as_str(),
                self.final_decision_fingerprint.as_str(),
            ]
            .iter()
            .any(|value| !canonical_fingerprint(value))
            || required_text(&self.recorded_by).is_none()
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(BenchmarkGenerationError::InvalidHistoricalConsumption);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkGenerationState {
    Draft,
    Ready,
    Active,
    Exhausted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchmarkGenerationEventKind {
    Created,
    HistoricalConsumptionRecorded,
    MarkedReady {
        confirmed_by: String,
    },
    Activated {
        activated_by: String,
    },
    IterationConsumed {
        experiment_run_id: Uuid,
        experiment_protocol_fingerprint: String,
        sealed_exposure_id: Uuid,
        sealed_exposure_fingerprint: String,
        final_decision_fingerprint: String,
    },
    Exhausted {
        reason: String,
    },
    Superseded {
        successor_generation_id: Uuid,
        successor_generation_fingerprint: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkGenerationEvent {
    pub schema_version: u32,
    pub id: Uuid,
    pub generation_id: Uuid,
    pub generation_fingerprint: String,
    pub sequence: u32,
    pub previous_event_fingerprint: Option<String>,
    pub event: BenchmarkGenerationEventKind,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkGenerationEvent {
    fn create(
        generation: &BenchmarkGeneration,
        sequence: u32,
        previous_event_fingerprint: Option<String>,
        event: BenchmarkGenerationEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<Self, BenchmarkGenerationError> {
        let mut value = Self {
            schema_version: BENCHMARK_GENERATION_EVENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            generation_id: generation.id,
            generation_fingerprint: generation.fingerprint.clone(),
            sequence,
            previous_event_fingerprint,
            event,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), BenchmarkGenerationError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkGenerationError::FingerprintMismatch(
                "benchmark generation event",
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkGenerationError> {
        artifact_core::fingerprint(&(
            self.schema_version,
            self.id,
            self.generation_id,
            self.generation_fingerprint.as_str(),
            self.sequence,
            self.previous_event_fingerprint.as_deref(),
            &self.event,
            self.created_at,
        ))
        .map_err(map_fingerprint)
    }

    fn validate_fields(&self) -> Result<(), BenchmarkGenerationError> {
        if self.schema_version != BENCHMARK_GENERATION_EVENT_SCHEMA_VERSION
            || self.id.is_nil()
            || self.generation_id.is_nil()
            || !canonical_fingerprint(&self.generation_fingerprint)
            || self.sequence == 0
            || self.sequence == 1 && self.previous_event_fingerprint.is_some()
            || self.sequence > 1
                && !self
                    .previous_event_fingerprint
                    .as_deref()
                    .is_some_and(canonical_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(BenchmarkGenerationError::InvalidEvent);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkGenerationView {
    pub generation_id: Uuid,
    pub state: BenchmarkGenerationState,
    pub consuming_experiment_run_id: Option<Uuid>,
    pub last_sequence: u32,
    pub last_event_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

/// The two lifecycle events that storage must append in one transaction.
///
/// Keeping the pair explicit prevents a successor becoming active while its
/// exhausted predecessor still appears current, or vice versa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessorActivation {
    pub predecessor_superseded: BenchmarkGenerationEvent,
    pub successor_activated: BenchmarkGenerationEvent,
}

pub fn prepare_successor_activation(
    predecessor: &BenchmarkGeneration,
    predecessor_view: &BenchmarkGenerationView,
    successor: &BenchmarkGeneration,
    successor_view: &BenchmarkGenerationView,
    activated_by: impl Into<String>,
    created_at: DateTime<Utc>,
) -> Result<SuccessorActivation, BenchmarkGenerationError> {
    predecessor.validate_integrity()?;
    successor.validate_integrity()?;
    if successor.predecessor_id != Some(predecessor.id)
        || successor.predecessor_fingerprint.as_deref() != Some(predecessor.fingerprint.as_str())
        || predecessor_view.generation_id != predecessor.id
        || predecessor_view.state != BenchmarkGenerationState::Exhausted
        || successor_view.generation_id != successor.id
        || successor_view.state != BenchmarkGenerationState::Ready
    {
        return Err(BenchmarkGenerationError::SuccessorMismatch);
    }
    successor
        .freshness
        .as_ref()
        .ok_or(BenchmarkGenerationError::InvalidFreshnessAuthority)?
        .validate_at(created_at)?;
    let activated_by = activated_by.into();
    if required_text(&activated_by).is_none() {
        return Err(BenchmarkGenerationError::IllegalTransition);
    }
    Ok(SuccessorActivation {
        predecessor_superseded: predecessor_view.next_event(
            predecessor,
            BenchmarkGenerationEventKind::Superseded {
                successor_generation_id: successor.id,
                successor_generation_fingerprint: successor.fingerprint.clone(),
            },
            created_at,
        )?,
        successor_activated: successor_view.next_event(
            successor,
            BenchmarkGenerationEventKind::Activated { activated_by },
            created_at,
        )?,
    })
}

impl BenchmarkGenerationView {
    pub fn next_event(
        &self,
        generation: &BenchmarkGeneration,
        event: BenchmarkGenerationEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<BenchmarkGenerationEvent, BenchmarkGenerationError> {
        BenchmarkGenerationEvent::create(
            generation,
            self.last_sequence
                .checked_add(1)
                .ok_or(BenchmarkGenerationError::InvalidEvent)?,
            Some(self.last_event_fingerprint.clone()),
            event,
            created_at,
        )
    }

    pub fn is_adaptive_eligible(&self) -> bool {
        self.state == BenchmarkGenerationState::Active && self.consuming_experiment_run_id.is_none()
    }
}

pub fn first_generation_event(
    generation: &BenchmarkGeneration,
    created_at: DateTime<Utc>,
) -> Result<BenchmarkGenerationEvent, BenchmarkGenerationError> {
    generation.validate_integrity()?;
    let event = if generation.historical_consumption.is_some() {
        BenchmarkGenerationEventKind::HistoricalConsumptionRecorded
    } else {
        BenchmarkGenerationEventKind::Created
    };
    BenchmarkGenerationEvent::create(generation, 1, None, event, created_at)
}

pub fn replay_benchmark_generation(
    generation: &BenchmarkGeneration,
    events: &[BenchmarkGenerationEvent],
) -> Result<BenchmarkGenerationView, BenchmarkGenerationError> {
    generation.validate_integrity()?;
    let first = events
        .first()
        .ok_or(BenchmarkGenerationError::EmptyJournal)?;
    let mut view = BenchmarkGenerationView {
        generation_id: generation.id,
        state: BenchmarkGenerationState::Draft,
        consuming_experiment_run_id: None,
        last_sequence: 0,
        last_event_fingerprint: String::new(),
        updated_at: first.created_at,
    };
    for event in events {
        event.validate_integrity()?;
        if event.generation_id != generation.id
            || event.generation_fingerprint != generation.fingerprint
            || event.sequence != view.last_sequence + 1
            || event.previous_event_fingerprint.as_deref()
                != if view.last_sequence == 0 {
                    None
                } else {
                    Some(view.last_event_fingerprint.as_str())
                }
            || event.created_at < view.updated_at
        {
            return Err(BenchmarkGenerationError::EventChain);
        }
        apply_event(generation, &mut view, event)?;
        view.last_sequence = event.sequence;
        view.last_event_fingerprint = event.fingerprint.clone();
        view.updated_at = event.created_at;
    }
    Ok(view)
}

fn apply_event(
    generation: &BenchmarkGeneration,
    view: &mut BenchmarkGenerationView,
    event: &BenchmarkGenerationEvent,
) -> Result<(), BenchmarkGenerationError> {
    match &event.event {
        BenchmarkGenerationEventKind::Created => {
            if generation.historical_consumption.is_some()
                || event.sequence != 1
                || view.last_sequence != 0
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
        }
        BenchmarkGenerationEventKind::HistoricalConsumptionRecorded => {
            let history = generation
                .historical_consumption
                .as_ref()
                .ok_or(BenchmarkGenerationError::IllegalTransition)?;
            if event.sequence != 1 || view.last_sequence != 0 {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            view.consuming_experiment_run_id = Some(history.experiment_run_id);
            view.state = BenchmarkGenerationState::Exhausted;
        }
        BenchmarkGenerationEventKind::MarkedReady { confirmed_by } => {
            if view.state != BenchmarkGenerationState::Draft
                || required_text(confirmed_by).is_none()
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            generation
                .freshness
                .as_ref()
                .ok_or(BenchmarkGenerationError::IllegalTransition)?
                .validate_at(event.created_at)?;
            view.state = BenchmarkGenerationState::Ready;
        }
        BenchmarkGenerationEventKind::Activated { activated_by } => {
            if view.state != BenchmarkGenerationState::Ready
                || required_text(activated_by).is_none()
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            generation
                .freshness
                .as_ref()
                .ok_or(BenchmarkGenerationError::IllegalTransition)?
                .validate_at(event.created_at)?;
            view.state = BenchmarkGenerationState::Active;
        }
        BenchmarkGenerationEventKind::IterationConsumed {
            experiment_run_id,
            experiment_protocol_fingerprint,
            sealed_exposure_id,
            sealed_exposure_fingerprint,
            final_decision_fingerprint,
        } => {
            if !view.is_adaptive_eligible()
                || experiment_run_id.is_nil()
                || sealed_exposure_id.is_nil()
                || !canonical_fingerprint(experiment_protocol_fingerprint)
                || !canonical_fingerprint(sealed_exposure_fingerprint)
                || !canonical_fingerprint(final_decision_fingerprint)
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            view.consuming_experiment_run_id = Some(*experiment_run_id);
            view.state = BenchmarkGenerationState::Exhausted;
        }
        BenchmarkGenerationEventKind::Exhausted { reason } => {
            if !matches!(
                view.state,
                BenchmarkGenerationState::Ready | BenchmarkGenerationState::Active
            ) || required_text(reason).is_none()
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            view.state = BenchmarkGenerationState::Exhausted;
        }
        BenchmarkGenerationEventKind::Superseded {
            successor_generation_id,
            successor_generation_fingerprint,
        } => {
            if view.state != BenchmarkGenerationState::Exhausted
                || successor_generation_id.is_nil()
                || *successor_generation_id == generation.id
                || !canonical_fingerprint(successor_generation_fingerprint)
            {
                return Err(BenchmarkGenerationError::IllegalTransition);
            }
            view.state = BenchmarkGenerationState::Superseded;
        }
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BenchmarkGenerationError {
    #[error("external benchmark authority is incomplete, unclean, unqualified, or unapproved")]
    InvalidExternalAuthority,
    #[error("historical consumed benchmark evidence is invalid")]
    InvalidHistoricalConsumption,
    #[error("benchmark authority is invalid: {0}")]
    InvalidAuthority(String),
    #[error("benchmark bundle and approved qualification do not match")]
    AuthorityMismatch,
    #[error("development suite key must be non-empty and canonical")]
    InvalidSuiteKey,
    #[error("every benchmark generation authority requires a sealed suite")]
    SealedSuiteRequired,
    #[error("benchmark generation development-suite authorities are not unique or consistent")]
    SuiteAuthoritySet,
    #[error("benchmark freshness authority is invalid")]
    InvalidFreshnessAuthority,
    #[error("freshness authority does not pin the generation sealed suite")]
    FreshnessMismatch,
    #[error("benchmark freshness authority is not valid at this time")]
    FreshnessExpired,
    #[error("benchmark generation is invalid")]
    InvalidGeneration,
    #[error("benchmark generation event is invalid")]
    InvalidEvent,
    #[error("benchmark generation journal is empty")]
    EmptyJournal,
    #[error("benchmark generation event chain changed or forked")]
    EventChain,
    #[error("benchmark generation lifecycle transition is not allowed")]
    IllegalTransition,
    #[error("successor generation does not exactly descend from the exhausted predecessor")]
    SuccessorMismatch,
    #[error("{0} fingerprint does not reproduce")]
    FingerprintMismatch(&'static str),
    #[error("could not fingerprint benchmark generation evidence: {0}")]
    Fingerprint(String),
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn required_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty() && trimmed == value).then_some(value)
}

fn map_fingerprint(error: artifact_core::FingerprintError) -> BenchmarkGenerationError {
    BenchmarkGenerationError::Fingerprint(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::benchmark_qualification::{
        BenchmarkCohortReadiness, BenchmarkQualificationPolicy,
        BenchmarkQualificationReviewDecision, BenchmarkQualificationReviewRequest,
        BenchmarkReadiness, SourceComposition, review_benchmark_qualification,
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn time(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, hour, 0, 0).unwrap()
    }

    fn authority(
        key: &str,
        development_suite_id: Uuid,
        sealed_suite_id: Uuid,
        suite_character: char,
    ) -> DevelopmentSuiteAuthority {
        let mut bundle = BenchmarkBundle {
            id: Uuid::new_v4(),
            development_suite_id,
            development_suite_fingerprint: digest(suite_character),
            sealed_suite_id: Some(sealed_suite_id),
            sealed_suite_fingerprint: Some(digest('b')),
            contamination_report_id: Uuid::new_v4(),
            contamination_report_fingerprint: digest('c'),
            created_at: time(0),
            fingerprint: String::new(),
        };
        bundle.fingerprint = bundle.reproduce_fingerprint().unwrap();

        let mut qualification = BenchmarkQualification {
            id: Uuid::new_v4(),
            protocol: crate::benchmark_qualification::BENCHMARK_QUALIFICATION_PROTOCOL.into(),
            benchmark_bundle_id: bundle.id,
            benchmark_bundle_fingerprint: bundle.fingerprint.clone(),
            development_suite_id,
            development_suite_fingerprint: digest(suite_character),
            sealed_suite_id: Some(sealed_suite_id),
            sealed_suite_fingerprint: Some(digest('b')),
            policy: BenchmarkQualificationPolicy::default(),
            readiness: BenchmarkReadiness::Ready,
            cohorts: vec![BenchmarkCohortReadiness {
                suite_id: development_suite_id,
                suite_fingerprint: digest(suite_character),
                cohort_id: Uuid::new_v4(),
                snapshot_id: Uuid::new_v4(),
                snapshot_fingerprint: digest('a'),
                population_fingerprint: digest('d'),
                total_support: 100,
                label_support: BTreeMap::from([("positive".into(), 100)]),
                required_slice_support: BTreeMap::new(),
                dimension_value_support: BTreeMap::new(),
                source_composition: SourceComposition {
                    generated_rows: 0,
                    imported_rows: 100,
                    distinct_producers: 1,
                    rows_by_producer: BTreeMap::from([("fixture".into(), 100)]),
                },
                normalized_duplicate_rows: 0,
                normalized_duplicate_rate: 0.0,
                minimum_text_characters: 10,
                median_text_characters: 20,
                maximum_text_characters: 30,
                reference_distribution_bound: true,
            }],
            issues: vec![],
            created_at: time(0),
            fingerprint: String::new(),
        };
        qualification.fingerprint = qualification.reproduce_fingerprint().unwrap();
        let review = review_benchmark_qualification(
            &qualification,
            BenchmarkQualificationReviewRequest {
                decision: BenchmarkQualificationReviewDecision::Approve,
                reviewed_by: "independent-reviewer".into(),
                rationale: "frozen population and support were independently checked".into(),
            },
        )
        .unwrap();
        DevelopmentSuiteAuthority::from_artifacts(key, &bundle, &qualification, &review).unwrap()
    }

    fn external_authority() -> ExternalBenchmarkAuthority {
        let population = digest('9');
        let mut contamination = ExternalContaminationEvidence {
            id: Uuid::new_v4(),
            population_fingerprint: population.clone(),
            policy: "zero_tolerance_all_pairwise_v1".into(),
            overlap_counts: [
                "exact_content",
                "group_identity",
                "normalized_content",
                "source_identity",
            ]
            .into_iter()
            .map(|key| (key.into(), 0))
            .collect(),
            checked_at: time(1),
            fingerprint: String::new(),
        };
        contamination.fingerprint = contamination.reproduce_fingerprint().unwrap();
        let mut qualification = ExternalQualificationEvidence {
            id: Uuid::new_v4(),
            population_fingerprint: population.clone(),
            minimum_overall_support: 100,
            observed_overall_support: 120,
            minimum_slice_support: BTreeMap::from([("hard".into(), 20)]),
            observed_slice_support: BTreeMap::from([("hard".into(), 24)]),
            ready: true,
            assessed_at: time(1),
            fingerprint: String::new(),
        };
        qualification.fingerprint = qualification.reproduce_fingerprint().unwrap();
        let mut approval = ExternalQualificationApproval {
            id: Uuid::new_v4(),
            qualification_id: qualification.id,
            qualification_fingerprint: qualification.fingerprint.clone(),
            approved: true,
            reviewed_by: "independent-reviewer".into(),
            independent: true,
            rationale: "support and zero-overlap evidence were independently reviewed".into(),
            reviewed_at: time(2),
            fingerprint: String::new(),
        };
        approval.fingerprint = approval.reproduce_fingerprint().unwrap();
        let mut authority = ExternalBenchmarkAuthority {
            schema_version: EXTERNAL_BENCHMARK_AUTHORITY_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            bundle_id: Uuid::new_v4(),
            development_suite_id: Uuid::new_v4(),
            development_suite_fingerprint: digest('4'),
            sealed_suite_id: Uuid::new_v4(),
            sealed_suite_fingerprint: digest('5'),
            acquisition_spec_fingerprint: digest('6'),
            source_manifest_fingerprint: digest('7'),
            population_fingerprint: population,
            acquired_by: "acquisition-owner".into(),
            contamination,
            qualification,
            approval,
            created_at: time(3),
            fingerprint: String::new(),
        };
        authority.fingerprint = authority.reproduce_fingerprint().unwrap();
        authority
    }

    #[test]
    fn external_authority_requires_clean_support_and_independent_approval() {
        let authority = external_authority();
        let normalized =
            DevelopmentSuiteAuthority::from_external_evidence("production", authority.clone())
                .unwrap();
        normalized.validate_shape().unwrap();

        let mut contaminated = authority.clone();
        contaminated
            .contamination
            .overlap_counts
            .insert("exact_content".into(), 1);
        contaminated.contamination.fingerprint =
            contaminated.contamination.reproduce_fingerprint().unwrap();
        contaminated.fingerprint = contaminated.reproduce_fingerprint().unwrap();
        assert!(
            DevelopmentSuiteAuthority::from_external_evidence("production", contaminated).is_err()
        );

        let mut self_approved = authority;
        self_approved.approval.reviewed_by = self_approved.acquired_by.clone();
        self_approved.approval.fingerprint =
            self_approved.approval.reproduce_fingerprint().unwrap();
        self_approved.fingerprint = self_approved.reproduce_fingerprint().unwrap();
        assert!(
            DevelopmentSuiteAuthority::from_external_evidence("production", self_approved).is_err()
        );
    }

    fn generation() -> BenchmarkGeneration {
        let sealed_id = Uuid::new_v4();
        let authorities = vec![
            authority("generic", Uuid::new_v4(), sealed_id, 'd'),
            authority("retired_post_scaling", Uuid::new_v4(), sealed_id, 'e'),
        ];
        let freshness = BenchmarkFreshnessAuthority::create(
            sealed_id,
            digest('b'),
            digest('d'),
            digest('e'),
            time(0),
            time(1),
            time(3),
            "freshness-reviewer",
            "successor population was acquired after the prior sealed exposure",
        )
        .unwrap();
        BenchmarkGeneration::create(None, authorities, freshness, time(1)).unwrap()
    }

    #[test]
    fn one_successor_generation_keeps_development_suites_distinct() {
        let generation = generation();
        assert_eq!(
            generation
                .development_suites
                .iter()
                .map(|value| value.suite_key.as_str())
                .collect::<Vec<_>>(),
            vec!["generic", "retired_post_scaling"]
        );
        generation.validate_integrity().unwrap();
    }

    #[test]
    fn historical_consumption_is_persisted_directly_as_exhausted() {
        let history = HistoricalBenchmarkConsumption::create(
            Uuid::new_v4(),
            digest('1'),
            Uuid::new_v4(),
            digest('2'),
            digest('3'),
            Uuid::new_v4(),
            digest('4'),
            digest('5'),
            time(1),
            "migration-reviewer",
        )
        .unwrap();
        let anchor = BenchmarkGeneration::historical_exhausted_anchor(history, time(2)).unwrap();
        let first = first_generation_event(&anchor, time(2)).unwrap();
        assert_eq!(
            first.event,
            BenchmarkGenerationEventKind::HistoricalConsumptionRecorded
        );
        let view = replay_benchmark_generation(&anchor, std::slice::from_ref(&first)).unwrap();
        assert_eq!(view.state, BenchmarkGenerationState::Exhausted);
        assert!(!view.is_adaptive_eligible());

        let illegal = view
            .next_event(
                &anchor,
                BenchmarkGenerationEventKind::MarkedReady {
                    confirmed_by: "operator".into(),
                },
                time(2),
            )
            .unwrap();
        assert!(replay_benchmark_generation(&anchor, &[first, illegal]).is_err());
    }

    #[test]
    fn sealed_consumption_immediately_exhausts_adaptive_authority() {
        let generation = generation();
        let first = first_generation_event(&generation, time(1)).unwrap();
        let view = replay_benchmark_generation(&generation, std::slice::from_ref(&first)).unwrap();
        let ready = view
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::MarkedReady {
                    confirmed_by: "operator".into(),
                },
                time(1),
            )
            .unwrap();
        let events = vec![first, ready];
        let view = replay_benchmark_generation(&generation, &events).unwrap();
        let active = view
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::Activated {
                    activated_by: "campaign-controller".into(),
                },
                time(1),
            )
            .unwrap();
        let mut events = [events, vec![active]].concat();
        let view = replay_benchmark_generation(&generation, &events).unwrap();
        assert!(view.is_adaptive_eligible());

        let consumed = view
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::IterationConsumed {
                    experiment_run_id: Uuid::new_v4(),
                    experiment_protocol_fingerprint: digest('1'),
                    sealed_exposure_id: Uuid::new_v4(),
                    sealed_exposure_fingerprint: digest('2'),
                    final_decision_fingerprint: digest('3'),
                },
                time(2),
            )
            .unwrap();
        events.push(consumed);
        let exhausted = replay_benchmark_generation(&generation, &events).unwrap();
        assert_eq!(exhausted.state, BenchmarkGenerationState::Exhausted);
        assert!(!exhausted.is_adaptive_eligible());
        let duplicate = exhausted
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::IterationConsumed {
                    experiment_run_id: Uuid::new_v4(),
                    experiment_protocol_fingerprint: digest('1'),
                    sealed_exposure_id: Uuid::new_v4(),
                    sealed_exposure_fingerprint: digest('2'),
                    final_decision_fingerprint: digest('3'),
                },
                time(2),
            )
            .unwrap();
        events.push(duplicate);
        assert_eq!(
            replay_benchmark_generation(&generation, &events),
            Err(BenchmarkGenerationError::IllegalTransition)
        );
    }

    #[test]
    fn expired_freshness_fails_closed_before_activation() {
        let generation = generation();
        let first = first_generation_event(&generation, time(1)).unwrap();
        let view = replay_benchmark_generation(&generation, std::slice::from_ref(&first)).unwrap();
        let ready = view
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::MarkedReady {
                    confirmed_by: "operator".into(),
                },
                time(1),
            )
            .unwrap();
        let events = vec![first, ready];
        let view = replay_benchmark_generation(&generation, &events).unwrap();
        let late = view
            .next_event(
                &generation,
                BenchmarkGenerationEventKind::Activated {
                    activated_by: "operator".into(),
                },
                time(3) + Duration::seconds(1),
            )
            .unwrap();
        assert_eq!(
            replay_benchmark_generation(&generation, &[events, vec![late]].concat()),
            Err(BenchmarkGenerationError::FreshnessExpired)
        );
    }
}
