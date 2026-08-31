//! Immutable row evidence and deterministic aggregate quality windows.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_quality_core::{
    assessment::{
        GeneratorEvaluatorRelationship, QualityIssueCode, QualityVerdict, RowQualityAssessment,
    },
    policy::BasisPoints,
};
use generation_core::domain::{GeneratedRow, ValidationStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    SupervisorError, contract::GenerationQualityContract, fingerprint, strategy::StrategyAssignment,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralOutcome {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractRowVerdict {
    Qualified,
    Borderline,
    Quarantined,
    Unassessed,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowCriterionFailure {
    AssignedLabel,
    LabelMargin,
    DimensionAdherence,
    DifficultyAdherence,
    AuthenticityAdherence,
    StrategyAdherence,
    LabelLeakage,
    ShortcutRisk,
    EvaluatorConfidence,
    ProviderQuarantine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentEvidence {
    pub assessment_id: Uuid,
    pub assessment_fingerprint: String,
    pub evaluator_fingerprint: String,
    pub generator_relationship: GeneratorEvaluatorRelationship,
    pub provider_verdict: QualityVerdict,
    pub assigned_label_score: BasisPoints,
    pub assigned_label_margin: i32,
    pub assigned_dimension_scores: BTreeMap<String, BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty_score: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity_score: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_score: Option<BasisPoints>,
    pub label_leakage_risk: BasisPoints,
    pub shortcut_risk: BasisPoints,
    pub confidence: BasisPoints,
    pub issue_codes: Vec<QualityIssueCode>,
}

impl AssessmentEvidence {
    pub fn from_assessment(
        assessment: &RowQualityAssessment,
        difficulty_dimension: Option<&str>,
        strategy_score: Option<BasisPoints>,
    ) -> Result<Self, SupervisorError> {
        if assessment.reproduce_fingerprint().map_err(|error| {
            SupervisorError::Integrity(format!("assessment fingerprint failed: {error}"))
        })? != assessment.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "row quality assessment fingerprint does not reproduce".into(),
            ));
        }
        let difficulty_score = difficulty_dimension
            .map(|name| {
                assessment
                    .assigned_dimension_scores
                    .get(name)
                    .copied()
                    .ok_or_else(|| {
                        SupervisorError::Validation(format!(
                            "assessment has no assigned score for difficulty dimension {name}"
                        ))
                    })
            })
            .transpose()?;
        Ok(Self {
            assessment_id: assessment.id,
            assessment_fingerprint: assessment.fingerprint.clone(),
            evaluator_fingerprint: assessment.evaluator.fingerprint.clone(),
            generator_relationship: assessment.generator_relationship,
            provider_verdict: assessment.verdict,
            assigned_label_score: assessment.assigned_label_score,
            assigned_label_margin: assessment.assigned_label_margin,
            assigned_dimension_scores: assessment.assigned_dimension_scores.clone(),
            difficulty_score,
            authenticity_score: assessment.authenticity_score,
            strategy_score,
            label_leakage_risk: assessment.label_leakage_risk,
            shortcut_risk: assessment.shortcut_risk,
            confidence: assessment.confidence,
            issue_codes: assessment.issue_codes.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowQualityObservation {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_fingerprint: String,
    pub supervisor_run_id: Uuid,
    pub segment_id: Uuid,
    pub generation_job_id: Uuid,
    pub generation_attempt_id: Uuid,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    pub generated_row_id: Uuid,
    pub generated_row_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_row_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_row_fingerprint: Option<String>,
    pub cell_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_directive_id: Option<Uuid>,
    pub structural_outcome: StructuralOutcome,
    pub normalized_text: String,
    pub template_signature: String,
    pub exact_duplicate: bool,
    pub normalized_duplicate: bool,
    #[serde(default)]
    pub observed_patterns: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<AssessmentEvidence>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RowQualityObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        contract: &GenerationQualityContract,
        supervisor_run_id: Uuid,
        segment_id: Uuid,
        generation_attempt_id: Uuid,
        prompt_version_id: Uuid,
        prompt_version_fingerprint: impl Into<String>,
        row: &GeneratedRow,
        source_row: Option<(Uuid, String)>,
        assignment: Option<&StrategyAssignment>,
        exact_duplicate: bool,
        normalized_duplicate: bool,
        observed_patterns: BTreeSet<String>,
        assessment: Option<AssessmentEvidence>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        contract.validate()?;
        let structural_outcome = match row.validation_status {
            ValidationStatus::Accepted => StructuralOutcome::Accepted,
            ValidationStatus::Rejected => StructuralOutcome::Rejected,
        };
        if structural_outcome == StructuralOutcome::Rejected && assessment.is_some() {
            return Err(SupervisorError::Validation(
                "structurally rejected rows cannot carry qualification evidence".into(),
            ));
        }
        if let Some(assessment) = &assessment {
            if assessment.evaluator_fingerprint != contract.evaluator.fingerprint
                || assessment.generator_relationship != contract.generator_evaluator_relationship
            {
                return Err(SupervisorError::Integrity(
                    "row assessment does not use the contract evaluator relationship".into(),
                ));
            }
        }
        if let Some(assignment) = assignment {
            assignment.validate()?;
            if assignment.plan_id != contract.plan.id
                || assignment.plan_fingerprint != contract.plan.fingerprint
                || assignment.cell_key != row.cell_key
                || contract.strategy_context.as_ref().is_none_or(|binding| {
                    binding.id != assignment.strategy_context_id
                        || binding.fingerprint != assignment.strategy_context_fingerprint
                })
            {
                return Err(SupervisorError::Integrity(
                    "row strategy assignment is outside the contract or generated cell".into(),
                ));
            }
        }
        if source_row.as_ref().is_some_and(|(id, _)| id.is_nil()) {
            return Err(SupervisorError::Validation(
                "source row identity must not be nil".into(),
            ));
        }
        let prompt_version_fingerprint = prompt_version_fingerprint.into();
        if [
            id,
            supervisor_run_id,
            segment_id,
            generation_attempt_id,
            prompt_version_id,
            row.id,
            row.generation_job_id,
        ]
        .contains(&Uuid::nil())
            || row.dataset_id != contract.dataset.id
            || prompt_version_fingerprint.trim().is_empty()
            || row.cell_key.trim().is_empty()
        {
            return Err(SupervisorError::Validation(
                "row observation identities and bindings must be present".into(),
            ));
        }
        let (source_row_id, source_row_fingerprint) = source_row.unzip();
        if source_row_id.is_some() != source_row_fingerprint.is_some()
            || source_row_fingerprint
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(SupervisorError::Validation(
                "source row identity and fingerprint must be supplied together".into(),
            ));
        }
        let mut value = Self {
            id,
            contract_id: contract.id,
            contract_fingerprint: contract.fingerprint.clone(),
            supervisor_run_id,
            segment_id,
            generation_job_id: row.generation_job_id,
            generation_attempt_id,
            prompt_version_id,
            prompt_version_fingerprint,
            generated_row_id: row.id,
            generated_row_fingerprint: fingerprint(row)?,
            source_row_id,
            source_row_fingerprint,
            cell_key: row.cell_key.clone(),
            strategy_assignment_fingerprint: assignment.map(|value| value.fingerprint.clone()),
            strategy_directive_id: assignment.and_then(|value| value.directive_id),
            structural_outcome,
            normalized_text: row.normalized_text.clone(),
            template_signature: template_signature(&row.normalized_text),
            exact_duplicate,
            normalized_duplicate,
            observed_patterns: observed_patterns
                .into_iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect(),
            assessment,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate(contract)?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(&self, contract: &GenerationQualityContract) -> Result<(), SupervisorError> {
        if self.contract_id != contract.id
            || self.contract_fingerprint != contract.fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "row observation fingerprint or contract binding is invalid".into(),
            ));
        }
        if self.structural_outcome == StructuralOutcome::Rejected && self.assessment.is_some() {
            return Err(SupervisorError::Integrity(
                "rejected row contains impossible assessment evidence".into(),
            ));
        }
        Ok(())
    }

    pub fn contract_verdict(&self, contract: &GenerationQualityContract) -> ContractRowVerdict {
        if self.structural_outcome == StructuralOutcome::Rejected {
            return ContractRowVerdict::Invalid;
        }
        let Some(evidence) = &self.assessment else {
            return ContractRowVerdict::Unassessed;
        };
        if !self.criterion_failures(contract).is_empty() {
            ContractRowVerdict::Quarantined
        } else if evidence.provider_verdict == QualityVerdict::Borderline {
            ContractRowVerdict::Borderline
        } else {
            ContractRowVerdict::Qualified
        }
    }

    pub fn criterion_failures(
        &self,
        contract: &GenerationQualityContract,
    ) -> BTreeSet<RowCriterionFailure> {
        let Some(evidence) = &self.assessment else {
            return BTreeSet::new();
        };
        let thresholds = &contract.row_thresholds;
        let mut failures = BTreeSet::new();
        if evidence.assigned_label_score < thresholds.minimum_assigned_label_score {
            failures.insert(RowCriterionFailure::AssignedLabel);
        }
        if evidence.assigned_label_margin < 0
            || u16::try_from(evidence.assigned_label_margin)
                .map_or(true, |value| value < thresholds.minimum_label_margin.get())
        {
            failures.insert(RowCriterionFailure::LabelMargin);
        }
        if evidence
            .assigned_dimension_scores
            .values()
            .any(|score| *score < thresholds.minimum_dimension_score)
        {
            failures.insert(RowCriterionFailure::DimensionAdherence);
        }
        let optional_fails = |value: Option<BasisPoints>, minimum: Option<BasisPoints>| {
            minimum.is_some_and(|minimum| value.is_none_or(|value| value < minimum))
        };
        if optional_fails(
            evidence.difficulty_score,
            thresholds.minimum_difficulty_score,
        ) {
            failures.insert(RowCriterionFailure::DifficultyAdherence);
        }
        if optional_fails(
            evidence.authenticity_score,
            thresholds.minimum_authenticity_score,
        ) {
            failures.insert(RowCriterionFailure::AuthenticityAdherence);
        }
        if self.strategy_directive_id.is_some()
            && optional_fails(evidence.strategy_score, thresholds.minimum_strategy_score)
        {
            failures.insert(RowCriterionFailure::StrategyAdherence);
        }
        if evidence.label_leakage_risk > thresholds.maximum_label_leakage_risk {
            failures.insert(RowCriterionFailure::LabelLeakage);
        }
        if evidence.shortcut_risk > thresholds.maximum_shortcut_risk {
            failures.insert(RowCriterionFailure::ShortcutRisk);
        }
        if evidence.confidence < thresholds.minimum_evaluator_confidence {
            failures.insert(RowCriterionFailure::EvaluatorConfidence);
        }
        if evidence.provider_verdict == QualityVerdict::Quarantined {
            failures.insert(RowCriterionFailure::ProviderQuarantine);
        }
        failures
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QualityScope {
    Cell {
        cell_key: String,
    },
    CellStrategy {
        cell_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directive_id: Option<Uuid>,
    },
}

impl QualityScope {
    pub fn cell_key(&self) -> &str {
        match self {
            Self::Cell { cell_key } | Self::CellStrategy { cell_key, .. } => cell_key,
        }
    }

    fn contains(&self, row: &RowQualityObservation) -> bool {
        match self {
            Self::Cell { cell_key } => row.cell_key == *cell_key,
            Self::CellStrategy {
                cell_key,
                directive_id,
            } => row.cell_key == *cell_key && row.strategy_directive_id == *directive_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityWindowKind {
    InitialCanary,
    Rolling,
    RevisionCanary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowEvidenceReference {
    pub observation_id: Uuid,
    pub observation_fingerprint: String,
    pub generated_row_id: Uuid,
    pub generated_row_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityEvidenceManifest {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub supervisor_run_id: Uuid,
    pub window_id: Uuid,
    pub members: Vec<RowEvidenceReference>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl QualityEvidenceManifest {
    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCounts {
    pub attempted: u32,
    pub structurally_accepted: u32,
    pub assessed: u32,
    pub qualified: u32,
    pub borderline: u32,
    pub quarantined: u32,
    pub unassessed: u32,
    pub invalid: u32,
    pub exact_duplicates: u32,
    pub normalized_duplicates: u32,
    pub template_repetitions: u32,
    pub shortcut_rows: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRates {
    pub qualified: BasisPoints,
    pub borderline: BasisPoints,
    pub quarantined: BasisPoints,
    pub unassessed: BasisPoints,
    pub invalid: BasisPoints,
    pub exact_duplicates: BasisPoints,
    pub normalized_duplicates: BasisPoints,
    pub template_repetitions: BasisPoints,
    pub shortcut_concentration: BasisPoints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreMeans {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_label: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_leakage_risk: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_risk: Option<BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<BasisPoints>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextLengthSummary {
    pub minimum_characters: u32,
    pub maximum_characters: u32,
    pub mean_characters: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchQualityObservation {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_fingerprint: String,
    pub supervisor_run_id: Uuid,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    pub kind: QualityWindowKind,
    pub sequence: u32,
    pub scope: QualityScope,
    pub manifest_id: Uuid,
    pub manifest_fingerprint: String,
    pub counts: BatchCounts,
    pub rates: BatchRates,
    pub score_means: ScoreMeans,
    pub criterion_failures: BTreeMap<RowCriterionFailure, u32>,
    pub issue_code_counts: BTreeMap<QualityIssueCode, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_lengths: Option<TextLengthSummary>,
    pub observed_patterns: BTreeSet<String>,
    pub missing_required_patterns: BTreeSet<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedQualityWindow {
    pub manifest: QualityEvidenceManifest,
    pub observation: BatchQualityObservation,
}

impl BatchQualityObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate(
        id: Uuid,
        manifest_id: Uuid,
        contract: &GenerationQualityContract,
        supervisor_run_id: Uuid,
        prompt_version_id: Uuid,
        prompt_version_fingerprint: impl Into<String>,
        kind: QualityWindowKind,
        sequence: u32,
        scope: QualityScope,
        rows: &[RowQualityObservation],
        created_at: DateTime<Utc>,
    ) -> Result<AggregatedQualityWindow, SupervisorError> {
        if id.is_nil()
            || manifest_id.is_nil()
            || supervisor_run_id.is_nil()
            || prompt_version_id.is_nil()
            || rows.is_empty()
        {
            return Err(SupervisorError::Validation(
                "quality window identities and row evidence must be present".into(),
            ));
        }
        contract.validate()?;
        for row in rows {
            row.validate(contract)?;
            if row.supervisor_run_id != supervisor_run_id
                || row.prompt_version_id != prompt_version_id
                || !scope.contains(row)
            {
                return Err(SupervisorError::Integrity(
                    "quality window row lies outside its run, prompt, or scope".into(),
                ));
            }
        }
        let prompt_version_fingerprint = prompt_version_fingerprint.into();
        if rows
            .iter()
            .any(|row| row.prompt_version_fingerprint != prompt_version_fingerprint)
        {
            return Err(SupervisorError::Integrity(
                "quality window mixes prompt guidance versions".into(),
            ));
        }

        let members = rows
            .iter()
            .map(|row| RowEvidenceReference {
                observation_id: row.id,
                observation_fingerprint: row.fingerprint.clone(),
                generated_row_id: row.generated_row_id,
                generated_row_fingerprint: row.generated_row_fingerprint.clone(),
                assessment_id: row.assessment.as_ref().map(|value| value.assessment_id),
                assessment_fingerprint: row
                    .assessment
                    .as_ref()
                    .map(|value| value.assessment_fingerprint.clone()),
            })
            .collect::<Vec<_>>();
        let mut manifest = QualityEvidenceManifest {
            id: manifest_id,
            contract_id: contract.id,
            supervisor_run_id,
            window_id: id,
            members,
            created_at,
            fingerprint: String::new(),
        };
        manifest.fingerprint = manifest.reproduce_fingerprint()?;

        let mut counts = BatchCounts {
            attempted: u32::try_from(rows.len())
                .map_err(|_| SupervisorError::Validation("quality window is too large".into()))?,
            structurally_accepted: 0,
            assessed: 0,
            qualified: 0,
            borderline: 0,
            quarantined: 0,
            unassessed: 0,
            invalid: 0,
            exact_duplicates: 0,
            normalized_duplicates: 0,
            template_repetitions: 0,
            shortcut_rows: 0,
        };
        let mut normalized_seen = BTreeSet::new();
        let mut templates_seen = BTreeSet::new();
        let mut observed_patterns = BTreeSet::new();
        let mut text_lengths = Vec::new();
        let mut label_scores = Vec::new();
        let mut difficulty_scores = Vec::new();
        let mut authenticity_scores = Vec::new();
        let mut strategy_scores = Vec::new();
        let mut leakage_scores = Vec::new();
        let mut shortcut_scores = Vec::new();
        let mut confidence_scores = Vec::new();
        let mut criterion_failures = BTreeMap::<RowCriterionFailure, u32>::new();
        let mut issue_code_counts = BTreeMap::<QualityIssueCode, u32>::new();

        for row in rows {
            if row.structural_outcome == StructuralOutcome::Accepted {
                counts.structurally_accepted += 1;
                text_lengths
                    .push(u32::try_from(row.normalized_text.chars().count()).unwrap_or(u32::MAX));
            }
            if row.exact_duplicate {
                counts.exact_duplicates += 1;
            }
            if row.normalized_duplicate
                || (!row.normalized_text.is_empty()
                    && !normalized_seen.insert(row.normalized_text.clone()))
            {
                counts.normalized_duplicates += 1;
            }
            if !row.template_signature.is_empty()
                && !templates_seen.insert(row.template_signature.clone())
            {
                counts.template_repetitions += 1;
            }
            observed_patterns.extend(row.observed_patterns.iter().cloned());
            match row.contract_verdict(contract) {
                ContractRowVerdict::Qualified => counts.qualified += 1,
                ContractRowVerdict::Borderline => counts.borderline += 1,
                ContractRowVerdict::Quarantined => counts.quarantined += 1,
                ContractRowVerdict::Unassessed => counts.unassessed += 1,
                ContractRowVerdict::Invalid => counts.invalid += 1,
            }
            for failure in row.criterion_failures(contract) {
                *criterion_failures.entry(failure).or_default() += 1;
            }
            if let Some(evidence) = &row.assessment {
                counts.assessed += 1;
                label_scores.push(evidence.assigned_label_score);
                difficulty_scores.extend(evidence.difficulty_score);
                authenticity_scores.extend(evidence.authenticity_score);
                strategy_scores.extend(evidence.strategy_score);
                leakage_scores.push(evidence.label_leakage_risk);
                shortcut_scores.push(evidence.shortcut_risk);
                confidence_scores.push(evidence.confidence);
                if evidence
                    .issue_codes
                    .contains(&QualityIssueCode::ShortcutArtifact)
                {
                    counts.shortcut_rows += 1;
                }
                for issue in &evidence.issue_codes {
                    *issue_code_counts.entry(*issue).or_default() += 1;
                }
            }
        }
        let denominator = counts.attempted;
        let rates = BatchRates {
            qualified: rate(counts.qualified, denominator)?,
            borderline: rate(counts.borderline, denominator)?,
            quarantined: rate(counts.quarantined, denominator)?,
            unassessed: rate(counts.unassessed, denominator)?,
            invalid: rate(counts.invalid, denominator)?,
            exact_duplicates: rate(counts.exact_duplicates, denominator)?,
            normalized_duplicates: rate(counts.normalized_duplicates, denominator)?,
            template_repetitions: rate(counts.template_repetitions, denominator)?,
            shortcut_concentration: rate(counts.shortcut_rows, denominator)?,
        };
        let score_means = ScoreMeans {
            assigned_label: mean(&label_scores)?,
            difficulty: mean(&difficulty_scores)?,
            authenticity: mean(&authenticity_scores)?,
            strategy: mean(&strategy_scores)?,
            label_leakage_risk: mean(&leakage_scores)?,
            shortcut_risk: mean(&shortcut_scores)?,
            confidence: mean(&confidence_scores)?,
        };
        let text_lengths = if text_lengths.is_empty() {
            None
        } else {
            Some(TextLengthSummary {
                minimum_characters: *text_lengths.iter().min().expect("non-empty"),
                maximum_characters: *text_lengths.iter().max().expect("non-empty"),
                mean_characters: u32::try_from(
                    text_lengths
                        .iter()
                        .map(|value| u64::from(*value))
                        .sum::<u64>()
                        / u64::try_from(text_lengths.len()).expect("length fits u64"),
                )
                .unwrap_or(u32::MAX),
            })
        };
        let missing_required_patterns = contract
            .batch_thresholds
            .required_patterns
            .difference(&observed_patterns)
            .cloned()
            .collect();
        let mut observation = Self {
            id,
            contract_id: contract.id,
            contract_fingerprint: contract.fingerprint.clone(),
            supervisor_run_id,
            prompt_version_id,
            prompt_version_fingerprint,
            kind,
            sequence,
            scope,
            manifest_id,
            manifest_fingerprint: manifest.fingerprint.clone(),
            counts,
            rates,
            score_means,
            criterion_failures,
            issue_code_counts,
            text_lengths,
            observed_patterns,
            missing_required_patterns,
            created_at,
            fingerprint: String::new(),
        };
        observation.fingerprint = observation.reproduce_fingerprint()?;
        observation.validate(contract, &manifest)?;
        Ok(AggregatedQualityWindow {
            manifest,
            observation,
        })
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn validate(
        &self,
        contract: &GenerationQualityContract,
        manifest: &QualityEvidenceManifest,
    ) -> Result<(), SupervisorError> {
        if self.contract_id != contract.id
            || self.contract_fingerprint != contract.fingerprint
            || manifest.contract_id != contract.id
            || manifest.supervisor_run_id != self.supervisor_run_id
            || manifest.window_id != self.id
            || manifest.id != self.manifest_id
            || manifest.reproduce_fingerprint()? != manifest.fingerprint
            || manifest.fingerprint != self.manifest_fingerprint
            || manifest.members.len()
                != usize::try_from(self.counts.attempted).unwrap_or(usize::MAX)
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "quality window or protected manifest does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

fn rate(numerator: u32, denominator: u32) -> Result<BasisPoints, SupervisorError> {
    if denominator == 0 {
        return Ok(BasisPoints::ZERO);
    }
    let value = u16::try_from(u64::from(numerator) * 10_000 / u64::from(denominator))
        .map_err(|_| SupervisorError::Integrity("batch rate exceeds 100 percent".into()))?;
    BasisPoints::new(value).map_err(|error| SupervisorError::Integrity(error.to_string()))
}

fn mean(values: &[BasisPoints]) -> Result<Option<BasisPoints>, SupervisorError> {
    if values.is_empty() {
        return Ok(None);
    }
    let total = values
        .iter()
        .map(|value| u64::from(value.get()))
        .sum::<u64>();
    let mean = u16::try_from(total / u64::try_from(values.len()).expect("length fits u64"))
        .map_err(|_| SupervisorError::Integrity("mean score exceeds basis points".into()))?;
    BasisPoints::new(mean)
        .map(Some)
        .map_err(|error| SupervisorError::Integrity(error.to_string()))
}

pub fn template_signature(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_digits = false;
    for character in text.trim().to_lowercase().chars() {
        if character.is_ascii_digit() {
            if !in_digits {
                output.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            if character.is_whitespace() {
                if !output.ends_with(' ') {
                    output.push(' ');
                }
            } else {
                output.push(character);
            }
        }
    }
    output.trim().to_owned()
}
