//! Immutable authority binding benchmark suites to one clean global leakage check.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use dataset_core::domain::SnapshotSplit;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    benchmark::{BenchmarkSuite, BenchmarkSuiteKind},
    contamination::{
        ContaminationKind, ContaminationPolicy, ContaminationReport, ContaminationStatus,
    },
};

/// One immutable, decision-grade benchmark configuration.
///
/// The bundle pins suite identities and the single global contamination report
/// that checked their combined cohort population. It intentionally has no
/// contamination-override input or field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkBundle {
    pub id: Uuid,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Option<Uuid>,
    pub sealed_suite_fingerprint: Option<String>,
    pub contamination_report_id: Uuid,
    pub contamination_report_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkBundle {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkBundleError> {
        bundle_fingerprint(self)
    }

    pub fn validate_fingerprint(&self) -> Result<(), BenchmarkBundleError> {
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkBundleError::BundleFingerprintMismatch);
        }
        Ok(())
    }

    pub fn binding(&self) -> Result<BenchmarkBundleBinding, BenchmarkBundleError> {
        self.validate_fingerprint()?;
        Ok(BenchmarkBundleBinding {
            bundle_id: self.id,
            bundle_fingerprint: self.fingerprint.clone(),
            development_suite_id: self.development_suite_id,
            development_suite_fingerprint: self.development_suite_fingerprint.clone(),
            sealed_suite_id: self.sealed_suite_id,
            sealed_suite_fingerprint: self.sealed_suite_fingerprint.clone(),
            contamination_report_id: self.contamination_report_id,
            contamination_report_fingerprint: self.contamination_report_fingerprint.clone(),
        })
    }
}

/// Compact workflow-facing pin for a persisted [`BenchmarkBundle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkBundleBinding {
    pub bundle_id: Uuid,
    pub bundle_fingerprint: String,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Option<Uuid>,
    pub sealed_suite_fingerprint: Option<String>,
    pub contamination_report_id: Uuid,
    pub contamination_report_fingerprint: String,
}

impl BenchmarkBundleBinding {
    pub fn reproduce_bundle_fingerprint(&self) -> Result<String, BenchmarkBundleError> {
        pins_fingerprint(BenchmarkBundlePins {
            development_suite_id: self.development_suite_id,
            development_suite_fingerprint: &self.development_suite_fingerprint,
            sealed_suite_id: self.sealed_suite_id,
            sealed_suite_fingerprint: self.sealed_suite_fingerprint.as_deref(),
            contamination_report_id: self.contamination_report_id,
            contamination_report_fingerprint: &self.contamination_report_fingerprint,
        })
    }

    pub fn validate_fingerprint(&self) -> Result<(), BenchmarkBundleError> {
        if self.reproduce_bundle_fingerprint()? != self.bundle_fingerprint {
            return Err(BenchmarkBundleError::BundleFingerprintMismatch);
        }
        Ok(())
    }

