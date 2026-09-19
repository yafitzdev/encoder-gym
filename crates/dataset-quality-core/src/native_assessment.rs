//! Blind native semantic admission for generated training rows.
//!
//! This contract is deliberately separate from classification `SourceRow`
//! audits. A reviewer sees the generated question, bounded native context and
//! legal candidate semantics, but never the inherited label. The host records
//! that blind answer before a second target-fit pass and derives the admission
//! verdict from private label authority.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{QualityError, fingerprint};

pub const NATIVE_ASSESSMENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_NATIVE_ASSESSMENT_BATCH: usize = 8;
pub const MAX_NATIVE_CANDIDATES: usize = 128;

/// Provider usage may be unavailable on custom endpoints. Missing values are
/// explicit so the optimization store can retain the reserved ceiling instead
/// of interpreting an SDK placeholder zero as free work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeReviewUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_microusd: Option<u64>,
}

impl NativeReviewUsage {
    pub fn exceeds(&self, input: u64, output: u64, cost: u64) -> bool {
        self.input_tokens.is_some_and(|value| value > input)
            || self.output_tokens.is_some_and(|value| value > output)
            || self.cost_microusd.is_some_and(|value| value > cost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeCandidateSemantics {
    pub candidate_id: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub fingerprint: String,
}

impl NativeCandidateSemantics {
    pub fn new(
        candidate_id: impl Into<String>,
        description: impl Into<String>,
        mut capabilities: Vec<String>,
    ) -> Result<Self, QualityError> {
        capabilities.sort();
        capabilities.dedup();
        let mut value = Self {
            candidate_id: candidate_id.into(),
            description: description.into(),
            capabilities,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if !valid_id(&self.candidate_id)
            || !valid_text(&self.description, 2_000)
            || self.capabilities.is_empty()
            || self.capabilities.len() > 32
            || !strict_strings(&self.capabilities, 256)
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "native candidate semantics are invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

/// Bounded task-owned facts needed to interpret the question. Explicit label
/// and desired-answer field names are prohibited as a defense in depth; native
/// adapters still own the semantic projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBlindContext {
    pub facts: BTreeMap<String, Value>,
    pub fingerprint: String,
}

impl NativeBlindContext {
    pub fn new(facts: BTreeMap<String, Value>) -> Result<Self, QualityError> {
        let mut value = Self {
            facts,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.facts.len() > 64
            || self.facts.keys().any(|key| {
                !valid_id(key)
                    || matches!(
                        normalized_key(key).as_str(),
                        "label"
                            | "labels"
                            | "expectedanswer"
                            | "desiredanswer"
                            | "acceptabletools"
                            | "hardnegativetools"
                    )
            })
            || serde_json::to_vec(&self.facts)
                .map_err(|error| QualityError::Validation(error.to_string()))?
                .len()
                > 32_768
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "blind native context is invalid or contains label authority".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&self.facts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBlindRow {
    pub row_id: String,
    pub row_fingerprint: String,
    pub question: String,
    pub context: NativeBlindContext,
    pub candidates: Vec<NativeCandidateSemantics>,
}

impl NativeBlindRow {
    pub fn validate(&self) -> Result<(), QualityError> {
        if !valid_id(&self.row_id)
            || !canonical_fingerprint(&self.row_fingerprint)
            || !valid_text(&self.question, 8_192)
            || self.candidates.is_empty()
            || self.candidates.len() > MAX_NATIVE_CANDIDATES
        {
            return Err(QualityError::Validation(
                "blind native row identity, question, or candidate count is invalid".into(),
            ));
        }
        self.context.validate()?;
        let mut previous = None;
        for candidate in &self.candidates {
            candidate.validate()?;
            if previous.is_some_and(|value: &str| value >= candidate.candidate_id.as_str()) {
                return Err(QualityError::Validation(
                    "native candidates must be unique and sorted by candidate ID".into(),
                ));
            }
            previous = Some(&candidate.candidate_id);
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, QualityError> {
        self.validate()?;
        fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBlindAssessmentRequest {
    pub schema_version: u32,
    pub id: Uuid,
    pub run_id: Uuid,
    pub iteration: u32,
    pub evaluator_identity_fingerprint: String,
    pub rows: Vec<NativeBlindRow>,
    pub fingerprint: String,
}

impl NativeBlindAssessmentRequest {
    pub fn new(
        id: Uuid,
        run_id: Uuid,
        iteration: u32,
        evaluator_identity_fingerprint: impl Into<String>,
        mut rows: Vec<NativeBlindRow>,
    ) -> Result<Self, QualityError> {
        rows.sort_by(|left, right| left.row_id.cmp(&right.row_id));
        let mut value = Self {
            schema_version: NATIVE_ASSESSMENT_SCHEMA_VERSION,
            id,
            run_id,
            iteration,
            evaluator_identity_fingerprint: evaluator_identity_fingerprint.into(),
            rows,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.schema_version != NATIVE_ASSESSMENT_SCHEMA_VERSION
            || self.id.is_nil()
            || self.run_id.is_nil()
            || !(1..=10).contains(&self.iteration)
            || !canonical_fingerprint(&self.evaluator_identity_fingerprint)
            || self.rows.is_empty()
            || self.rows.len() > MAX_NATIVE_ASSESSMENT_BATCH
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "blind native assessment request identity or fingerprint is invalid".into(),
            ));
        }
        let mut previous = None;
        for row in &self.rows {
            row.validate()?;
            if previous.is_some_and(|value: &str| value >= row.row_id.as_str()) {
                return Err(QualityError::Validation(
                    "blind native assessment rows must be unique and sorted".into(),
                ));
            }
            previous = Some(&row.row_id);
        }
        if serde_json::to_vec(self)
            .map_err(|error| QualityError::Validation(error.to_string()))?
            .len()
            > 262_144
        {
            return Err(QualityError::Validation(
                "blind native assessment request exceeds 256 KiB".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAssessmentIssueCode {
    NoSupportedCandidate,
    AmbiguousQuestion,
    ContextInconsistent,
    CandidateSemanticsInsufficient,
    UnsupportedQuestion,
    TargetMismatch,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBlindAssessmentDraft {
    pub row_id: String,
    pub row_fingerprint: String,
    pub request_fingerprint: String,
    pub supported_candidate_ids: Vec<String>,
    pub ambiguous: bool,
    pub context_consistent: bool,
    pub issue_codes: Vec<NativeAssessmentIssueCode>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBlindAssessmentEvidence {
    pub draft: NativeBlindAssessmentDraft,
    pub fingerprint: String,
}

impl NativeBlindAssessmentEvidence {
    pub fn record(
        request: &NativeBlindAssessmentRequest,
        mut draft: NativeBlindAssessmentDraft,
    ) -> Result<Self, QualityError> {
        request.validate()?;
        draft.supported_candidate_ids.sort();
        draft.supported_candidate_ids.dedup();
        draft.issue_codes.sort();
        draft.issue_codes.dedup();
        let row = request
            .rows
            .iter()
            .find(|row| row.row_id == draft.row_id)
            .ok_or_else(|| {
                QualityError::Validation("blind assessment invents a request row".into())
            })?;
        let vocabulary = row
            .candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<BTreeSet<_>>();
        if draft.row_fingerprint != row.row_fingerprint
            || draft.request_fingerprint != request.fingerprint
            || draft
                .supported_candidate_ids
                .iter()
                .any(|candidate| !vocabulary.contains(candidate.as_str()))
            || draft.issue_codes.len() > 8
            || !valid_text(&draft.rationale, 1_000)
        {
            return Err(QualityError::Validation(
                "blind native assessment output is malformed or changes request authority".into(),
            ));
        }
        let fingerprint = fingerprint(&draft)?;
        Ok(Self { draft, fingerprint })
    }

    pub fn validate(&self, request: &NativeBlindAssessmentRequest) -> Result<(), QualityError> {
        if Self::record(request, self.draft.clone())? != *self {
            return Err(QualityError::Integrity(
                "blind native assessment evidence does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRepairStrategy {
    LabelPreservingVariants,
    ExistingAnchorContrast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeMetricDirection {
    Increase,
    Decrease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTargetBrief {
    pub strategy: NativeRepairStrategy,
    pub cluster_keys: Vec<String>,
    pub target_metric: String,
    pub desired_direction: NativeMetricDirection,
    pub fingerprint: String,
}

impl NativeTargetBrief {
    pub fn new(
        strategy: NativeRepairStrategy,
        mut cluster_keys: Vec<String>,
        target_metric: impl Into<String>,
        desired_direction: NativeMetricDirection,
    ) -> Result<Self, QualityError> {
        cluster_keys.sort();
        cluster_keys.dedup();
        let mut value = Self {
            strategy,
            cluster_keys,
            target_metric: target_metric.into(),
            desired_direction,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.cluster_keys.is_empty()
            || self.cluster_keys.len() > 4
            || !strict_strings(&self.cluster_keys, 128)
            || !valid_text(&self.target_metric, 128)
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "native target brief is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTargetFitRow {
    pub row: NativeBlindRow,
    pub blind_assessment_fingerprint: String,
    pub target: NativeTargetBrief,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTargetFitRequest {
    pub schema_version: u32,
    pub id: Uuid,
    pub run_id: Uuid,
    pub iteration: u32,
    pub evaluator_identity_fingerprint: String,
    pub blind_request_fingerprint: String,
    pub rows: Vec<NativeTargetFitRow>,
    pub fingerprint: String,
}

impl NativeTargetFitRequest {
    pub fn new(
        id: Uuid,
        blind_request: &NativeBlindAssessmentRequest,
        evaluator_identity_fingerprint: impl Into<String>,
        rows: Vec<(NativeBlindAssessmentEvidence, NativeTargetBrief)>,
    ) -> Result<Self, QualityError> {
        blind_request.validate()?;
        let mut bound = Vec::with_capacity(rows.len());
        for (evidence, target) in rows {
            evidence.validate(blind_request)?;
            target.validate()?;
            let row = blind_request
                .rows
                .iter()
                .find(|row| row.row_id == evidence.draft.row_id)
                .expect("validated blind evidence has a request row")
                .clone();
            bound.push(NativeTargetFitRow {
                row,
                blind_assessment_fingerprint: evidence.fingerprint,
                target,
            });
        }
        bound.sort_by(|left, right| left.row.row_id.cmp(&right.row.row_id));
        let mut value = Self {
            schema_version: NATIVE_ASSESSMENT_SCHEMA_VERSION,
            id,
            run_id: blind_request.run_id,
            iteration: blind_request.iteration,
            evaluator_identity_fingerprint: evaluator_identity_fingerprint.into(),
            blind_request_fingerprint: blind_request.fingerprint.clone(),
            rows: bound,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.schema_version != NATIVE_ASSESSMENT_SCHEMA_VERSION
            || self.id.is_nil()
            || self.run_id.is_nil()
            || !(1..=10).contains(&self.iteration)
            || !canonical_fingerprint(&self.evaluator_identity_fingerprint)
            || !canonical_fingerprint(&self.blind_request_fingerprint)
            || self.rows.is_empty()
            || self.rows.len() > MAX_NATIVE_ASSESSMENT_BATCH
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "native target-fit request identity or fingerprint is invalid".into(),
            ));
        }
        let mut previous = None;
        for row in &self.rows {
            row.row.validate()?;
            row.target.validate()?;
            if !canonical_fingerprint(&row.blind_assessment_fingerprint)
                || previous.is_some_and(|value: &str| value >= row.row.row_id.as_str())
            {
                return Err(QualityError::Validation(
                    "native target-fit rows are invalid, repeated, or unsorted".into(),
                ));
            }
            previous = Some(&row.row.row_id);
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTargetFitDraft {
    pub row_id: String,
    pub row_fingerprint: String,
    pub request_fingerprint: String,
    pub blind_assessment_fingerprint: String,
    pub target_fits: bool,
    pub issue_codes: Vec<NativeAssessmentIssueCode>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeTargetFitEvidence {
    pub draft: NativeTargetFitDraft,
    pub fingerprint: String,
}

impl NativeTargetFitEvidence {
    pub fn record(
        request: &NativeTargetFitRequest,
        mut draft: NativeTargetFitDraft,
    ) -> Result<Self, QualityError> {
        request.validate()?;
        draft.issue_codes.sort();
        draft.issue_codes.dedup();
        let row = request
            .rows
            .iter()
            .find(|row| row.row.row_id == draft.row_id)
            .ok_or_else(|| QualityError::Validation("target fit invents a request row".into()))?;
        if draft.row_fingerprint != row.row.row_fingerprint
            || draft.request_fingerprint != request.fingerprint
            || draft.blind_assessment_fingerprint != row.blind_assessment_fingerprint
            || draft.issue_codes.len() > 8
            || !valid_text(&draft.rationale, 1_000)
        {
            return Err(QualityError::Validation(
                "native target-fit output is malformed or changes blind evidence".into(),
            ));
        }
        let fingerprint = fingerprint(&draft)?;
        Ok(Self { draft, fingerprint })
    }

    pub fn validate(&self, request: &NativeTargetFitRequest) -> Result<(), QualityError> {
        if Self::record(request, self.draft.clone())? != *self {
            return Err(QualityError::Integrity(
                "native target-fit evidence does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

/// Host-private inherited-label authority. This is persisted for admission but
/// is never nested in either reviewer request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeLabelAuthority {
    pub row_id: String,
    pub row_fingerprint: String,
    pub expected_candidate_ids: Vec<String>,
    pub label_policy_version: String,
    pub fingerprint: String,
}

impl NativeLabelAuthority {
    pub fn new(
        row: &NativeBlindRow,
        mut expected_candidate_ids: Vec<String>,
        label_policy_version: impl Into<String>,
    ) -> Result<Self, QualityError> {
        row.validate()?;
        expected_candidate_ids.sort();
        expected_candidate_ids.dedup();
        let vocabulary = row
            .candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<BTreeSet<_>>();
        if expected_candidate_ids.is_empty()
            || expected_candidate_ids
                .iter()
                .any(|candidate| !vocabulary.contains(candidate.as_str()))
        {
            return Err(QualityError::Validation(
                "native label authority is empty or outside the legal candidates".into(),
            ));
        }
        let mut value = Self {
            row_id: row.row_id.clone(),
            row_fingerprint: row.row_fingerprint.clone(),
            expected_candidate_ids,
            label_policy_version: label_policy_version.into(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if !valid_id(&self.row_id)
            || !canonical_fingerprint(&self.row_fingerprint)
            || self.expected_candidate_ids.is_empty()
            || !strict_strings(&self.expected_candidate_ids, 128)
            || !valid_text(&self.label_policy_version, 128)
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "native label authority is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAdmissionDecision {
    Admitted,
    BlindAssessmentMissing,
    TargetFitMissing,
    Unsupported,
    Ambiguous,
    ContextInconsistent,
    LabelMismatch,
    TargetMismatch,
    ReviewerIssues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeAdmissionEvidence {
    pub row_id: String,
    pub row_fingerprint: String,
    pub authority_fingerprint: String,
    pub blind_assessment_fingerprint: Option<String>,
    pub target_fit_fingerprint: Option<String>,
    pub decision: NativeAdmissionDecision,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "pass", content = "request", rename_all = "snake_case")]
pub enum NativeReviewRequest {
    Blind(NativeBlindAssessmentRequest),
    TargetFit(NativeTargetFitRequest),
}

impl NativeReviewRequest {
    pub fn validate(&self) -> Result<(), QualityError> {
        match self {
            Self::Blind(value) => value.validate(),
            Self::TargetFit(value) => value.validate(),
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            Self::Blind(value) => value.id,
            Self::TargetFit(value) => value.id,
        }
    }

    pub fn run_id(&self) -> Uuid {
        match self {
            Self::Blind(value) => value.run_id,
            Self::TargetFit(value) => value.run_id,
        }
    }

    pub fn iteration(&self) -> u32 {
        match self {
            Self::Blind(value) => value.iteration,
            Self::TargetFit(value) => value.iteration,
        }
    }

    pub fn fingerprint(&self) -> &str {
        match self {
            Self::Blind(value) => &value.fingerprint,
            Self::TargetFit(value) => &value.fingerprint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeReviewOperationCategory {
    BlindSemanticAssessment,
    RepairTargetFitAssessment,
}

impl NativeReviewOperationCategory {
    pub fn matches(self, request: &NativeReviewRequest) -> bool {
        matches!(
            (self, request),
            (Self::BlindSemanticAssessment, NativeReviewRequest::Blind(_))
                | (
                    Self::RepairTargetFitAssessment,
                    NativeReviewRequest::TargetFit(_)
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeReviewCallReservation {
    pub id: Uuid,
    pub category: NativeReviewOperationCategory,
    pub request: NativeReviewRequest,
    pub attempt: u32,
    pub input_token_ceiling: u64,
    pub output_token_ceiling: u64,
    pub cost_ceiling_microusd: u64,
}

impl NativeReviewCallReservation {
    pub fn validate(&self) -> Result<(), QualityError> {
        self.request.validate()?;
        if self.id.is_nil()
            || !self.category.matches(&self.request)
            || !(1..=2).contains(&self.attempt)
            || self.input_token_ceiling == 0
            || self.output_token_ceiling == 0
        {
            return Err(QualityError::Validation(
                "native review reservation is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "pass", content = "assessments", rename_all = "snake_case")]
pub enum NativeReviewResponse {
    Blind(Vec<NativeBlindAssessmentEvidence>),
    TargetFit(Vec<NativeTargetFitEvidence>),
}

impl NativeReviewResponse {
    pub fn validate(&self, request: &NativeReviewRequest) -> Result<(), QualityError> {
        match (self, request) {
            (Self::Blind(values), NativeReviewRequest::Blind(request)) => {
                if values.len() != request.rows.len() {
                    return Err(QualityError::Validation(
                        "native blind response does not cover its request".into(),
                    ));
                }
                for (value, row) in values.iter().zip(&request.rows) {
                    value.validate(request)?;
                    if value.draft.row_id != row.row_id {
                        return Err(QualityError::Validation(
                            "native blind response order changed".into(),
                        ));
                    }
                }
            }
            (Self::TargetFit(values), NativeReviewRequest::TargetFit(request)) => {
                if values.len() != request.rows.len() {
                    return Err(QualityError::Validation(
                        "native target-fit response does not cover its request".into(),
                    ));
                }
                for (value, row) in values.iter().zip(&request.rows) {
                    value.validate(request)?;
                    if value.draft.row_id != row.row.row_id {
                        return Err(QualityError::Validation(
                            "native target-fit response order changed".into(),
                        ));
                    }
                }
            }
            _ => {
                return Err(QualityError::Validation(
                    "native review response pass differs from its request".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeReviewFailureKind {
    Configuration,
    Authentication,
    InvalidResponse,
    RateLimit,
    Transport,
    Provider,
    Budget,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeReviewFailure {
    pub kind: NativeReviewFailureKind,
    pub summary: String,
}

impl NativeReviewFailure {
    pub fn validate(&self) -> Result<(), QualityError> {
        if !valid_text(&self.summary, 400) {
            return Err(QualityError::Validation(
                "native review failure summary is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeReviewCallOutcome {
    pub reservation: NativeReviewCallReservation,
    pub usage: NativeReviewUsage,
    pub response: Option<NativeReviewResponse>,
    pub failure: Option<NativeReviewFailure>,
    pub interrupted: bool,
}

impl NativeReviewCallOutcome {
    pub fn validate(&self) -> Result<(), QualityError> {
        self.reservation.validate()?;
        if usize::from(self.response.is_some())
            + usize::from(self.failure.is_some())
            + usize::from(self.interrupted)
            != 1
        {
            return Err(QualityError::Validation(
                "native review outcome must be successful, failed, or interrupted".into(),
            ));
        }
        if let Some(response) = &self.response {
            response.validate(&self.reservation.request)?;
            if self.usage.exceeds(
                self.reservation.input_token_ceiling,
                self.reservation.output_token_ceiling,
                self.reservation.cost_ceiling_microusd,
            ) {
                return Err(QualityError::Validation(
                    "native review overrun cannot produce accepted evidence".into(),
                ));
            }
        }
        if let Some(failure) = &self.failure {
            failure.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeAdmissionRecord {
    pub run_id: Uuid,
    pub iteration: u32,
    pub evaluator_identity_fingerprint: String,
    pub authority: NativeLabelAuthority,
    pub blind: NativeBlindAssessmentEvidence,
    pub target_fit: NativeTargetFitEvidence,
    pub admission: NativeAdmissionEvidence,
    pub fingerprint: String,
}

impl NativeAdmissionRecord {
    pub fn new(
        run_id: Uuid,
        iteration: u32,
        evaluator_identity_fingerprint: impl Into<String>,
        authority: NativeLabelAuthority,
        blind: NativeBlindAssessmentEvidence,
        target_fit: NativeTargetFitEvidence,
    ) -> Result<Self, QualityError> {
        let admission = decide_native_admission(&authority, Some(&blind), Some(&target_fit))?;
        let mut value = Self {
            run_id,
            iteration,
            evaluator_identity_fingerprint: evaluator_identity_fingerprint.into(),
            authority,
            blind,
            target_fit,
            admission,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        self.authority.validate()?;
        self.admission
            .validate(&self.authority, Some(&self.blind), Some(&self.target_fit))?;
        if self.run_id.is_nil()
            || !(1..=10).contains(&self.iteration)
            || !canonical_fingerprint(&self.evaluator_identity_fingerprint)
            || self.fingerprint != self.reproduce_fingerprint()?
        {
            return Err(QualityError::Validation(
                "native admission record identity is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

impl NativeAdmissionEvidence {
    pub fn admitted(&self) -> bool {
        self.decision == NativeAdmissionDecision::Admitted
    }

    pub fn validate(
        &self,
        authority: &NativeLabelAuthority,
        blind: Option<&NativeBlindAssessmentEvidence>,
        target_fit: Option<&NativeTargetFitEvidence>,
    ) -> Result<(), QualityError> {
        if decide_native_admission(authority, blind, target_fit)? != *self {
            return Err(QualityError::Integrity(
                "native admission evidence does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

pub fn decide_native_admission(
    authority: &NativeLabelAuthority,
    blind: Option<&NativeBlindAssessmentEvidence>,
    target_fit: Option<&NativeTargetFitEvidence>,
) -> Result<NativeAdmissionEvidence, QualityError> {
    authority.validate()?;
    if let Some(value) = blind
        && (value.draft.row_id != authority.row_id
            || value.draft.row_fingerprint != authority.row_fingerprint
            || value.fingerprint != fingerprint(&value.draft)?)
    {
        return Err(QualityError::Integrity(
            "blind assessment does not belong to native label authority".into(),
        ));
    }
    if let Some(value) = target_fit
        && (value.draft.row_id != authority.row_id
            || value.draft.row_fingerprint != authority.row_fingerprint
            || value.fingerprint != fingerprint(&value.draft)?)
    {
        return Err(QualityError::Integrity(
            "target-fit assessment does not belong to native label authority".into(),
        ));
    }
    let decision = match (blind, target_fit) {
        (None, _) => NativeAdmissionDecision::BlindAssessmentMissing,
        (Some(_), None) => NativeAdmissionDecision::TargetFitMissing,
        (Some(blind), Some(target))
            if !blind.draft.issue_codes.is_empty() || !target.draft.issue_codes.is_empty() =>
        {
            NativeAdmissionDecision::ReviewerIssues
        }
        (Some(blind), Some(_)) if blind.draft.supported_candidate_ids.is_empty() => {
            NativeAdmissionDecision::Unsupported
        }
        (Some(blind), Some(_)) if blind.draft.ambiguous => NativeAdmissionDecision::Ambiguous,
        (Some(blind), Some(_)) if !blind.draft.context_consistent => {
            NativeAdmissionDecision::ContextInconsistent
        }
        (Some(blind), Some(_))
            if blind.draft.supported_candidate_ids != authority.expected_candidate_ids =>
        {
            NativeAdmissionDecision::LabelMismatch
        }
        (Some(_), Some(target)) if !target.draft.target_fits => {
            NativeAdmissionDecision::TargetMismatch
        }
        (Some(_), Some(_)) => NativeAdmissionDecision::Admitted,
    };
    let mut value = NativeAdmissionEvidence {
        row_id: authority.row_id.clone(),
        row_fingerprint: authority.row_fingerprint.clone(),
        authority_fingerprint: authority.fingerprint.clone(),
        blind_assessment_fingerprint: blind.map(|value| value.fingerprint.clone()),
        target_fit_fingerprint: target_fit.map(|value| value.fingerprint.clone()),
        decision,
        fingerprint: String::new(),
    };
    value.fingerprint = fingerprint(&value_without_fingerprint(&value))?;
    Ok(value)
}

fn value_without_fingerprint(value: &NativeAdmissionEvidence) -> NativeAdmissionEvidence {
    let mut value = value.clone();
    value.fingerprint.clear();
    value
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn valid_id(value: &str) -> bool {
    valid_text(value, 128) && !value.chars().any(char::is_control)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= maximum
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn strict_strings(values: &[String], maximum: usize) -> bool {
    values.iter().all(|value| valid_text(value, maximum))
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn normalized_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(value: &str) -> String {
        fingerprint(&value).unwrap()
    }

    fn row() -> NativeBlindRow {
        NativeBlindRow {
            row_id: "generation-task:0".into(),
            row_fingerprint: fp("row"),
            question: "Find the saved invoice".into(),
            context: NativeBlindContext::new(BTreeMap::from([
                ("taskKind".into(), Value::String("route".into())),
                ("previousCandidates".into(), Value::Array(vec![])),
            ]))
            .unwrap(),
            candidates: vec![
                NativeCandidateSemantics::new("read", "Find stored records", vec!["search".into()])
                    .unwrap(),
                NativeCandidateSemantics::new(
                    "write",
                    "Modify stored records",
                    vec!["update".into()],
                )
                .unwrap(),
            ],
        }
    }

    fn request() -> NativeBlindAssessmentRequest {
        NativeBlindAssessmentRequest::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            fp("reviewer"),
            vec![row()],
        )
        .unwrap()
    }

    fn blind(
        request: &NativeBlindAssessmentRequest,
        supported: &[&str],
    ) -> NativeBlindAssessmentEvidence {
        NativeBlindAssessmentEvidence::record(
            request,
            NativeBlindAssessmentDraft {
                row_id: request.rows[0].row_id.clone(),
                row_fingerprint: request.rows[0].row_fingerprint.clone(),
                request_fingerprint: request.fingerprint.clone(),
                supported_candidate_ids: supported.iter().map(|value| (*value).into()).collect(),
                ambiguous: false,
                context_consistent: true,
                issue_codes: vec![],
                rationale: "The question requests retrieval from stored records.".into(),
            },
        )
        .unwrap()
    }

    fn target(
        request: &NativeBlindAssessmentRequest,
        blind: &NativeBlindAssessmentEvidence,
    ) -> NativeTargetFitEvidence {
        let request = NativeTargetFitRequest::new(
            Uuid::new_v4(),
            request,
            fp("reviewer"),
            vec![(
                blind.clone(),
                NativeTargetBrief::new(
                    NativeRepairStrategy::LabelPreservingVariants,
                    vec!["cluster-read".into()],
                    "recall_at_1",
                    NativeMetricDirection::Increase,
                )
                .unwrap(),
            )],
        )
        .unwrap();
        NativeTargetFitEvidence::record(
            &request,
            NativeTargetFitDraft {
                row_id: request.rows[0].row.row_id.clone(),
                row_fingerprint: request.rows[0].row.row_fingerprint.clone(),
                request_fingerprint: request.fingerprint.clone(),
                blind_assessment_fingerprint: blind.fingerprint.clone(),
                target_fits: true,
                issue_codes: vec![],
                rationale: "The question is a label-preserving retrieval variant.".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn reviewer_requests_are_label_blind_and_separate_target_fit_from_the_answer() {
        let request = request();
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(!serialized.contains("expectedCandidate"));
        assert!(!serialized.contains("acceptable_tools"));
        assert!(!serialized.contains("hard_negative"));
        let blind = blind(&request, &["read"]);
        let fit_request = NativeTargetFitRequest::new(
            Uuid::new_v4(),
            &request,
            fp("reviewer"),
            vec![(
                blind,
                NativeTargetBrief::new(
                    NativeRepairStrategy::LabelPreservingVariants,
                    vec!["cluster-read".into()],
                    "recall_at_1",
                    NativeMetricDirection::Increase,
                )
                .unwrap(),
            )],
        )
        .unwrap();
        let serialized = serde_json::to_string(&fit_request).unwrap();
        assert!(!serialized.contains("supportedCandidateIds"));
        assert!(!serialized.contains("expectedCandidate"));
    }

    #[test]
    fn wrong_label_and_unassessed_rows_fail_closed_even_when_schema_is_valid() {
        let request = request();
        let authority = NativeLabelAuthority::new(
            &request.rows[0],
            vec!["read".into()],
            "nomos-compatible-answer-set-v1",
        )
        .unwrap();
        assert_eq!(
            decide_native_admission(&authority, None, None)
                .unwrap()
                .decision,
            NativeAdmissionDecision::BlindAssessmentMissing
        );
        let wrong = blind(&request, &["write"]);
        let fit = target(&request, &wrong);
        assert_eq!(
            decide_native_admission(&authority, Some(&wrong), Some(&fit))
                .unwrap()
                .decision,
            NativeAdmissionDecision::LabelMismatch
        );
        let correct = blind(&request, &["read"]);
        let fit = target(&request, &correct);
        let admitted = decide_native_admission(&authority, Some(&correct), Some(&fit)).unwrap();
        assert!(admitted.admitted());
        admitted
            .validate(&authority, Some(&correct), Some(&fit))
            .unwrap();
    }

    #[test]
    fn reviewer_cannot_invent_candidates_or_smuggle_label_authority_into_context() {
        let request = request();
        let mut draft = blind(&request, &["read"]).draft;
        draft.supported_candidate_ids = vec!["invented".into()];
        assert!(NativeBlindAssessmentEvidence::record(&request, draft).is_err());
        assert!(
            NativeBlindContext::new(BTreeMap::from([(
                "acceptable_tools".into(),
                Value::Array(vec![Value::String("read".into())]),
            )]))
            .is_err()
        );
    }

    #[test]
    fn durable_review_calls_are_single_pass_bounded_and_fail_closed() {
        let request = request();
        let evidence = blind(&request, &["read"]);
        let reservation = NativeReviewCallReservation {
            id: Uuid::new_v4(),
            category: NativeReviewOperationCategory::BlindSemanticAssessment,
            request: NativeReviewRequest::Blind(request.clone()),
            attempt: 1,
            input_token_ceiling: 100,
            output_token_ceiling: 50,
            cost_ceiling_microusd: 10,
        };
        reservation.validate().unwrap();
        NativeReviewCallOutcome {
            reservation: reservation.clone(),
            usage: NativeReviewUsage::default(),
            response: Some(NativeReviewResponse::Blind(vec![evidence])),
            failure: None,
            interrupted: false,
        }
        .validate()
        .unwrap();

        let mut mismatched = reservation.clone();
        mismatched.category = NativeReviewOperationCategory::RepairTargetFitAssessment;
        assert!(mismatched.validate().is_err());
        let contradictory = NativeReviewCallOutcome {
            reservation,
            usage: NativeReviewUsage::default(),
            response: None,
            failure: Some(NativeReviewFailure {
                kind: NativeReviewFailureKind::Provider,
                summary: "Provider failed.".into(),
            }),
            interrupted: true,
        };
        assert!(contradictory.validate().is_err());
    }

    #[test]
    fn admission_record_reproduces_host_private_authority_and_both_reviews() {
        let request = request();
        let authority = NativeLabelAuthority::new(
            &request.rows[0],
            vec!["read".into()],
            "nomos-compatible-answer-set-v1",
        )
        .unwrap();
        let blind = blind(&request, &["read"]);
        let target_fit = target(&request, &blind);
        let record = NativeAdmissionRecord::new(
            request.run_id,
            request.iteration,
            request.evaluator_identity_fingerprint.clone(),
            authority,
            blind,
            target_fit,
        )
        .unwrap();
        assert!(record.admission.admitted());
        record.validate().unwrap();

        let mut tampered = record;
        tampered.admission.decision = NativeAdmissionDecision::LabelMismatch;
        assert!(tampered.validate().is_err());
    }
}
