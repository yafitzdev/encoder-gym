//! Immutable clearance of trainer-visible snapshot members against a benchmark bundle.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::{
    domain::{DatasetSnapshot, SnapshotMember, SnapshotSplit},
    splitting::verify_snapshot,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    benchmark_bundle::BenchmarkBundle,
    contamination::{
        ContaminationKind, ContaminationPolicy, ContaminationReport, ContaminationStatus,
    },
    governance::{
        CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision, EvaluationCohort,
    },
};

pub const TRAINING_BENCHMARK_CHECK_PROTOCOL: &str = "training-benchmark-exact-v1";

/// Versioned declaration of which immutable snapshot members can influence a
/// trainer. A protocol change necessarily creates a distinct clearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingInputProtocol {
    TrainAndValidationV1,
}

impl TrainingInputProtocol {
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::TrainAndValidationV1 => "train_and_validation_v1",
        }
    }

    pub const fn consumes(self, split: SnapshotSplit) -> bool {
        match self {
            Self::TrainAndValidationV1 => {
                matches!(split, SnapshotSplit::Train | SnapshotSplit::Validation)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingCohortEvidence {
    pub cohort: EvaluationCohort,
    pub role: CohortRoleDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingCohortBinding {
    pub split: SnapshotSplit,
    pub member_count: u64,
    pub cohort_id: Uuid,
    pub cohort_fingerprint: String,
    pub role_decision_id: Uuid,
    pub role_decision_fingerprint: String,
}

/// Immutable proof that the exact trainer-visible population was compared with
/// the exact benchmark authority under the platform's strict leakage policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingBenchmarkCheck {
    pub id: Uuid,
    pub training_snapshot_id: Uuid,
    pub training_snapshot_fingerprint: String,
    pub check_protocol_version: String,
    pub training_input_protocol: TrainingInputProtocol,
    pub training_population_fingerprint: String,
    pub training_member_count: u64,
    pub training_cohorts: Vec<TrainingCohortBinding>,
    pub benchmark_bundle_id: Uuid,
    pub benchmark_bundle_fingerprint: String,
    pub benchmark_cohort_ids: Vec<Uuid>,
    pub contamination_report_id: Uuid,
    pub contamination_report_fingerprint: String,
    pub status: ContaminationStatus,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl TrainingBenchmarkCheck {
    pub fn reproduce_fingerprint(&self) -> Result<String, TrainingBenchmarkError> {
        check_fingerprint(self)
    }

    pub fn validate_integrity(&self) -> Result<(), TrainingBenchmarkError> {
        validate_check_shape(self)?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(TrainingBenchmarkError::CheckFingerprintMismatch);
        }
        Ok(())
    }

    pub const fn training_allowed(&self) -> bool {
        matches!(self.status, ContaminationStatus::Clean)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrainingBenchmarkError {
    #[error("training snapshot evidence is invalid: {0}")]
    Snapshot(String),
    #[error("trainer-visible population requires at least one train member")]
    EmptyTrainingPopulation,
    #[error("could not fingerprint trainer-visible population: {0}")]
    PopulationFingerprint(String),
    #[error("training cohort bindings do not exactly cover non-empty trainer-visible splits")]
    TrainingCohortCoverage,
    #[error("training cohort is invalid or does not bind the training snapshot: {0}")]
    TrainingCohort(Uuid),
    #[error("training cohort role is invalid, stale, or not active Training: {0}")]
    TrainingRole(Uuid),
    #[error("benchmark bundle fingerprint does not reproduce")]
    BenchmarkBundleFingerprint,
    #[error("benchmark global contamination report does not match the bundle")]
    BenchmarkReportBinding,
    #[error("benchmark global contamination report is not strict and clean")]
    BenchmarkReportNotClean,
    #[error("training contamination report fingerprint does not reproduce")]
    ContaminationFingerprint,
    #[error("training contamination report must use the strict bundle group policy")]
    ContaminationPolicy,
    #[error("training contamination report participant set is not exact")]
    ContaminationCoverage,
    #[error("training contamination report participant bindings are not exact")]
    ContaminationBindings,
    #[error("training contamination report clean status is inconsistent with its findings")]
    CleanReportFindings,
    #[error("training contamination report blocked status has no blocking evidence")]
    BlockedReportEvidence,
    #[error("training-benchmark check shape is invalid")]
    CheckShape,
    #[error("training-benchmark check fingerprint does not reproduce")]
    CheckFingerprintMismatch,
    #[error("could not fingerprint training-benchmark check: {0}")]
    Fingerprint(String),
}

#[allow(clippy::too_many_arguments)]
pub fn build_training_benchmark_check(
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
    protocol: TrainingInputProtocol,
    mut training_evidence: Vec<TrainingCohortEvidence>,
    bundle: &BenchmarkBundle,
    benchmark_report: &ContaminationReport,
    contamination_report: &ContaminationReport,
) -> Result<TrainingBenchmarkCheck, TrainingBenchmarkError> {
    verify_snapshot(snapshot, members)
        .map_err(|error| TrainingBenchmarkError::Snapshot(error.to_string()))?;
    let selected = selected_members(members, protocol);
    if !selected
        .iter()
        .any(|member| member.split == SnapshotSplit::Train)
    {
        return Err(TrainingBenchmarkError::EmptyTrainingPopulation);
    }
    let population_fingerprint = population_fingerprint(snapshot, protocol, &selected)?;
    let split_counts = selected.iter().fold(BTreeMap::new(), |mut counts, member| {
        *counts.entry(member.split).or_insert(0_u64) += 1;
        counts
    });

    training_evidence.sort_by_key(|evidence| evidence.cohort.split);
    let expected_splits = split_counts.keys().copied().collect::<Vec<_>>();
    let supplied_splits = training_evidence
        .iter()
        .map(|evidence| evidence.cohort.split)
        .collect::<Vec<_>>();
    if expected_splits != supplied_splits {
        return Err(TrainingBenchmarkError::TrainingCohortCoverage);
    }

    let mut training_cohorts = Vec::with_capacity(training_evidence.len());
    let mut training_ids = BTreeSet::new();
    let mut expected_cohort_fingerprints = BTreeMap::new();
    let mut expected_role_fingerprints = BTreeMap::new();
    for evidence in &training_evidence {
        let cohort = &evidence.cohort;
        let role = &evidence.role;
        if cohort.id.is_nil()
            || !training_ids.insert(cohort.id)
            || cohort.snapshot_id != snapshot.id
            || cohort.snapshot_fingerprint != snapshot.fingerprint
            || cohort.origin != CohortOrigin::InternalSnapshot
            || cohort
                .reproduce_fingerprint()
                .map_err(|_| TrainingBenchmarkError::TrainingCohort(cohort.id))?
                != cohort.fingerprint
        {
            return Err(TrainingBenchmarkError::TrainingCohort(cohort.id));
        }
        if role.cohort_id != cohort.id
            || role.role != CohortRole::Training
            || role.disposition != CohortDisposition::Active
            || role
                .reproduce_fingerprint()
                .map_err(|_| TrainingBenchmarkError::TrainingRole(cohort.id))?
                != role.fingerprint
        {
            return Err(TrainingBenchmarkError::TrainingRole(cohort.id));
        }
        expected_cohort_fingerprints.insert(cohort.id, cohort.fingerprint.clone());
        expected_role_fingerprints.insert(cohort.id, role.fingerprint.clone());
        training_cohorts.push(TrainingCohortBinding {
            split: cohort.split,
            member_count: split_counts[&cohort.split],
            cohort_id: cohort.id,
            cohort_fingerprint: cohort.fingerprint.clone(),
            role_decision_id: role.id,
            role_decision_fingerprint: role.fingerprint.clone(),
        });
    }

    if bundle
        .reproduce_fingerprint()
        .map_err(|_| TrainingBenchmarkError::BenchmarkBundleFingerprint)?
        != bundle.fingerprint
    {
        return Err(TrainingBenchmarkError::BenchmarkBundleFingerprint);
    }
    validate_benchmark_report(bundle, benchmark_report)?;
    let benchmark_ids = canonical_ids(&benchmark_report.cohort_ids)
        .ok_or(TrainingBenchmarkError::BenchmarkReportBinding)?;
    if !training_ids.is_disjoint(&benchmark_ids) {
        return Err(TrainingBenchmarkError::ContaminationCoverage);
    }
    expected_cohort_fingerprints.extend(benchmark_report.cohort_fingerprints.clone());
    expected_role_fingerprints.extend(benchmark_report.role_decision_fingerprints.clone());

    if contamination_report
        .reproduce_fingerprint()
        .map_err(|_| TrainingBenchmarkError::ContaminationFingerprint)?
        != contamination_report.fingerprint
    {
        return Err(TrainingBenchmarkError::ContaminationFingerprint);
    }
    if contamination_report.policy != ContaminationPolicy::default()
        || contamination_report.group_dimension != benchmark_report.group_dimension
    {
        return Err(TrainingBenchmarkError::ContaminationPolicy);
    }
    let expected_ids = training_ids
        .union(&benchmark_ids)
        .copied()
        .collect::<BTreeSet<_>>();
    if canonical_ids(&contamination_report.cohort_ids) != Some(expected_ids.clone()) {
        return Err(TrainingBenchmarkError::ContaminationCoverage);
    }
    if contamination_report
        .cohort_fingerprints
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != expected_ids
        || contamination_report
            .role_decision_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != expected_ids
        || contamination_report.cohort_fingerprints != expected_cohort_fingerprints
        || contamination_report.role_decision_fingerprints != expected_role_fingerprints
    {
        return Err(TrainingBenchmarkError::ContaminationBindings);
    }
    validate_report_status(contamination_report)?;

    let mut value = TrainingBenchmarkCheck {
        id: Uuid::new_v4(),
        training_snapshot_id: snapshot.id,
        training_snapshot_fingerprint: snapshot.fingerprint.clone(),
        check_protocol_version: TRAINING_BENCHMARK_CHECK_PROTOCOL.into(),
        training_input_protocol: protocol,
        training_population_fingerprint: population_fingerprint,
        training_member_count: selected.len() as u64,
        training_cohorts,
        benchmark_bundle_id: bundle.id,
        benchmark_bundle_fingerprint: bundle.fingerprint.clone(),
        benchmark_cohort_ids: benchmark_ids.into_iter().collect(),
        contamination_report_id: contamination_report.id,
        contamination_report_fingerprint: contamination_report.fingerprint.clone(),
        status: contamination_report.status,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    value.fingerprint = check_fingerprint(&value)?;
    value.validate_integrity()?;
    Ok(value)
}

pub fn training_population_fingerprint(
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
    protocol: TrainingInputProtocol,
) -> Result<String, TrainingBenchmarkError> {
    verify_snapshot(snapshot, members)
        .map_err(|error| TrainingBenchmarkError::Snapshot(error.to_string()))?;
    let selected = selected_members(members, protocol);
    if !selected
        .iter()
        .any(|member| member.split == SnapshotSplit::Train)
    {
        return Err(TrainingBenchmarkError::EmptyTrainingPopulation);
    }
    population_fingerprint(snapshot, protocol, &selected)
}

fn selected_members(
    members: &[SnapshotMember],
    protocol: TrainingInputProtocol,
) -> Vec<&SnapshotMember> {
    members
        .iter()
        .filter(|member| protocol.consumes(member.split))
        .collect()
}

fn population_fingerprint(
    snapshot: &DatasetSnapshot,
    protocol: TrainingInputProtocol,
    members: &[&SnapshotMember],
) -> Result<String, TrainingBenchmarkError> {
    artifact_core::fingerprint(&serde_json::json!({
        "training_snapshot_id": snapshot.id,
        "training_snapshot_fingerprint": snapshot.fingerprint,
        "training_input_protocol": protocol,
        "members": members,
    }))
    .map_err(|error| TrainingBenchmarkError::PopulationFingerprint(error.to_string()))
}

fn validate_benchmark_report(
    bundle: &BenchmarkBundle,
    report: &ContaminationReport,
) -> Result<(), TrainingBenchmarkError> {
    if report.id != bundle.contamination_report_id
        || report.fingerprint != bundle.contamination_report_fingerprint
        || report
            .reproduce_fingerprint()
            .map_err(|_| TrainingBenchmarkError::BenchmarkReportBinding)?
            != report.fingerprint
    {
        return Err(TrainingBenchmarkError::BenchmarkReportBinding);
    }
    if report.status != ContaminationStatus::Clean
        || report.policy != ContaminationPolicy::default()
        || !report.findings.is_empty()
        || !report.reasons.is_empty()
        || !has_exact_zero_counts(report)
    {
        return Err(TrainingBenchmarkError::BenchmarkReportNotClean);
    }
    let ids =
        canonical_ids(&report.cohort_ids).ok_or(TrainingBenchmarkError::BenchmarkReportBinding)?;
    if ids.is_empty()
        || report
            .cohort_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != ids
        || report
            .role_decision_fingerprints
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != ids
    {
        return Err(TrainingBenchmarkError::BenchmarkReportBinding);
    }
    Ok(())
}

fn validate_report_status(report: &ContaminationReport) -> Result<(), TrainingBenchmarkError> {
    let exact_kinds = exact_kinds();
    if report.counts.keys().copied().collect::<BTreeSet<_>>() != exact_kinds {
        return Err(TrainingBenchmarkError::ContaminationBindings);
    }
    match report.status {
        ContaminationStatus::Clean => {
            if !report.findings.is_empty()
                || !report.reasons.is_empty()
                || report.counts.values().any(|count| *count != 0)
            {
                return Err(TrainingBenchmarkError::CleanReportFindings);
            }
        }
        ContaminationStatus::Blocked => {
            if report.findings.is_empty()
                || report.reasons.is_empty()
                || report.counts.values().all(|count| *count == 0)
            {
                return Err(TrainingBenchmarkError::BlockedReportEvidence);
            }
        }
    }
    Ok(())
}

fn validate_check_shape(value: &TrainingBenchmarkCheck) -> Result<(), TrainingBenchmarkError> {
    let training_ids = value
        .training_cohorts
        .iter()
        .map(|binding| binding.cohort_id)
        .collect::<BTreeSet<_>>();
    let benchmark_ids = value
        .benchmark_cohort_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let splits = value
        .training_cohorts
        .iter()
        .map(|binding| binding.split)
        .collect::<Vec<_>>();
    let expected_member_count = value
        .training_cohorts
        .iter()
        .map(|binding| binding.member_count)
        .sum::<u64>();
    if value.id.is_nil()
        || value.training_snapshot_id.is_nil()
        || value.benchmark_bundle_id.is_nil()
        || value.contamination_report_id.is_nil()
        || value.training_snapshot_fingerprint.trim().is_empty()
        || value.check_protocol_version != TRAINING_BENCHMARK_CHECK_PROTOCOL
        || value.training_population_fingerprint.trim().is_empty()
        || value.benchmark_bundle_fingerprint.trim().is_empty()
        || value.contamination_report_fingerprint.trim().is_empty()
        || value.training_cohorts.is_empty()
        || value.benchmark_cohort_ids.is_empty()
        || training_ids.len() != value.training_cohorts.len()
        || benchmark_ids.len() != value.benchmark_cohort_ids.len()
        || !training_ids.is_disjoint(&benchmark_ids)
        || !value.benchmark_cohort_ids.is_sorted()
        || splits.first() != Some(&SnapshotSplit::Train)
        || !splits.is_sorted()
        || splits.contains(&SnapshotSplit::Test)
        || splits.windows(2).any(|pair| pair[0] == pair[1])
        || value.training_cohorts.iter().any(|binding| {
            binding.member_count == 0
                || binding.cohort_id.is_nil()
                || binding.role_decision_id.is_nil()
                || binding.cohort_fingerprint.trim().is_empty()
                || binding.role_decision_fingerprint.trim().is_empty()
        })
        || expected_member_count != value.training_member_count
    {
        return Err(TrainingBenchmarkError::CheckShape);
    }
    Ok(())
}

fn canonical_ids(ids: &[Uuid]) -> Option<BTreeSet<Uuid>> {
    let values = ids.iter().copied().collect::<BTreeSet<_>>();
    (values.len() == ids.len() && ids.is_sorted()).then_some(values)
}

fn has_exact_zero_counts(report: &ContaminationReport) -> bool {
    report.counts.keys().copied().collect::<BTreeSet<_>>() == exact_kinds()
        && report.counts.values().all(|count| *count == 0)
}

fn exact_kinds() -> BTreeSet<ContaminationKind> {
    [
        ContaminationKind::SourceRow,
        ContaminationKind::ExactText,
        ContaminationKind::NormalizedText,
        ContaminationKind::Group,
    ]
    .into_iter()
    .collect()
}

fn check_fingerprint(value: &TrainingBenchmarkCheck) -> Result<String, TrainingBenchmarkError> {
    artifact_core::fingerprint(&serde_json::json!({
        "training_snapshot_id": value.training_snapshot_id,
        "training_snapshot_fingerprint": value.training_snapshot_fingerprint,
        "check_protocol_version": value.check_protocol_version,
        "training_input_protocol": value.training_input_protocol,
        "training_population_fingerprint": value.training_population_fingerprint,
        "training_member_count": value.training_member_count,
        "training_cohorts": value.training_cohorts,
        "benchmark_bundle_id": value.benchmark_bundle_id,
        "benchmark_bundle_fingerprint": value.benchmark_bundle_fingerprint,
        "benchmark_cohort_ids": value.benchmark_cohort_ids,
        "contamination_report_id": value.contamination_report_id,
        "contamination_report_fingerprint": value.contamination_report_fingerprint,
        "status": value.status,
    }))
    .map_err(|error| TrainingBenchmarkError::Fingerprint(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use dataset_core::{
        domain::{SnapshotSplit, SourceProvenance, SourceRow, SplitConfiguration, SplitRatios},
        splitting::build_snapshot,
    };
    use evaluation_core::domain::EvaluationProtocol;

    use super::*;
    use crate::{
        benchmark::{
            AcceptanceContract, BenchmarkCohortEvidence, BenchmarkCohortRequest, BenchmarkMetric,
            BenchmarkSuiteRequest, MetricRequirement, MetricTarget, build_benchmark_suite,
        },
        benchmark_bundle::build_benchmark_bundle,
        contamination::{CohortContaminationInput, ContaminationMember, check_contamination},
    };

    struct Fixture {
        snapshot: DatasetSnapshot,
        members: Vec<SnapshotMember>,
        training: Vec<TrainingCohortEvidence>,
        bundle: BenchmarkBundle,
        benchmark_report: ContaminationReport,
        benchmark_inputs: Vec<CohortContaminationInput>,
    }

    fn fixture(benchmark_first_text: impl FnOnce(&[SnapshotMember]) -> String) -> Fixture {
        let (snapshot, members) = make_snapshot(
            "training",
            (0..10).map(|index| format!("training example {index}")),
            SplitRatios::new(0.6, 0.2, 0.2).expect("ratios"),
        );
        let training = [SnapshotSplit::Train, SnapshotSplit::Validation]
            .into_iter()
            .filter(|split| members.iter().any(|member| member.split == *split))
            .map(|split| {
                let cohort = EvaluationCohort::new(
                    format!("training {split:?}"),
                    snapshot.id,
                    snapshot.fingerprint.clone(),
                    split,
                    CohortOrigin::InternalSnapshot,
                )
                .expect("training cohort");
                let role = CohortRoleDecision::initial(&cohort, CohortRole::Training, "fixture")
                    .expect("training role");
                TrainingCohortEvidence { cohort, role }
            })
            .collect::<Vec<_>>();
        let benchmark_first_text = benchmark_first_text(&members);

        let (benchmark_snapshot, benchmark_members) = make_snapshot(
            "benchmark",
            std::iter::once(benchmark_first_text)
                .chain((1..4).map(|index| format!("benchmark example {index}"))),
            SplitRatios::new(0.0, 0.0, 1.0).expect("ratios"),
        );
        let benchmark_cohort = EvaluationCohort::new(
            "development",
            benchmark_snapshot.id,
            benchmark_snapshot.fingerprint.clone(),
            SnapshotSplit::Test,
            CohortOrigin::InternalSnapshot,
        )
        .expect("benchmark cohort");
        let benchmark_role =
            CohortRoleDecision::initial(&benchmark_cohort, CohortRole::Development, "fixture")
                .expect("benchmark role");
        let benchmark_inputs = vec![CohortContaminationInput {
            cohort: benchmark_cohort.clone(),
            role: benchmark_role.clone(),
            members: benchmark_members
                .iter()
                .map(|member| ContaminationMember::from_snapshot_member(member, None))
                .collect(),
        }];
        let benchmark_report = check_contamination(
            benchmark_inputs.clone(),
            None,
            ContaminationPolicy::default(),
        )
        .expect("benchmark report");
        let suite = build_benchmark_suite(
            BenchmarkSuiteRequest {
                name: "development".into(),
                kind: crate::benchmark::BenchmarkSuiteKind::Development,
                task: "classification".into(),
                labels: vec!["account".into(), "billing".into()],
                required_model_formats: Vec::new(),
                cohorts: vec![BenchmarkCohortRequest {
                    cohort_id: benchmark_cohort.id,
                    protocol: EvaluationProtocol::default(),
                    disclosure: crate::governance::DisclosureLevel::RowContent,
                    adaptation_eligible: true,
                }],
                contract: AcceptanceContract {
                    metric_requirements: vec![MetricRequirement {
                        target: MetricTarget::Overall,
                        metric: BenchmarkMetric::Accuracy,
                        minimum: Some(0.5),
                        maximum: None,
                        minimum_support: 1,
                    }],
                    regression: None,
                },
            },
            vec![BenchmarkCohortEvidence {
                cohort: benchmark_cohort,
                role: benchmark_role,
            }],
            &benchmark_report,
            None,
        )
        .expect("suite");
        let bundle = build_benchmark_bundle(&suite, None, &benchmark_report).expect("bundle");
        Fixture {
            snapshot,
            members,
            training,
            bundle,
            benchmark_report,
            benchmark_inputs,
        }
    }

    fn make_snapshot(
        name: &str,
        texts: impl IntoIterator<Item = String>,
        ratios: SplitRatios,
    ) -> (DatasetSnapshot, Vec<SnapshotMember>) {
        let dataset_id = Uuid::new_v4();
        let rows = texts
            .into_iter()
            .enumerate()
            .map(|(index, text)| SourceRow {
                id: Uuid::new_v4(),
                dataset_id,
                text,
                label: if index % 2 == 0 {
                    "account".into()
                } else {
                    "billing".into()
                },
                dimensions: BTreeMap::new(),
                fields: BTreeMap::new(),
                provenance: SourceProvenance::Imported {
                    import_id: Uuid::new_v4(),
                    source_path: format!("{name}.jsonl"),
                    source_row_number: index as u64 + 1,
                },
                created_at: Utc::now(),
            })
            .collect();
        build_snapshot(
            dataset_id,
            name,
            None,
            SplitConfiguration::new(ratios, 42),
            rows,
        )
        .expect("snapshot")
    }

    fn check(fixture: &Fixture) -> TrainingBenchmarkCheck {
        let mut inputs = fixture
            .training
            .iter()
            .map(|evidence| CohortContaminationInput {
                cohort: evidence.cohort.clone(),
                role: evidence.role.clone(),
                members: fixture
                    .members
                    .iter()
                    .filter(|member| member.split == evidence.cohort.split)
                    .map(|member| ContaminationMember::from_snapshot_member(member, None))
                    .collect(),
            })
            .collect::<Vec<_>>();
        inputs.extend(fixture.benchmark_inputs.clone());
        let report = check_contamination(inputs, None, ContaminationPolicy::default())
            .expect("combined report");
        build_training_benchmark_check(
            &fixture.snapshot,
            &fixture.members,
            TrainingInputProtocol::TrainAndValidationV1,
            fixture.training.clone(),
            &fixture.bundle,
            &fixture.benchmark_report,
            &report,
        )
        .expect("check")
    }

    #[test]
    fn clean_check_pins_exact_trainer_population_and_bundle() {
        let fixture = fixture(|_| "unseen benchmark example".into());
        let check = check(&fixture);
        assert!(check.training_allowed());
        assert_eq!(
            check.training_member_count,
            fixture
                .members
                .iter()
                .filter(|member| matches!(
                    member.split,
                    SnapshotSplit::Train | SnapshotSplit::Validation
                ))
                .count() as u64
        );
        assert_eq!(check.benchmark_bundle_id, fixture.bundle.id);
        check.validate_integrity().expect("integrity");
    }

    #[test]
    fn trainer_visible_exact_overlap_is_durable_blocked_evidence() {
        let fixture = fixture(|members| {
            members
                .iter()
                .find(|member| member.split == SnapshotSplit::Train)
                .expect("train member")
                .text
                .clone()
        });
        let check = check(&fixture);
        assert!(!check.training_allowed());
        assert_eq!(check.status, ContaminationStatus::Blocked);
    }

    #[test]
    fn shared_source_identity_blocks_even_when_text_differs() {
        let mut fixture = fixture(|_| "unseen benchmark example".into());
        let training_source = fixture
            .members
            .iter()
            .find(|member| member.split == SnapshotSplit::Train)
            .expect("train member")
            .source_row_id;
        fixture.benchmark_inputs[0].members[0].source_row_id = training_source;

        assert!(!check(&fixture).training_allowed());
    }

    #[test]
    fn normalized_text_overlap_blocks_without_exact_text_equality() {
        let fixture = fixture(|members| {
            let text = &members
                .iter()
                .find(|member| member.split == SnapshotSplit::Train)
                .expect("train member")
                .text;
            format!("  {}  ", text.to_uppercase().replace(' ', "   "))
        });

        assert!(!check(&fixture).training_allowed());
    }

    #[test]
    fn validation_overlap_is_treated_as_training_influence() {
        let fixture = fixture(|members| {
            members
                .iter()
                .find(|member| member.split == SnapshotSplit::Validation)
                .expect("validation member")
                .text
                .clone()
        });
        assert!(!check(&fixture).training_allowed());
    }

    #[test]
    fn test_only_overlap_is_outside_versioned_trainer_input_protocol() {
        let fixture = fixture(|members| {
            members
                .iter()
                .find(|member| member.split == SnapshotSplit::Test)
                .expect("test member")
                .text
                .clone()
        });
        assert!(check(&fixture).training_allowed());
    }

    #[test]
    fn population_fingerprint_includes_snapshot_member_identity() {
        let fixture = fixture(|_| "unseen benchmark example".into());
        let original = training_population_fingerprint(
            &fixture.snapshot,
            &fixture.members,
            TrainingInputProtocol::TrainAndValidationV1,
        )
        .expect("fingerprint");
        let mut changed = fixture.members.clone();
        let member = changed
            .iter_mut()
            .find(|member| member.split == SnapshotSplit::Train)
            .expect("train member");
        member.id = Uuid::new_v4();
        let changed = training_population_fingerprint(
            &fixture.snapshot,
            &changed,
            TrainingInputProtocol::TrainAndValidationV1,
        )
        .expect("fingerprint");
        assert_ne!(original, changed);
    }
}