    pub fn validate_bundle(&self, bundle: &BenchmarkBundle) -> Result<(), BenchmarkBundleError> {
        self.validate_fingerprint()?;
        bundle.validate_fingerprint()?;
        let expected = BenchmarkBundleBinding {
            bundle_id: bundle.id,
            bundle_fingerprint: bundle.fingerprint.clone(),
            development_suite_id: bundle.development_suite_id,
            development_suite_fingerprint: bundle.development_suite_fingerprint.clone(),
            sealed_suite_id: bundle.sealed_suite_id,
            sealed_suite_fingerprint: bundle.sealed_suite_fingerprint.clone(),
            contamination_report_id: bundle.contamination_report_id,
            contamination_report_fingerprint: bundle.contamination_report_fingerprint.clone(),
        };
        if self != &expected {
            return Err(BenchmarkBundleError::BindingMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BenchmarkBundleError {
    #[error("benchmark suite fingerprint does not reproduce: {0}")]
    SuiteFingerprintMismatch(Uuid),
    #[error("benchmark suite is invalid: {suite_id}: {reason}")]
    InvalidSuite { suite_id: Uuid, reason: String },
    #[error("global contamination report fingerprint does not reproduce")]
    ContaminationFingerprintMismatch,
    #[error("development suite must have the development kind")]
    DevelopmentSuiteKind,
    #[error("sealed suite must have the sealed_acceptance kind")]
    SealedSuiteKind,
    #[error("development and sealed suites must describe the same task")]
    TaskMismatch,
    #[error("development and sealed suites must use the same ordered labels")]
    LabelsMismatch,
    #[error("development and sealed suites must have distinct identities")]
    DuplicateSuiteId,
    #[error("development and sealed suites must have distinct fingerprints")]
    DuplicateSuiteFingerprint,
    #[error("cohort appears more than once in a benchmark bundle: {0}")]
    DuplicateCohort(Uuid),
    #[error("development and sealed suites share cohort {0}")]
    SharedCohort(Uuid),
    #[error("development and sealed suites share snapshot {snapshot_id} split {split:?}")]
    SharedSnapshotSplit {
        snapshot_id: Uuid,
        split: SnapshotSplit,
    },
    #[error("global contamination report must be clean; overrides are not accepted")]
    ContaminationNotClean,
    #[error("global contamination report must use the zero-tolerance policy")]
    ContaminationPolicyNotStrict,
    #[error("clean global contamination report contains findings or inconsistent counts")]
    ContaminationFindings,
    #[error("global contamination report does not cover exactly the suite cohort union")]
    ContaminationCoverage,
    #[error("global contamination report has orphan or foreign cohort bindings")]
    ContaminationBindings,
    #[error("global contamination report cohort fingerprint differs for {0}")]
    CohortFingerprintMismatch(Uuid),
    #[error("global contamination report role-decision fingerprint differs for {0}")]
    RoleDecisionFingerprintMismatch(Uuid),
    #[error("benchmark bundle fingerprint does not reproduce")]
    BundleFingerprintMismatch,
    #[error("benchmark bundle binding does not match its bundle")]
    BindingMismatch,
    #[error("could not fingerprint benchmark bundle: {0}")]
    Fingerprint(String),
}

/// Builds the authority that makes development and sealed evidence jointly eligible.
///
/// The global report must be independently built over the exact combined
/// cohort population. A blocked report cannot be authorized through this API.
pub fn build_benchmark_bundle(
    development: &BenchmarkSuite,
    sealed: Option<&BenchmarkSuite>,
    contamination: &ContaminationReport,
) -> Result<BenchmarkBundle, BenchmarkBundleError> {
    validate_suite(development)?;
    if development.kind != BenchmarkSuiteKind::Development {
        return Err(BenchmarkBundleError::DevelopmentSuiteKind);
    }
    if let Some(sealed) = sealed {
        validate_suite(sealed)?;
        if sealed.kind != BenchmarkSuiteKind::SealedAcceptance {
            return Err(BenchmarkBundleError::SealedSuiteKind);
        }
        if development.task != sealed.task {
            return Err(BenchmarkBundleError::TaskMismatch);
        }
        if development.labels != sealed.labels {
            return Err(BenchmarkBundleError::LabelsMismatch);
        }
        if development.id == sealed.id {
            return Err(BenchmarkBundleError::DuplicateSuiteId);
        }
        if development.fingerprint == sealed.fingerprint {
            return Err(BenchmarkBundleError::DuplicateSuiteFingerprint);
        }
    }
    if contamination
        .reproduce_fingerprint()
        .map_err(|error| BenchmarkBundleError::Fingerprint(error.to_string()))?
        != contamination.fingerprint
    {
        return Err(BenchmarkBundleError::ContaminationFingerprintMismatch);
    }
    if contamination.status != ContaminationStatus::Clean {
        return Err(BenchmarkBundleError::ContaminationNotClean);
    }
    if contamination.policy != ContaminationPolicy::default() {
        return Err(BenchmarkBundleError::ContaminationPolicyNotStrict);
    }
    let expected_kinds = [
        ContaminationKind::SourceRow,
        ContaminationKind::ExactText,
        ContaminationKind::NormalizedText,
        ContaminationKind::Group,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if contamination
        .counts
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != expected_kinds
        || contamination.counts.values().any(|count| *count != 0)
        || !contamination.findings.is_empty()
        || !contamination.reasons.is_empty()
    {
        return Err(BenchmarkBundleError::ContaminationFindings);
    }

    let development_ids = suite_cohort_ids(development)?;
    let sealed_ids = sealed
        .map(suite_cohort_ids)
        .transpose()?
        .unwrap_or_default();
    if let Some(shared) = development_ids.intersection(&sealed_ids).next() {
        return Err(BenchmarkBundleError::SharedCohort(*shared));
    }
    if let Some(sealed) = sealed {
        let development_evidence = development
            .cohorts
            .iter()
            .map(|cohort| (cohort.snapshot_id, cohort.split))
            .collect::<BTreeSet<_>>();
        if let Some((snapshot_id, split)) = sealed
            .cohorts
            .iter()
            .map(|cohort| (cohort.snapshot_id, cohort.split))
            .find(|evidence| development_evidence.contains(evidence))
        {
            return Err(BenchmarkBundleError::SharedSnapshotSplit { snapshot_id, split });
        }
    }

    let suite_ids = development_ids
        .union(&sealed_ids)
        .copied()
        .collect::<BTreeSet<_>>();
    let report_ids = exact_report_cohort_ids(contamination)?;
    if report_ids != suite_ids {
        return Err(BenchmarkBundleError::ContaminationCoverage);
    }
    if contamination
        .cohort_fingerprints
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != report_ids
        || contamination
            .role_decision_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != report_ids
    {
        return Err(BenchmarkBundleError::ContaminationBindings);
    }
    for cohort in development
        .cohorts
        .iter()
        .chain(sealed.into_iter().flat_map(|suite| &suite.cohorts))
    {
        if contamination.cohort_fingerprints.get(&cohort.cohort_id)
            != Some(&cohort.cohort_fingerprint)
        {
            return Err(BenchmarkBundleError::CohortFingerprintMismatch(
                cohort.cohort_id,
            ));
        }
        if contamination
            .role_decision_fingerprints
            .get(&cohort.cohort_id)
            != Some(&cohort.role_decision_fingerprint)
        {
            return Err(BenchmarkBundleError::RoleDecisionFingerprintMismatch(
                cohort.cohort_id,
            ));
        }
    }

    let mut bundle = BenchmarkBundle {
        id: Uuid::new_v4(),
        development_suite_id: development.id,
        development_suite_fingerprint: development.fingerprint.clone(),
        sealed_suite_id: sealed.map(|suite| suite.id),
        sealed_suite_fingerprint: sealed.map(|suite| suite.fingerprint.clone()),
        contamination_report_id: contamination.id,
        contamination_report_fingerprint: contamination.fingerprint.clone(),
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    bundle.fingerprint = bundle_fingerprint(&bundle)?;
    bundle.validate_fingerprint()?;
    Ok(bundle)
}

fn validate_suite(suite: &BenchmarkSuite) -> Result<(), BenchmarkBundleError> {
    suite.validate_integrity().map_err(|error| match error {
        crate::benchmark::BenchmarkError::FingerprintMismatch => {
            BenchmarkBundleError::SuiteFingerprintMismatch(suite.id)
        }
        other => BenchmarkBundleError::InvalidSuite {
            suite_id: suite.id,
            reason: other.to_string(),
        },
    })
}

fn suite_cohort_ids(suite: &BenchmarkSuite) -> Result<BTreeSet<Uuid>, BenchmarkBundleError> {
    let mut ids = BTreeSet::new();
    for cohort in &suite.cohorts {
        if !ids.insert(cohort.cohort_id) {
            return Err(BenchmarkBundleError::DuplicateCohort(cohort.cohort_id));
        }
    }
    Ok(ids)
}

fn exact_report_cohort_ids(
    contamination: &ContaminationReport,
) -> Result<BTreeSet<Uuid>, BenchmarkBundleError> {
    let mut ids = BTreeSet::new();
    for cohort_id in &contamination.cohort_ids {
        if !ids.insert(*cohort_id) {
            return Err(BenchmarkBundleError::DuplicateCohort(*cohort_id));
        }
    }
    Ok(ids)
}

fn bundle_fingerprint(bundle: &BenchmarkBundle) -> Result<String, BenchmarkBundleError> {
    pins_fingerprint(BenchmarkBundlePins {
        development_suite_id: bundle.development_suite_id,
        development_suite_fingerprint: &bundle.development_suite_fingerprint,
        sealed_suite_id: bundle.sealed_suite_id,
        sealed_suite_fingerprint: bundle.sealed_suite_fingerprint.as_deref(),
        contamination_report_id: bundle.contamination_report_id,
        contamination_report_fingerprint: &bundle.contamination_report_fingerprint,
    })
}

#[derive(Serialize)]
struct BenchmarkBundlePins<'a> {
    development_suite_id: Uuid,
    development_suite_fingerprint: &'a str,
    sealed_suite_id: Option<Uuid>,
    sealed_suite_fingerprint: Option<&'a str>,
    contamination_report_id: Uuid,
    contamination_report_fingerprint: &'a str,
}

fn pins_fingerprint(pins: BenchmarkBundlePins<'_>) -> Result<String, BenchmarkBundleError> {
    artifact_core::fingerprint(&pins)
        .map_err(|error| BenchmarkBundleError::Fingerprint(error.to_string()))
}

#[cfg(test)]
mod tests {
    use dataset_core::domain::SnapshotSplit;
    use evaluation_core::domain::EvaluationProtocol;

    use super::*;
    use crate::{
        benchmark::{
            AcceptanceContract, BenchmarkCohortEvidence, BenchmarkCohortRequest, BenchmarkMetric,
            BenchmarkSuiteRequest, MetricRequirement, MetricTarget, build_benchmark_suite,
        },
        contamination::{
            CohortContaminationInput, ContaminationMember, ContaminationPolicy, check_contamination,
        },
        governance::{
            CohortOrigin, CohortRole, CohortRoleDecision, DisclosureLevel, EvaluationCohort,
        },
    };

    #[derive(Clone)]
    struct CohortFixture {
        evidence: BenchmarkCohortEvidence,
        member: ContaminationMember,
    }

    fn cohort_fixture(
        name: &str,
        role: CohortRole,
        snapshot_id: Uuid,
        split: SnapshotSplit,
    ) -> CohortFixture {
        let cohort = EvaluationCohort::new(
            name,
            snapshot_id,
            format!("sha256:{name}-snapshot"),
            split,
            CohortOrigin::InternalSnapshot,
        )
        .expect("cohort");
        let role = CohortRoleDecision::initial(&cohort, role, "bundle fixture").expect("role");
        let member = ContaminationMember {
            snapshot_member_id: Uuid::new_v4(),
            snapshot_id,
            source_row_id: Uuid::new_v4(),
            text: format!("unique {name} example"),
            group_id: None,
        };
        CohortFixture {
            evidence: BenchmarkCohortEvidence { cohort, role },
            member,
        }
    }

    fn contamination(fixtures: &[CohortFixture]) -> ContaminationReport {
        check_contamination(
            fixtures
                .iter()
                .map(|fixture| CohortContaminationInput {
                    cohort: fixture.evidence.cohort.clone(),
                    role: fixture.evidence.role.clone(),
                    members: vec![fixture.member.clone()],
                })
                .collect(),
            None,
            ContaminationPolicy::default(),
        )
        .expect("clean contamination report")
    }

    fn suite(kind: BenchmarkSuiteKind, fixtures: &[CohortFixture]) -> BenchmarkSuite {
        let local_report = contamination(fixtures);
        let sealed = kind == BenchmarkSuiteKind::SealedAcceptance;
        let split = fixtures[0].evidence.cohort.split;
        build_benchmark_suite(
            BenchmarkSuiteRequest {
                name: format!("{kind:?}"),
                kind,
                task: "classify support requests".into(),
                labels: vec!["billing".into(), "fraud".into()],
                required_model_formats: vec!["fixture".into()],
                cohorts: fixtures
                    .iter()
                    .map(|fixture| BenchmarkCohortRequest {
                        cohort_id: fixture.evidence.cohort.id,
                        protocol: EvaluationProtocol {
                            split,
                            ..EvaluationProtocol::default()
                        },
                        disclosure: if sealed {
                            DisclosureLevel::Aggregate
                        } else {
                            DisclosureLevel::Slices
                        },
                        adaptation_eligible: !sealed,
                    })
                    .collect(),
                contract: AcceptanceContract {
                    metric_requirements: vec![MetricRequirement {
                        target: MetricTarget::Overall,
                        metric: BenchmarkMetric::Accuracy,
                        minimum: Some(0.75),
                        maximum: None,
                        minimum_support: 1,
                    }],
                    regression: None,
                },
            },
            fixtures
                .iter()
                .map(|fixture| fixture.evidence.clone())
                .collect(),
            &local_report,
            None,
        )
        .expect("suite")
    }

    fn valid_pair() -> (
        BenchmarkSuite,
        BenchmarkSuite,
        ContaminationReport,
        CohortFixture,
        CohortFixture,
    ) {
        let development = cohort_fixture(
            "development",
            CohortRole::Development,
            Uuid::new_v4(),
            SnapshotSplit::Validation,
        );
        let sealed = cohort_fixture(
            "sealed",
            CohortRole::SealedAcceptance,
            Uuid::new_v4(),
            SnapshotSplit::Test,
        );
        let development_suite = suite(
            BenchmarkSuiteKind::Development,
            std::slice::from_ref(&development),
        );
        let sealed_suite = suite(
            BenchmarkSuiteKind::SealedAcceptance,
            std::slice::from_ref(&sealed),
        );
        let global_report = contamination(&[development.clone(), sealed.clone()]);
        (
            development_suite,
            sealed_suite,
            global_report,
            development,
            sealed,
        )
    }

    #[test]
    fn builds_valid_development_only_bundle_and_binding() {
        let development = cohort_fixture(
            "development",
            CohortRole::Development,
            Uuid::new_v4(),
            SnapshotSplit::Validation,
        );
        let suite = suite(
            BenchmarkSuiteKind::Development,
            std::slice::from_ref(&development),
        );
        let report = contamination(std::slice::from_ref(&development));
        let bundle = build_benchmark_bundle(&suite, None, &report).expect("bundle");
        bundle.validate_fingerprint().expect("fingerprint");
        assert_eq!(bundle.development_suite_id, suite.id);
        assert_eq!(bundle.sealed_suite_id, None);
        let binding = bundle.binding().expect("binding");
        binding.validate_bundle(&bundle).expect("binding matches");
    }

    #[test]
    fn builds_valid_development_and_sealed_bundle() {
        let (development, sealed, report, _, _) = valid_pair();
        let bundle = build_benchmark_bundle(&development, Some(&sealed), &report).expect("bundle");
        assert_eq!(bundle.sealed_suite_id, Some(sealed.id));
        assert_eq!(
            bundle.sealed_suite_fingerprint.as_deref(),
            Some(sealed.fingerprint.as_str())
        );
        assert_eq!(
            bundle.reproduce_fingerprint().expect("fingerprint"),
            bundle.fingerprint
        );
    }

    #[test]
    fn rejects_shared_cohorts_and_identical_snapshot_split_evidence() {
        let (development, mut sealed, _, development_fixture, _sealed_fixture) = valid_pair();
        sealed.cohorts[0] = development.cohorts[0].clone();
        sealed.cohorts[0].role = CohortRole::ExternalBenchmark;
        sealed.cohorts[0].disclosure = DisclosureLevel::Aggregate;
        sealed.cohorts[0].adaptation_eligible = false;
        sealed.fingerprint = sealed.reproduce_fingerprint().expect("sealed fingerprint");
        let report = contamination(std::slice::from_ref(&development_fixture));
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &report),
            Err(BenchmarkBundleError::SharedCohort(
                development.cohorts[0].cohort_id
            ))
        );

        let duplicate_evidence = cohort_fixture(
            "sealed-same-evidence",
            CohortRole::SealedAcceptance,
            development.cohorts[0].snapshot_id,
            development.cohorts[0].split,
        );
        let sealed = suite(
            BenchmarkSuiteKind::SealedAcceptance,
            std::slice::from_ref(&duplicate_evidence),
        );
        let report = contamination(&[development_fixture, duplicate_evidence]);
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &report),
            Err(BenchmarkBundleError::SharedSnapshotSplit {
                snapshot_id: development.cohorts[0].snapshot_id,
                split: development.cohorts[0].split,
            })
        );
    }

    #[test]
    fn rejects_cross_task_or_label_incompatible_suites() {
        let (development, sealed, report, _, _) = valid_pair();
        let mut wrong_task = sealed.clone();
        wrong_task.task = "classify unrelated documents".into();
        wrong_task.fingerprint = wrong_task
            .reproduce_fingerprint()
            .expect("suite fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&wrong_task), &report),
            Err(BenchmarkBundleError::TaskMismatch)
        );

        let mut wrong_labels = sealed.clone();
        wrong_labels.labels.reverse();
        wrong_labels.fingerprint = wrong_labels
            .reproduce_fingerprint()
            .expect("suite fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&wrong_labels), &report),
            Err(BenchmarkBundleError::LabelsMismatch)
        );
    }

    #[test]
    fn rejects_foreign_coverage_and_orphan_report_bindings() {
        let (development, sealed, _, development_fixture, sealed_fixture) = valid_pair();
        let foreign = cohort_fixture(
            "foreign",
            CohortRole::Diagnostic,
            Uuid::new_v4(),
            SnapshotSplit::Test,
        );
        let foreign_report =
            contamination(&[development_fixture.clone(), sealed_fixture.clone(), foreign]);
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &foreign_report),
            Err(BenchmarkBundleError::ContaminationCoverage)
        );

