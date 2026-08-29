use std::collections::{BTreeMap, BTreeSet};

use analysis_core::contract::DiagnosticContract;
use generation_core::{
    dimensions::expand_generation_cells,
    domain::{DatasetDefinition, GenerationCell},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::protocol::{OptimizationProtocol, RecommendationKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentCellCoverage {
    pub cell: GenerationCell,
    pub accepted: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationSourceIdentity {
    pub analysis_report_id: Uuid,
    pub analysis_fingerprint: String,
    pub analysis_protocol_fingerprint: String,
    pub diagnostic_contract_fingerprint: String,
    pub evaluation_run_id: Uuid,
    pub evaluation_input_fingerprint: String,
    pub evaluation_protocol_fingerprint: String,
    pub cohort_fingerprint: String,
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub coverage_fingerprint: String,
    pub comparison_id: Option<Uuid>,
    pub comparison_fingerprint: Option<String>,
    pub optimization_protocol_fingerprint: String,
    pub training_configuration_space_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationEvidence {
    pub source_identity: OptimizationSourceIdentity,
    pub diagnostics: DiagnosticContract,
    pub current_coverage: Vec<CurrentCellCoverage>,
    pub fingerprint: String,
}

pub fn canonical_coverage(
    dataset: &DatasetDefinition,
    accepted_by_cell: &BTreeMap<String, u32>,
) -> Result<Vec<CurrentCellCoverage>, OptimizationEvidenceError> {
    let valid_cells = expand_generation_cells(dataset)
        .into_iter()
        .map(|cell| (cell.key(), cell))
        .collect::<BTreeMap<_, _>>();
    let unknown_coverage = accepted_by_cell
        .keys()
        .filter(|key| !valid_cells.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown_coverage.is_empty() {
        return Err(OptimizationEvidenceError::UnknownCoverageCells(
            unknown_coverage,
        ));
    }
    Ok(valid_cells
        .into_iter()
        .map(|(key, cell)| CurrentCellCoverage {
            cell,
            accepted: accepted_by_cell.get(&key).copied().unwrap_or(0),
        })
        .collect())
}

pub fn coverage_fingerprint(
    dataset_id: Uuid,
    coverage: &[CurrentCellCoverage],
) -> Result<String, OptimizationEvidenceError> {
    artifact_core::fingerprint(&CoverageFingerprintInput {
        dataset_id,
        cells: coverage,
    })
    .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))
}

impl OptimizationEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        diagnostics: DiagnosticContract,
        dataset: &DatasetDefinition,
        snapshot_id: Uuid,
        snapshot_fingerprint: impl Into<String>,
        accepted_by_cell: &BTreeMap<String, u32>,
        protocol: &OptimizationProtocol,
        training_configuration_space_fingerprint: Option<String>,
        require_complete_source: bool,
    ) -> Result<Self, OptimizationEvidenceError> {
        protocol.validate()?;
        diagnostics.validate(require_complete_source)?;
        let protocol_fingerprint = protocol.fingerprint()?;
        let snapshot_fingerprint = snapshot_fingerprint.into();
        if snapshot_id == Uuid::nil() || snapshot_fingerprint.trim().is_empty() {
            return Err(OptimizationEvidenceError::SnapshotIdentity);
        }
        let training_requested = protocol
            .recommendation_kinds
            .contains(&RecommendationKind::TrainingConfiguration);
        if training_requested != training_configuration_space_fingerprint.is_some() {
            return Err(OptimizationEvidenceError::TrainingConfigurationSpace);
        }
        if training_configuration_space_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| fingerprint.trim().is_empty())
        {
            return Err(OptimizationEvidenceError::TrainingConfigurationSpace);
        }

        let current_coverage = canonical_coverage(dataset, accepted_by_cell)?;
        let dataset_fingerprint = artifact_core::fingerprint(dataset)
            .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?;
        let coverage_fingerprint = coverage_fingerprint(dataset.id, &current_coverage)?;
        let source = &diagnostics.source_identity;
        let source_identity = OptimizationSourceIdentity {
            analysis_report_id: diagnostics.analysis_report_id,
            analysis_fingerprint: diagnostics.analysis_fingerprint.clone(),
            analysis_protocol_fingerprint: diagnostics.analysis_protocol_fingerprint.clone(),
            diagnostic_contract_fingerprint: diagnostics.fingerprint.clone(),
            evaluation_run_id: source.evaluation_run_id,
            evaluation_input_fingerprint: source.evaluation_input_fingerprint.clone(),
            evaluation_protocol_fingerprint: source.evaluation_protocol_fingerprint.clone(),
            cohort_fingerprint: source.cohort_fingerprint.clone(),
            dataset_id: dataset.id,
            dataset_fingerprint,
            snapshot_id,
            snapshot_fingerprint,
            coverage_fingerprint,
            comparison_id: source.comparison_id,
            comparison_fingerprint: source.comparison_fingerprint.clone(),
            optimization_protocol_fingerprint: protocol_fingerprint,
            training_configuration_space_fingerprint,
        };
        let mut evidence = Self {
            source_identity,
            diagnostics,
            current_coverage,
            fingerprint: String::new(),
        };
        evidence.fingerprint = evidence
            .reproduce_fingerprint()
            .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?;
        Ok(evidence)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.fingerprint.clear();
        artifact_core::fingerprint(&input)
    }

    pub fn rebind_protocol(
        &self,
        protocol: &OptimizationProtocol,
        training_configuration_space_fingerprint: Option<String>,
    ) -> Result<Self, OptimizationEvidenceError> {
        protocol.validate()?;
        self.diagnostics.validate(true)?;
        if self
            .reproduce_fingerprint()
            .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?
            != self.fingerprint
        {
            return Err(OptimizationEvidenceError::EvidenceFingerprint);
        }
        let expected_coverage_fingerprint = artifact_core::fingerprint(&CoverageFingerprintInput {
            dataset_id: self.source_identity.dataset_id,
            cells: &self.current_coverage,
        })
        .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?;
        if expected_coverage_fingerprint != self.source_identity.coverage_fingerprint {
            return Err(OptimizationEvidenceError::CoverageFingerprint);
        }
        let training_requested = protocol
            .recommendation_kinds
            .contains(&RecommendationKind::TrainingConfiguration);
        if training_requested != training_configuration_space_fingerprint.is_some()
            || training_configuration_space_fingerprint
                .as_ref()
                .is_some_and(|fingerprint| fingerprint.trim().is_empty())
        {
            return Err(OptimizationEvidenceError::TrainingConfigurationSpace);
        }
        let mut rebound = self.clone();
        rebound.source_identity.optimization_protocol_fingerprint = protocol.fingerprint()?;
        rebound
            .source_identity
            .training_configuration_space_fingerprint = training_configuration_space_fingerprint;
        rebound.fingerprint.clear();
        rebound.fingerprint = rebound
            .reproduce_fingerprint()
            .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?;
        rebound.validate(protocol, true)?;
        Ok(rebound)
    }

    pub fn validate(
        &self,
        protocol: &OptimizationProtocol,
        require_complete_source: bool,
    ) -> Result<(), OptimizationEvidenceError> {
        protocol.validate()?;
        self.diagnostics.validate(require_complete_source)?;
        if self.source_identity.analysis_report_id != self.diagnostics.analysis_report_id
            || self.source_identity.analysis_fingerprint != self.diagnostics.analysis_fingerprint
            || self.source_identity.analysis_protocol_fingerprint
                != self.diagnostics.analysis_protocol_fingerprint
            || self.source_identity.diagnostic_contract_fingerprint != self.diagnostics.fingerprint
            || self.source_identity.evaluation_run_id
                != self.diagnostics.source_identity.evaluation_run_id
            || self.source_identity.evaluation_input_fingerprint
                != self
                    .diagnostics
                    .source_identity
                    .evaluation_input_fingerprint
            || self.source_identity.evaluation_protocol_fingerprint
                != self
                    .diagnostics
                    .source_identity
                    .evaluation_protocol_fingerprint
            || self.source_identity.cohort_fingerprint
                != self.diagnostics.source_identity.cohort_fingerprint
            || self.source_identity.comparison_id != self.diagnostics.source_identity.comparison_id
            || self.source_identity.comparison_fingerprint
                != self.diagnostics.source_identity.comparison_fingerprint
            || self.source_identity.optimization_protocol_fingerprint != protocol.fingerprint()?
        {
            return Err(OptimizationEvidenceError::SourceMismatch);
        }
        let mut keys = BTreeSet::new();
        if self.current_coverage.iter().any(|coverage| {
            !keys.insert(coverage.cell.key())
                || coverage.cell.label.trim().is_empty()
                || coverage
                    .cell
                    .dimensions
                    .iter()
                    .any(|(name, value)| name.trim().is_empty() || value.trim().is_empty())
        }) {
            return Err(OptimizationEvidenceError::Coverage);
        }
        let expected_coverage_fingerprint = artifact_core::fingerprint(&CoverageFingerprintInput {
            dataset_id: self.source_identity.dataset_id,
            cells: &self.current_coverage,
        })
        .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?;
        if expected_coverage_fingerprint != self.source_identity.coverage_fingerprint {
            return Err(OptimizationEvidenceError::CoverageFingerprint);
        }
        if self
            .reproduce_fingerprint()
            .map_err(|error| OptimizationEvidenceError::Fingerprint(error.to_string()))?
            != self.fingerprint
        {
            return Err(OptimizationEvidenceError::EvidenceFingerprint);
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct CoverageFingerprintInput<'a> {
    dataset_id: Uuid,
    cells: &'a [CurrentCellCoverage],
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OptimizationEvidenceError {
    #[error(transparent)]
    Protocol(#[from] crate::protocol::OptimizationProtocolError),
    #[error(transparent)]
    Diagnostic(#[from] analysis_core::contract::DiagnosticContractError),
    #[error("snapshot identity must contain a non-nil ID and non-empty fingerprint")]
    SnapshotIdentity,
    #[error("training candidates require one exact supported configuration-space fingerprint")]
    TrainingConfigurationSpace,
    #[error("current coverage contains cells outside the dataset definition: {0:?}")]
    UnknownCoverageCells(Vec<String>),
    #[error("optimization evidence source identities do not match")]
    SourceMismatch,
    #[error("current coverage is duplicated or contains invalid cell identities")]
    Coverage,
    #[error("current coverage fingerprint does not reproduce")]
    CoverageFingerprint,
    #[error("optimization evidence fingerprint does not reproduce")]
    EvidenceFingerprint,
    #[error("could not fingerprint optimization evidence: {0}")]
    Fingerprint(String),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use analysis_core::contract::{
        CellDiagnosticEvidence, DiagnosticContract, DiagnosticSourceIdentity,
    };
    use analysis_core::domain::{FindingIdentity, FindingKind};
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use uuid::Uuid;

    use crate::protocol::OptimizationProtocol;

    use super::{OptimizationEvidence, OptimizationEvidenceError};

    #[test]
    fn binds_diagnostics_dataset_snapshot_coverage_and_protocol() {
        let dataset = dataset();
        let diagnostics = diagnostics();
        let protocol = OptimizationProtocol::legacy(20, 1);
        let cell = generation_core::dimensions::expand_generation_cells(&dataset)
            .into_iter()
            .next()
            .expect("cell");
        let evidence = OptimizationEvidence::new(
            diagnostics,
            &dataset,
            Uuid::new_v4(),
            "sha256:snapshot",
            &BTreeMap::from([(cell.key(), 12)]),
            &protocol,
            None,
            true,
        )
        .expect("optimization evidence");

        evidence.validate(&protocol, true).expect("valid evidence");
        assert_eq!(evidence.current_coverage.len(), 4);
        assert_eq!(
            evidence
                .current_coverage
                .iter()
                .find(|coverage| coverage.cell == cell)
                .map(|coverage| coverage.accepted),
            Some(12)
        );
        assert_eq!(
            evidence.reproduce_fingerprint().expect("fingerprint"),
            evidence.fingerprint
        );
    }

    #[test]
    fn rejects_unknown_coverage_and_training_space_mismatches() {
        let dataset = dataset();
        let protocol = OptimizationProtocol::legacy(20, 1);
        let error = OptimizationEvidence::new(
            diagnostics(),
            &dataset,
            Uuid::new_v4(),
            "sha256:snapshot",
            &BTreeMap::from([("unknown".into(), 1)]),
            &protocol,
            None,
            true,
        )
        .expect_err("unknown coverage");
        assert!(matches!(
            error,
            OptimizationEvidenceError::UnknownCoverageCells(_)
        ));
    }

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("dimension"),
            ],
        )
        .expect("dataset")
    }

    fn diagnostics() -> DiagnosticContract {
        let identity = BTreeMap::from([
            ("label".into(), "billing".into()),
            ("style".into(), "clean".into()),
        ]);
        let mut cell = CellDiagnosticEvidence {
            finding_key: FindingIdentity {
                kind: FindingKind::Cell,
                attributes: identity.clone(),
            }
            .key(),
            finding_fingerprint: "sha256:finding".into(),
            identity,
            support: 10,
            error_count: 4,
            error_rate: 0.4,
            error_rate_lift: 0.2,
            error_share: 1.0,
            high_confidence_error_severity: 0.6,
            marginal_error_count: 4,
            cumulative_error_coverage: 1.0,
            comparison: None,
            latest_review: None,
            evidence_fingerprint: String::new(),
        };
        cell.evidence_fingerprint = cell.reproduce_fingerprint().expect("cell fingerprint");
        let mut diagnostics = DiagnosticContract {
            analysis_report_id: Uuid::new_v4(),
            analysis_fingerprint: "sha256:analysis".into(),
            analysis_protocol_fingerprint: "sha256:analysis-protocol".into(),
            source_identity: DiagnosticSourceIdentity {
                evaluation_run_id: Uuid::new_v4(),
                evaluation_input_fingerprint: "sha256:evaluation".into(),
                evaluation_protocol_fingerprint: "sha256:evaluation-protocol".into(),
                cohort_fingerprint: "sha256:cohort".into(),
                prediction_count: 20,
                comparison_id: None,
                comparison_fingerprint: None,
                legacy: false,
            },
            minimum_support: 1,
            prediction_count: 20,
            error_count: 4,
            baseline_error_rate: 0.2,
            cells: vec![cell],
            fingerprint: String::new(),
        };
        diagnostics.fingerprint = diagnostics
            .reproduce_fingerprint()
            .expect("diagnostic fingerprint");
        diagnostics
    }
}
