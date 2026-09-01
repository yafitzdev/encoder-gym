use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use evaluation_core::domain::{EvaluationProtocol, SliceIdentity};
use research_core::evidence::ResearchEvidence;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use workflow_core::{
    benchmark::{
        AcceptanceContract, BenchmarkSuiteKind, MetricTarget, validate_acceptance_contract,
    },
    benchmark_qualification::BenchmarkQualificationPolicy,
    governance::{CohortOrigin, CohortRole, DisclosureLevel},
};

use crate::{
    BenchmarkArchitectError,
    brief::ResolvedBenchmarkArchitectBrief,
    fingerprint,
    lifecycle::{BenchmarkArchitectRun, BenchmarkArchitectRunState},
    required,
};

pub const BENCHMARK_BLUEPRINT_SCHEMA_VERSION: u32 = 1;
pub const ACQUISITION_HANDOFF_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintIssueSeverity {
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintIssue {
    pub severity: BlueprintIssueSeverity,
    pub code: String,
    #[serde(default)]
    pub suite_kind: Option<BenchmarkSuiteKind>,
    #[serde(default)]
    pub cohort_key: Option<String>,
    pub message: String,
    #[serde(default)]
    pub observed: Option<f64>,
    #[serde(default)]
    pub required: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintValidationReport {
    pub valid: bool,
    pub uncertainty_support_floor: u64,
    pub issues: Vec<BlueprintIssue>,
    pub fingerprint: String,
}

impl BlueprintValidationReport {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortFreshnessPolicy {
    pub maximum_age_days: u32,
    pub maximum_workflow_iterations: u32,
    pub maximum_adaptive_exposures: u32,
    pub maximum_acceptance_exposures: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkCohortBlueprint {
    pub key: String,
    pub name: String,
    pub purpose: String,
    pub origin: CohortOrigin,
    pub role: CohortRole,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub protocol: EvaluationProtocol,
    pub minimum_total_support: u64,
    pub minimum_label_support: BTreeMap<String, u64>,
    #[serde(default)]
    pub required_dimension_values: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub required_slice_support: BTreeMap<String, u64>,
    pub required_source_classes: Vec<String>,
    pub minimum_distinct_producers: u64,
    pub freshness: CohortFreshnessPolicy,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSuiteBlueprint {
    pub kind: BenchmarkSuiteKind,
    pub name: String,
    pub purpose: String,
    #[serde(default)]
    pub required_model_formats: Vec<String>,
    pub cohorts: Vec<BenchmarkCohortBlueprint>,
    pub contract: AcceptanceContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskCoverageRequirement {
    pub key: String,
    pub deployment_risk_key: String,
    pub cohort_keys: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub supporting_evidence_ids: Vec<Uuid>,
    #[serde(default)]
    pub conflicting_evidence_ids: Vec<Uuid>,
    #[serde(default)]
    pub inference: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkBlueprintDraft {
    pub summary: String,
    pub suites: Vec<BenchmarkSuiteBlueprint>,
    pub qualification_policy: BenchmarkQualificationPolicy,
    pub risk_coverage: Vec<RiskCoverageRequirement>,
    #[serde(default)]
    pub tradeoffs: Vec<String>,
    #[serde(default)]
    pub evidence_gaps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkArchitectureProposal {
    pub id: Uuid,
    pub schema_version: u32,
    pub run_id: Uuid,
    pub run_fingerprint: String,
    pub brief_id: Uuid,
    pub brief_fingerprint: String,
    pub summary: String,
    pub suites: Vec<BenchmarkSuiteBlueprint>,
    pub qualification_policy: BenchmarkQualificationPolicy,
    pub risk_coverage: Vec<RiskCoverageRequirement>,
    pub tradeoffs: Vec<String>,
    pub evidence_gaps: Vec<String>,
    pub evidence_fingerprints: BTreeMap<Uuid, String>,
    pub validation: BlueprintValidationReport,
    pub acquisition_requirements: Vec<BenchmarkAcquisitionRequirement>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkArchitectureProposal {
    pub fn create(
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &BenchmarkArchitectRun,
        evidence: &[ResearchEvidence],
        mut draft: BenchmarkBlueprintDraft,
    ) -> Result<Self, BenchmarkArchitectError> {
        if brief.reproduce_fingerprint()? != brief.fingerprint
            || run.reproduce_specification_fingerprint()? != run.specification_fingerprint
            || run.brief_id != brief.id
            || run.brief_fingerprint != brief.fingerprint
            || run.state != BenchmarkArchitectRunState::Running
        {
            return Err(BenchmarkArchitectError::Integrity(
                "proposal inputs do not reproduce or belong together".into(),
            ));
        }
        validate_evidence(run.id, evidence)?;
        normalize_draft(&mut draft)?;
        let validation = validate_blueprint(brief, evidence, &draft)?;
        if !validation.valid {
            return Err(BenchmarkArchitectError::Validation(format!(
                "blueprint is blocked: {}",
                validation
                    .issues
                    .iter()
                    .filter(|issue| issue.severity == BlueprintIssueSeverity::Blocking)
                    .map(|issue| issue.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let acquisition_requirements = derive_acquisition_requirements(&draft.suites);
        let evidence_fingerprints = evidence
            .iter()
            .map(|value| (value.id, value.fingerprint.clone()))
            .collect();
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: BENCHMARK_BLUEPRINT_SCHEMA_VERSION,
            run_id: run.id,
            run_fingerprint: run.specification_fingerprint.clone(),
            brief_id: brief.id,
            brief_fingerprint: brief.fingerprint.clone(),
            summary: draft.summary,
            suites: draft.suites,
            qualification_policy: draft.qualification_policy,
            risk_coverage: draft.risk_coverage,
            tradeoffs: draft.tradeoffs,
            evidence_gaps: draft.evidence_gaps,
            evidence_fingerprints,
            validation,
            acquisition_requirements,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate_integrity(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        evidence: &[ResearchEvidence],
    ) -> Result<(), BenchmarkArchitectError> {
        if self.reproduce_fingerprint()? != self.fingerprint
            || self.brief_id != brief.id
            || self.brief_fingerprint != brief.fingerprint
            || self.evidence_fingerprints
                != evidence
                    .iter()
                    .map(|value| (value.id, value.fingerprint.clone()))
                    .collect()
            || self.acquisition_requirements != derive_acquisition_requirements(&self.suites)
        {
            return Err(BenchmarkArchitectError::Integrity(
                "proposal pins, evidence, or acquisition requirements do not reproduce".into(),
            ));
        }
        let draft = self.as_draft();
        let expected = validate_blueprint(brief, evidence, &draft)?;
        if expected != self.validation || !expected.valid {
            return Err(BenchmarkArchitectError::Integrity(
                "proposal validation no longer reproduces".into(),
            ));
        }
        Ok(())
    }

    fn as_draft(&self) -> BenchmarkBlueprintDraft {
        BenchmarkBlueprintDraft {
            summary: self.summary.clone(),
            suites: self.suites.clone(),
            qualification_policy: self.qualification_policy.clone(),
            risk_coverage: self.risk_coverage.clone(),
            tradeoffs: self.tradeoffs.clone(),
            evidence_gaps: self.evidence_gaps.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkAcquisitionRequirement {
    pub cohort_key: String,
    pub suite_kind: BenchmarkSuiteKind,
    pub origin: CohortOrigin,
    pub role: CohortRole,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub protocol: EvaluationProtocol,
    pub minimum_total_support: u64,
    pub minimum_label_support: BTreeMap<String, u64>,
    pub required_dimension_values: BTreeMap<String, Vec<String>>,
    pub required_slice_support: BTreeMap<String, u64>,
    pub required_source_classes: Vec<String>,
    pub minimum_distinct_producers: u64,
    pub freshness: CohortFreshnessPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkArchitectureReviewDecision {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkArchitectureReview {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub predecessor_id: Option<Uuid>,
    pub decision: BenchmarkArchitectureReviewDecision,
    pub reviewer: String,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkArchitectureReview {
    pub fn create(
        proposal: &BenchmarkArchitectureProposal,
        predecessor: Option<&Self>,
        decision: BenchmarkArchitectureReviewDecision,
        reviewer: String,
        rationale: String,
    ) -> Result<Self, BenchmarkArchitectError> {
        if proposal.reproduce_fingerprint()? != proposal.fingerprint
            || predecessor.is_some_and(|value| value.proposal_id != proposal.id)
        {
            return Err(BenchmarkArchitectError::Integrity(
                "review inputs do not reproduce or belong together".into(),
            ));
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            predecessor_id: predecessor.map(|value| value.id),
            decision,
            reviewer: required(reviewer, "review.reviewer")?,
            rationale: required(rationale, "review.rationale")?,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkAcquisitionHandoff {
    pub id: Uuid,
    pub schema_version: u32,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub task: String,
    pub labels: Vec<String>,
    pub requirements: Vec<BenchmarkAcquisitionRequirement>,
    pub qualification_policy: BenchmarkQualificationPolicy,
    pub suite_contracts: BTreeMap<BenchmarkSuiteKind, AcceptanceContract>,
    pub risk_coverage: Vec<RiskCoverageRequirement>,
    pub unresolved_evidence_gaps: Vec<String>,
    pub authority_notice: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl BenchmarkAcquisitionHandoff {
    pub fn create(
        brief: &ResolvedBenchmarkArchitectBrief,
        proposal: &BenchmarkArchitectureProposal,
        approval: &BenchmarkArchitectureReview,
        evidence: &[ResearchEvidence],
    ) -> Result<Self, BenchmarkArchitectError> {
        proposal.validate_integrity(brief, evidence)?;
        if approval.reproduce_fingerprint()? != approval.fingerprint
            || approval.proposal_id != proposal.id
            || approval.proposal_fingerprint != proposal.fingerprint
            || approval.decision != BenchmarkArchitectureReviewDecision::Approve
        {
            return Err(BenchmarkArchitectError::Integrity(
                "handoff requires the current reproducible human approval".into(),
            ));
        }
        let suite_contracts = proposal
            .suites
            .iter()
            .map(|suite| (suite.kind, suite.contract.clone()))
            .collect();
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: ACQUISITION_HANDOFF_SCHEMA_VERSION,
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            approval_id: approval.id,
            approval_fingerprint: approval.fingerprint.clone(),
            task: brief.task.clone(),
            labels: brief.labels.clone(),
            requirements: proposal.acquisition_requirements.clone(),
            qualification_policy: proposal.qualification_policy.clone(),
            suite_contracts,
            risk_coverage: proposal.risk_coverage.clone(),
            unresolved_evidence_gaps: proposal.evidence_gaps.clone(),
            authority_notice: "This handoff does not authorize benchmark use. Import, strict contamination checks, deterministic qualification, and independent approval remain required.".into(),
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCohortFacts {
    pub cohort_key: String,
    pub suite_kind: BenchmarkSuiteKind,
    pub origin: CohortOrigin,
    pub role: CohortRole,
    pub disclosure: DisclosureLevel,
    pub adaptation_eligible: bool,
    pub protocol: EvaluationProtocol,
    pub total_support: u64,
    pub label_support: BTreeMap<String, u64>,
    #[serde(default)]
    pub dimension_value_support: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub slice_support: BTreeMap<String, u64>,
    pub source_classes: Vec<String>,
    pub distinct_producers: u64,
    pub acquired_at: DateTime<Utc>,
    pub workflow_iterations: u32,
    pub adaptive_exposures: u32,
    pub acceptance_exposures: u32,
    pub retired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionConformanceState {
    Conformant,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionConformance {
    pub handoff_id: Uuid,
    pub handoff_fingerprint: String,
    pub state: AcquisitionConformanceState,
    pub issues: Vec<BlueprintIssue>,
    pub assessed_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AcquisitionConformance {
    pub fn reproduce_fingerprint(&self) -> Result<String, BenchmarkArchitectError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

/// Compares row-free persisted candidate summaries with an approved sourcing
/// handoff. It does not create cohorts or infer benchmark authority.
pub fn assess_acquisition_conformance(
    handoff: &BenchmarkAcquisitionHandoff,
    candidates: &[CandidateCohortFacts],
    assessed_at: DateTime<Utc>,
) -> Result<AcquisitionConformance, BenchmarkArchitectError> {
    if handoff.reproduce_fingerprint()? != handoff.fingerprint {
        return Err(BenchmarkArchitectError::Integrity(
            "acquisition handoff fingerprint does not reproduce".into(),
        ));
    }
    let requirements = handoff
        .requirements
        .iter()
        .map(|value| (value.cohort_key.as_str(), value))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    let mut issues = Vec::new();
    for candidate in candidates {
        if actual
            .insert(candidate.cohort_key.as_str(), candidate)
            .is_some()
        {
            block(
                &mut issues,
                "duplicate_candidate_cohort",
                Some(candidate.suite_kind),
                Some(candidate.cohort_key.clone()),
                "candidate cohort key appears more than once",
            );
        }
    }
    for key in actual
        .keys()
        .filter(|key| !requirements.contains_key(**key))
    {
        block(
            &mut issues,
            "unexpected_candidate_cohort",
            None,
            Some((*key).to_owned()),
            "candidate cohort is outside the approved handoff",
        );
    }
    for (key, requirement) in requirements {
        let Some(candidate) = actual.get(key) else {
            block(
                &mut issues,
                "missing_candidate_cohort",
                Some(requirement.suite_kind),
                Some(key.to_owned()),
                "approved acquisition requirement has no candidate cohort",
            );
            continue;
        };
        assess_candidate(requirement, candidate, assessed_at, &mut issues);
    }
    issues.sort_by(issue_order);
    let mut value = AcquisitionConformance {
        handoff_id: handoff.id,
        handoff_fingerprint: handoff.fingerprint.clone(),
        state: if issues
            .iter()
            .any(|issue| issue.severity == BlueprintIssueSeverity::Blocking)
        {
            AcquisitionConformanceState::Blocked
        } else {
            AcquisitionConformanceState::Conformant
        },
        issues,
        assessed_at,
        fingerprint: String::new(),
    };
    value.fingerprint = value.reproduce_fingerprint()?;
    Ok(value)
}

fn assess_candidate(
    requirement: &BenchmarkAcquisitionRequirement,
    candidate: &CandidateCohortFacts,
    assessed_at: DateTime<Utc>,
    issues: &mut Vec<BlueprintIssue>,
) {
    let key = Some(candidate.cohort_key.clone());
    if candidate.suite_kind != requirement.suite_kind
        || candidate.origin != requirement.origin
        || candidate.role != requirement.role
        || candidate.disclosure != requirement.disclosure
        || candidate.adaptation_eligible != requirement.adaptation_eligible
        || candidate.protocol != requirement.protocol
    {
        block(
            issues,
            "candidate_authority_mismatch",
            Some(requirement.suite_kind),
            key.clone(),
            "candidate role, disclosure, origin, or protocol differs from the handoff",
        );
    }
    if candidate.total_support < requirement.minimum_total_support {
        issue_numeric(
            issues,
            "candidate_total_support",
            Some(requirement.suite_kind),
            key.clone(),
            "candidate total support is insufficient",
            candidate.total_support,
            requirement.minimum_total_support,
        );
    }
    for (label, minimum) in &requirement.minimum_label_support {
        let observed = candidate.label_support.get(label).copied().unwrap_or(0);
        if observed < *minimum {
            issue_numeric(
                issues,
                "candidate_label_support",
                Some(requirement.suite_kind),
                key.clone(),
                format!("candidate label {label:?} support is insufficient"),
                observed,
                *minimum,
            );
        }
    }
    for (dimension, values) in &requirement.required_dimension_values {
        for value in values {
            if candidate
                .dimension_value_support
                .get(dimension)
                .and_then(|counts| counts.get(value))
                .copied()
                .unwrap_or(0)
                == 0
            {
                block(
                    issues,
                    "candidate_dimension_coverage",
                    Some(requirement.suite_kind),
                    key.clone(),
                    format!("candidate lacks required {dimension}={value}"),
                );
            }
        }
    }
    for (slice, minimum) in &requirement.required_slice_support {
        let observed = candidate.slice_support.get(slice).copied().unwrap_or(0);
        if observed < *minimum {
            issue_numeric(
                issues,
                "candidate_slice_support",
                Some(requirement.suite_kind),
                key.clone(),
                format!("candidate slice {slice:?} support is insufficient"),
                observed,
                *minimum,
            );
        }
    }
    let source_classes = candidate.source_classes.iter().collect::<BTreeSet<_>>();
    if requirement
        .required_source_classes
        .iter()
        .any(|class| !source_classes.contains(class))
        || candidate.distinct_producers < requirement.minimum_distinct_producers
    {
        block(
            issues,
            "candidate_source_diversity",
            Some(requirement.suite_kind),
            key.clone(),
            "candidate source classes or producer diversity are insufficient",
        );
    }
    let age_days = assessed_at
        .signed_duration_since(candidate.acquired_at)
        .num_days();
    if candidate.retired
        || age_days < 0
        || u32::try_from(age_days)
            .ok()
            .is_none_or(|age| age > requirement.freshness.maximum_age_days)
        || candidate.workflow_iterations > requirement.freshness.maximum_workflow_iterations
        || candidate.adaptive_exposures > requirement.freshness.maximum_adaptive_exposures
        || candidate.acceptance_exposures > requirement.freshness.maximum_acceptance_exposures
    {
        block(
            issues,
            "candidate_stale_or_exposed",
            Some(requirement.suite_kind),
            key,
            "candidate exceeded an age, iteration, exposure, or retirement boundary",
        );
    }
}

pub fn validate_blueprint(
    brief: &ResolvedBenchmarkArchitectBrief,
    evidence: &[ResearchEvidence],
    draft: &BenchmarkBlueprintDraft,
) -> Result<BlueprintValidationReport, BenchmarkArchitectError> {
    brief.reproduce_fingerprint().and_then(|fingerprint| {
        if fingerprint == brief.fingerprint {
            Ok(())
        } else {
            Err(BenchmarkArchitectError::Integrity(
                "brief fingerprint does not reproduce".into(),
            ))
        }
    })?;
    validate_evidence_ids(evidence)?;
    draft
        .qualification_policy
        .validate()
        .map_err(|error| BenchmarkArchitectError::Validation(error.to_string()))?;
    let floor = draft
        .qualification_policy
        .binomial_support_floor()
        .map_err(|error| BenchmarkArchitectError::Validation(error.to_string()))?;
    let evidence_ids = evidence
        .iter()
        .map(|value| value.id)
        .collect::<BTreeSet<_>>();
    let risk_keys = brief
        .risks
        .iter()
        .map(|value| value.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut issues = Vec::new();
    let mut suite_kinds = BTreeSet::new();
    let mut cohort_keys = BTreeSet::new();
    for suite in &draft.suites {
        if !suite_kinds.insert(suite.kind) {
            block(
                &mut issues,
                "duplicate_suite_kind",
                Some(suite.kind),
                None,
                "each suite kind may appear at most once",
            );
        }
        if suite.cohorts.is_empty() {
            block(
                &mut issues,
                "empty_suite",
                Some(suite.kind),
                None,
                "a benchmark suite requires at least one cohort",
            );
        }
        if let Err(error) = validate_acceptance_contract(&suite.contract, &brief.labels, suite.kind)
        {
            block(
                &mut issues,
                "invalid_acceptance_contract",
                Some(suite.kind),
                None,
                error.to_string(),
            );
        }
        for cohort in &suite.cohorts {
            let cohort_key = cohort.key.clone();
            if !cohort_keys.insert(cohort_key.clone()) {
                block(
                    &mut issues,
                    "duplicate_cohort_key",
                    Some(suite.kind),
                    Some(cohort_key.clone()),
                    "cohort keys must be globally unique",
                );
            }
            validate_cohort(brief, suite, cohort, floor, &mut issues);
        }
    }
    if !suite_kinds.contains(&BenchmarkSuiteKind::Development) {
        block(
            &mut issues,
            "development_suite_required",
            None,
            None,
            "benchmark architecture requires a development suite",
        );
    }
    if !suite_kinds.contains(&BenchmarkSuiteKind::SealedAcceptance) {
        warn(
            &mut issues,
            "sealed_suite_missing",
            None,
            None,
            "no sealed acceptance suite is proposed; final generalization remains unverified",
        );
    }
    let mut covered_risks = BTreeSet::new();
    let mut requirement_keys = BTreeSet::new();
    for requirement in &draft.risk_coverage {
        if !requirement_keys.insert(requirement.key.clone()) {
            block(
                &mut issues,
                "duplicate_risk_requirement",
                None,
                None,
                "risk coverage keys must be unique",
            );
        }
        if !risk_keys.contains(requirement.deployment_risk_key.as_str()) {
            block(
                &mut issues,
                "unknown_deployment_risk",
                None,
                None,
                format!(
                    "risk coverage {:?} names an unknown deployment risk",
                    requirement.key
                ),
            );
        } else {
            covered_risks.insert(requirement.deployment_risk_key.as_str());
        }
        if requirement.cohort_keys.is_empty()
            || requirement
                .cohort_keys
                .iter()
                .any(|key| !cohort_keys.contains(key))
        {
            block(
                &mut issues,
                "invalid_risk_cohort_binding",
                None,
                None,
                format!(
                    "risk coverage {:?} must bind existing cohort keys",
                    requirement.key
                ),
            );
        }
        let all_evidence = requirement
            .supporting_evidence_ids
            .iter()
            .chain(&requirement.conflicting_evidence_ids);
        if all_evidence.clone().any(|id| !evidence_ids.contains(id)) {
            block(
                &mut issues,
                "unknown_evidence",
                None,
                None,
                format!(
                    "risk coverage {:?} references missing run evidence",
                    requirement.key
                ),
            );
        }
        if requirement.supporting_evidence_ids.is_empty() && !requirement.inference {
            block(
                &mut issues,
                "unsupported_risk_claim",
                None,
                None,
                format!(
                    "risk coverage {:?} needs evidence or inference=true",
                    requirement.key
                ),
            );
        }
    }
    for risk in risk_keys.difference(&covered_risks) {
        block(
            &mut issues,
            "uncovered_deployment_risk",
            None,
            None,
            format!("deployment risk {risk:?} has no benchmark coverage"),
        );
    }
    if draft
        .suites
        .iter()
        .flat_map(|suite| &suite.cohorts)
        .all(|cohort| cohort.origin == CohortOrigin::InternalSnapshot)
    {
        warn(
            &mut issues,
            "external_evidence_missing",
            None,
            None,
            "all proposed cohorts are internal; real-world representativeness remains unverified",
        );
    }
    warn(
        &mut issues,
        "reference_distribution_unverified",
        None,
        None,
        "a blueprint proposes coverage but does not prove real-world prevalence",
    );
    issues.sort_by(issue_order);
    let mut report = BlueprintValidationReport {
        valid: !issues
            .iter()
            .any(|issue| issue.severity == BlueprintIssueSeverity::Blocking),
        uncertainty_support_floor: floor,
        issues,
        fingerprint: String::new(),
    };
    report.fingerprint = report.reproduce_fingerprint()?;
    Ok(report)
}

fn validate_cohort(
    brief: &ResolvedBenchmarkArchitectBrief,
    suite: &BenchmarkSuiteBlueprint,
    cohort: &BenchmarkCohortBlueprint,
    floor: u64,
    issues: &mut Vec<BlueprintIssue>,
) {
    let key = Some(cohort.key.clone());
    if let Err(error) = cohort.protocol.validate_for_labels(&brief.labels) {
        block(
            issues,
            "invalid_evaluation_protocol",
            Some(suite.kind),
            key.clone(),
            error.to_string(),
        );
    }
    let role_ok = match suite.kind {
        BenchmarkSuiteKind::Development => matches!(
            cohort.role,
            CohortRole::Development | CohortRole::Diagnostic | CohortRole::ExternalBenchmark
        ),
        BenchmarkSuiteKind::SealedAcceptance => matches!(
            cohort.role,
            CohortRole::SealedAcceptance | CohortRole::ExternalBenchmark
        ),
    };
    if !role_ok {
        block(
            issues,
            "incompatible_cohort_role",
            Some(suite.kind),
            key.clone(),
            "cohort role is incompatible with suite kind",
        );
    }
    if cohort.origin == CohortOrigin::ExternalBenchmark
        && cohort.role != CohortRole::ExternalBenchmark
        || cohort.origin == CohortOrigin::InternalSnapshot
            && cohort.role == CohortRole::ExternalBenchmark
    {
        block(
            issues,
            "origin_role_mismatch",
            Some(suite.kind),
            key.clone(),
            "cohort origin and role do not agree",
        );
    }
    if suite.kind == BenchmarkSuiteKind::SealedAcceptance
        && (cohort.disclosure != DisclosureLevel::Aggregate || cohort.adaptation_eligible)
    {
        block(
            issues,
            "unsafe_sealed_disclosure",
            Some(suite.kind),
            key.clone(),
            "sealed cohorts must be aggregate-only and adaptation-ineligible",
        );
    }
    let required_total = floor.max(
        suite
            .contract
            .metric_requirements
            .iter()
            .filter(|requirement| requirement.target == MetricTarget::Overall)
            .map(|requirement| requirement.minimum_support)
            .max()
            .unwrap_or(1),
    );
    if cohort.minimum_total_support < required_total
        || cohort.minimum_total_support < brief_label_total_floor(brief, floor)
    {
        issue_numeric(
            issues,
            "underpowered_total_support",
            Some(suite.kind),
            key.clone(),
            "cohort total support is below its statistical/label decision floor",
            cohort.minimum_total_support,
            required_total.max(brief_label_total_floor(brief, floor)),
        );
    }
    if cohort.minimum_label_support.keys().collect::<BTreeSet<_>>()
        != brief.labels.iter().collect::<BTreeSet<_>>()
    {
        block(
            issues,
            "incomplete_label_support",
            Some(suite.kind),
            key.clone(),
            "every label must have an explicit support requirement",
        );
    }
    for label in &brief.labels {
        let metric_floor = suite
            .contract
            .metric_requirements
            .iter()
            .filter_map(|requirement| match &requirement.target {
                MetricTarget::Label { label: target } if target == label => {
                    Some(requirement.minimum_support)
                }
                _ => None,
            })
            .max()
            .unwrap_or(1);
        let required = floor
            .max(metric_floor)
            .max(cohort.protocol.minimum_slice_support);
        let observed = cohort
            .minimum_label_support
            .get(label)
            .copied()
            .unwrap_or(0);
        if observed < required {
            issue_numeric(
                issues,
                "underpowered_label_support",
                Some(suite.kind),
                key.clone(),
                format!("label {label:?} support is below its decision floor"),
                observed,
                required,
            );
        }
    }
    for (slice, support) in &cohort.required_slice_support {
        if SliceIdentity::parse_key_for_labels(slice, &brief.labels).is_err() {
            block(
                issues,
                "invalid_slice_requirement",
                Some(suite.kind),
                key.clone(),
                format!("slice requirement {slice:?} is not canonical"),
            );
        }
        let required = floor.max(cohort.protocol.minimum_slice_support);
        if *support < required {
            issue_numeric(
                issues,
                "underpowered_slice_support",
                Some(suite.kind),
                key.clone(),
                format!("slice {slice:?} support is below its decision floor"),
                *support,
                required,
            );
        }
    }
    if cohort.minimum_distinct_producers == 0 || cohort.required_source_classes.is_empty() {
        block(
            issues,
            "source_diversity_required",
            Some(suite.kind),
            key.clone(),
            "cohort requires at least one source class and one producer",
        );
    }
    validate_freshness(suite.kind, cohort, issues);
}

fn validate_freshness(
    kind: BenchmarkSuiteKind,
    cohort: &BenchmarkCohortBlueprint,
    issues: &mut Vec<BlueprintIssue>,
) {
    let key = Some(cohort.key.clone());
    if cohort.freshness.maximum_age_days == 0 || cohort.freshness.maximum_workflow_iterations == 0 {
        block(
            issues,
            "unbounded_freshness",
            Some(kind),
            key.clone(),
            "age and workflow-iteration freshness limits must be finite and positive",
        );
    }
    match kind {
        BenchmarkSuiteKind::Development => {
            if cohort.freshness.maximum_adaptive_exposures == 0
                || cohort.freshness.maximum_acceptance_exposures != 0
            {
                block(
                    issues,
                    "invalid_development_freshness",
                    Some(kind),
                    key,
                    "development cohorts need positive adaptive exposure and zero acceptance exposure limits",
                );
            }
        }
        BenchmarkSuiteKind::SealedAcceptance => {
            if cohort.freshness.maximum_adaptive_exposures != 0
                || cohort.freshness.maximum_acceptance_exposures != 1
            {
                block(
                    issues,
                    "invalid_sealed_freshness",
                    Some(kind),
                    key,
                    "sealed cohorts permit zero adaptive and exactly one aggregate acceptance exposure",
                );
            }
        }
    }
}

fn brief_label_total_floor(brief: &ResolvedBenchmarkArchitectBrief, floor: u64) -> u64 {
    floor.saturating_mul(brief.labels.len() as u64)
}

fn derive_acquisition_requirements(
    suites: &[BenchmarkSuiteBlueprint],
) -> Vec<BenchmarkAcquisitionRequirement> {
    let mut values = suites
        .iter()
        .flat_map(|suite| {
            suite
                .cohorts
                .iter()
                .map(|cohort| BenchmarkAcquisitionRequirement {
                    cohort_key: cohort.key.clone(),
                    suite_kind: suite.kind,
                    origin: cohort.origin,
                    role: cohort.role,
                    disclosure: cohort.disclosure,
                    adaptation_eligible: cohort.adaptation_eligible,
                    protocol: cohort.protocol.clone(),
                    minimum_total_support: cohort.minimum_total_support,
                    minimum_label_support: cohort.minimum_label_support.clone(),
                    required_dimension_values: cohort.required_dimension_values.clone(),
                    required_slice_support: cohort.required_slice_support.clone(),
                    required_source_classes: cohort.required_source_classes.clone(),
                    minimum_distinct_producers: cohort.minimum_distinct_producers,
                    freshness: cohort.freshness.clone(),
                })
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.cohort_key.cmp(&right.cohort_key));
    values
}

fn normalize_draft(draft: &mut BenchmarkBlueprintDraft) -> Result<(), BenchmarkArchitectError> {
    draft.summary = required(std::mem::take(&mut draft.summary), "proposal.summary")?;
    draft.tradeoffs = normalized_list(std::mem::take(&mut draft.tradeoffs), "tradeoffs")?;
    draft.evidence_gaps =
        normalized_list(std::mem::take(&mut draft.evidence_gaps), "evidence_gaps")?;
    for suite in &mut draft.suites {
        suite.name = required(std::mem::take(&mut suite.name), "suite.name")?;
        suite.purpose = required(std::mem::take(&mut suite.purpose), "suite.purpose")?;
        suite.required_model_formats = normalized_list(
            std::mem::take(&mut suite.required_model_formats),
            "suite.required_model_formats",
        )?;
        for cohort in &mut suite.cohorts {
            cohort.key = required(std::mem::take(&mut cohort.key), "cohort.key")?;
            cohort.name = required(std::mem::take(&mut cohort.name), "cohort.name")?;
            cohort.purpose = required(std::mem::take(&mut cohort.purpose), "cohort.purpose")?;
            cohort.rationale = required(std::mem::take(&mut cohort.rationale), "cohort.rationale")?;
            cohort.required_source_classes = normalized_list(
                std::mem::take(&mut cohort.required_source_classes),
                "cohort.required_source_classes",
            )?;
            for values in cohort.required_dimension_values.values_mut() {
                *values =
                    normalized_list(std::mem::take(values), "cohort.required_dimension_values")?;
            }
        }
    }
    for requirement in &mut draft.risk_coverage {
        requirement.key = required(std::mem::take(&mut requirement.key), "risk_coverage.key")?;
        requirement.deployment_risk_key = required(
            std::mem::take(&mut requirement.deployment_risk_key),
            "risk_coverage.deployment_risk_key",
        )?;
        requirement.cohort_keys = normalized_list(
            std::mem::take(&mut requirement.cohort_keys),
            "risk_coverage.cohort_keys",
        )?;
        requirement.rationale = required(
            std::mem::take(&mut requirement.rationale),
            "risk_coverage.rationale",
        )?;
        normalize_ids(&mut requirement.supporting_evidence_ids);
        normalize_ids(&mut requirement.conflicting_evidence_ids);
    }
    Ok(())
}

fn validate_evidence(
    run_id: Uuid,
    evidence: &[ResearchEvidence],
) -> Result<(), BenchmarkArchitectError> {
    validate_evidence_ids(evidence)?;
    if evidence.iter().any(|value| value.run_id != run_id) {
        return Err(BenchmarkArchitectError::Integrity(
            "proposal evidence belongs to another run".into(),
        ));
    }
    Ok(())
}

fn validate_evidence_ids(evidence: &[ResearchEvidence]) -> Result<(), BenchmarkArchitectError> {
    let mut ids = BTreeSet::new();
    for value in evidence {
        if !ids.insert(value.id)
            || value
                .reproduce_fingerprint()
                .map_err(|error| BenchmarkArchitectError::Integrity(error.to_string()))?
                != value.fingerprint
        {
            return Err(BenchmarkArchitectError::Integrity(
                "research evidence is duplicate or fingerprint-invalid".into(),
            ));
        }
    }
    Ok(())
}

fn normalized_list(
    values: Vec<String>,
    field: &str,
) -> Result<Vec<String>, BenchmarkArchitectError> {
    let mut set = BTreeSet::new();
    for value in values {
        let value = required(value, field)?;
        if !set.insert(value.clone()) {
            return Err(BenchmarkArchitectError::Validation(format!(
                "{field} contains duplicate {value:?}"
            )));
        }
    }
    Ok(set.into_iter().collect())
}

fn normalize_ids(values: &mut Vec<Uuid>) {
    values.sort();
    values.dedup();
}

fn block(
    issues: &mut Vec<BlueprintIssue>,
    code: &str,
    suite_kind: Option<BenchmarkSuiteKind>,
    cohort_key: Option<String>,
    message: impl Into<String>,
) {
    issues.push(BlueprintIssue {
        severity: BlueprintIssueSeverity::Blocking,
        code: code.into(),
        suite_kind,
        cohort_key,
        message: message.into(),
        observed: None,
        required: None,
    });
}

fn warn(
    issues: &mut Vec<BlueprintIssue>,
    code: &str,
    suite_kind: Option<BenchmarkSuiteKind>,
    cohort_key: Option<String>,
    message: impl Into<String>,
) {
    issues.push(BlueprintIssue {
        severity: BlueprintIssueSeverity::Warning,
        code: code.into(),
        suite_kind,
        cohort_key,
        message: message.into(),
        observed: None,
        required: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn issue_numeric(
    issues: &mut Vec<BlueprintIssue>,
    code: &str,
    suite_kind: Option<BenchmarkSuiteKind>,
    cohort_key: Option<String>,
    message: impl Into<String>,
    observed: u64,
    required: u64,
) {
    issues.push(BlueprintIssue {
        severity: BlueprintIssueSeverity::Blocking,
        code: code.into(),
        suite_kind,
        cohort_key,
        message: message.into(),
        observed: Some(observed as f64),
        required: Some(required as f64),
    });
}

fn issue_order(left: &BlueprintIssue, right: &BlueprintIssue) -> std::cmp::Ordering {
    (
        left.severity,
        left.code.as_str(),
        left.suite_kind,
        left.cohort_key.as_deref(),
    )
        .cmp(&(
            right.severity,
            right.code.as_str(),
            right.suite_kind,
            right.cohort_key.as_deref(),
        ))
}

#[cfg(test)]
mod tests {
    use research_core::{
        brief::SourcePolicy,
        evidence::{EvidenceConfidence, ResearchEvidence, ResearchEvidenceDraft},
    };
    use workflow_core::{
        benchmark::{BenchmarkMetric, MetricRequirement},
        benchmark_qualification::QualificationConfidence,
    };

    use crate::{brief::tests::brief, lifecycle::BenchmarkArchitectRun};

    use super::*;

    fn evidence(run_id: Uuid) -> ResearchEvidence {
        ResearchEvidence::create(
            run_id,
            Uuid::new_v4(),
            &SourcePolicy::default(),
            ResearchEvidenceDraft {
                url: "https://example.com/fraud-support-study".into(),
                title: "Fraud support study".into(),
                query: "support fraud billing confusion".into(),
                source_class: "industry_report".into(),
                content_hash: "sha256:page".into(),
                excerpt: "Users often describe fraud as an unexpected charge.".into(),
                location: None,
                observation: "Fraud and billing language overlap.".into(),
                applicability: "Boundary cohort design.".into(),
                confidence: EvidenceConfidence::High,
            },
        )
        .unwrap()
    }

    fn cohort(kind: BenchmarkSuiteKind, key: &str, role: CohortRole) -> BenchmarkCohortBlueprint {
        BenchmarkCohortBlueprint {
            key: key.into(),
            name: key.into(),
            purpose: "Measure production-like boundaries".into(),
            origin: CohortOrigin::InternalSnapshot,
            role,
            disclosure: DisclosureLevel::Aggregate,
            adaptation_eligible: kind == BenchmarkSuiteKind::Development,
            protocol: EvaluationProtocol {
                minimum_slice_support: 20,
                ..EvaluationProtocol::default()
            },
            minimum_total_support: 400,
            minimum_label_support: BTreeMap::from([("billing".into(), 200), ("fraud".into(), 200)]),
            required_dimension_values: BTreeMap::from([("channel".into(), vec!["chat".into()])]),
            required_slice_support: BTreeMap::new(),
            required_source_classes: vec!["reviewed_real_sample".into()],
            minimum_distinct_producers: 2,
            freshness: CohortFreshnessPolicy {
                maximum_age_days: 180,
                maximum_workflow_iterations: 3,
                maximum_adaptive_exposures: if kind == BenchmarkSuiteKind::Development {
                    10
                } else {
                    0
                },
                maximum_acceptance_exposures: if kind == BenchmarkSuiteKind::SealedAcceptance {
                    1
                } else {
                    0
                },
            },
            rationale: "Measure the highest-consequence class boundary.".into(),
        }
    }

    fn contract(kind: BenchmarkSuiteKind) -> AcceptanceContract {
        AcceptanceContract {
            metric_requirements: vec![MetricRequirement {
                target: MetricTarget::Overall,
                metric: BenchmarkMetric::MacroF1,
                minimum: Some(0.85),
                maximum: None,
                minimum_support: 100,
            }],
            regression: (kind == BenchmarkSuiteKind::Development).then_some(
                workflow_core::benchmark::RegressionRequirement {
                    max_accuracy_drop: Some(0.02),
                    max_macro_f1_drop: None,
                    minimum_accuracy_delta_lower_bound: None,
                    minimum_macro_f1_delta_lower_bound: None,
                    require_mcnemar_significance: false,
                },
            ),
        }
    }

    fn draft(evidence_id: Uuid) -> BenchmarkBlueprintDraft {
        BenchmarkBlueprintDraft {
            summary: "Use development and single-use sealed evidence.".into(),
            suites: vec![
                BenchmarkSuiteBlueprint {
                    kind: BenchmarkSuiteKind::Development,
                    name: "development".into(),
                    purpose: "Guide bounded iteration".into(),
                    required_model_formats: vec![],
                    cohorts: vec![cohort(
                        BenchmarkSuiteKind::Development,
                        "dev-boundary",
                        CohortRole::Development,
                    )],
                    contract: contract(BenchmarkSuiteKind::Development),
                },
                BenchmarkSuiteBlueprint {
                    kind: BenchmarkSuiteKind::SealedAcceptance,
                    name: "sealed".into(),
                    purpose: "One final acceptance decision".into(),
                    required_model_formats: vec![],
                    cohorts: vec![cohort(
                        BenchmarkSuiteKind::SealedAcceptance,
                        "sealed-boundary",
                        CohortRole::SealedAcceptance,
                    )],
                    contract: contract(BenchmarkSuiteKind::SealedAcceptance),
                },
            ],
            qualification_policy: BenchmarkQualificationPolicy {
                minimum_overall_support: 100,
                minimum_label_support: 20,
                confidence: QualificationConfidence::NinetyFive,
                maximum_proportion_margin_of_error: 0.10,
                maximum_normalized_duplicate_rate: 0.01,
                minimum_distinct_producers: 2,
                maximum_single_producer_share: 0.8,
                maximum_label_imbalance_ratio: 2.0,
            },
            risk_coverage: vec![RiskCoverageRequirement {
                key: "fraud-boundary".into(),
                deployment_risk_key: "fraud_as_billing".into(),
                cohort_keys: vec!["dev-boundary".into(), "sealed-boundary".into()],
                rationale: "Both adaptive and final evidence must cover the boundary.".into(),
                supporting_evidence_ids: vec![evidence_id],
                conflicting_evidence_ids: vec![],
                inference: false,
            }],
            tradeoffs: vec!["More boundary cases reduce easy-example share.".into()],
            evidence_gaps: vec!["True channel prevalence is not yet measured.".into()],
        }
    }

    #[test]
    fn valid_blueprint_compiles_to_reviewable_sourcing_requirements() {
        let brief = brief();
        let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let evidence = evidence(run.id);
        let proposal = BenchmarkArchitectureProposal::create(
            &brief,
            &run,
            std::slice::from_ref(&evidence),
            draft(evidence.id),
        )
        .unwrap();
        assert!(proposal.validation.valid);
        assert_eq!(proposal.acquisition_requirements.len(), 2);
        assert!(
            proposal
                .validation
                .issues
                .iter()
                .any(|issue| issue.code == "reference_distribution_unverified")
        );

        let approval = BenchmarkArchitectureReview::create(
            &proposal,
            None,
            BenchmarkArchitectureReviewDecision::Approve,
            "operator".into(),
            "The evidence and decision contract are appropriate.".into(),
        )
        .unwrap();
        let handoff =
            BenchmarkAcquisitionHandoff::create(&brief, &proposal, &approval, &[evidence]).unwrap();
        assert_eq!(handoff.requirements.len(), 2);
        assert!(handoff.authority_notice.contains("does not authorize"));
    }

    #[test]
    fn underpowered_and_adaptive_sealed_blueprints_are_blocked() {
        let brief = brief();
        let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let evidence = evidence(run.id);
        let mut value = draft(evidence.id);
        let sealed = &mut value.suites[1].cohorts[0];
        sealed.minimum_total_support = 1;
        sealed.adaptation_eligible = true;
        sealed.freshness.maximum_adaptive_exposures = 1;
        let report = validate_blueprint(&brief, &[evidence], &value).unwrap();
        assert!(!report.valid);
        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("unsafe_sealed_disclosure"));
        assert!(codes.contains("underpowered_total_support"));
        assert!(codes.contains("invalid_sealed_freshness"));
    }

    #[test]
    fn sealed_contract_cannot_request_adaptive_slice_metrics() {
        let brief = brief();
        let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let evidence = evidence(run.id);
        let mut value = draft(evidence.id);
        value.suites[1].contract.metric_requirements[0].target = MetricTarget::Label {
            label: "fraud".into(),
        };
        let report = validate_blueprint(&brief, &[evidence], &value).unwrap();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "invalid_acceptance_contract")
        );
    }

    #[test]
    fn acquisition_conformance_catches_exposure_staleness_without_row_access() {
        let brief = brief();
        let mut run = BenchmarkArchitectRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
        run.start().unwrap();
        let evidence = evidence(run.id);
        let proposal = BenchmarkArchitectureProposal::create(
            &brief,
            &run,
            std::slice::from_ref(&evidence),
            draft(evidence.id),
        )
        .unwrap();
        let approval = BenchmarkArchitectureReview::create(
            &proposal,
            None,
            BenchmarkArchitectureReviewDecision::Approve,
            "operator".into(),
            "approved".into(),
        )
        .unwrap();
        let handoff = BenchmarkAcquisitionHandoff::create(
            &brief,
            &proposal,
            &approval,
            std::slice::from_ref(&evidence),
        )
        .unwrap();
        let now = Utc::now();
        let candidates = handoff
            .requirements
            .iter()
            .map(|requirement| CandidateCohortFacts {
                cohort_key: requirement.cohort_key.clone(),
                suite_kind: requirement.suite_kind,
                origin: requirement.origin,
                role: requirement.role,
                disclosure: requirement.disclosure,
                adaptation_eligible: requirement.adaptation_eligible,
                protocol: requirement.protocol.clone(),
                total_support: requirement.minimum_total_support,
                label_support: requirement.minimum_label_support.clone(),
                dimension_value_support: requirement
                    .required_dimension_values
                    .iter()
                    .map(|(dimension, values)| {
                        (
                            dimension.clone(),
                            values.iter().map(|value| (value.clone(), 1)).collect(),
                        )
                    })
                    .collect(),
                slice_support: requirement.required_slice_support.clone(),
                source_classes: requirement.required_source_classes.clone(),
                distinct_producers: requirement.minimum_distinct_producers,
                acquired_at: now,
                workflow_iterations: 0,
                adaptive_exposures: 0,
                acceptance_exposures: if requirement.suite_kind
                    == BenchmarkSuiteKind::SealedAcceptance
                {
                    2
                } else {
                    0
                },
                retired: false,
            })
            .collect::<Vec<_>>();
        let assessment = assess_acquisition_conformance(&handoff, &candidates, now).unwrap();
        assert_eq!(assessment.state, AcquisitionConformanceState::Blocked);
        assert!(
            assessment
                .issues
                .iter()
                .any(|issue| issue.code == "candidate_stale_or_exposed")
        );
    }
}
