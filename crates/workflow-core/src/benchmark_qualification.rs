//! Deterministic readiness evidence for an immutable benchmark bundle.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::{SnapshotMember, SourceProvenance};
use evaluation_core::domain::{SliceIdentity, SliceKind};
use generation_core::deduplication::normalize_text;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    benchmark::{BenchmarkSuite, MetricTarget},
    benchmark_bundle::BenchmarkBundle,
};

pub const BENCHMARK_QUALIFICATION_PROTOCOL: &str = "benchmark-readiness-v1";

/// Fixed confidence levels keep required-support calculations portable and
/// reproducible without depending on a statistics implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationConfidence {
    Eighty,
    Ninety,
    NinetyFive,
    NinetyNine,
}

impl QualificationConfidence {
    const fn z_score(self) -> f64 {
        match self {
            Self::Eighty => 1.281_551_565_544_600_4,
            Self::Ninety => 1.644_853_626_951_472_2,
            Self::NinetyFive => 1.959_963_984_540_054,
            Self::NinetyNine => 2.575_829_303_548_900_4,
        }
    }
}

/// Operator-owned, immutable readiness policy. These values are not acceptance
/// thresholds: they decide whether the population is capable of supporting the
/// suite's already-frozen acceptance contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkQualificationPolicy {
    pub minimum_overall_support: u64,
    pub minimum_label_support: u64,
    pub confidence: QualificationConfidence,
    pub maximum_proportion_margin_of_error: f64,
    pub maximum_normalized_duplicate_rate: f64,
    pub minimum_distinct_producers: u64,
    pub maximum_single_producer_share: f64,
    pub maximum_label_imbalance_ratio: f64,
}

impl Default for BenchmarkQualificationPolicy {
    fn default() -> Self {
        Self {
            minimum_overall_support: 100,
            minimum_label_support: 20,
            confidence: QualificationConfidence::NinetyFive,
            maximum_proportion_margin_of_error: 0.10,
            maximum_normalized_duplicate_rate: 0.01,
            minimum_distinct_producers: 1,
            maximum_single_producer_share: 1.0,
            maximum_label_imbalance_ratio: 10.0,
        }
    }
}

