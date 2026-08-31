//! Inspectable, provider-neutral prompt construction for blind quality audits.

use std::collections::{BTreeMap, BTreeSet};

use dataset_quality_core::{
    assessment::{
        AuthenticityEvaluatorGuidance, BlindEvaluatorRequest, EvaluatorGuidance, QualityIssueCode,
        SemanticTargetGuidance,
    },
    ports::{QualityEvaluationError, QualityEvaluationErrorKind},
};
use serde::Serialize;
use uuid::Uuid;

/// Prompt version used by the strict blind JSON protocol.
pub const STRICT_BLIND_JSON_PROMPT_VERSION: &str = "dataset-quality-blind-json-v1";

const SYSTEM_PROMPT: &str = r#"You are a blind semantic-quality assessor for text-classification candidates.
Assess only the candidate text against the complete allowed label and dimension vocabularies and the supplied guidance.
You are not shown the assigned label or assigned dimension values. Do not infer that the host prefers any target.
Return one JSON object and nothing else. It must contain exactly one `assessments` array entry for every candidate row.
For each entry, copy `source_row_id` and `source_row_fingerprint` exactly. Score every allowed label and every allowed value of every dimension with an integer from 0 through 10000 inclusive.
Only include `authenticity_score` when the request says it is required. Use only the listed issue codes. Keep `rationale` concise and evidence-based.
Do not return replacement text, a replacement label, assigned targets, dataset membership, inclusion or exclusion decisions, training advice, provenance, markdown, or any fields outside the exact assessment schema."#;

/// Fully rendered messages. The transport treats these strings as opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityEvaluatorPrompt {
    pub(crate) version: String,
    pub(crate) system_message: String,
    pub(crate) user_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptPolicyIdentity {
    pub(crate) version: String,
    pub(crate) fingerprint: String,
}

/// The only production prompt policy. It receives a structurally blind
/// projection rather than the complete evaluator request.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StrictBlindJsonPromptBuilder;

impl StrictBlindJsonPromptBuilder {
    fn version(self) -> &'static str {
        STRICT_BLIND_JSON_PROMPT_VERSION
    }

    fn build(
        self,
        input: BlindPromptInput<'_>,
    ) -> Result<QualityEvaluatorPrompt, QualityEvaluationError> {
        let payload = PromptPayload::from_input(input);
        let user_message = serde_json::to_string(&payload)
            .map_err(|_| configuration_error("could not encode the bounded evaluator prompt"))?;
        Ok(QualityEvaluatorPrompt {
            version: self.version().to_owned(),
            system_message: SYSTEM_PROMPT.to_owned(),
            user_message,
        })
    }
}

pub(crate) fn production_prompt_identity() -> Result<PromptPolicyIdentity, QualityEvaluationError> {
    let material = PromptPolicyMaterial {
        version: STRICT_BLIND_JSON_PROMPT_VERSION,
        system_message: SYSTEM_PROMPT,
        projected_request_fields: [
            "task_description",
            "allowed_labels",
            "allowed_dimensions",
            "guidance",
            "candidate_rows",
            "required_output",
        ],
        candidate_row_fields: ["source_row_id", "source_row_fingerprint", "text"],
        assessment_fields_without_authenticity: assessment_fields(false),
        assessment_fields_with_authenticity: assessment_fields(true),
        allowed_issue_codes: all_issue_codes(),
        score_minimum: 0,
        score_maximum: 10_000,
        integer_scores_only: true,
    };
    let fingerprint = artifact_core::fingerprint(&material)
        .map_err(|_| configuration_error("could not fingerprint evaluator prompt policy"))?;
    Ok(PromptPolicyIdentity {
        version: STRICT_BLIND_JSON_PROMPT_VERSION.to_owned(),
        fingerprint,
    })
}

