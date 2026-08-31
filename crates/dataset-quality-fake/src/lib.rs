//! Deterministic, offline blind-quality evaluator.
//!
//! This adapter is intentionally modest: lexical evidence is useful for
//! exercising the complete quality workflow, but it is not presented as a
//! semantic judge.  It reads only [`BlindEvaluatorRequest`] fields and emits
//! the exact normalized score shapes owned by `dataset-quality-core`.

use std::collections::{BTreeMap, BTreeSet};

use dataset_quality_core::{
    assessment::{
        BlindEvaluatorRequest, ConceptGuidance, EVALUATOR_REQUEST_SCHEMA_VERSION,
        EvaluatorExecutionLocation, EvaluatorIdentity, EvaluatorIndependence, QualityIssueCode,
        RowAssessmentDraft,
    },
    lifecycle::ProviderUsage,
    policy::BasisPoints,
    ports::{
        BoxFuture, EvaluatorBatchOutput, QualityEvaluationError, QualityEvaluationErrorKind,
        QualityEvaluator,
    },
};
use serde::Serialize;
use serde_json::json;

pub const FAKE_QUALITY_BACKEND: &str = "deterministic-fake";
pub const FAKE_QUALITY_MODEL: &str = "blind-lexical-v1";
pub const DEFAULT_EVALUATOR_PROTOCOL: &str = "quality-evaluator-v1";
const MAX_ROWS_PER_REQUEST: usize = 10_000;
const MAX_REPORTED_TOKENS: u64 = 1_000_000_000_000;

#[derive(Debug, Clone)]
pub struct FakeQualityEvaluator {
    identity: EvaluatorIdentity,
    seed: u64,
}

#[derive(Serialize)]
struct ConfigurationFingerprint<'a> {
    backend: &'a str,
    model: &'a str,
    protocol_version: &'a str,
    independence: EvaluatorIndependence,
    seed: u64,
}

impl Default for FakeQualityEvaluator {
    fn default() -> Self {
        Self::new(
            DEFAULT_EVALUATOR_PROTOCOL,
            EvaluatorIndependence::Primary,
            42,
        )
        .expect("static fake evaluator configuration is valid")
    }
}

impl FakeQualityEvaluator {
    pub fn new(
        protocol_version: impl Into<String>,
        independence: EvaluatorIndependence,
        seed: u64,
    ) -> Result<Self, QualityEvaluationError> {
        let protocol_version = protocol_version.into().trim().to_owned();
        if protocol_version.is_empty() {
            return Err(configuration_error(
                "evaluator protocol version must not be empty",
            ));
        }
        let model = match independence {
            EvaluatorIndependence::Primary => FAKE_QUALITY_MODEL.to_owned(),
            EvaluatorIndependence::IndependentReview => {
                format!("{FAKE_QUALITY_MODEL}-independent-{seed}")
            }
        };
        let configuration_fingerprint = artifact_core::fingerprint(&ConfigurationFingerprint {
            backend: FAKE_QUALITY_BACKEND,
            model: &model,
            protocol_version: &protocol_version,
            independence,
            seed,
        })
        .map_err(configuration_error)?;
        let identity = EvaluatorIdentity::new(
            FAKE_QUALITY_BACKEND,
            model,
            protocol_version,
            configuration_fingerprint,
            independence,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .map_err(configuration_error)?;
        Ok(Self { identity, seed })
    }
}

impl QualityEvaluator for FakeQualityEvaluator {
    fn identity(&self) -> EvaluatorIdentity {
        self.identity.clone()
    }

    fn evaluate(
        &self,
        request: BlindEvaluatorRequest,
    ) -> BoxFuture<'_, Result<EvaluatorBatchOutput, QualityEvaluationError>> {
        Box::pin(async move {
            validate_request(&request, &self.identity)?;
            let assessments = request
                .rows
                .iter()
                .map(|row| assess_row(&request, &row.text, row, self.seed))
                .collect::<Result<Vec<_>, _>>()?;
            let usage = usage(&request, &assessments)?;
            Ok(EvaluatorBatchOutput {
                assessments,
                usage,
                metadata: json!({
                    "algorithm": FAKE_QUALITY_MODEL,
                    "backend": self.identity.backend,
                    "blind_input_only": true,
                    "configuration_fingerprint": self.identity.configuration_fingerprint,
                    "deterministic": true,
                    "execution_location": "local_process",
                    "model": self.identity.model,
                    "protocol_version": self.identity.protocol_version,
                    "request_fingerprint": request.fingerprint,
                    "row_count": request.rows.len(),
                    "seed": self.seed,
                }),
            })
        })
    }
}