impl BenchmarkQualificationPolicy {
    pub fn validate(&self) -> Result<(), BenchmarkQualificationError> {
        if self.minimum_overall_support == 0 || self.minimum_label_support == 0 {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "minimum support values must be positive".into(),
            ));
        }
        if !self.maximum_proportion_margin_of_error.is_finite()
            || !(0.0..0.5).contains(&self.maximum_proportion_margin_of_error)
        {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "maximum_proportion_margin_of_error must be finite and in (0, 0.5)".into(),
            ));
        }
        if !self.maximum_normalized_duplicate_rate.is_finite()
            || !(0.0..=1.0).contains(&self.maximum_normalized_duplicate_rate)
        {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "maximum_normalized_duplicate_rate must be finite and in [0, 1]".into(),
            ));
        }
        if self.minimum_distinct_producers == 0 {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "minimum_distinct_producers must be positive".into(),
            ));
        }
        if !self.maximum_single_producer_share.is_finite()
            || !(0.0..=1.0).contains(&self.maximum_single_producer_share)
            || self.maximum_single_producer_share == 0.0
        {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "maximum_single_producer_share must be finite and in (0, 1]".into(),
            ));
        }
        if !self.maximum_label_imbalance_ratio.is_finite()
            || self.maximum_label_imbalance_ratio < 1.0
        {
            return Err(BenchmarkQualificationError::InvalidPolicy(
                "maximum_label_imbalance_ratio must be finite and at least 1".into(),
            ));
        }
        Ok(())
    }

    pub fn binomial_support_floor(&self) -> Result<u64, BenchmarkQualificationError> {
        self.validate()?;
        let z = self.confidence.z_score();
        Ok((z * z * 0.25
            / (self.maximum_proportion_margin_of_error * self.maximum_proportion_margin_of_error))
            .ceil() as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkReadiness {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationIssueSeverity {
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationIssue {
    pub severity: QualificationIssueSeverity,
    pub code: String,
    pub suite_id: Uuid,
    pub cohort_id: Uuid,
    pub target: Option<MetricTarget>,
    pub observed: Option<f64>,
    pub required: Option<f64>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceComposition {
    pub generated_rows: u64,
    pub imported_rows: u64,
    pub distinct_producers: u64,
    pub rows_by_producer: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkCohortReadiness {
    pub suite_id: Uuid,
    pub suite_fingerprint: String,
    pub cohort_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub population_fingerprint: String,
    pub total_support: u64,
    pub label_support: BTreeMap<String, u64>,
    pub required_slice_support: BTreeMap<String, u64>,
    pub dimension_value_support: BTreeMap<String, BTreeMap<String, u64>>,
    pub source_composition: SourceComposition,
    pub normalized_duplicate_rows: u64,
    pub normalized_duplicate_rate: f64,
    pub minimum_text_characters: u64,
    pub median_text_characters: u64,
    pub maximum_text_characters: u64,
    pub reference_distribution_bound: bool,
}

/// Immutable readiness evidence. It never contains benchmark row content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkQualification {
    pub id: Uuid,
    pub protocol: String,
    pub benchmark_bundle_id: Uuid,
    pub benchmark_bundle_fingerprint: String,
    pub development_suite_id: Uuid,
    pub development_suite_fingerprint: String,
    pub sealed_suite_id: Option<Uuid>,
    pub sealed_suite_fingerprint: Option<String>,
    pub policy: BenchmarkQualificationPolicy,
    pub readiness: BenchmarkReadiness,
    pub cohorts: Vec<BenchmarkCohortReadiness>,
    pub issues: Vec<QualificationIssue>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkQualification {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkQualificationError> {
        qualification_fingerprint(self)
    }

    pub fn validate_integrity(&self) -> Result<(), BenchmarkQualificationError> {
        self.policy.validate()?;
        let malformed = self.id.is_nil()
            || self.protocol != BENCHMARK_QUALIFICATION_PROTOCOL
            || self.benchmark_bundle_id.is_nil()
            || !canonical_fingerprint(&self.benchmark_bundle_fingerprint)
            || self.development_suite_id.is_nil()
            || !canonical_fingerprint(&self.development_suite_fingerprint)
            || self.sealed_suite_id.is_some() != self.sealed_suite_fingerprint.is_some()
            || self
                .sealed_suite_fingerprint
                .as_deref()
                .is_some_and(|value| !canonical_fingerprint(value))
            || self.cohorts.is_empty();
        if malformed {
            return Err(BenchmarkQualificationError::InvalidArtifact(format!(
                "identity, protocol, authority pins, or cohort evidence is malformed: protocol={}, bundle_fp_len={}, development_fp_len={}, sealed_id={}, sealed_fp={}, cohorts={}",
                self.protocol,
                self.benchmark_bundle_fingerprint.len(),
                self.development_suite_fingerprint.len(),
                self.sealed_suite_id.is_some(),
                self.sealed_suite_fingerprint.is_some(),
                self.cohorts.len()
            )));
        }
        if self.cohorts.windows(2).any(|values| {
            (values[0].suite_id, values[0].cohort_id) >= (values[1].suite_id, values[1].cohort_id)
        }) {
            return Err(BenchmarkQualificationError::InvalidArtifact(
                "cohort readiness evidence is not in canonical order".into(),
            ));
        }
        if self
            .issues
            .windows(2)
            .any(|values| issue_key(&values[0]) > issue_key(&values[1]))
        {
            return Err(BenchmarkQualificationError::InvalidArtifact(
                "qualification issues are not in canonical order".into(),
            ));
        }
        let expected = if self
            .issues
            .iter()
            .any(|issue| issue.severity == QualificationIssueSeverity::Blocking)
        {
            BenchmarkReadiness::Blocked
        } else {
            BenchmarkReadiness::Ready
        };
        if self.readiness != expected {
            return Err(BenchmarkQualificationError::InvalidArtifact(
                "readiness does not agree with issue severity".into(),
            ));
        }
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(BenchmarkQualificationError::FingerprintMismatch);
        }
        Ok(())
    }

    pub fn binding(&self) -> Result<BenchmarkQualificationBinding, BenchmarkQualificationError> {
        self.validate_integrity()?;
        Ok(BenchmarkQualificationBinding {
            qualification_id: self.id,
            qualification_fingerprint: self.fingerprint.clone(),
            benchmark_bundle_id: self.benchmark_bundle_id,
            benchmark_bundle_fingerprint: self.benchmark_bundle_fingerprint.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkQualificationBinding {
    pub qualification_id: Uuid,
    pub qualification_fingerprint: String,
    pub benchmark_bundle_id: Uuid,
    pub benchmark_bundle_fingerprint: String,
}

impl BenchmarkQualificationBinding {
    pub fn validate_qualification(
        &self,
        qualification: &BenchmarkQualification,
    ) -> Result<(), BenchmarkQualificationError> {
        qualification.validate_integrity()?;
        if qualification.readiness != BenchmarkReadiness::Ready
            || self.qualification_id != qualification.id
            || self.qualification_fingerprint != qualification.fingerprint
            || self.benchmark_bundle_id != qualification.benchmark_bundle_id
            || self.benchmark_bundle_fingerprint != qualification.benchmark_bundle_fingerprint
        {
            return Err(BenchmarkQualificationError::BindingMismatch);
        }
        Ok(())
    }
}

pub struct QualificationPopulation<'a> {
    pub cohort_id: Uuid,
    pub members: &'a [SnapshotMember],
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum BenchmarkQualificationError {
    #[error("invalid benchmark qualification policy: {0}")]
    InvalidPolicy(String),
    #[error("benchmark bundle is invalid: {0}")]
    InvalidBundle(String),
    #[error("benchmark suite is invalid: {0}")]
    InvalidSuite(String),
    #[error("qualification populations do not match the benchmark cohort union")]
    PopulationSet,
    #[error("qualification population is invalid for cohort {cohort_id}: {reason}")]
    InvalidPopulation { cohort_id: Uuid, reason: String },
    #[error("benchmark qualification artifact is invalid: {0}")]
    InvalidArtifact(String),
    #[error("benchmark qualification fingerprint does not reproduce")]
    FingerprintMismatch,
    #[error("benchmark qualification binding is not ready or does not match")]
    BindingMismatch,
    #[error("could not fingerprint benchmark qualification: {0}")]
    Fingerprint(String),
}

/// Qualifies every cohort independently because the acceptance contract is
/// evaluated independently for every cohort in a suite.
pub fn qualify_benchmark_bundle(
    bundle: &BenchmarkBundle,
    development: &BenchmarkSuite,
    sealed: Option<&BenchmarkSuite>,
    populations: Vec<QualificationPopulation<'_>>,
    policy: BenchmarkQualificationPolicy,
) -> Result<BenchmarkQualification, BenchmarkQualificationError> {
    policy.validate()?;
    bundle
        .validate_fingerprint()
        .map_err(|error| BenchmarkQualificationError::InvalidBundle(error.to_string()))?;
    bundle
        .binding()
        .and_then(|binding| binding.validate_bundle(bundle))
        .map_err(|error| BenchmarkQualificationError::InvalidBundle(error.to_string()))?;
    development
        .validate_integrity()
        .map_err(|error| BenchmarkQualificationError::InvalidSuite(error.to_string()))?;
    if development.id != bundle.development_suite_id
        || development.fingerprint != bundle.development_suite_fingerprint
    {
        return Err(BenchmarkQualificationError::InvalidBundle(
            "development suite does not match bundle authority".into(),
        ));
    }
    match (
        sealed,
        bundle.sealed_suite_id,
        &bundle.sealed_suite_fingerprint,
    ) {
        (None, None, None) => {}
        (Some(suite), Some(id), Some(fingerprint))
            if suite.id == id && &suite.fingerprint == fingerprint =>
        {
            suite
                .validate_integrity()
                .map_err(|error| BenchmarkQualificationError::InvalidSuite(error.to_string()))?;
        }
        _ => {
            return Err(BenchmarkQualificationError::InvalidBundle(
                "sealed suite does not match bundle authority".into(),
            ));
        }
    }

    let mut population_map = BTreeMap::new();
    for population in populations {
        if population_map
            .insert(population.cohort_id, population.members)
            .is_some()
        {
            return Err(BenchmarkQualificationError::PopulationSet);
        }
    }
    let expected_ids = development
        .cohorts
        .iter()
        .chain(sealed.into_iter().flat_map(|suite| suite.cohorts.iter()))
        .map(|cohort| cohort.cohort_id)
        .collect::<BTreeSet<_>>();
    if expected_ids != population_map.keys().copied().collect() {
        return Err(BenchmarkQualificationError::PopulationSet);
    }

    let binomial_floor = policy.binomial_support_floor()?;
    let mut cohorts = Vec::new();
    let mut issues = Vec::new();
    for suite in std::iter::once(development).chain(sealed) {
        for cohort in &suite.cohorts {
            let members = population_map[&cohort.cohort_id];
            validate_population(cohort.cohort_id, cohort.snapshot_id, cohort.split, members)?;
            let summary = summarize_population(suite, cohort, members)?;
            assess_support(
                suite,
                cohort.cohort_id,
                &summary,
                &policy,
                binomial_floor,
                &mut issues,
            );
            assess_quality(suite.id, cohort.cohort_id, &summary, &policy, &mut issues);
            cohorts.push(summary);
        }
        if suite
            .contract
            .regression
            .as_ref()
            .is_some_and(|requirement| requirement.require_mcnemar_significance)
        {
            for cohort in &suite.cohorts {
                issues.push(QualificationIssue {
                    severity: QualificationIssueSeverity::Warning,
                    code: "paired_power_is_conditional".into(),
                    suite_id: suite.id,
                    cohort_id: cohort.cohort_id,
                    target: None,
                    observed: None,
                    required: None,
                    message: "McNemar power also depends on the future baseline/candidate disagreement rate and cannot be established from population size alone".into(),
                });
            }
        }
    }
    cohorts.sort_by_key(|cohort| (cohort.suite_id, cohort.cohort_id));
    issues.sort_by_key(issue_key);
    let readiness = if issues
        .iter()
        .any(|issue| issue.severity == QualificationIssueSeverity::Blocking)
    {
        BenchmarkReadiness::Blocked
    } else {
        BenchmarkReadiness::Ready
    };
    let mut qualification = BenchmarkQualification {
        id: Uuid::new_v4(),
        protocol: BENCHMARK_QUALIFICATION_PROTOCOL.into(),
        benchmark_bundle_id: bundle.id,
        benchmark_bundle_fingerprint: bundle.fingerprint.clone(),
        development_suite_id: development.id,
        development_suite_fingerprint: development.fingerprint.clone(),
        sealed_suite_id: sealed.map(|suite| suite.id),
        sealed_suite_fingerprint: sealed.map(|suite| suite.fingerprint.clone()),
        policy,
        readiness,
        cohorts,
        issues,
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    qualification.fingerprint = qualification_fingerprint(&qualification)?;
    qualification.validate_integrity()?;
    Ok(qualification)
}

fn validate_population(
    cohort_id: Uuid,
    snapshot_id: Uuid,
    split: dataset_core::domain::SnapshotSplit,
    members: &[SnapshotMember],
) -> Result<(), BenchmarkQualificationError> {
    if members.is_empty() {
        return Err(invalid_population(cohort_id, "population is empty"));
    }
    let mut ids = BTreeSet::new();
    for member in members {
        if member.id.is_nil()
            || member.source_row_id.is_nil()
            || member.snapshot_id != snapshot_id
            || member.split != split
            || member.text.trim().is_empty()
            || member.label.trim().is_empty()
            || !ids.insert(member.id)
        {
            return Err(invalid_population(
                cohort_id,
                "member identity, snapshot, split, text, label, or uniqueness is invalid",
            ));
        }
    }
    Ok(())
}

fn summarize_population(
    suite: &BenchmarkSuite,
    cohort: &crate::benchmark::BenchmarkCohort,
    members: &[SnapshotMember],
) -> Result<BenchmarkCohortReadiness, BenchmarkQualificationError> {
    let mut label_support = suite
        .labels
        .iter()
        .cloned()
        .map(|label| (label, 0))
        .collect::<BTreeMap<_, _>>();
    let mut dimension_value_support = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut rows_by_producer = BTreeMap::new();
    let mut generated_rows = 0;
    let mut imported_rows = 0;
    let mut normalized_counts = BTreeMap::<String, u64>::new();
    let mut lengths = Vec::with_capacity(members.len());

    for member in members {
        let Some(count) = label_support.get_mut(&member.label) else {
            return Err(invalid_population(
                cohort.cohort_id,
                "member label is outside suite vocabulary",
            ));
        };
        *count += 1;
        for (name, value) in &member.dimensions {
            *dimension_value_support
                .entry(name.clone())
                .or_default()
                .entry(value.clone())
                .or_default() += 1;
        }
        let producer = match &member.source_provenance {
            SourceProvenance::Generated {
                generation_job_id,
                backend,
                model,
                ..
            } => {
                generated_rows += 1;
                format!("generated:{generation_job_id}:{backend}:{model}")
            }
            SourceProvenance::Imported { import_id, .. } => {
                imported_rows += 1;
                format!("imported:{import_id}")
            }
        };
        *rows_by_producer.entry(producer).or_default() += 1;
        *normalized_counts
            .entry(normalize_text(&member.text))
            .or_default() += 1;
        lengths.push(member.text.chars().count() as u64);
    }
    lengths.sort_unstable();
    let normalized_duplicate_rows = normalized_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum::<u64>();
    let total_support = members.len() as u64;

    let mut required_slice_support = BTreeMap::new();
    for requirement in &suite.contract.metric_requirements {
        if let MetricTarget::Slice { key } = &requirement.target {
            let identity = SliceIdentity::parse_key_for_labels(key, &suite.labels)
                .map_err(|error| BenchmarkQualificationError::InvalidSuite(error.to_string()))?;
            let support = members
                .iter()
                .filter(|member| member_matches_slice(member, &identity))
                .count() as u64;
            required_slice_support.insert(key.clone(), support);
        }
    }

    Ok(BenchmarkCohortReadiness {
        suite_id: suite.id,
        suite_fingerprint: suite.fingerprint.clone(),
        cohort_id: cohort.cohort_id,
        snapshot_id: cohort.snapshot_id,
        snapshot_fingerprint: cohort.snapshot_fingerprint.clone(),
        population_fingerprint: artifact_core::fingerprint(&{
            let mut population = members
                .iter()
                .map(|member| {
                    (
                        member.id,
                        member.source_row_id,
                        member.label.as_str(),
                        &member.dimensions,
                    )
                })
                .collect::<Vec<_>>();
            population.sort_by_key(|member| member.0);
            population
        })
        .map_err(map_fingerprint)?,
        total_support,
        label_support,
        required_slice_support,
        dimension_value_support,
        source_composition: SourceComposition {
            generated_rows,
            imported_rows,
            distinct_producers: rows_by_producer.len() as u64,
            rows_by_producer,
        },
        normalized_duplicate_rows,
        normalized_duplicate_rate: normalized_duplicate_rows as f64 / total_support as f64,
        minimum_text_characters: lengths[0],
        median_text_characters: lengths[(lengths.len() - 1) / 2],
        maximum_text_characters: *lengths.last().expect("non-empty population"),
        reference_distribution_bound: false,
    })
}

fn assess_support(
    suite: &BenchmarkSuite,
    cohort_id: Uuid,
    summary: &BenchmarkCohortReadiness,
    policy: &BenchmarkQualificationPolicy,
    binomial_floor: u64,
    issues: &mut Vec<QualificationIssue>,
) {
    let overall_contract_floor = suite
        .contract
        .metric_requirements
        .iter()
        .filter(|requirement| requirement.target == MetricTarget::Overall)
        .map(|requirement| requirement.minimum_support)
        .max()
        .unwrap_or(0);
    let overall_required = policy
        .minimum_overall_support
        .max(binomial_floor)
        .max(overall_contract_floor);
    support_issue(
        suite.id,
        cohort_id,
        MetricTarget::Overall,
        summary.total_support,
        overall_required,
        issues,
    );

    for (label, observed) in &summary.label_support {
        let target = MetricTarget::Label {
            label: label.clone(),
        };
        let contract_floor = suite
            .contract
            .metric_requirements
            .iter()
            .filter(|requirement| requirement.target == target)
            .map(|requirement| requirement.minimum_support)
            .max()
            .unwrap_or(0);
        let uncertainty_floor = if suite
            .contract
            .metric_requirements
            .iter()
            .any(|requirement| requirement.target == target)
        {
            binomial_floor
        } else {
            0
        };
        support_issue(
            suite.id,
            cohort_id,
            target,
            *observed,
            policy
                .minimum_label_support
                .max(contract_floor)
                .max(uncertainty_floor),
            issues,
        );
    }

    for requirement in &suite.contract.metric_requirements {
        let MetricTarget::Slice { key } = &requirement.target else {
            continue;
        };
        let observed = summary
            .required_slice_support
            .get(key)
            .copied()
            .unwrap_or(0);
        let required = requirement.minimum_support.max(binomial_floor).max(
            suite
                .cohorts
                .iter()
                .find(|cohort| cohort.cohort_id == cohort_id)
                .map(|cohort| cohort.protocol.minimum_slice_support)
                .unwrap_or(1),
        );
        support_issue(
            suite.id,
            cohort_id,
            requirement.target.clone(),
            observed,
            required,
            issues,
        );
    }
}

fn assess_quality(
    suite_id: Uuid,
    cohort_id: Uuid,
    summary: &BenchmarkCohortReadiness,
    policy: &BenchmarkQualificationPolicy,
    issues: &mut Vec<QualificationIssue>,
) {
    if summary.normalized_duplicate_rate > policy.maximum_normalized_duplicate_rate {
        issues.push(QualificationIssue {
            severity: QualificationIssueSeverity::Blocking,
            code: "normalized_duplicate_rate".into(),
            suite_id,
            cohort_id,
            target: None,
            observed: Some(summary.normalized_duplicate_rate),
            required: Some(policy.maximum_normalized_duplicate_rate),
            message: "normalized duplicate rate exceeds the qualification policy".into(),
        });
    }
    if summary.source_composition.distinct_producers < policy.minimum_distinct_producers {
        issues.push(QualificationIssue {
            severity: QualificationIssueSeverity::Blocking,
            code: "producer_diversity".into(),
            suite_id,
            cohort_id,
            target: None,
            observed: Some(summary.source_composition.distinct_producers as f64),
            required: Some(policy.minimum_distinct_producers as f64),
            message: "population has fewer distinct producers than required".into(),
        });
    }
    let maximum_producer_share = summary
        .source_composition
        .rows_by_producer
        .values()
        .copied()
        .max()
        .unwrap_or(0) as f64
        / summary.total_support as f64;
    if maximum_producer_share > policy.maximum_single_producer_share {
        issues.push(QualificationIssue {
            severity: QualificationIssueSeverity::Blocking,
            code: "producer_concentration".into(),
            suite_id,
            cohort_id,
            target: None,
            observed: Some(maximum_producer_share),
            required: Some(policy.maximum_single_producer_share),
            message: "one producer contributes more rows than the qualification policy permits"
                .into(),
        });
    }
    let minimum_label = summary.label_support.values().copied().min().unwrap_or(0);
    let maximum_label = summary.label_support.values().copied().max().unwrap_or(0);
    let imbalance = if minimum_label == 0 {
        f64::INFINITY
    } else {
        maximum_label as f64 / minimum_label as f64
    };
    if imbalance > policy.maximum_label_imbalance_ratio {
        issues.push(QualificationIssue {
            severity: QualificationIssueSeverity::Blocking,
            code: "label_imbalance".into(),
            suite_id,
            cohort_id,
            target: None,
            observed: imbalance.is_finite().then_some(imbalance),
            required: Some(policy.maximum_label_imbalance_ratio),
            message: "label support imbalance exceeds the qualification policy".into(),
        });
    }
    issues.push(QualificationIssue {
        severity: QualificationIssueSeverity::Warning,
        code: "representativeness_not_established".into(),
        suite_id,
        cohort_id,
        target: None,
        observed: None,
        required: None,
        message: "distribution and source composition are recorded, but no external reference distribution is bound".into(),
    });
    issues.push(QualificationIssue {
        severity: QualificationIssueSeverity::Warning,
        code: "semantic_quality_is_structural_only".into(),
        suite_id,
        cohort_id,
        target: None,
        observed: None,
        required: None,
        message: "semantic quality evidence is limited to labels, text presence, and normalized duplicates".into(),
    });
}

fn support_issue(
    suite_id: Uuid,
    cohort_id: Uuid,
    target: MetricTarget,
    observed: u64,
    required: u64,
    issues: &mut Vec<QualificationIssue>,
) {
    if observed < required {
        issues.push(QualificationIssue {
            severity: QualificationIssueSeverity::Blocking,
            code: "insufficient_support".into(),
            suite_id,
            cohort_id,
            target: Some(target),
            observed: Some(observed as f64),
            required: Some(required as f64),
            message: "benchmark target support is below its deterministic readiness floor".into(),
        });
    }
}

fn member_matches_slice(member: &SnapshotMember, identity: &SliceIdentity) -> bool {
    match identity.kind {
        SliceKind::ExpectedLabel => identity.attributes.get("label") == Some(&member.label),
        SliceKind::DimensionValue => identity
            .attributes
            .get("dimension")
            .zip(identity.attributes.get("value"))
            .is_some_and(|(name, value)| member.dimensions.get(name) == Some(value)),
        SliceKind::Cell => identity.attributes.iter().all(|(name, value)| {
            if name == "label" {
                &member.label == value
            } else {
                member.dimensions.get(name) == Some(value)
            }
        }),
        SliceKind::DimensionIntersection => identity
            .attributes
            .iter()
            .all(|(name, value)| member.dimensions.get(name) == Some(value)),
    }
}

fn qualification_fingerprint(
    qualification: &BenchmarkQualification,
) -> Result<String, BenchmarkQualificationError> {
    artifact_core::fingerprint(&(
        qualification.protocol.as_str(),
        qualification.benchmark_bundle_id,
        qualification.benchmark_bundle_fingerprint.as_str(),
        qualification.development_suite_id,
        qualification.development_suite_fingerprint.as_str(),
        qualification.sealed_suite_id,
        qualification.sealed_suite_fingerprint.as_deref(),
        &qualification.policy,
        qualification.readiness,
        &qualification.cohorts,
        &qualification.issues,
    ))
    .map_err(map_fingerprint)
}

fn issue_key(
    issue: &QualificationIssue,
) -> (Uuid, Uuid, QualificationIssueSeverity, String, String) {
    (
        issue.suite_id,
        issue.cohort_id,
        issue.severity,
        issue.code.clone(),
        issue
            .target
            .as_ref()
            .and_then(|target| serde_json::to_string(target).ok())
            .unwrap_or_default(),
    )
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn invalid_population(cohort_id: Uuid, reason: impl Into<String>) -> BenchmarkQualificationError {
    BenchmarkQualificationError::InvalidPopulation {
        cohort_id,
        reason: reason.into(),
    }
}

fn map_fingerprint(error: artifact_core::FingerprintError) -> BenchmarkQualificationError {
    BenchmarkQualificationError::Fingerprint(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use dataset_core::domain::{SnapshotMember, SnapshotSplit, SourceProvenance};
    use evaluation_core::domain::EvaluationProtocol;

    use super::*;
    use crate::{
        benchmark::{
            AcceptanceContract, BenchmarkCohort, BenchmarkMetric, BenchmarkSuite,
            BenchmarkSuiteKind, MetricRequirement,
        },
        governance::{CohortRole, DisclosureLevel},
    };

    fn fixture(rows_per_label: usize) -> (BenchmarkBundle, BenchmarkSuite, Vec<SnapshotMember>) {
        let suite_id = Uuid::new_v4();
        let cohort_id = Uuid::new_v4();
        let snapshot_id = Uuid::new_v4();
        let fingerprint = "a".repeat(64);
        let protocol = EvaluationProtocol::default();
        let mut suite = BenchmarkSuite {
            id: suite_id,
            name: "development".into(),
            kind: BenchmarkSuiteKind::Development,
            task: "intent".into(),
            labels: vec!["a".into(), "b".into()],
            required_model_formats: Vec::new(),
            cohorts: vec![BenchmarkCohort {
                cohort_id,
                cohort_fingerprint: fingerprint.clone(),
                evaluation_cohort_fingerprint: artifact_core::fingerprint(&(
                    fingerprint.as_str(),
                    SnapshotSplit::Test,
                ))
                .unwrap(),
                snapshot_id,
                snapshot_fingerprint: fingerprint.clone(),
                split: SnapshotSplit::Test,
                role: CohortRole::Development,
                role_decision_id: Uuid::new_v4(),
                role_decision_fingerprint: fingerprint.clone(),
                protocol_fingerprint: protocol.fingerprint().unwrap(),
                protocol,
                disclosure: DisclosureLevel::Predictions,
                adaptation_eligible: true,
            }],
            contract: AcceptanceContract {
                metric_requirements: vec![MetricRequirement {
                    target: MetricTarget::Overall,
                    metric: BenchmarkMetric::Accuracy,
                    minimum: Some(0.8),
                    maximum: None,
                    minimum_support: 1,
                }],
                regression: None,
            },
            contamination_report_id: Uuid::new_v4(),
            contamination_report_fingerprint: fingerprint.clone(),
            contamination_override_fingerprint: None,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        suite.fingerprint = suite.reproduce_fingerprint().unwrap();
        suite.validate_integrity().unwrap();
        let mut bundle = BenchmarkBundle {
            id: Uuid::new_v4(),
            development_suite_id: suite.id,
            development_suite_fingerprint: suite.fingerprint.clone(),
            sealed_suite_id: None,
            sealed_suite_fingerprint: None,
            contamination_report_id: Uuid::new_v4(),
            contamination_report_fingerprint: fingerprint,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        bundle.fingerprint = bundle.reproduce_fingerprint().unwrap();
        let mut members = Vec::new();
        for label in ["a", "b"] {
            for index in 0..rows_per_label {
                members.push(SnapshotMember {
                    id: Uuid::new_v4(),
                    snapshot_id,
                    source_row_id: Uuid::new_v4(),
                    split: SnapshotSplit::Test,
                    text: format!("{label} example {index}"),
                    label: label.into(),
                    dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
                    fields: BTreeMap::new(),
                    source_provenance: SourceProvenance::Imported {
                        import_id: Uuid::new_v4(),
                        source_path: "fixture.jsonl".into(),
                        source_row_number: index as u64 + 1,
                    },
                    source_created_at: Utc::now(),
                });
            }
        }
        (bundle, suite, members)
    }

    #[test]
    fn blocks_populations_that_cannot_support_the_decision() {
        let (bundle, suite, members) = fixture(10);
        let qualification = qualify_benchmark_bundle(
            &bundle,
            &suite,
            None,
            vec![QualificationPopulation {
                cohort_id: suite.cohorts[0].cohort_id,
                members: &members,
            }],
            BenchmarkQualificationPolicy::default(),
        )
        .unwrap();

        assert_eq!(qualification.readiness, BenchmarkReadiness::Blocked);
        assert!(qualification.issues.iter().any(|issue| {
            issue.code == "insufficient_support"
                && issue.target == Some(MetricTarget::Overall)
                && issue.required == Some(100.0)
        }));
        qualification.validate_integrity().unwrap();
    }

    #[test]
    fn produces_ready_evidence_without_claiming_representativeness() {
        let (bundle, suite, members) = fixture(60);
        let qualification = qualify_benchmark_bundle(
            &bundle,
            &suite,
            None,
            vec![QualificationPopulation {
                cohort_id: suite.cohorts[0].cohort_id,
                members: &members,
            }],
            BenchmarkQualificationPolicy::default(),
        )
        .unwrap();

        assert_eq!(qualification.readiness, BenchmarkReadiness::Ready);
        assert!(qualification.issues.iter().any(|issue| {
            issue.severity == QualificationIssueSeverity::Warning
                && issue.code == "representativeness_not_established"
        }));
        assert_eq!(qualification.cohorts[0].total_support, 120);
        assert_eq!(qualification.cohorts[0].label_support["a"], 60);
        qualification.binding().unwrap();
    }

    #[test]
    fn normalized_duplicates_and_source_concentration_are_policy_failures() {
        let (bundle, suite, mut members) = fixture(60);
        let one_import = Uuid::new_v4();
        for member in &mut members {
            member.source_provenance = SourceProvenance::Imported {
                import_id: one_import,
                source_path: "fixture.jsonl".into(),
                source_row_number: 1,
            };
        }
        members[1].text = members[0].text.to_uppercase();
        let policy = BenchmarkQualificationPolicy {
            maximum_normalized_duplicate_rate: 0.0,
            maximum_single_producer_share: 0.75,
            ..BenchmarkQualificationPolicy::default()
        };
        let qualification = qualify_benchmark_bundle(
            &bundle,
            &suite,
            None,
            vec![QualificationPopulation {
                cohort_id: suite.cohorts[0].cohort_id,
                members: &members,
            }],
            policy,
        )
        .unwrap();

        assert_eq!(qualification.readiness, BenchmarkReadiness::Blocked);
        assert!(
            qualification
                .issues
                .iter()
                .any(|issue| issue.code == "normalized_duplicate_rate")
        );
        assert!(
            qualification
                .issues
                .iter()
                .any(|issue| issue.code == "producer_concentration")
        );
    }
}
