use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{EvidenceRole, ExternalProjectSnapshot, ModelArtifactIdentity};
use crate::{EncoderExperimentError, canonical_sha256, fingerprint, required};

pub const METRIC_CONTRACT_SCHEMA_VERSION: u32 = 1;
pub const EVALUATION_REPORT_SCHEMA_VERSION: u32 = 1;
pub const REFERENCED_EVALUATION_REPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinition {
    pub key: String,
    pub direction: MetricDirection,
}

impl MetricDefinition {
    pub fn new(
        key: impl Into<String>,
        direction: MetricDirection,
    ) -> Result<Self, EncoderExperimentError> {
        Ok(Self {
            key: required(key, "metric key")?,
            direction,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricGateCondition {
    AtLeast { value: f64 },
    AtMost { value: f64 },
    MinimumImprovement { value: f64 },
    MaximumRegression { value: f64 },
}

impl MetricGateCondition {
    fn validate(&self) -> Result<(), EncoderExperimentError> {
        let value = match self {
            Self::AtLeast { value }
            | Self::AtMost { value }
            | Self::MinimumImprovement { value }
            | Self::MaximumRegression { value } => *value,
        };
        if !value.is_finite()
            || matches!(self, Self::MaximumRegression { .. }) && value.is_sign_negative()
        {
            return Err(EncoderExperimentError::Validation(
                "metric gate values must be finite and regression tolerance cannot be negative"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricGate {
    pub key: String,
    pub role: EvidenceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_key: Option<String>,
    pub condition: MetricGateCondition,
}

impl MetricGate {
    pub fn new(
        key: impl Into<String>,
        role: EvidenceRole,
        condition: MetricGateCondition,
    ) -> Result<Self, EncoderExperimentError> {
        if !matches!(
            role,
            EvidenceRole::Development | EvidenceRole::SealedAcceptance
        ) {
            return Err(EncoderExperimentError::Validation(
                "metric gates may use only development or sealed evidence".into(),
            ));
        }
        condition.validate()?;
        Ok(Self {
            key: required(key, "metric gate key")?,
            role,
            suite_key: None,
            condition,
        })
    }

    /// Restricts this gate to one named suite. An unscoped gate applies to
    /// every suite with the matching evidence role.
    pub fn for_suite(
        key: impl Into<String>,
        role: EvidenceRole,
        suite_key: impl Into<String>,
        condition: MetricGateCondition,
    ) -> Result<Self, EncoderExperimentError> {
        let mut gate = Self::new(key, role, condition)?;
        gate.suite_key = Some(required(suite_key, "metric gate suite key")?);
        Ok(gate)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricContract {
    pub schema_version: u32,
    pub definitions: Vec<MetricDefinition>,
    pub primary_metric: String,
    pub gates: Vec<MetricGate>,
    pub fingerprint: String,
}

impl MetricContract {
    pub fn create(
        mut definitions: Vec<MetricDefinition>,
        primary_metric: impl Into<String>,
        mut gates: Vec<MetricGate>,
    ) -> Result<Self, EncoderExperimentError> {
        definitions.sort_by(|left, right| left.key.cmp(&right.key));
        gates.sort_by(|left, right| {
            left.role
                .cmp(&right.role)
                .then_with(|| left.suite_key.cmp(&right.suite_key))
                .then_with(|| left.key.cmp(&right.key))
        });
        let mut value = Self {
            schema_version: METRIC_CONTRACT_SCHEMA_VERSION,
            definitions,
            primary_metric: required(primary_metric, "primary metric")?,
            gates,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), EncoderExperimentError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderExperimentError::Integrity(
                "metric contract fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "definitions": self.definitions,
            "primary_metric": self.primary_metric,
            "gates": self.gates,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderExperimentError> {
        if self.schema_version != METRIC_CONTRACT_SCHEMA_VERSION
            || self.definitions.is_empty()
            || self.gates.is_empty()
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "metric contract fields are invalid".into(),
            ));
        }
        let definitions = self
            .definitions
            .iter()
            .map(|value| value.key.as_str())
            .collect::<BTreeSet<_>>();
        if definitions.len() != self.definitions.len()
            || !definitions.contains(self.primary_metric.as_str())
            || self
                .definitions
                .windows(2)
                .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(EncoderExperimentError::Validation(
                "metric definitions must be unique and include the primary metric".into(),
            ));
        }
        let mut gate_keys = BTreeSet::new();
        for gate in &self.gates {
            if !definitions.contains(gate.key.as_str()) {
                return Err(EncoderExperimentError::Validation(format!(
                    "metric gate references unknown metric {}",
                    gate.key
                )));
            }
            if gate
                .suite_key
                .as_deref()
                .is_some_and(|value| value.trim() != value || value.is_empty())
                || !gate_keys.insert((gate.role, gate.suite_key.as_deref(), gate.key.as_str()))
            {
                return Err(EncoderExperimentError::Validation(
                    "metric gates must have canonical scopes and unique role/suite/metric keys"
                        .into(),
                ));
            }
            gate.condition.validate()?;
        }
        if !self
            .gates
            .iter()
            .any(|gate| gate.role == EvidenceRole::Development)
            || !self
                .gates
                .iter()
                .any(|gate| gate.role == EvidenceRole::SealedAcceptance)
        {
            return Err(EncoderExperimentError::Validation(
                "metric contract requires development and sealed gates".into(),
            ));
        }
        Ok(())
    }

    fn definition(&self, key: &str) -> Option<&MetricDefinition> {
        self.definitions.iter().find(|value| value.key == key)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReportReference {
    pub source_project_snapshot_id: Uuid,
    pub source_project_snapshot_fingerprint: String,
    pub source_protocol_id: Uuid,
    pub source_protocol_fingerprint: String,
    pub source_report_id: Uuid,
    pub source_report_fingerprint: String,
}

impl EvaluationReportReference {
    pub(crate) fn create(
        source_project: &ExternalProjectSnapshot,
        source_protocol_id: Uuid,
        source_protocol_fingerprint: String,
        source_report: &EvaluationReport,
    ) -> Result<Self, EncoderExperimentError> {
        let value = Self {
            source_project_snapshot_id: source_project.id,
            source_project_snapshot_fingerprint: source_project.fingerprint.clone(),
            source_protocol_id,
            source_protocol_fingerprint,
            source_report_id: source_report.id,
            source_report_fingerprint: source_report.fingerprint.clone(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), EncoderExperimentError> {
        if self.source_project_snapshot_id.is_nil()
            || !canonical_sha256(&self.source_project_snapshot_fingerprint)
            || self.source_protocol_id.is_nil()
            || !canonical_sha256(&self.source_protocol_fingerprint)
            || self.source_report_id.is_nil()
            || !canonical_sha256(&self.source_report_fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "referenced evaluation provenance is incomplete".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReport {
    pub schema_version: u32,
    pub id: Uuid,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub model: ModelArtifactIdentity,
    pub evidence_role: EvidenceRole,
    pub suite_key: String,
    pub suite_fingerprint: String,
    pub metric_contract_fingerprint: String,
    pub metrics: BTreeMap<String, f64>,
    pub support: u64,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<EvaluationReportReference>,
    pub fingerprint: String,
}

impl EvaluationReport {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        project: &ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        evidence_role: EvidenceRole,
        suite_key: impl Into<String>,
        suite_fingerprint: impl Into<String>,
        contract: &MetricContract,
        metrics: BTreeMap<String, f64>,
        support: u64,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        project.validate_integrity()?;
        contract.validate_integrity()?;
        if !matches!(
            evidence_role,
            EvidenceRole::Development | EvidenceRole::SealedAcceptance
        ) {
            return Err(EncoderExperimentError::Validation(
                "evaluation report requires development or sealed evidence".into(),
            ));
        }
        let mut value = Self {
            schema_version: EVALUATION_REPORT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            project_snapshot_id: project.id,
            project_snapshot_fingerprint: project.fingerprint.clone(),
            model,
            evidence_role,
            suite_key: required(suite_key, "evaluation suite key")?,
            suite_fingerprint: suite_fingerprint.into(),
            metric_contract_fingerprint: contract.fingerprint.clone(),
            metrics,
            support,
            created_at,
            reference: None,
            fingerprint: String::new(),
        };
        value.validate_fields(contract)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_referenced(
        id: Uuid,
        target_project: &ExternalProjectSnapshot,
        source_report: &Self,
        reference: EvaluationReportReference,
        contract: &MetricContract,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        target_project.validate_integrity()?;
        source_report.validate_fields(contract)?;
        reference.validate()?;
        let mut value = Self {
            schema_version: REFERENCED_EVALUATION_REPORT_SCHEMA_VERSION,
            id,
            project_snapshot_id: target_project.id,
            project_snapshot_fingerprint: target_project.fingerprint.clone(),
            model: target_project.baseline_model.clone(),
            evidence_role: source_report.evidence_role,
            suite_key: source_report.suite_key.clone(),
            suite_fingerprint: source_report.suite_fingerprint.clone(),
            metric_contract_fingerprint: source_report.metric_contract_fingerprint.clone(),
            metrics: source_report.metrics.clone(),
            support: source_report.support,
            created_at,
            reference: Some(reference),
            fingerprint: String::new(),
        };
        value.validate_fields(contract)?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(
        &self,
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
    ) -> Result<(), EncoderExperimentError> {
        if self.project_snapshot_id != project.id
            || self.project_snapshot_fingerprint != project.fingerprint
        {
            return Err(EncoderExperimentError::Integrity(
                "evaluation report project binding changed".into(),
            ));
        }
        self.validate_fields(contract)?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderExperimentError::Integrity(
                "evaluation report fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        if self.schema_version == EVALUATION_REPORT_SCHEMA_VERSION {
            return fingerprint(&serde_json::json!({
                "schema_version": self.schema_version,
                "id": self.id,
                "project_snapshot_id": self.project_snapshot_id,
                "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
                "model": self.model,
                "evidence_role": self.evidence_role,
                "suite_key": self.suite_key,
                "suite_fingerprint": self.suite_fingerprint,
                "metric_contract_fingerprint": self.metric_contract_fingerprint,
                "metrics": self.metrics,
                "support": self.support,
                "created_at": self.created_at,
            }));
        }
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "project_snapshot_id": self.project_snapshot_id,
            "project_snapshot_fingerprint": self.project_snapshot_fingerprint,
            "model": self.model,
            "evidence_role": self.evidence_role,
            "suite_key": self.suite_key,
            "suite_fingerprint": self.suite_fingerprint,
            "metric_contract_fingerprint": self.metric_contract_fingerprint,
            "metrics": self.metrics,
            "support": self.support,
            "created_at": self.created_at,
            "reference": self.reference,
        }))
    }

    fn validate_fields(&self, contract: &MetricContract) -> Result<(), EncoderExperimentError> {
        contract.validate_integrity()?;
        self.model.validate()?;
        if !matches!(
            self.schema_version,
            EVALUATION_REPORT_SCHEMA_VERSION | REFERENCED_EVALUATION_REPORT_SCHEMA_VERSION
        ) || self.id.is_nil()
            || self.schema_version == EVALUATION_REPORT_SCHEMA_VERSION && self.reference.is_some()
            || self.schema_version == REFERENCED_EVALUATION_REPORT_SCHEMA_VERSION
                && self.reference.is_none()
            || !matches!(
                self.evidence_role,
                EvidenceRole::Development | EvidenceRole::SealedAcceptance
            )
            || self.suite_key.trim() != self.suite_key
            || self.suite_key.is_empty()
            || !canonical_sha256(&self.suite_fingerprint)
            || self.metric_contract_fingerprint != contract.fingerprint
            || self.support == 0
            || self.metrics.len() != contract.definitions.len()
            || self.metrics.values().any(|value| !value.is_finite())
            || self
                .metrics
                .keys()
                .any(|key| contract.definition(key).is_none())
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "evaluation report is not canonical or complete".into(),
            ));
        }
        if let Some(reference) = &self.reference {
            reference.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricGateResult {
    pub key: String,
    pub baseline: f64,
    pub candidate: f64,
    pub direction_adjusted_improvement: f64,
    pub condition: MetricGateCondition,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateVerdict {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAssessment {
    pub id: Uuid,
    pub project_snapshot_id: Uuid,
    pub baseline_report_id: Uuid,
    pub candidate_report_id: Uuid,
    pub evidence_role: EvidenceRole,
    pub primary_metric: String,
    pub primary_improvement: f64,
    pub gates: Vec<MetricGateResult>,
    pub verdict: CandidateVerdict,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CandidateAssessment {
    pub fn validate_integrity(
        &self,
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        baseline: &EvaluationReport,
        candidate: &EvaluationReport,
    ) -> Result<(), EncoderExperimentError> {
        let mut expected =
            assess_candidate(project, contract, baseline, candidate, self.created_at)?;
        expected.id = self.id;
        expected.fingerprint = expected.reproduce_fingerprint()?;
        if expected != *self {
            return Err(EncoderExperimentError::Integrity(
                "candidate assessment changed or was not derived from its reports".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "id": self.id,
            "project_snapshot_id": self.project_snapshot_id,
            "baseline_report_id": self.baseline_report_id,
            "candidate_report_id": self.candidate_report_id,
            "evidence_role": self.evidence_role,
            "primary_metric": self.primary_metric,
            "primary_improvement": self.primary_improvement,
            "gates": self.gates,
            "verdict": self.verdict,
            "created_at": self.created_at,
        }))
    }
}

pub fn assess_candidate(
    project: &ExternalProjectSnapshot,
    contract: &MetricContract,
    baseline: &EvaluationReport,
    candidate: &EvaluationReport,
    created_at: DateTime<Utc>,
) -> Result<CandidateAssessment, EncoderExperimentError> {
    project.validate_integrity()?;
    contract.validate_integrity()?;
    baseline.validate_integrity(project, contract)?;
    candidate.validate_integrity(project, contract)?;
    if baseline.evidence_role != candidate.evidence_role
        || baseline.suite_key != candidate.suite_key
        || baseline.suite_fingerprint != candidate.suite_fingerprint
        || baseline.model.id == candidate.model.id
        || baseline.model.fingerprint == candidate.model.fingerprint
    {
        return Err(EncoderExperimentError::Validation(
            "candidate assessment requires distinct models on the exact same suite".into(),
        ));
    }
    let role = candidate.evidence_role;
    let relevant = contract
        .gates
        .iter()
        .filter(|gate| {
            gate.role == role
                && gate
                    .suite_key
                    .as_deref()
                    .is_none_or(|suite_key| suite_key == candidate.suite_key)
        })
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return Err(EncoderExperimentError::Validation(
            "candidate assessment has no gates for its evidence role".into(),
        ));
    }
    let mut gates = Vec::with_capacity(relevant.len());
    for gate in relevant {
        let definition = contract
            .definition(&gate.key)
            .expect("gate metric was validated");
        let baseline_value = baseline.metrics[&gate.key];
        let candidate_value = candidate.metrics[&gate.key];
        let improvement = match definition.direction {
            MetricDirection::HigherIsBetter => stable_metric_delta(candidate_value, baseline_value),
            MetricDirection::LowerIsBetter => stable_metric_delta(baseline_value, candidate_value),
        };
        let passed = match gate.condition {
            MetricGateCondition::AtLeast { value } => candidate_value >= value,
            MetricGateCondition::AtMost { value } => candidate_value <= value,
            MetricGateCondition::MinimumImprovement { value } => improvement >= value,
            MetricGateCondition::MaximumRegression { value } => improvement >= -value,
        };
        gates.push(MetricGateResult {
            key: gate.key.clone(),
            baseline: baseline_value,
            candidate: candidate_value,
            direction_adjusted_improvement: improvement,
            condition: gate.condition.clone(),
            passed,
        });
    }
    let primary = contract
        .definition(&contract.primary_metric)
        .expect("primary metric was validated");
    let raw_primary = stable_metric_delta(
        candidate.metrics[&primary.key],
        baseline.metrics[&primary.key],
    );
    let primary_improvement = match primary.direction {
        MetricDirection::HigherIsBetter => raw_primary,
        MetricDirection::LowerIsBetter => -raw_primary,
    };
    let verdict = if gates.iter().all(|gate| gate.passed) {
        CandidateVerdict::Passed
    } else {
        CandidateVerdict::Failed
    };
    let mut assessment = CandidateAssessment {
        id: Uuid::new_v4(),
        project_snapshot_id: project.id,
        baseline_report_id: baseline.id,
        candidate_report_id: candidate.id,
        evidence_role: role,
        primary_metric: primary.key.clone(),
        primary_improvement,
        gates,
        verdict,
        created_at,
        fingerprint: String::new(),
    };
    assessment.fingerprint = assessment.reproduce_fingerprint()?;
    Ok(assessment)
}

fn stable_metric_delta(left: f64, right: f64) -> f64 {
    const SCALE: f64 = 1_000_000_000_000.0;
    ((left - right) * SCALE).round() / SCALE
}

pub fn select_development_candidate(
    assessments: &[CandidateAssessment],
) -> Result<Option<&CandidateAssessment>, EncoderExperimentError> {
    if assessments
        .iter()
        .any(|value| value.evidence_role != EvidenceRole::Development)
    {
        return Err(EncoderExperimentError::Validation(
            "sealed evidence cannot participate in candidate selection".into(),
        ));
    }
    Ok(assessments
        .iter()
        .filter(|value| value.verdict == CandidateVerdict::Passed)
        .max_by(|left, right| {
            left.primary_improvement
                .total_cmp(&right.primary_improvement)
                .then_with(|| right.candidate_report_id.cmp(&left.candidate_report_id))
        }))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCandidateEvidence {
    pub candidate_id: Uuid,
    pub suite_assessments: BTreeMap<String, CandidateAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentSelection {
    pub candidate_id: Uuid,
    pub minimum_primary_improvement: f64,
    pub mean_primary_improvement: f64,
    pub suite_assessment_ids: BTreeMap<String, Uuid>,
}

/// Selects only candidates that pass every independent development suite.
/// The conservative score maximizes the worst suite first, then the mean;
/// the immutable candidate id provides the final stable tie-break.
pub fn select_multi_development_candidate(
    expected_suite_keys: &[String],
    candidates: &[DevelopmentCandidateEvidence],
) -> Result<Option<DevelopmentSelection>, EncoderExperimentError> {
    if expected_suite_keys.is_empty()
        || expected_suite_keys
            .iter()
            .any(|key| key.trim() != key || key.is_empty())
        || expected_suite_keys
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(EncoderExperimentError::Validation(
            "development suite keys must be non-empty, unique, and ordered".into(),
        ));
    }
    let expected = expected_suite_keys.iter().collect::<BTreeSet<_>>();
    let mut candidate_ids = BTreeSet::new();
    let mut eligible = Vec::new();
    for candidate in candidates {
        if candidate.candidate_id.is_nil()
            || !candidate_ids.insert(candidate.candidate_id)
            || candidate.suite_assessments.keys().collect::<BTreeSet<_>>() != expected
            || candidate
                .suite_assessments
                .values()
                .any(|assessment| assessment.evidence_role != EvidenceRole::Development)
        {
            return Err(EncoderExperimentError::Validation(
                "candidate development evidence must cover every exact suite once".into(),
            ));
        }
        if candidate
            .suite_assessments
            .values()
            .any(|assessment| assessment.verdict != CandidateVerdict::Passed)
        {
            continue;
        }
        let improvements = candidate
            .suite_assessments
            .values()
            .map(|assessment| assessment.primary_improvement)
            .collect::<Vec<_>>();
        let minimum = improvements
            .iter()
            .copied()
            .min_by(f64::total_cmp)
            .expect("suite evidence is non-empty");
        let mean = stable_metric_delta(
            improvements.iter().sum::<f64>() / improvements.len() as f64,
            0.0,
        );
        eligible.push(DevelopmentSelection {
            candidate_id: candidate.candidate_id,
            minimum_primary_improvement: minimum,
            mean_primary_improvement: mean,
            suite_assessment_ids: candidate
                .suite_assessments
                .iter()
                .map(|(key, assessment)| (key.clone(), assessment.id))
                .collect(),
        });
    }
    Ok(eligible.into_iter().max_by(|left, right| {
        left.minimum_primary_improvement
            .total_cmp(&right.minimum_primary_improvement)
            .then_with(|| {
                left.mean_primary_improvement
                    .total_cmp(&right.mean_primary_improvement)
            })
            .then_with(|| right.candidate_id.cmp(&left.candidate_id))
    }))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::domain::{
        BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, ParameterValue,
        TrainingCandidate,
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos",
            EncoderTaskKind::RetrievalRanking,
            "14e0a16",
            digest('a'),
            BackendIdentity::new("nomos", "v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('c'))
                    .unwrap(),
                ExternalArtifactIdentity::new("dev", EvidenceRole::Development, 1, digest('d'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('e'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "onnx", 1, digest('f')).unwrap(),
            json!({"top_k":3}),
            time(0),
        )
        .unwrap()
    }

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
    }

    fn contract() -> MetricContract {
        MetricContract::create(
            vec![
                MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap(),
                MetricDefinition::new("recall_at_3", MetricDirection::HigherIsBetter).unwrap(),
                MetricDefinition::new("invalid_call_rate", MetricDirection::LowerIsBetter).unwrap(),
            ],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MinimumImprovement { value: 0.001 },
                )
                .unwrap(),
                MetricGate::new(
                    "recall_at_3",
                    EvidenceRole::Development,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "invalid_call_rate",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::AtMost { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn report(
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        model: ModelArtifactIdentity,
        role: EvidenceRole,
        mrr: f64,
        recall: f64,
        invalid: f64,
        second: u32,
    ) -> EvaluationReport {
        EvaluationReport::create(
            project,
            model,
            role,
            if role == EvidenceRole::Development {
                "dev"
            } else {
                "sealed"
            },
            if role == EvidenceRole::Development {
                digest('1')
            } else {
                digest('2')
            },
            contract,
            BTreeMap::from([
                ("mrr".into(), mrr),
                ("recall_at_3".into(), recall),
                ("invalid_call_rate".into(), invalid),
            ]),
            100,
            time(second),
        )
        .unwrap()
    }

    #[test]
    fn direction_aware_gates_require_real_improvement_without_regression() {
        let project = project();
        let contract = contract();
        let baseline = report(
            &project,
            &contract,
            project.baseline_model.clone(),
            EvidenceRole::Development,
            0.895,
            0.942,
            0.0,
            1,
        );
        let candidate_model =
            ModelArtifactIdentity::new("candidate", "onnx", 1, digest('9')).unwrap();
        let candidate = report(
            &project,
            &contract,
            candidate_model,
            EvidenceRole::Development,
            0.899,
            0.942,
            0.0,
            2,
        );
        let assessment =
            assess_candidate(&project, &contract, &baseline, &candidate, time(3)).unwrap();
        assert_eq!(assessment.verdict, CandidateVerdict::Passed);
        assert_eq!(assessment.primary_improvement, 0.004);
    }

    #[test]
    fn sealed_reports_cannot_select_an_optimization_candidate() {
        let project = project();
        let contract = contract();
        let baseline = report(
            &project,
            &contract,
            project.baseline_model.clone(),
            EvidenceRole::SealedAcceptance,
            0.9,
            1.0,
            0.0,
            1,
        );
        let candidate = report(
            &project,
            &contract,
            ModelArtifactIdentity::new("candidate", "onnx", 1, digest('9')).unwrap(),
            EvidenceRole::SealedAcceptance,
            0.9,
            1.0,
            0.0,
            2,
        );
        let assessment =
            assess_candidate(&project, &contract, &baseline, &candidate, time(3)).unwrap();
        assert!(select_development_candidate(&[assessment]).is_err());
    }

    #[test]
    fn nomos_candidate_parameters_remain_adapter_owned_and_finite() {
        let project = project();
        let candidate = TrainingCandidate::create(
            &project,
            1,
            600,
            BTreeMap::from([
                ("learning_rate".into(), ParameterValue::Number(0.000003)),
                ("loss".into(), ParameterValue::Text("triplet".into())),
            ]),
        )
        .unwrap();
        candidate.validate_integrity(&project).unwrap();
    }

    fn selection_assessment(
        project_id: Uuid,
        improvement: f64,
        verdict: CandidateVerdict,
        second: u32,
    ) -> CandidateAssessment {
        CandidateAssessment {
            id: Uuid::new_v4(),
            project_snapshot_id: project_id,
            baseline_report_id: Uuid::new_v4(),
            candidate_report_id: Uuid::new_v4(),
            evidence_role: EvidenceRole::Development,
            primary_metric: "mrr".into(),
            primary_improvement: improvement,
            gates: vec![],
            verdict,
            created_at: time(second),
            fingerprint: digest('a'),
        }
    }

    #[test]
    fn multi_suite_selection_requires_every_suite_and_uses_worst_suite_first() {
        let project_id = Uuid::new_v4();
        let first_id = Uuid::from_u128(1);
        let second_id = Uuid::from_u128(2);
        let evidence = vec![
            DevelopmentCandidateEvidence {
                candidate_id: first_id,
                suite_assessments: BTreeMap::from([
                    (
                        "generic".into(),
                        selection_assessment(project_id, 0.05, CandidateVerdict::Passed, 1),
                    ),
                    (
                        "retired".into(),
                        selection_assessment(project_id, -0.01, CandidateVerdict::Failed, 2),
                    ),
                ]),
            },
            DevelopmentCandidateEvidence {
                candidate_id: second_id,
                suite_assessments: BTreeMap::from([
                    (
                        "generic".into(),
                        selection_assessment(project_id, 0.02, CandidateVerdict::Passed, 3),
                    ),
                    (
                        "retired".into(),
                        selection_assessment(project_id, 0.01, CandidateVerdict::Passed, 4),
                    ),
                ]),
            },
        ];
        let selected =
            select_multi_development_candidate(&["generic".into(), "retired".into()], &evidence)
                .unwrap()
                .unwrap();
        assert_eq!(selected.candidate_id, second_id);
        assert_eq!(selected.minimum_primary_improvement, 0.01);
        assert_eq!(selected.mean_primary_improvement, 0.015);
    }
}