fn validate_request(
    request: &BlindEvaluatorRequest,
    identity: &EvaluatorIdentity,
) -> Result<(), QualityEvaluationError> {
    if request.schema_version != EVALUATOR_REQUEST_SCHEMA_VERSION
        || request.id.is_nil()
        || request.audit_plan_id.is_nil()
        || request.audit_run_id.is_nil()
        || request.attempt_id.is_nil()
        || request.request_sequence == 0
        || request.attempt_number == 0
        || request.evaluator_identity_fingerprint != identity.fingerprint
        || request.resolved_guidance_fingerprint.is_empty()
        || request
            .guidance
            .reproduce_fingerprint()
            .map_err(configuration_error)?
            != request.resolved_guidance_fingerprint
        || request.fingerprint.is_empty()
        || request
            .reproduce_fingerprint()
            .map_err(configuration_error)?
            != request.fingerprint
    {
        return Err(configuration_error(
            "blind evaluator request identity or fingerprint is invalid",
        ));
    }
    if request.evaluator_protocol_version != identity.protocol_version {
        return Err(configuration_error(
            "blind evaluator request protocol does not match the fake evaluator",
        ));
    }
    if request.budget.maximum_input_tokens == 0
        || request.budget.maximum_output_tokens == 0
        || request.budget.maximum_total_tokens == 0
        || request.budget.maximum_total_tokens
            > request
                .budget
                .maximum_input_tokens
                .saturating_add(request.budget.maximum_output_tokens)
    {
        return Err(configuration_error(
            "blind evaluator request budget must be finite and internally consistent",
        ));
    }
    if request.task_description.trim().is_empty()
        || !strictly_sorted_nonempty(&request.allowed_labels)
    {
        return Err(configuration_error(
            "blind evaluator task and label vocabulary must be normalized",
        ));
    }
    if request
        .allowed_dimensions
        .iter()
        .any(|(name, values)| name.trim().is_empty() || !strictly_sorted_nonempty(values))
    {
        return Err(configuration_error(
            "blind evaluator dimension vocabulary must be normalized",
        ));
    }
    if request.rows.is_empty() || request.rows.len() > MAX_ROWS_PER_REQUEST {
        return Err(configuration_error(format!(
            "blind evaluator request must contain between 1 and {MAX_ROWS_PER_REQUEST} rows"
        )));
    }
    let mut previous = None;
    for row in &request.rows {
        if row.source_row_id.is_nil()
            || row.source_row_fingerprint.trim().is_empty()
            || row.plan_item_fingerprint.trim().is_empty()
            || row.text.trim().is_empty()
            || previous.is_some_and(|id| id >= row.source_row_id)
        {
            return Err(configuration_error(
                "blind evaluator rows must be complete, unique, and canonically ordered",
            ));
        }
        previous = Some(row.source_row_id);
    }
    Ok(())
}