pub(crate) fn build_production_prompt(
    request: &BlindEvaluatorRequest,
) -> Result<QualityEvaluatorPrompt, QualityEvaluationError> {
    validate_prompt_input(request)?;
    StrictBlindJsonPromptBuilder.build(BlindPromptInput::from_request(request))
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptPolicyMaterial<'a> {
    version: &'a str,
    system_message: &'a str,
    projected_request_fields: [&'a str; 6],
    candidate_row_fields: [&'a str; 3],
    assessment_fields_without_authenticity: Vec<&'a str>,
    assessment_fields_with_authenticity: Vec<&'a str>,
    allowed_issue_codes: Vec<String>,
    score_minimum: u16,
    score_maximum: u16,
    integer_scores_only: bool,
}

struct BlindPromptInput<'a> {
    task_description: &'a str,
    allowed_labels: &'a [String],
    allowed_dimensions: &'a BTreeMap<String, Vec<String>>,
    guidance: PromptGuidance<'a>,
    candidate_rows: Vec<PromptRow<'a>>,
    include_authenticity: bool,
}

impl<'a> BlindPromptInput<'a> {
    fn from_request(request: &'a BlindEvaluatorRequest) -> Self {
        Self {
            task_description: &request.task_description,
            allowed_labels: &request.allowed_labels,
            allowed_dimensions: &request.allowed_dimensions,
            guidance: PromptGuidance::from_guidance(&request.guidance),
            candidate_rows: request
                .rows
                .iter()
                .map(|row| PromptRow {
                    source_row_id: row.source_row_id,
                    source_row_fingerprint: &row.source_row_fingerprint,
                    text: &row.text,
                })
                .collect(),
            include_authenticity: request.guidance.authenticity.is_some(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptPayload<'a> {
    task_description: &'a str,
    allowed_labels: &'a [String],
    allowed_dimensions: &'a BTreeMap<String, Vec<String>>,
    guidance: PromptGuidance<'a>,
    candidate_rows: Vec<PromptRow<'a>>,
    required_output: RequiredOutput<'a>,
}

impl<'a> PromptPayload<'a> {
    fn from_input(input: BlindPromptInput<'a>) -> Self {
        let assessment_count = input.candidate_rows.len();
        Self {
            task_description: input.task_description,
            allowed_labels: input.allowed_labels,
            allowed_dimensions: input.allowed_dimensions,
            guidance: input.guidance,
            candidate_rows: input.candidate_rows,
            required_output: RequiredOutput {
                assessment_count,
                label_score_keys: input.allowed_labels,
                dimension_score_keys: input.allowed_dimensions,
                score_minimum: 0,
                score_maximum: 10_000,
                integer_scores_only: true,
                authenticity_score: if input.include_authenticity {
                    AuthenticityRequirement::Required
                } else {
                    AuthenticityRequirement::MustBeOmitted
                },
                allowed_issue_codes: all_issue_codes(),
                exact_assessment_fields: assessment_fields(input.include_authenticity),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptRow<'a> {
    source_row_id: Uuid,
    source_row_fingerprint: &'a str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptGuidance<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<&'a SemanticTargetGuidance>,
    dimensions: &'a BTreeMap<String, SemanticTargetGuidance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticity: Option<PromptAuthenticityGuidance<'a>>,
}

impl<'a> PromptGuidance<'a> {
    fn from_guidance(guidance: &'a EvaluatorGuidance) -> Self {
        let dimensions = match &guidance.semantic {
            Some(semantic) => &semantic.dimensions,
            None => empty_dimensions(),
        };
        Self {
            labels: guidance
                .semantic
                .as_ref()
                .and_then(|semantic| semantic.labels.as_ref()),
            dimensions,
            authenticity: guidance
                .authenticity
                .as_ref()
                .map(PromptAuthenticityGuidance::from),
        }
    }
}

fn empty_dimensions() -> &'static BTreeMap<String, SemanticTargetGuidance> {
    static EMPTY: std::sync::OnceLock<BTreeMap<String, SemanticTargetGuidance>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(BTreeMap::new)
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptAuthenticityGuidance<'a> {
    summary: &'a str,
    instructions: &'a [String],
    caveats: &'a [String],
}

impl<'a> From<&'a AuthenticityEvaluatorGuidance> for PromptAuthenticityGuidance<'a> {
    fn from(value: &'a AuthenticityEvaluatorGuidance) -> Self {
        Self {
            summary: &value.summary,
            instructions: &value.instructions,
            caveats: &value.caveats,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum AuthenticityRequirement {
    Required,
    MustBeOmitted,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RequiredOutput<'a> {
    assessment_count: usize,
    label_score_keys: &'a [String],
    dimension_score_keys: &'a BTreeMap<String, Vec<String>>,
    score_minimum: u16,
    score_maximum: u16,
    integer_scores_only: bool,
    authenticity_score: AuthenticityRequirement,
    allowed_issue_codes: Vec<String>,
    exact_assessment_fields: Vec<&'static str>,
}

fn all_issue_codes() -> Vec<String> {
    [
        QualityIssueCode::AssignedLabelUnsupported,
        QualityIssueCode::CompetingLabelAmbiguity,
        QualityIssueCode::DimensionNonAdherence,
        QualityIssueCode::AuthenticityNonAdherence,
        QualityIssueCode::LabelLeakage,
        QualityIssueCode::ShortcutArtifact,
        QualityIssueCode::InternalContradiction,
        QualityIssueCode::TemplateArtifact,
        QualityIssueCode::UnnaturalLanguage,
        QualityIssueCode::InsufficientContext,
        QualityIssueCode::PotentiallySensitiveData,
        QualityIssueCode::HarmfulContent,
    ]
    .into_iter()
    .map(|code| issue_code_name(code).to_owned())
    .collect()
}

fn issue_code_name(code: QualityIssueCode) -> &'static str {
    match code {
        QualityIssueCode::AssignedLabelUnsupported => "assigned_label_unsupported",
        QualityIssueCode::CompetingLabelAmbiguity => "competing_label_ambiguity",
        QualityIssueCode::DimensionNonAdherence => "dimension_non_adherence",
        QualityIssueCode::AuthenticityNonAdherence => "authenticity_non_adherence",
        QualityIssueCode::LabelLeakage => "label_leakage",
        QualityIssueCode::ShortcutArtifact => "shortcut_artifact",
        QualityIssueCode::InternalContradiction => "internal_contradiction",
        QualityIssueCode::TemplateArtifact => "template_artifact",
        QualityIssueCode::UnnaturalLanguage => "unnatural_language",
        QualityIssueCode::InsufficientContext => "insufficient_context",
        QualityIssueCode::PotentiallySensitiveData => "potentially_sensitive_data",
        QualityIssueCode::HarmfulContent => "harmful_content",
    }
}

fn assessment_fields(include_authenticity: bool) -> Vec<&'static str> {
    let mut fields = vec![
        "source_row_id",
        "source_row_fingerprint",
        "label_scores",
        "dimension_scores",
        "label_leakage_risk",
        "shortcut_risk",
        "confidence",
        "issue_codes",
        "rationale",
    ];
    if include_authenticity {
        fields.push("authenticity_score");
    }
    fields
}

fn validate_prompt_input(request: &BlindEvaluatorRequest) -> Result<(), QualityEvaluationError> {
    if request.fingerprint.is_empty()
        || request
            .reproduce_fingerprint()
            .map_err(|_| configuration_error("evaluator request fingerprint could not reproduce"))?
            != request.fingerprint
        || request.task_description.trim().is_empty()
        || request.allowed_labels.is_empty()
        || request.rows.is_empty()
        || request.guidance.reproduce_fingerprint().map_err(|_| {
            configuration_error("evaluator guidance fingerprint could not reproduce")
        })? != request.resolved_guidance_fingerprint
    {
        return Err(configuration_error(
            "evaluator request is incomplete or its fingerprint does not reproduce",
        ));
    }
    if request
        .allowed_labels
        .iter()
        .any(|label| label.trim().is_empty())
        || request.allowed_dimensions.iter().any(|(name, values)| {
            name.trim().is_empty()
                || values.is_empty()
                || values.iter().any(|value| value.trim().is_empty())
        })
        || request.rows.iter().any(|row| {
            row.source_row_id.is_nil()
                || row.source_row_fingerprint.trim().is_empty()
                || row.text.trim().is_empty()
        })
    {
        return Err(configuration_error(
            "evaluator request contains an empty vocabulary, row identity, or candidate text",
        ));
    }
    let label_set = request
        .allowed_labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if label_set.len() != request.allowed_labels.len()
        || request.allowed_dimensions.values().any(|values| {
            values
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                != values.len()
        })
        || !request
            .rows
            .windows(2)
            .all(|pair| pair[0].source_row_id < pair[1].source_row_id)
    {
        return Err(configuration_error(
            "evaluator request vocabularies or row order are not canonical and unique",
        ));
    }
    Ok(())
}

fn configuration_error(message: &'static str) -> QualityEvaluationError {
    QualityEvaluationError::new(QualityEvaluationErrorKind::Configuration, message)
}