        let mut orphan = contamination(&[development_fixture, sealed_fixture]);
        orphan
            .cohort_fingerprints
            .remove(&development.cohorts[0].cohort_id);
        orphan.fingerprint = orphan.reproduce_fingerprint().expect("orphan fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &orphan),
            Err(BenchmarkBundleError::ContaminationBindings)
        );

        let mut foreign_binding = contamination(&[
            cohort_fixture(
                "development-binding",
                CohortRole::Development,
                development.cohorts[0].snapshot_id,
                development.cohorts[0].split,
            ),
            cohort_fixture(
                "sealed-binding",
                CohortRole::SealedAcceptance,
                sealed.cohorts[0].snapshot_id,
                sealed.cohorts[0].split,
            ),
        ]);
        foreign_binding.cohort_ids = report_cohort_ids(&development, &sealed);
        foreign_binding.cohort_fingerprints = report_cohort_fingerprints(&development, &sealed);
        foreign_binding.role_decision_fingerprints =
            report_role_fingerprints(&development, &sealed);
        foreign_binding
            .cohort_fingerprints
            .insert(Uuid::new_v4(), "sha256:foreign".into());
        foreign_binding.fingerprint = foreign_binding
            .reproduce_fingerprint()
            .expect("foreign binding fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &foreign_binding),
            Err(BenchmarkBundleError::ContaminationBindings)
        );
    }

    #[test]
    fn rejects_artifact_and_bundle_tampering() {
        let (development, sealed, report, _, _) = valid_pair();
        let mut tampered_suite = development.clone();
        tampered_suite.name = "tampered".into();
        assert_eq!(
            build_benchmark_bundle(&tampered_suite, Some(&sealed), &report),
            Err(BenchmarkBundleError::SuiteFingerprintMismatch(
                development.id
            ))
        );

        let mut tampered_report = report.clone();
        tampered_report.reasons.push("tampered".into());
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &tampered_report),
            Err(BenchmarkBundleError::ContaminationFingerprintMismatch)
        );

        let mut bundle =
            build_benchmark_bundle(&development, Some(&sealed), &report).expect("bundle");
        let binding = bundle.binding().expect("binding");
        let mut tampered_binding = binding.clone();
        tampered_binding.contamination_report_fingerprint = "sha256:tampered".into();
        assert_eq!(
            tampered_binding.validate_bundle(&bundle),
            Err(BenchmarkBundleError::BundleFingerprintMismatch)
        );
        tampered_binding.bundle_fingerprint = tampered_binding
            .reproduce_bundle_fingerprint()
            .expect("tampered binding fingerprint");
        assert_eq!(
            tampered_binding.validate_bundle(&bundle),
            Err(BenchmarkBundleError::BindingMismatch)
        );
        bundle.development_suite_fingerprint = "sha256:tampered".into();
        assert_eq!(
            bundle.validate_fingerprint(),
            Err(BenchmarkBundleError::BundleFingerprintMismatch)
        );
        assert_eq!(
            binding.validate_bundle(&bundle),
            Err(BenchmarkBundleError::BundleFingerprintMismatch)
        );
    }

    #[test]
    fn rejects_mismatched_report_cohort_and_role_bindings() {
        let (development, sealed, report, development_fixture, sealed_fixture) = valid_pair();
        let mut wrong_cohort = report.clone();
        wrong_cohort.cohort_fingerprints.insert(
            development.cohorts[0].cohort_id,
            "sha256:wrong-cohort".into(),
        );
        wrong_cohort.fingerprint = wrong_cohort
            .reproduce_fingerprint()
            .expect("wrong cohort fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &wrong_cohort),
            Err(BenchmarkBundleError::CohortFingerprintMismatch(
                development.cohorts[0].cohort_id
            ))
        );

        let mut wrong_role = contamination(&[development_fixture, sealed_fixture]);
        wrong_role
            .role_decision_fingerprints
            .insert(sealed.cohorts[0].cohort_id, "sha256:wrong-role".into());
        wrong_role.fingerprint = wrong_role
            .reproduce_fingerprint()
            .expect("wrong role fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &wrong_role),
            Err(BenchmarkBundleError::RoleDecisionFingerprintMismatch(
                sealed.cohorts[0].cohort_id
            ))
        );
    }

    #[test]
    fn rejects_blocked_global_report_without_an_override_path() {
        let (development, sealed, _, development_fixture, mut sealed_fixture) = valid_pair();
        sealed_fixture.member.source_row_id = development_fixture.member.source_row_id;
        sealed_fixture.member.text = development_fixture.member.text.clone();
        let blocked = contamination(&[development_fixture, sealed_fixture]);
        assert_eq!(blocked.status, ContaminationStatus::Blocked);
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &blocked),
            Err(BenchmarkBundleError::ContaminationNotClean)
        );
    }

    #[test]
    fn rejects_thresholded_or_forged_clean_global_reports() {
        let (development, sealed, _, development_fixture, mut sealed_fixture) = valid_pair();
        sealed_fixture.member.source_row_id = development_fixture.member.source_row_id;
        sealed_fixture.member.text = development_fixture.member.text.clone();
        let thresholded = check_contamination(
            vec![
                CohortContaminationInput {
                    cohort: development_fixture.evidence.cohort.clone(),
                    role: development_fixture.evidence.role.clone(),
                    members: vec![development_fixture.member.clone()],
                },
                CohortContaminationInput {
                    cohort: sealed_fixture.evidence.cohort.clone(),
                    role: sealed_fixture.evidence.role.clone(),
                    members: vec![sealed_fixture.member.clone()],
                },
            ],
            None,
            ContaminationPolicy {
                max_source_overlap: 1,
                max_exact_text_overlap: 1,
                max_normalized_text_overlap: 1,
                max_group_overlap: 0,
            },
        )
        .expect("thresholded report");
        assert_eq!(thresholded.status, ContaminationStatus::Clean);
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &thresholded),
            Err(BenchmarkBundleError::ContaminationPolicyNotStrict)
        );

        let mut forged_clean = thresholded;
        forged_clean.policy = ContaminationPolicy::default();
        forged_clean.reasons.clear();
        forged_clean.fingerprint = forged_clean
            .reproduce_fingerprint()
            .expect("forged clean fingerprint");
        assert_eq!(
            build_benchmark_bundle(&development, Some(&sealed), &forged_clean),
            Err(BenchmarkBundleError::ContaminationFindings)
        );
    }

    fn report_cohort_ids(development: &BenchmarkSuite, sealed: &BenchmarkSuite) -> Vec<Uuid> {
        development
            .cohorts
            .iter()
            .chain(&sealed.cohorts)
            .map(|cohort| cohort.cohort_id)
            .collect()
    }

    fn report_cohort_fingerprints(
        development: &BenchmarkSuite,
        sealed: &BenchmarkSuite,
    ) -> std::collections::BTreeMap<Uuid, String> {
        development
            .cohorts
            .iter()
            .chain(&sealed.cohorts)
            .map(|cohort| (cohort.cohort_id, cohort.cohort_fingerprint.clone()))
            .collect()
    }

    fn report_role_fingerprints(
        development: &BenchmarkSuite,
        sealed: &BenchmarkSuite,
    ) -> std::collections::BTreeMap<Uuid, String> {
        development
            .cohorts
            .iter()
            .chain(&sealed.cohorts)
            .map(|cohort| (cohort.cohort_id, cohort.role_decision_fingerprint.clone()))
            .collect()
    }
}