fn strictly_sorted_nonempty(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| !value.trim().is_empty())
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn assess_row(
    request: &BlindEvaluatorRequest,
    text: &str,
    row: &dataset_quality_core::assessment::BlindEvaluatorRow,
    seed: u64,
) -> Result<RowAssessmentDraft, QualityEvaluationError> {
    let text_evidence = TextEvidence::new(text);
    let label_guidance = request
        .guidance
        .semantic
        .as_ref()
        .and_then(|guidance| guidance.labels.as_ref());
    let label_scores = request
        .allowed_labels
        .iter()
        .map(|label| {
            let guidance = label_guidance.and_then(|values| values.entries.get(label));
            Ok((
                label.clone(),
                score_candidate(&text_evidence, label, guidance, seed, "label")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, QualityEvaluationError>>()?;

    let dimension_scores = request
        .allowed_dimensions
        .iter()
        .map(|(dimension, values)| {
            let guidance = request
                .guidance
                .semantic
                .as_ref()
                .and_then(|semantic| semantic.dimensions.get(dimension));
            let scores = values
                .iter()
                .map(|value| {
                    let concept = guidance.and_then(|values| values.entries.get(value));
                    Ok((
                        value.clone(),
                        score_candidate(&text_evidence, value, concept, seed, dimension)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, QualityEvaluationError>>()?;
            Ok((dimension.clone(), scores))
        })
        .collect::<Result<BTreeMap<_, _>, QualityEvaluationError>>()?;

    let label_leakage_risk = label_leakage_risk(&text_evidence, &request.allowed_labels)?;
    let shortcut_risk = shortcut_risk(&text_evidence)?;
    let authenticity_score = request
        .guidance
        .authenticity
        .as_ref()
        .map(|guidance| authenticity_score(&text_evidence, guidance, seed))
        .transpose()?;
    let confidence = confidence(&label_scores, &dimension_scores)?;
    let issue_codes = issues(
        &text_evidence,
        &label_scores,
        &dimension_scores,
        label_leakage_risk,
        shortcut_risk,
        authenticity_score,
    );

    Ok(RowAssessmentDraft {
        source_row_id: row.source_row_id,
        source_row_fingerprint: row.source_row_fingerprint.clone(),
        label_scores,
        dimension_scores,
        authenticity_score,
        label_leakage_risk,
        shortcut_risk,
        confidence,
        issue_codes,
        rationale: "Deterministic blind lexical evidence was scored against every allowed label and dimension value.".into(),
    })
}

struct TextEvidence {
    normalized: String,
    tokens: BTreeSet<String>,
    token_count: usize,
    alphabetic_characters: usize,
    character_count: usize,
    assignment_marker: bool,
}

impl TextEvidence {
    fn new(text: &str) -> Self {
        let normalized = normalize(text);
        let tokens = tokens(&normalized);
        let assignment_marker = ["label", "category", "class"]
            .iter()
            .any(|value| tokens.contains(*value))
            && (text.contains(':') || text.contains('='));
        Self {
            normalized,
            token_count: text.split_whitespace().count(),
            alphabetic_characters: text.chars().filter(|value| value.is_alphabetic()).count(),
            character_count: text.chars().count(),
            tokens,
            assignment_marker,
        }
    }

    fn contains_phrase(&self, value: &str) -> bool {
        let phrase = normalize(value);
        if phrase.is_empty() {
            return false;
        }
        let padded_text = format!(" {} ", self.normalized);
        padded_text.contains(&format!(" {phrase} "))
    }
}

fn score_candidate(
    text: &TextEvidence,
    candidate: &str,
    guidance: Option<&ConceptGuidance>,
    seed: u64,
    namespace: &str,
) -> Result<BasisPoints, QualityEvaluationError> {
    let candidate_tokens = tokens(&normalize(candidate));
    let primary_overlap = overlap(&text.tokens, &candidate_tokens);
    let guidance_tokens = guidance.map_or_else(BTreeSet::new, concept_tokens);
    let guidance_overlap = overlap(&text.tokens, &guidance_tokens);
    let mut score = 1_500_u32;
    if text.contains_phrase(candidate) {
        score += 4_000;
    }
    score += ratio_points(primary_overlap, candidate_tokens.len(), 3_000);
    score += ratio_points(guidance_overlap, guidance_tokens.len(), 1_200);
    score += u32::from(stable_jitter(text, candidate, namespace, seed) % 301);
    basis(score.min(10_000) as u16)
}

fn concept_tokens(guidance: &ConceptGuidance) -> BTreeSet<String> {
    guidance
        .description
        .iter()
        .chain(guidance.examples.iter())
        .chain(guidance.inclusion_rules.iter())
        .flat_map(|value| tokens(&normalize(value)))
        .collect()
}

fn overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> usize {
    left.intersection(right).count()
}

fn ratio_points(found: usize, total: usize, maximum: u32) -> u32 {
    if total == 0 {
        0
    } else {
        ((u128::from(maximum) * found as u128) / total as u128) as u32
    }
}

fn label_leakage_risk(
    text: &TextEvidence,
    labels: &[String],
) -> Result<BasisPoints, QualityEvaluationError> {
    let explicit = labels.iter().any(|label| text.contains_phrase(label));
    basis(if text.assignment_marker {
        9_500
    } else if explicit {
        1_000
    } else {
        500
    })
}

fn shortcut_risk(text: &TextEvidence) -> Result<BasisPoints, QualityEvaluationError> {
    let template = [
        "synthetic example",
        "generated example",
        "lorem ipsum",
        "as an ai",
        "training sample",
    ]
    .iter()
    .any(|value| text.normalized.contains(value));
    let structural_marker = text.normalized.contains(" dimensions ")
        || text.normalized.contains(" expected label ")
        || text.normalized.contains(" target label ");
    basis(if template || structural_marker {
        9_000
    } else if text.token_count < 3 {
        4_000
    } else {
        700
    })
}

fn authenticity_score(
    text: &TextEvidence,
    guidance: &dataset_quality_core::assessment::AuthenticityEvaluatorGuidance,
    seed: u64,
) -> Result<BasisPoints, QualityEvaluationError> {
    let guidance_tokens = guidance
        .instructions
        .iter()
        .chain(std::iter::once(&guidance.summary))
        .flat_map(|value| tokens(&normalize(value)))
        .collect::<BTreeSet<_>>();
    let naturalness = if text.token_count >= 4
        && text.alphabetic_characters.saturating_mul(2) >= text.character_count
    {
        2_000
    } else {
        500
    };
    let score = 5_000
        + naturalness
        + ratio_points(
            overlap(&text.tokens, &guidance_tokens),
            guidance_tokens.len(),
            2_000,
        )
        + u32::from(stable_jitter(text, &guidance.summary, "authenticity", seed) % 201);
    basis(score.min(10_000) as u16)
}

fn confidence(
    labels: &BTreeMap<String, BasisPoints>,
    dimensions: &BTreeMap<String, BTreeMap<String, BasisPoints>>,
) -> Result<BasisPoints, QualityEvaluationError> {
    let mut margins = Vec::with_capacity(dimensions.len() + 1);
    margins.push(top_margin(labels.values().map(|value| value.get())));
    margins.extend(
        dimensions
            .values()
            .map(|values| top_margin(values.values().map(|value| value.get()))),
    );
    let average_margin = if margins.is_empty() {
        0
    } else {
        margins.iter().copied().map(u32::from).sum::<u32>() / margins.len() as u32
    };
    basis((5_000 + average_margin.min(4_500)) as u16)
}

fn top_margin(values: impl Iterator<Item = u16>) -> u16 {
    let mut first = 0;
    let mut second = 0;
    for value in values {
        if value >= first {
            second = first;
            first = value;
        } else if value > second {
            second = value;
        }
    }
    first.saturating_sub(second)
}

fn issues(
    text: &TextEvidence,
    labels: &BTreeMap<String, BasisPoints>,
    dimensions: &BTreeMap<String, BTreeMap<String, BasisPoints>>,
    leakage: BasisPoints,
    shortcut: BasisPoints,
    authenticity: Option<BasisPoints>,
) -> Vec<QualityIssueCode> {
    let mut issues = BTreeSet::new();
    if top_margin(labels.values().map(|value| value.get())) < 800 {
        issues.insert(QualityIssueCode::CompetingLabelAmbiguity);
    }
    if dimensions
        .values()
        .any(|scores| scores.values().map(|value| value.get()).max().unwrap_or(0) < 5_000)
    {
        issues.insert(QualityIssueCode::DimensionNonAdherence);
    }
    if authenticity.is_some_and(|value| value.get() < 5_500) {
        issues.insert(QualityIssueCode::AuthenticityNonAdherence);
    }
    if leakage.get() >= 6_000 {
        issues.insert(QualityIssueCode::LabelLeakage);
    }
    if shortcut.get() >= 6_000 {
        issues.insert(QualityIssueCode::ShortcutArtifact);
        issues.insert(QualityIssueCode::TemplateArtifact);
    }
    if text.token_count < 3 {
        issues.insert(QualityIssueCode::InsufficientContext);
    }
    if text.character_count > 0
        && text.alphabetic_characters.saturating_mul(3) < text.character_count
    {
        issues.insert(QualityIssueCode::UnnaturalLanguage);
    }
    issues.into_iter().collect()
}

fn usage(
    request: &BlindEvaluatorRequest,
    assessments: &[RowAssessmentDraft],
) -> Result<ProviderUsage, QualityEvaluationError> {
    let request_bytes = serde_json::to_vec(request)
        .map_err(invalid_response_error)?
        .len();
    let response_bytes = serde_json::to_vec(assessments)
        .map_err(invalid_response_error)?
        .len();
    let input_tokens = estimated_tokens(request_bytes)?;
    let output_tokens = estimated_tokens(response_bytes)?;
    let total_tokens = input_tokens
        .checked_add(output_tokens)
        .filter(|value| *value <= MAX_REPORTED_TOKENS)
        .ok_or_else(|| {
            invalid_response_error("fake evaluator usage exceeds its reporting bound")
        })?;
    if input_tokens > request.budget.maximum_input_tokens
        || output_tokens > request.budget.maximum_output_tokens
        || total_tokens > request.budget.maximum_total_tokens
    {
        return Err(configuration_error(
            "fake evaluator output would exceed the exact request token budget",
        ));
    }
    let usage = ProviderUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cost_microusd: 0,
    };
    usage.validate().map_err(invalid_response_error)?;
    Ok(usage)
}

fn estimated_tokens(bytes: usize) -> Result<u64, QualityEvaluationError> {
    let bytes = u64::try_from(bytes).map_err(invalid_response_error)?;
    let tokens = bytes
        .checked_add(3)
        .map(|value| value / 4)
        .filter(|value| *value <= MAX_REPORTED_TOKENS)
        .ok_or_else(|| invalid_response_error("fake evaluator token estimate overflowed"))?;
    Ok(tokens.max(1))
}

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut needs_space = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if needs_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.extend(character.to_lowercase());
            needs_space = false;
        } else {
            needs_space = true;
        }
    }
    normalized
}

fn tokens(normalized: &str) -> BTreeSet<String> {
    normalized.split_whitespace().map(str::to_owned).collect()
}

fn stable_jitter(text: &TextEvidence, candidate: &str, namespace: &str, seed: u64) -> u16 {
    text.normalized
        .bytes()
        .chain(candidate.bytes())
        .chain(namespace.bytes())
        .fold(seed ^ 0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        }) as u16
}

fn basis(value: u16) -> Result<BasisPoints, QualityEvaluationError> {
    BasisPoints::new(value).map_err(invalid_response_error)
}

fn configuration_error(error: impl std::fmt::Display) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Configuration, error.to_string())
}

fn invalid_response_error(error: impl std::fmt::Display) -> QualityEvaluationError {
    QualityEvaluationError::new(
        QualityEvaluationErrorKind::InvalidResponse,
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::domain::{SourceProvenance, SourceRow};
    use dataset_quality_core::{
        assessment::{
            BlindEvaluatorRequest, EvaluatorGuidance, EvaluatorIndependence,
            EvaluatorRequestBudget, RowQualityAssessment,
        },
        policy::{AuditMode, EvaluatorEgressPolicy, QualityPolicyPresetControls, QualityPreset},
        population::{AuditPlan, GuidanceReferences},
        ports::QualityEvaluator,
    };
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use uuid::Uuid;

    use super::{FakeQualityEvaluator, TextEvidence, label_leakage_risk};

    #[test]
    fn natural_label_language_is_distinct_from_an_explicit_assignment_marker() {
        let labels = vec!["account security".into(), "payment dispute".into()];
        let natural = label_leakage_risk(
            &TextEvidence::new("An easy payment dispute about a duplicate charge."),
            &labels,
        )
        .expect("natural-language risk");
        let assigned = label_leakage_risk(&TextEvidence::new("label: payment dispute"), &labels)
            .expect("assignment-marker risk");

        assert_eq!(natural.get(), 1_000);
        assert_eq!(assigned.get(), 9_500);
    }

    #[test]
    fn independent_fake_reviewers_have_distinct_pinned_model_identities() {
        let first = FakeQualityEvaluator::new(
            "quality-evaluator-v1",
            EvaluatorIndependence::IndependentReview,
            1,
        )
        .expect("first reviewer");
        let second = FakeQualityEvaluator::new(
            "quality-evaluator-v1",
            EvaluatorIndependence::IndependentReview,
            2,
        )
        .expect("second reviewer");

        assert_ne!(first.identity().model, second.identity().model);
        assert_ne!(first.identity().fingerprint, second.identity().fingerprint);
    }

    #[tokio::test]
    async fn repeated_evaluation_is_byte_for_byte_deterministic_and_complete() {
        let (plan, row) = fixture("payment dispute", "easy");
        let evaluator = FakeQualityEvaluator::default();
        let request = request(&plan, row, &evaluator);

        let first = evaluator
            .evaluate(request.clone())
            .await
            .expect("first evaluation");
        let second = evaluator
            .evaluate(request)
            .await
            .expect("second evaluation");

        assert_eq!(first, second);
        assert_eq!(
            first.usage.total_tokens,
            first.usage.input_tokens + first.usage.output_tokens
        );
        assert_eq!(first.usage.cost_microusd, 0);
        assert_eq!(first.assessments.len(), 1);
        let draft = &first.assessments[0];
        assert_eq!(
            draft.label_scores.keys().cloned().collect::<Vec<_>>(),
            vec!["account security", "payment dispute"]
        );
        assert_eq!(
            draft.dimension_scores["difficulty"]
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["easy", "hard"]
        );
    }

    #[tokio::test]
    async fn scores_depend_on_blind_text_and_vocab_not_hidden_assignments() {
        let (left_plan, left_row) = fixture("payment dispute", "easy");
        let (right_plan, right_row) = fixture("account security", "hard");
        let evaluator = FakeQualityEvaluator::default();
        let left = evaluator
            .evaluate(request(&left_plan, left_row, &evaluator))
            .await
            .expect("left evaluation");
        let right = evaluator
            .evaluate(request(&right_plan, right_row, &evaluator))
            .await
            .expect("right evaluation");
        let left = &left.assessments[0];
        let right = &right.assessments[0];

        assert_eq!(left.label_scores, right.label_scores);
        assert_eq!(left.dimension_scores, right.dimension_scores);
        assert_eq!(left.authenticity_score, right.authenticity_score);
        assert_eq!(left.label_leakage_risk, right.label_leakage_risk);
        assert_eq!(left.shortcut_risk, right.shortcut_risk);
        assert_eq!(left.confidence, right.confidence);
        assert_eq!(left.issue_codes, right.issue_codes);
        assert_eq!(left.rationale, right.rationale);
        assert!(left.label_scores["payment dispute"] > left.label_scores["account security"]);
    }

    #[tokio::test]
    async fn output_is_accepted_by_host_normalization_without_target_leakage() {
        let (plan, row) = fixture("payment dispute", "easy");
        let evaluator = FakeQualityEvaluator::default();
        let request = request(&plan, row, &evaluator);
        let output = evaluator
            .evaluate(request.clone())
            .await
            .expect("evaluation");
        let assessment = RowQualityAssessment::create(
            &plan,
            &plan.items[0],
            &request,
            evaluator.identity(),
            output.assessments[0].clone(),
            &[],
            Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("host-normalized assessment");

        assessment.verify_integrity(&plan).expect("integrity");
        assert_eq!(assessment.source_row_id, plan.items[0].source_row_id);
        assert!(!assessment.rationale.contains(&plan.items[0].cell.label));
    }

    #[tokio::test]
    async fn tampered_blind_request_is_rejected_before_scoring() {
        let (plan, row) = fixture("payment dispute", "easy");
        let evaluator = FakeQualityEvaluator::default();
        let mut request = request(&plan, row, &evaluator);
        request.rows[0].text.push_str(" tampered");

        let error = evaluator
            .evaluate(request)
            .await
            .expect_err("tampered request must fail closed");

        assert_eq!(
            error.kind,
            dataset_quality_core::ports::QualityEvaluationErrorKind::Configuration
        );
        assert!(error.message.contains("fingerprint"));
    }

    fn fixture(assigned_label: &str, assigned_difficulty: &str) -> (AuditPlan, SourceRow) {
        let dataset = DatasetDefinition::with_identity(
            Uuid::new_v4(),
            "support",
            "Classify support requests",
            vec!["account security".into(), "payment dispute".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset");
        let row = SourceRow {
            id: Uuid::new_v4(),
            dataset_id: dataset.id,
            text: "A straightforward payment dispute about a duplicate charge.".into(),
            label: assigned_label.into(),
            dimensions: BTreeMap::from([("difficulty".into(), assigned_difficulty.into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Imported {
                import_id: Uuid::new_v4(),
                source_path: "fixture.jsonl".into(),
                source_row_number: 1,
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("time"),
        };
        let policy = QualityPreset::Balanced
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("policy");
        let plan = AuditPlan::with_identity(
            Uuid::new_v4(),
            &dataset,
            policy,
            GuidanceReferences::default(),
            EvaluatorGuidance::default()
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            "quality-evaluator-v1",
            vec![row.clone()],
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("plan");
        (plan, row)
    }

    fn request(
        plan: &AuditPlan,
        row: SourceRow,
        evaluator: &FakeQualityEvaluator,
    ) -> BlindEvaluatorRequest {
        let guidance = EvaluatorGuidance::default();
        BlindEvaluatorRequest::create(
            plan,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            1,
            &evaluator.identity(),
            vec![row],
            guidance.clone(),
            guidance
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            EvaluatorRequestBudget {
                maximum_input_tokens: 100_000,
                maximum_output_tokens: 100_000,
                maximum_total_tokens: 200_000,
                maximum_cost_microusd: None,
            },
        )
        .expect("request")
    }
}
