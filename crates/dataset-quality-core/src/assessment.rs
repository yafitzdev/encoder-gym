//! Blind evaluator evidence and host-derived quality verdicts.
//!
//! The evaluator sees the candidate text and complete allowed vocabularies, but
//! never the source row's assigned label or dimension values.  Its response is
//! evidence, not a selection decision: this module validates the exact score
//! shape and applies the pinned integer policy on the host.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::SourceRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    QualityError, fingerprint,
    policy::{BasisPoints, BorderlineReviewPolicy},
    population::{
        AuditPlan, AuditPlanItem, AuditSelection, CheckedAuditPlan, GuidanceReference,
        ProvenanceStratum,
    },
    required,
};

pub const ASSESSMENT_SCHEMA_VERSION: u32 = 1;
pub const EVALUATOR_REQUEST_SCHEMA_VERSION: u32 = 1;
pub const MAX_ISSUE_CODES: usize = 16;
pub const MAX_RATIONALE_CHARACTERS: usize = 1_000;
const MAX_GUIDANCE_ITEMS: usize = 128;
const MAX_GUIDANCE_ITEM_CHARACTERS: usize = 2_000;

/// Whether this evaluator is the primary reviewer or a separately configured
/// reviewer used for additional evidence.  The declaration is verified against
/// prior assessments; it is not accepted as proof of independence by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorIndependence {
    Primary,
    IndependentReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorExecutionLocation {
    LocalProcess,
    ExternalService,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorIdentity {
    pub backend: String,
    pub model: String,
    pub protocol_version: String,
    pub configuration_fingerprint: String,
    pub independence: EvaluatorIndependence,
    pub execution_location: EvaluatorExecutionLocation,
    pub fingerprint: String,
}

impl EvaluatorIdentity {
    pub fn new(
        backend: impl Into<String>,
        model: impl Into<String>,
        protocol_version: impl Into<String>,
        configuration_fingerprint: impl Into<String>,
        independence: EvaluatorIndependence,
        execution_location: EvaluatorExecutionLocation,
    ) -> Result<Self, QualityError> {
        let mut value = Self {
            backend: required(backend, "evaluator.backend")?,
            model: required(model, "evaluator.model")?,
            protocol_version: required(protocol_version, "evaluator.protocol_version")?,
            configuration_fingerprint: required(
                configuration_fingerprint,
                "evaluator.configuration_fingerprint",
            )?,
            independence,
            execution_location,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if required(self.backend.clone(), "evaluator.backend")? != self.backend
            || required(self.model.clone(), "evaluator.model")? != self.model
            || required(self.protocol_version.clone(), "evaluator.protocol_version")?
                != self.protocol_version
            || required(
                self.configuration_fingerprint.clone(),
                "evaluator.configuration_fingerprint",
            )? != self.configuration_fingerprint
        {
            return Err(QualityError::Validation(
                "evaluator identity strings must be non-empty and already normalized".into(),
            ));
        }
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(QualityError::Integrity(
                "evaluator identity fingerprint does not reproduce".into(),
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

/// Relationship computed by the host from immutable source provenance.  This
/// is deliberately separate from `EvaluatorIdentity::independence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorEvaluatorRelationship {
    NotApplicable,
    IndependentBackend,
    SharedBackend,
    SharedBackendAndModel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptGuidance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub counterexamples: Vec<String>,
    #[serde(default)]
    pub inclusion_rules: Vec<String>,
    #[serde(default)]
    pub exclusion_rules: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTargetGuidance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub entries: BTreeMap<String, ConceptGuidance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEvaluatorGuidance {
    pub reference: GuidanceReference,
    /// Exact immutable semantic binding decisions which produced the layered
    /// context. The context reference identifies this audit's resolved view;
    /// these parents make its origin reproducible without consulting current
    /// bindings.
    #[serde(default)]
    pub sources: Vec<GuidanceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<SemanticTargetGuidance>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, SemanticTargetGuidance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticityEvaluatorGuidance {
    pub reference: GuidanceReference,
    pub summary: String,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorGuidance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<SemanticEvaluatorGuidance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity: Option<AuthenticityEvaluatorGuidance>,
}

impl EvaluatorGuidance {
    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(self)
    }

    /// Verifies that this is the exact, bounded guidance payload pinned by an
    /// immutable audit plan. Adapters persist this payload beside the plan so
    /// execution never has to reconstruct it from mutable current bindings.
    pub fn verify_against(&self, plan: &AuditPlan) -> Result<(), QualityError> {
        validate_guidance(plan, self)?;
        if self.reproduce_fingerprint()? != plan.resolved_guidance_fingerprint {
            return Err(QualityError::Integrity(
                "resolved evaluator guidance fingerprint does not match the audit plan".into(),
            ));
        }
        Ok(())
    }
}

/// A row sent to an evaluator.  Do not add an assigned label, assigned
/// dimensions, cell, or source provenance to this structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindEvaluatorRow {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub plan_item_fingerprint: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorRequestBudget {
    pub maximum_input_tokens: u64,
    pub maximum_output_tokens: u64,
    pub maximum_total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_cost_microusd: Option<u64>,
}

impl EvaluatorRequestBudget {
    pub fn validate_against(&self, plan: &AuditPlan) -> Result<(), QualityError> {
        if self.maximum_input_tokens == 0
            || self.maximum_output_tokens == 0
            || self.maximum_total_tokens == 0
            || self.maximum_input_tokens > plan.policy.budgets.maximum_input_tokens
            || self.maximum_output_tokens > plan.policy.budgets.maximum_output_tokens
            || self.maximum_total_tokens > plan.policy.budgets.maximum_total_tokens
            || self.maximum_total_tokens
                > self
                    .maximum_input_tokens
                    .saturating_add(self.maximum_output_tokens)
        {
            return Err(QualityError::Validation(
                "evaluator request token budget is zero, contradictory, or exceeds the audit budget"
                    .into(),
            ));
        }
        match (
            self.maximum_cost_microusd,
            plan.policy.budgets.maximum_cost_microusd,
        ) {
            (Some(request), Some(audit)) if request <= audit => {}
            (Some(_), None) => {}
            (None, None) => {}
            _ => {
                return Err(QualityError::Validation(
                    "evaluator request cost budget exceeds or omits the finite audit cost budget"
                        .into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindEvaluatorRequest {
    pub id: Uuid,
    pub schema_version: u32,
    pub audit_plan_id: Uuid,
    pub audit_plan_fingerprint: String,
    pub audit_run_id: Uuid,
    pub attempt_id: Uuid,
    pub request_sequence: u32,
    pub attempt_number: u32,
    pub evaluator_identity_fingerprint: String,
    pub evaluator_protocol_version: String,
    pub task_description: String,
    pub allowed_labels: Vec<String>,
    pub allowed_dimensions: BTreeMap<String, Vec<String>>,
    pub guidance: EvaluatorGuidance,
    pub resolved_guidance_fingerprint: String,
    pub budget: EvaluatorRequestBudget,
    pub rows: Vec<BlindEvaluatorRow>,
    pub fingerprint: String,
}

impl BlindEvaluatorRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        plan: &AuditPlan,
        request_id: Uuid,
        run_id: Uuid,
        attempt_id: Uuid,
        request_sequence: u32,
        attempt_number: u32,
        evaluator: &EvaluatorIdentity,
        rows: Vec<SourceRow>,
        guidance: EvaluatorGuidance,
        resolved_guidance_fingerprint: impl Into<String>,
        budget: EvaluatorRequestBudget,
    ) -> Result<Self, QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        Self::create_with_context(
            &checked,
            request_id,
            run_id,
            attempt_id,
            request_sequence,
            attempt_number,
            evaluator,
            rows,
            guidance,
            resolved_guidance_fingerprint,
            budget,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_context(
        checked: &CheckedAuditPlan<'_>,
        request_id: Uuid,
        run_id: Uuid,
        attempt_id: Uuid,
        request_sequence: u32,
        attempt_number: u32,
        evaluator: &EvaluatorIdentity,
        rows: Vec<SourceRow>,
        guidance: EvaluatorGuidance,
        resolved_guidance_fingerprint: impl Into<String>,
        budget: EvaluatorRequestBudget,
    ) -> Result<Self, QualityError> {
        let plan = checked.plan();
        evaluator.validate()?;
        if request_id.is_nil()
            || run_id.is_nil()
            || attempt_id.is_nil()
            || request_sequence == 0
            || request_sequence > plan.policy.budgets.maximum_evaluator_requests
            || attempt_number == 0
            || attempt_number > plan.policy.budgets.maximum_attempts_per_request
        {
            return Err(QualityError::Validation(
                "evaluator request requires non-nil identities and bounded positive sequence numbers"
                    .into(),
            ));
        }
        if evaluator.protocol_version != plan.evaluator_protocol_version {
            return Err(QualityError::Validation(
                "evaluator request identity uses a protocol not pinned by the audit plan".into(),
            ));
        }
        budget.validate_against(plan)?;
        if rows.is_empty() {
            return Err(QualityError::Validation(
                "evaluator request batch must not be empty".into(),
            ));
        }
        if rows.len() > plan.policy.budgets.maximum_rows_per_batch as usize {
            return Err(QualityError::Validation(format!(
                "evaluator request batch contains {} rows, exceeding the pinned maximum {}",
                rows.len(),
                plan.policy.budgets.maximum_rows_per_batch
            )));
        }
        validate_guidance(plan, &guidance)?;
        let resolved_guidance_fingerprint = required(
            resolved_guidance_fingerprint,
            "resolved evaluator guidance fingerprint",
        )?;
        if guidance.reproduce_fingerprint()? != resolved_guidance_fingerprint
            || resolved_guidance_fingerprint != plan.resolved_guidance_fingerprint
        {
            return Err(QualityError::Integrity(
                "resolved evaluator guidance fingerprint does not match the exact guidance payload"
                    .into(),
            ));
        }

        let mut seen = BTreeSet::new();
        let mut blind_rows = Vec::with_capacity(rows.len());
        for row in rows {
            if !seen.insert(row.id) {
                return Err(QualityError::Validation(format!(
                    "source row {} appears more than once in evaluator request",
                    row.id
                )));
            }
            let item = checked.item(row.id).ok_or_else(|| {
                QualityError::Integrity(format!(
                    "source row {} is not pinned by audit plan {}",
                    row.id, plan.id
                ))
            })?;
            if item.selection != AuditSelection::Selected {
                return Err(QualityError::Validation(format!(
                    "source row {} is report-only and cannot be evaluated",
                    row.id
                )));
            }
            let source_row_fingerprint = fingerprint(&row)?;
            if source_row_fingerprint != item.source_row_fingerprint {
                return Err(QualityError::Integrity(format!(
                    "source row {} changed after the audit plan was created",
                    row.id
                )));
            }
            let text = required(row.text, "evaluator_request.rows.text")?;
            blind_rows.push(BlindEvaluatorRow {
                source_row_id: row.id,
                source_row_fingerprint,
                plan_item_fingerprint: fingerprint(item)?,
                text,
            });
        }
        blind_rows.sort_by_key(|row| row.source_row_id);

        let mut value = Self {
            id: request_id,
            schema_version: EVALUATOR_REQUEST_SCHEMA_VERSION,
            audit_plan_id: plan.id,
            audit_plan_fingerprint: plan.fingerprint.clone(),
            audit_run_id: run_id,
            attempt_id,
            request_sequence,
            attempt_number,
            evaluator_identity_fingerprint: evaluator.fingerprint.clone(),
            evaluator_protocol_version: plan.evaluator_protocol_version.clone(),
            task_description: plan.dataset_schema.task_description.clone(),
            allowed_labels: plan.dataset_schema.labels.clone(),
            allowed_dimensions: plan
                .dataset_schema
                .dimensions
                .iter()
                .map(|dimension| (dimension.name.clone(), dimension.values.clone()))
                .collect(),
            guidance,
            resolved_guidance_fingerprint,
            budget,
            rows: blind_rows,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_with_context(checked, evaluator)?;
        Ok(value)
    }

    pub fn verify_integrity(
        &self,
        plan: &AuditPlan,
        evaluator: &EvaluatorIdentity,
    ) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.verify_with_context(&checked, evaluator)
    }

    pub fn verify_with_context(
        &self,
        checked: &CheckedAuditPlan<'_>,
        evaluator: &EvaluatorIdentity,
    ) -> Result<(), QualityError> {
        let plan = checked.plan();
        evaluator.validate()?;
        if self.id.is_nil()
            || self.schema_version != EVALUATOR_REQUEST_SCHEMA_VERSION
            || self.audit_plan_id != plan.id
            || self.audit_plan_fingerprint != plan.fingerprint
            || self.audit_run_id.is_nil()
            || self.attempt_id.is_nil()
            || self.request_sequence == 0
            || self.request_sequence > plan.policy.budgets.maximum_evaluator_requests
            || self.attempt_number == 0
            || self.attempt_number > plan.policy.budgets.maximum_attempts_per_request
            || self.evaluator_identity_fingerprint != evaluator.fingerprint
            || self.evaluator_protocol_version != plan.evaluator_protocol_version
            || evaluator.protocol_version != plan.evaluator_protocol_version
            || self.task_description != plan.dataset_schema.task_description
            || self.allowed_labels != plan.dataset_schema.labels
            || self.allowed_dimensions
                != plan
                    .dataset_schema
                    .dimensions
                    .iter()
                    .map(|dimension| (dimension.name.clone(), dimension.values.clone()))
                    .collect()
        {
            return Err(QualityError::Integrity(
                "evaluator request identity, evaluator, or schema binding is invalid".into(),
            ));
        }
        self.budget.validate_against(plan)?;
        validate_guidance(plan, &self.guidance)?;
        if self.guidance.reproduce_fingerprint()? != self.resolved_guidance_fingerprint
            || self.resolved_guidance_fingerprint != plan.resolved_guidance_fingerprint
        {
            return Err(QualityError::Integrity(
                "evaluator request resolved-guidance fingerprint does not reproduce".into(),
            ));
        }
        if self.rows.is_empty()
            || self.rows.len() > plan.policy.budgets.maximum_rows_per_batch as usize
            || !self
                .rows
                .windows(2)
                .all(|pair| pair[0].source_row_id < pair[1].source_row_id)
        {
            return Err(QualityError::Integrity(
                "evaluator request rows must be non-empty, bounded, unique, and canonical".into(),
            ));
        }
        for row in &self.rows {
            let item = checked.item(row.source_row_id).ok_or_else(|| {
                QualityError::Integrity(
                    "evaluator request contains a row outside its audit plan".into(),
                )
            })?;
            if item.selection != AuditSelection::Selected
                || row.source_row_fingerprint != item.source_row_fingerprint
                || row.plan_item_fingerprint != fingerprint(item)?
                || required(row.text.clone(), "evaluator_request.rows.text")? != row.text
            {
                return Err(QualityError::Integrity(
                    "evaluator request row does not bind an exact selected plan item".into(),
                ));
            }
        }
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(QualityError::Integrity(
                "evaluator request fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    /// Stable logical-request identity shared by transport retries. Attempt-
    /// local IDs and the attempt ordinal are intentionally excluded.
    pub fn reproduce_retry_payload_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.id = Uuid::nil();
        value.attempt_id = Uuid::nil();
        value.attempt_number = 0;
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

impl RowQualityAssessment {
    /// Revalidates a loaded immutable assessment against its authoritative
    /// plan. Request integrity is verified separately by persistence because
    /// the assessment intentionally retains only the request identity and
    /// fingerprint, not a second copy of candidate text.
    pub fn verify_integrity(&self, plan: &AuditPlan) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.verify_with_context(&checked)
    }

    pub fn verify_with_context(&self, checked: &CheckedAuditPlan<'_>) -> Result<(), QualityError> {
        let plan = checked.plan();
        self.evaluator.validate()?;
        if self.id.is_nil()
            || self.schema_version != ASSESSMENT_SCHEMA_VERSION
            || self.audit_plan_id != plan.id
            || self.audit_plan_fingerprint != plan.fingerprint
            || self.audit_run_id.is_nil()
            || self.request_id.is_nil()
            || self.attempt_id.is_nil()
            || self.request_sequence == 0
            || self.request_sequence > plan.policy.budgets.maximum_evaluator_requests
            || self.attempt_number == 0
            || self.attempt_number > plan.policy.budgets.maximum_attempts_per_request
            || self.evaluator.protocol_version != plan.evaluator_protocol_version
            || required(
                self.request_fingerprint.clone(),
                "assessment.request_fingerprint",
            )? != self.request_fingerprint
        {
            return Err(QualityError::Integrity(
                "assessment identity, plan, request, attempt, or evaluator binding is invalid"
                    .into(),
            ));
        }
        if self.evaluator.independence == EvaluatorIndependence::IndependentReview
            && plan.policy.borderline_review_policy == BorderlineReviewPolicy::None
        {
            return Err(QualityError::Integrity(
                "assessment declares an independent review forbidden by the pinned policy".into(),
            ));
        }
        let item = checked.item(self.source_row_id).ok_or_else(|| {
            QualityError::Integrity(format!(
                "assessment source row {} is absent from the audit plan",
                self.source_row_id
            ))
        })?;
        if item.selection != AuditSelection::Selected
            || item.source_row_fingerprint != self.source_row_fingerprint
            || fingerprint(item)? != self.plan_item_fingerprint
        {
            return Err(QualityError::Integrity(
                "assessment does not bind the exact selected audit-plan item".into(),
            ));
        }
        let shape = RowAssessmentDraft {
            source_row_id: self.source_row_id,
            source_row_fingerprint: self.source_row_fingerprint.clone(),
            label_scores: self.label_scores.clone(),
            dimension_scores: self.dimension_scores.clone(),
            authenticity_score: self.authenticity_score,
            label_leakage_risk: self.label_leakage_risk,
            shortcut_risk: self.shortcut_risk,
            confidence: self.confidence,
            issue_codes: self.issue_codes.clone(),
            rationale: self.rationale.clone(),
        };
        validate_score_shape(plan, &shape)?;
        if validate_issue_codes(self.issue_codes.clone())? != self.issue_codes
            || bounded_required(
                self.rationale.clone(),
                "assessment.rationale",
                MAX_RATIONALE_CHARACTERS,
            )? != self.rationale
        {
            return Err(QualityError::Integrity(
                "assessment issue codes or rationale are not canonical".into(),
            ));
        }

        let assigned_label_score = self.label_scores[&item.cell.label];
        let strongest_competing = self
            .label_scores
            .iter()
            .filter(|(label, _)| *label != &item.cell.label)
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)));
        let (competing_label, competing_score, label_margin) = match strongest_competing {
            Some((label, score)) => (
                Some(label.clone()),
                Some(*score),
                i32::from(assigned_label_score.get()) - i32::from(score.get()),
            ),
            None => (None, None, 0),
        };
        let assigned_dimension_scores = item
            .cell
            .dimensions
            .iter()
            .map(|(dimension, assigned)| {
                (
                    dimension.clone(),
                    self.dimension_scores[dimension][assigned],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let outcomes = derive_outcomes(
            plan,
            assigned_label_score,
            competing_score,
            label_margin,
            &assigned_dimension_scores,
            self.authenticity_score,
            self.label_leakage_risk,
            self.shortcut_risk,
            self.confidence,
        );
        if self.assigned_label_score != assigned_label_score
            || self.strongest_competing_label != competing_label
            || self.strongest_competing_score != competing_score
            || self.assigned_label_margin != label_margin
            || self.assigned_dimension_scores != assigned_dimension_scores
            || self.outcomes != outcomes
            || self.verdict != outcomes.verdict()
            || self.generator_relationship != generator_relationship(item, &self.evaluator)
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "assessment host-derived evidence, verdict, or fingerprint does not reproduce"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn verify_request_binding(
        &self,
        plan: &AuditPlan,
        request: &BlindEvaluatorRequest,
    ) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.verify_request_binding_with_context(&checked, request)
    }

    pub fn verify_request_binding_with_context(
        &self,
        checked: &CheckedAuditPlan<'_>,
        request: &BlindEvaluatorRequest,
    ) -> Result<(), QualityError> {
        self.verify_with_context(checked)?;
        request.verify_with_context(checked, &self.evaluator)?;
        if self.audit_run_id != request.audit_run_id
            || self.request_id != request.id
            || self.request_fingerprint != request.fingerprint
            || self.attempt_id != request.attempt_id
            || self.request_sequence != request.request_sequence
            || self.attempt_number != request.attempt_number
            || !request
                .rows
                .iter()
                .any(|row| row.source_row_id == self.source_row_id)
        {
            return Err(QualityError::Integrity(
                "assessment does not bind its exact durable evaluator request".into(),
            ));
        }
        Ok(())
    }
}

/// Known, bounded evaluator observations.  Threshold failures are retained in
/// `CriterionOutcomes`; issue codes add qualitative evidence and never replace
/// host policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityIssueCode {
    AssignedLabelUnsupported,
    CompetingLabelAmbiguity,
    DimensionNonAdherence,
    AuthenticityNonAdherence,
    LabelLeakage,
    ShortcutArtifact,
    InternalContradiction,
    TemplateArtifact,
    UnnaturalLanguage,
    InsufficientContext,
    PotentiallySensitiveData,
    HarmfulContent,
}

/// Strict provider-normalized response for one blind row.  Unknown JSON fields
/// are rejected so a provider cannot smuggle replacement text or target facts
/// into persisted evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowAssessmentDraft {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub label_scores: BTreeMap<String, BasisPoints>,
    pub dimension_scores: BTreeMap<String, BTreeMap<String, BasisPoints>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity_score: Option<BasisPoints>,
    pub label_leakage_risk: BasisPoints,
    pub shortcut_risk: BasisPoints,
    pub confidence: BasisPoints,
    #[serde(default)]
    pub issue_codes: Vec<QualityIssueCode>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionOutcome {
    Pass,
    Borderline,
    Fail,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityVerdict {
    Qualified,
    Borderline,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionOutcomes {
    pub assigned_label: CriterionOutcome,
    pub label_margin: CriterionOutcome,
    pub dimensions: BTreeMap<String, CriterionOutcome>,
    pub authenticity: CriterionOutcome,
    pub label_leakage: CriterionOutcome,
    pub shortcut_risk: CriterionOutcome,
    pub confidence: CriterionOutcome,
}

impl CriterionOutcomes {
    pub fn verdict(&self) -> QualityVerdict {
        let outcomes = [
            self.assigned_label,
            self.label_margin,
            self.authenticity,
            self.label_leakage,
            self.shortcut_risk,
            self.confidence,
        ]
        .into_iter()
        .chain(self.dimensions.values().copied());
        let mut saw_borderline = false;
        for outcome in outcomes {
            match outcome {
                CriterionOutcome::Fail => return QualityVerdict::Quarantined,
                CriterionOutcome::Borderline => saw_borderline = true,
                CriterionOutcome::Pass | CriterionOutcome::NotApplicable => {}
            }
        }
        if saw_borderline {
            QualityVerdict::Borderline
        } else {
            QualityVerdict::Qualified
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowQualityAssessment {
    pub id: Uuid,
    pub schema_version: u32,
    pub audit_plan_id: Uuid,
    pub audit_plan_fingerprint: String,
    pub audit_run_id: Uuid,
    pub plan_item_fingerprint: String,
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub request_id: Uuid,
    pub request_fingerprint: String,
    pub attempt_id: Uuid,
    pub request_sequence: u32,
    pub attempt_number: u32,
    pub evaluator: EvaluatorIdentity,
    pub generator_relationship: GeneratorEvaluatorRelationship,
    pub label_scores: BTreeMap<String, BasisPoints>,
    pub assigned_label_score: BasisPoints,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strongest_competing_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strongest_competing_score: Option<BasisPoints>,
    pub assigned_label_margin: i32,
    pub dimension_scores: BTreeMap<String, BTreeMap<String, BasisPoints>>,
    pub assigned_dimension_scores: BTreeMap<String, BasisPoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity_score: Option<BasisPoints>,
    pub label_leakage_risk: BasisPoints,
    pub shortcut_risk: BasisPoints,
    pub confidence: BasisPoints,
    pub issue_codes: Vec<QualityIssueCode>,
    pub rationale: String,
    pub outcomes: CriterionOutcomes,
    pub verdict: QualityVerdict,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl RowQualityAssessment {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        plan: &AuditPlan,
        item: &AuditPlanItem,
        request: &BlindEvaluatorRequest,
        evaluator: EvaluatorIdentity,
        draft: RowAssessmentDraft,
        prior_assessments: &[Self],
        created_at: DateTime<Utc>,
    ) -> Result<Self, QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        Self::create_with_context(
            &checked,
            item,
            request,
            evaluator,
            draft,
            prior_assessments,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_context(
        checked: &CheckedAuditPlan<'_>,
        item: &AuditPlanItem,
        request: &BlindEvaluatorRequest,
        evaluator: EvaluatorIdentity,
        draft: RowAssessmentDraft,
        prior_assessments: &[Self],
        created_at: DateTime<Utc>,
    ) -> Result<Self, QualityError> {
        let plan = checked.plan();
        validate_binding(checked, item, request, &evaluator, &draft)?;
        validate_review_independence(
            checked,
            item,
            request,
            &evaluator,
            prior_assessments,
            created_at,
        )?;
        validate_score_shape(plan, &draft)?;
        let issue_codes = validate_issue_codes(draft.issue_codes)?;
        let rationale = bounded_required(
            draft.rationale,
            "assessment.rationale",
            MAX_RATIONALE_CHARACTERS,
        )?;

        let assigned_label_score = draft.label_scores[&item.cell.label];
        let strongest_competing = draft
            .label_scores
            .iter()
            .filter(|(label, _)| *label != &item.cell.label)
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)));
        let (strongest_competing_label, strongest_competing_score, assigned_label_margin) =
            match strongest_competing {
                Some((label, score)) => (
                    Some(label.clone()),
                    Some(*score),
                    i32::from(assigned_label_score.get()) - i32::from(score.get()),
                ),
                None => (None, None, 0),
            };
        let assigned_dimension_scores = item
            .cell
            .dimensions
            .iter()
            .map(|(dimension, assigned)| {
                (
                    dimension.clone(),
                    draft.dimension_scores[dimension][assigned],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let outcomes = derive_outcomes(
            plan,
            assigned_label_score,
            strongest_competing_score,
            assigned_label_margin,
            &assigned_dimension_scores,
            draft.authenticity_score,
            draft.label_leakage_risk,
            draft.shortcut_risk,
            draft.confidence,
        );
        let verdict = outcomes.verdict();
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: ASSESSMENT_SCHEMA_VERSION,
            audit_plan_id: plan.id,
            audit_plan_fingerprint: plan.fingerprint.clone(),
            audit_run_id: request.audit_run_id,
            plan_item_fingerprint: fingerprint(item)?,
            source_row_id: item.source_row_id,
            source_row_fingerprint: item.source_row_fingerprint.clone(),
            request_id: request.id,
            request_fingerprint: request.fingerprint.clone(),
            attempt_id: request.attempt_id,
            request_sequence: request.request_sequence,
            attempt_number: request.attempt_number,
            generator_relationship: generator_relationship(item, &evaluator),
            evaluator,
            label_scores: draft.label_scores,
            assigned_label_score,
            strongest_competing_label,
            strongest_competing_score,
            assigned_label_margin,
            dimension_scores: draft.dimension_scores,
            assigned_dimension_scores,
            authenticity_score: draft.authenticity_score,
            label_leakage_risk: draft.label_leakage_risk,
            shortcut_risk: draft.shortcut_risk,
            confidence: draft.confidence,
            issue_codes,
            rationale,
            outcomes,
            verdict,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssessmentSequenceSummary {
    pub effective_verdict: QualityVerdict,
    pub completed_required_reviews: usize,
    pub required_reviews: usize,
    pub conflicting_evidence: bool,
}

impl AssessmentSequenceSummary {
    pub const fn is_complete(self) -> bool {
        self.completed_required_reviews == self.required_reviews
    }
}

/// Validates the complete immutable evidence sequence for one source row.
/// Reviewers are positional: the first configured independent identity owns
/// the first follow-up, and so on. This makes retries and later reproduction
/// deterministic rather than trusting caller-provided slice ordering.
pub fn verify_assessment_sequence(
    plan: &AuditPlan,
    run_id: Uuid,
    primary_evaluator: &EvaluatorIdentity,
    independent_reviewers: &[EvaluatorIdentity],
    assessments: &[&RowQualityAssessment],
) -> Result<AssessmentSequenceSummary, QualityError> {
    let checked = CheckedAuditPlan::new(plan)?;
    verify_assessment_sequence_with_context(
        &checked,
        run_id,
        primary_evaluator,
        independent_reviewers,
        assessments,
    )
}

pub fn verify_assessment_sequence_with_context(
    checked: &CheckedAuditPlan<'_>,
    run_id: Uuid,
    primary_evaluator: &EvaluatorIdentity,
    independent_reviewers: &[EvaluatorIdentity],
    assessments: &[&RowQualityAssessment],
) -> Result<AssessmentSequenceSummary, QualityError> {
    if assessments.is_empty() || run_id.is_nil() {
        return Err(QualityError::Integrity(
            "an assessment sequence requires a run and at least one assessment".into(),
        ));
    }
    primary_evaluator.validate()?;
    for reviewer in independent_reviewers {
        reviewer.validate()?;
    }

    let source_row_id = assessments[0].source_row_id;
    let source_row_fingerprint = &assessments[0].source_row_fingerprint;
    let mut ids = BTreeSet::new();
    let mut requests = BTreeSet::new();
    let mut attempts = BTreeSet::new();
    let mut primary = Vec::new();
    let mut reviews = Vec::new();
    for assessment in assessments {
        assessment.verify_with_context(checked)?;
        if assessment.audit_run_id != run_id
            || assessment.source_row_id != source_row_id
            || &assessment.source_row_fingerprint != source_row_fingerprint
            || !ids.insert(assessment.id)
            || !requests.insert(assessment.request_id)
            || !attempts.insert(assessment.attempt_id)
        {
            return Err(QualityError::Integrity(
                "assessment sequence crosses rows or runs, or repeats evidence identity".into(),
            ));
        }
        match assessment.evaluator.independence {
            EvaluatorIndependence::Primary => primary.push(*assessment),
            EvaluatorIndependence::IndependentReview => reviews.push(*assessment),
        }
    }
    if primary.len() != 1 || primary[0].evaluator != *primary_evaluator {
        return Err(QualityError::Integrity(
            "assessment sequence must contain exactly the run's pinned primary evaluator".into(),
        ));
    }
    let primary = primary[0];
    reviews.sort_by_key(|assessment| {
        (
            assessment.request_sequence,
            assessment.created_at,
            assessment.id,
        )
    });
    if reviews.len() > independent_reviewers.len() {
        return Err(QualityError::Integrity(
            "assessment sequence exceeds its pinned independent-review identities".into(),
        ));
    }
    if primary.verdict != QualityVerdict::Borderline && !reviews.is_empty() {
        return Err(QualityError::Integrity(
            "only a fully verified borderline primary may receive independent reviews".into(),
        ));
    }
    for (index, review) in reviews.iter().enumerate() {
        if review.evaluator != independent_reviewers[index]
            || review.request_sequence <= primary.request_sequence
            || review.created_at <= primary.created_at
            || (index > 0
                && (review.request_sequence <= reviews[index - 1].request_sequence
                    || review.created_at <= reviews[index - 1].created_at))
        {
            return Err(QualityError::Integrity(
                "independent assessment order or evaluator identity does not match the pinned review sequence"
                    .into(),
            ));
        }
    }
    let required_reviews = if primary.verdict == QualityVerdict::Borderline {
        independent_reviewers.len()
    } else {
        0
    };
    let verdicts = assessments
        .iter()
        .map(|assessment| assessment.verdict)
        .collect::<BTreeSet<_>>();
    Ok(AssessmentSequenceSummary {
        effective_verdict: assessments
            .iter()
            .map(|assessment| assessment.verdict)
            .max()
            .expect("non-empty assessment sequence"),
        completed_required_reviews: reviews.len(),
        required_reviews,
        conflicting_evidence: verdicts.len() > 1,
    })
}

fn validate_binding(
    checked: &CheckedAuditPlan<'_>,
    item: &AuditPlanItem,
    request: &BlindEvaluatorRequest,
    evaluator: &EvaluatorIdentity,
    draft: &RowAssessmentDraft,
) -> Result<(), QualityError> {
    evaluator.validate()?;
    request.verify_with_context(checked, evaluator)?;
    if item.selection != AuditSelection::Selected || checked.item(item.source_row_id) != Some(item)
    {
        return Err(QualityError::Integrity(
            "assessment plan, item, request, or evaluator binding does not reproduce".into(),
        ));
    }
    let request_row = request
        .rows
        .iter()
        .find(|row| row.source_row_id == item.source_row_id)
        .ok_or_else(|| {
            QualityError::Integrity(format!(
                "request does not contain plan item {}",
                item.source_row_id
            ))
        })?;
    if request
        .rows
        .iter()
        .filter(|row| row.source_row_id == item.source_row_id)
        .count()
        != 1
        || request_row.source_row_fingerprint != item.source_row_fingerprint
        || request_row.plan_item_fingerprint != fingerprint(item)?
        || draft.source_row_id != item.source_row_id
        || draft.source_row_fingerprint != item.source_row_fingerprint
    {
        return Err(QualityError::Integrity(
            "assessment response is not bound to the exact requested source row".into(),
        ));
    }
    Ok(())
}

fn validate_review_independence(
    checked: &CheckedAuditPlan<'_>,
    item: &AuditPlanItem,
    request: &BlindEvaluatorRequest,
    evaluator: &EvaluatorIdentity,
    prior: &[RowQualityAssessment],
    created_at: DateTime<Utc>,
) -> Result<(), QualityError> {
    let plan = checked.plan();
    let mut assessment_ids = BTreeSet::new();
    let mut request_ids = BTreeSet::new();
    let mut attempt_ids = BTreeSet::new();
    let mut evaluator_fingerprints = BTreeSet::new();
    for assessment in prior {
        assessment.verify_with_context(checked)?;
        if assessment.audit_run_id != request.audit_run_id
            || assessment.source_row_id != item.source_row_id
            || assessment.source_row_fingerprint != item.source_row_fingerprint
            || !assessment_ids.insert(assessment.id)
            || !request_ids.insert(assessment.request_id)
            || !attempt_ids.insert(assessment.attempt_id)
            || !evaluator_fingerprints.insert(assessment.evaluator.fingerprint.clone())
            || assessment.created_at >= created_at
            || assessment.request_sequence >= request.request_sequence
        {
            return Err(QualityError::Integrity(
                "prior assessment is duplicate, non-canonical, or belongs to another run or plan item"
                    .into(),
            ));
        }
    }
    if prior.is_empty() {
        if evaluator.independence != EvaluatorIndependence::Primary {
            return Err(QualityError::Validation(
                "an initial assessment must use a primary evaluator identity".into(),
            ));
        }
        return Ok(());
    }

    let maximum_additional = match plan.policy.borderline_review_policy {
        BorderlineReviewPolicy::None => {
            return Err(QualityError::Validation(
                "the pinned policy does not permit additional assessments".into(),
            ));
        }
        BorderlineReviewPolicy::Independent {
            maximum_additional_assessments,
        } => maximum_additional_assessments,
    };
    if prior.len() > maximum_additional as usize {
        return Err(QualityError::Validation(
            "the pinned independent-review assessment limit is exhausted".into(),
        ));
    }
    let primary = prior
        .iter()
        .filter(|assessment| assessment.evaluator.independence == EvaluatorIndependence::Primary)
        .collect::<Vec<_>>();
    if primary.len() != 1
        || prior
            .iter()
            .filter(|assessment| {
                assessment.evaluator.independence == EvaluatorIndependence::IndependentReview
            })
            .count()
            != prior.len() - 1
        || primary[0].verdict != QualityVerdict::Borderline
        || evaluator.independence != EvaluatorIndependence::IndependentReview
    {
        return Err(QualityError::Validation(
            "additional evidence requires a borderline primary assessment and an independent reviewer"
                .into(),
        ));
    }
    if prior.iter().any(|assessment| {
        assessment.evaluator.fingerprint == evaluator.fingerprint
            || assessment.evaluator.configuration_fingerprint == evaluator.configuration_fingerprint
            || (assessment.evaluator.backend == evaluator.backend
                && assessment.evaluator.model == evaluator.model)
    }) {
        return Err(QualityError::Validation(
            "independent review must use a distinct evaluator configuration and backend/model identity"
                .into(),
        ));
    }
    Ok(())
}

fn validate_score_shape(plan: &AuditPlan, draft: &RowAssessmentDraft) -> Result<(), QualityError> {
    let expected_labels = plan
        .dataset_schema
        .labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_labels = draft
        .label_scores
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual_labels != expected_labels {
        return Err(QualityError::Validation(format!(
            "assessment label score shape mismatch: expected {expected_labels:?}, got {actual_labels:?}"
        )));
    }
    let expected_dimensions = plan
        .dataset_schema
        .dimensions
        .iter()
        .map(|dimension| dimension.name.as_str())
        .collect::<BTreeSet<_>>();
    let actual_dimensions = draft
        .dimension_scores
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual_dimensions != expected_dimensions {
        return Err(QualityError::Validation(format!(
            "assessment dimension score shape mismatch: expected {expected_dimensions:?}, got {actual_dimensions:?}"
        )));
    }
    for dimension in &plan.dataset_schema.dimensions {
        let actual_values = draft.dimension_scores[&dimension.name]
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected_values = dimension
            .values
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_values != expected_values {
            return Err(QualityError::Validation(format!(
                "assessment score shape mismatch for dimension {:?}: expected {expected_values:?}, got {actual_values:?}",
                dimension.name
            )));
        }
    }
    let expects_authenticity = plan.guidance.authenticity_context.is_some();
    if draft.authenticity_score.is_some() != expects_authenticity {
        return Err(QualityError::Validation(format!(
            "assessment authenticity score must be {} when the audit plan {} authenticity guidance",
            if expects_authenticity {
                "present"
            } else {
                "absent"
            },
            if expects_authenticity {
                "pins"
            } else {
                "does not pin"
            }
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn derive_outcomes(
    plan: &AuditPlan,
    assigned_label_score: BasisPoints,
    strongest_competing_score: Option<BasisPoints>,
    assigned_label_margin: i32,
    assigned_dimension_scores: &BTreeMap<String, BasisPoints>,
    authenticity_score: Option<BasisPoints>,
    label_leakage_risk: BasisPoints,
    shortcut_risk: BasisPoints,
    confidence: BasisPoints,
) -> CriterionOutcomes {
    let thresholds = &plan.policy.thresholds;
    let borderline = thresholds.borderline_margin;
    let dimensions = assigned_dimension_scores
        .iter()
        .map(|(dimension, score)| {
            (
                dimension.clone(),
                classify_minimum(
                    *score,
                    thresholds.minimum_dimension_adherence_score,
                    borderline,
                ),
            )
        })
        .collect();
    CriterionOutcomes {
        assigned_label: classify_minimum(
            assigned_label_score,
            thresholds.minimum_assigned_label_score,
            borderline,
        ),
        label_margin: strongest_competing_score.map_or(CriterionOutcome::NotApplicable, |_| {
            classify_signed_minimum(
                assigned_label_margin,
                i32::from(thresholds.minimum_label_margin.get()),
                i32::from(borderline.get()),
            )
        }),
        dimensions,
        authenticity: match (authenticity_score, thresholds.minimum_authenticity_score) {
            (Some(actual), Some(minimum)) => classify_minimum(actual, minimum, borderline),
            _ => CriterionOutcome::NotApplicable,
        },
        label_leakage: classify_maximum(
            label_leakage_risk,
            thresholds.maximum_label_leakage_risk,
            borderline,
        ),
        shortcut_risk: classify_maximum(
            shortcut_risk,
            thresholds.maximum_shortcut_risk,
            borderline,
        ),
        confidence: classify_minimum(
            confidence,
            thresholds.minimum_evaluator_confidence,
            borderline,
        ),
    }
}

fn classify_minimum(
    actual: BasisPoints,
    threshold: BasisPoints,
    borderline_margin: BasisPoints,
) -> CriterionOutcome {
    classify_signed_minimum(
        i32::from(actual.get()),
        i32::from(threshold.get()),
        i32::from(borderline_margin.get()),
    )
}

fn classify_signed_minimum(
    actual: i32,
    threshold: i32,
    borderline_margin: i32,
) -> CriterionOutcome {
    if actual >= threshold {
        CriterionOutcome::Pass
    } else if actual >= threshold - borderline_margin {
        CriterionOutcome::Borderline
    } else {
        CriterionOutcome::Fail
    }
}

fn classify_maximum(
    actual: BasisPoints,
    threshold: BasisPoints,
    borderline_margin: BasisPoints,
) -> CriterionOutcome {
    if actual.get() <= threshold.get() {
        CriterionOutcome::Pass
    } else if u32::from(actual.get())
        <= u32::from(threshold.get()) + u32::from(borderline_margin.get())
    {
        CriterionOutcome::Borderline
    } else {
        CriterionOutcome::Fail
    }
}

fn validate_issue_codes(
    mut issue_codes: Vec<QualityIssueCode>,
) -> Result<Vec<QualityIssueCode>, QualityError> {
    if issue_codes.len() > MAX_ISSUE_CODES {
        return Err(QualityError::Validation(format!(
            "assessment contains more than {MAX_ISSUE_CODES} issue codes"
        )));
    }
    let original_len = issue_codes.len();
    issue_codes.sort_unstable();
    issue_codes.dedup();
    if issue_codes.len() != original_len {
        return Err(QualityError::Validation(
            "assessment issue codes must not contain duplicates".into(),
        ));
    }
    Ok(issue_codes)
}

fn generator_relationship(
    item: &AuditPlanItem,
    evaluator: &EvaluatorIdentity,
) -> GeneratorEvaluatorRelationship {
    match &item.provenance_stratum {
        ProvenanceStratum::Imported { .. } => GeneratorEvaluatorRelationship::NotApplicable,
        ProvenanceStratum::Generated { backend, model, .. } if backend == &evaluator.backend => {
            if model == &evaluator.model {
                GeneratorEvaluatorRelationship::SharedBackendAndModel
            } else {
                GeneratorEvaluatorRelationship::SharedBackend
            }
        }
        ProvenanceStratum::Generated { .. } => GeneratorEvaluatorRelationship::IndependentBackend,
    }
}

fn validate_guidance(plan: &AuditPlan, guidance: &EvaluatorGuidance) -> Result<(), QualityError> {
    validate_guidance_reference(
        plan.guidance.semantic_context.as_ref(),
        guidance.semantic.as_ref().map(|value| &value.reference),
        "semantic",
    )?;
    validate_guidance_reference(
        plan.guidance.authenticity_context.as_ref(),
        guidance.authenticity.as_ref().map(|value| &value.reference),
        "authenticity",
    )?;
    if let Some(semantic) = &guidance.semantic {
        validate_semantic_guidance(plan, semantic)?;
    }
    if let Some(authenticity) = &guidance.authenticity {
        bounded_required(
            authenticity.summary.clone(),
            "guidance.authenticity.summary",
            MAX_GUIDANCE_ITEM_CHARACTERS,
        )?;
        validate_guidance_list(
            &authenticity.instructions,
            "guidance.authenticity.instructions",
        )?;
        validate_guidance_list(&authenticity.caveats, "guidance.authenticity.caveats")?;
    }
    Ok(())
}

fn validate_guidance_reference(
    expected: Option<&GuidanceReference>,
    actual: Option<&GuidanceReference>,
    kind: &str,
) -> Result<(), QualityError> {
    if expected != actual {
        return Err(QualityError::Integrity(format!(
            "evaluator {kind} guidance does not match the context pinned by the audit plan"
        )));
    }
    Ok(())
}

fn validate_semantic_guidance(
    plan: &AuditPlan,
    guidance: &SemanticEvaluatorGuidance,
) -> Result<(), QualityError> {
    if guidance.sources.is_empty()
        || guidance.sources.len() > MAX_GUIDANCE_ITEMS
        || guidance.sources.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(QualityError::Validation(
            "semantic guidance sources must be non-empty, unique, and canonically sorted".into(),
        ));
    }
    for source in &guidance.sources {
        source.validate()?;
    }
    if let Some(labels) = &guidance.labels {
        validate_optional_guidance_text(&labels.description, "guidance.labels.description")?;
        let allowed = plan.dataset_schema.labels.iter().collect::<BTreeSet<_>>();
        if labels.entries.keys().any(|name| !allowed.contains(name)) {
            return Err(QualityError::Validation(
                "semantic guidance contains an unknown label".into(),
            ));
        }
        for (name, value) in &labels.entries {
            validate_concept_guidance(value, &format!("guidance.labels.entries.{name}"))?;
        }
    }
    for (dimension, target) in &guidance.dimensions {
        let allowed_values = plan
            .dataset_schema
            .dimensions
            .iter()
            .find(|candidate| candidate.name == *dimension)
            .map(|candidate| &candidate.values)
            .ok_or_else(|| {
                QualityError::Validation(format!(
                    "semantic guidance contains unknown dimension {dimension:?}"
                ))
            })?;
        validate_optional_guidance_text(
            &target.description,
            &format!("guidance.dimensions.{dimension}.description"),
        )?;
        let allowed_values = allowed_values.iter().collect::<BTreeSet<_>>();
        if target
            .entries
            .keys()
            .any(|name| !allowed_values.contains(name))
        {
            return Err(QualityError::Validation(format!(
                "semantic guidance for {dimension:?} contains an unknown value"
            )));
        }
        for (name, value) in &target.entries {
            validate_concept_guidance(
                value,
                &format!("guidance.dimensions.{dimension}.entries.{name}"),
            )?;
        }
    }
    Ok(())
}

fn validate_concept_guidance(value: &ConceptGuidance, path: &str) -> Result<(), QualityError> {
    validate_optional_guidance_text(&value.description, &format!("{path}.description"))?;
    validate_guidance_list(&value.examples, &format!("{path}.examples"))?;
    validate_guidance_list(&value.counterexamples, &format!("{path}.counterexamples"))?;
    validate_guidance_list(&value.inclusion_rules, &format!("{path}.inclusion_rules"))?;
    validate_guidance_list(&value.exclusion_rules, &format!("{path}.exclusion_rules"))?;
    Ok(())
}

fn validate_optional_guidance_text(value: &Option<String>, path: &str) -> Result<(), QualityError> {
    if let Some(value) = value {
        bounded_required(value.clone(), path, MAX_GUIDANCE_ITEM_CHARACTERS)?;
    }
    Ok(())
}

fn validate_guidance_list(values: &[String], path: &str) -> Result<(), QualityError> {
    if values.len() > MAX_GUIDANCE_ITEMS {
        return Err(QualityError::Validation(format!(
            "{path} contains more than {MAX_GUIDANCE_ITEMS} entries"
        )));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        let value = bounded_required(value.clone(), path, MAX_GUIDANCE_ITEM_CHARACTERS)?;
        if !seen.insert(value) {
            return Err(QualityError::Validation(format!(
                "{path} contains duplicate guidance"
            )));
        }
    }
    Ok(())
}

fn bounded_required(
    value: impl Into<String>,
    field: &str,
    maximum_characters: usize,
) -> Result<String, QualityError> {
    let value = required(value, field)?;
    if value.chars().count() > maximum_characters {
        return Err(QualityError::Validation(format!(
            "{field} exceeds {maximum_characters} characters"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::domain::{SourceProvenance, SourceRow};
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use serde_json::Value;

    use crate::{
        policy::{
            AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, GuidanceReferences},
    };

    use super::*;

    fn bp(value: u16) -> BasisPoints {
        BasisPoints::new(value).expect("valid basis points")
    }

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::with_identity(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").expect("dataset ID"),
            "support",
            "Classify support requests",
            vec!["fraud".into(), "billing".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["hard".into(), "easy".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset")
    }

    fn row(dataset_id: Uuid) -> SourceRow {
        SourceRow {
            id: Uuid::parse_str("20000000-0000-4000-8000-000000000001").expect("row ID"),
            dataset_id,
            text: "Why was I charged twice?".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Generated {
                generation_job_id: Uuid::parse_str("30000000-0000-4000-8000-000000000001")
                    .expect("job ID"),
                backend: "generator-backend".into(),
                model: "generator-model".into(),
                construction_plan_fingerprint: None,
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("time"),
        }
    }

    fn plan() -> (AuditPlan, SourceRow) {
        let dataset = dataset();
        let row = row(dataset.id);
        let policy = QualityPreset::Balanced
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("policy");
        let guidance = EvaluatorGuidance::default();
        let resolved_guidance_fingerprint = guidance
            .reproduce_fingerprint()
            .expect("empty guidance fingerprint");
        let plan = AuditPlan::with_identity(
            Uuid::parse_str("40000000-0000-4000-8000-000000000001").expect("plan ID"),
            &dataset,
            policy,
            GuidanceReferences::default(),
            resolved_guidance_fingerprint,
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
        row: &SourceRow,
        discriminator: u128,
        evaluator: &EvaluatorIdentity,
    ) -> BlindEvaluatorRequest {
        let guidance = EvaluatorGuidance::default();
        BlindEvaluatorRequest::create(
            plan,
            Uuid::from_u128(0x50000000000040008000000000000000 + discriminator),
            Uuid::parse_str("51000000-0000-4000-8000-000000000001").expect("run ID"),
            Uuid::from_u128(0x52000000000040008000000000000000 + discriminator),
            u32::try_from(discriminator).expect("request sequence"),
            1,
            evaluator,
            vec![row.clone()],
            guidance,
            plan.resolved_guidance_fingerprint.clone(),
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 10_000,
                maximum_total_tokens: 20_000,
                maximum_cost_microusd: None,
            },
        )
        .expect("request")
    }

    fn evaluator(independence: EvaluatorIndependence) -> EvaluatorIdentity {
        EvaluatorIdentity::new(
            "quality-backend",
            "quality-model",
            "quality-evaluator-v1",
            "sha256:quality-configuration",
            independence,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator")
    }

    fn draft(
        item: &AuditPlanItem,
        assigned_score: u16,
        competing_score: u16,
    ) -> RowAssessmentDraft {
        RowAssessmentDraft {
            source_row_id: item.source_row_id,
            source_row_fingerprint: item.source_row_fingerprint.clone(),
            label_scores: BTreeMap::from([
                ("billing".into(), bp(assigned_score)),
                ("fraud".into(), bp(competing_score)),
            ]),
            dimension_scores: BTreeMap::from([(
                "difficulty".into(),
                BTreeMap::from([("easy".into(), bp(8_500)), ("hard".into(), bp(1_500))]),
            )]),
            authenticity_score: None,
            label_leakage_risk: bp(1_000),
            shortcut_risk: bp(1_000),
            confidence: bp(8_000),
            issue_codes: vec![],
            rationale: "The text supports the scored label ranking.".into(),
        }
    }

    fn assess(
        plan: &AuditPlan,
        row: &SourceRow,
        draft: RowAssessmentDraft,
        discriminator: u128,
    ) -> Result<RowQualityAssessment, QualityError> {
        let evaluator = evaluator(EvaluatorIndependence::Primary);
        let request = request(plan, row, discriminator, &evaluator);
        RowQualityAssessment::create(
            plan,
            &plan.items[0],
            &request,
            evaluator,
            draft,
            &[],
            Utc.with_ymd_and_hms(2026, 1, 4, 0, 0, discriminator as u32)
                .single()
                .expect("time"),
        )
    }

    #[test]
    fn structurally_plausible_but_semantically_wrong_row_is_quarantined() {
        let (plan, row) = plan();
        let item = &plan.items[0];
        // The source says "billing", while the blind evaluator finds very
        // strong support for "fraud". The host, not the evaluator, discovers
        // and applies that mismatch.
        let assessment =
            assess(&plan, &row, draft(item, 1_000, 9_500), 1).expect("valid assessment evidence");

        assert_eq!(assessment.assigned_label_score, bp(1_000));
        assert_eq!(
            assessment.strongest_competing_label.as_deref(),
            Some("fraud")
        );
        assert_eq!(assessment.assigned_label_margin, -8_500);
        assert_eq!(assessment.outcomes.assigned_label, CriterionOutcome::Fail);
        assert_eq!(assessment.verdict, QualityVerdict::Quarantined);
        assert!(assessment.verify_integrity(&plan).is_ok());
    }

    #[test]
    fn malformed_partial_and_extra_responses_are_rejected() {
        let (plan, row) = plan();
        let item = &plan.items[0];

        let mut missing_label = draft(item, 8_500, 1_000);
        missing_label.label_scores.remove("fraud");
        assert!(assess(&plan, &row, missing_label, 2).is_err());

        let mut extra_label = draft(item, 8_500, 1_000);
        extra_label.label_scores.insert("account".into(), bp(5_000));
        assert!(assess(&plan, &row, extra_label, 3).is_err());

        let mut partial_dimension = draft(item, 8_500, 1_000);
        partial_dimension
            .dimension_scores
            .get_mut("difficulty")
            .expect("dimension")
            .remove("hard");
        assert!(assess(&plan, &row, partial_dimension, 4).is_err());

        let mut encoded = serde_json::to_value(draft(item, 8_500, 1_000)).expect("JSON draft");
        encoded
            .as_object_mut()
            .expect("object")
            .insert("replacement_label".into(), Value::String("fraud".into()));
        assert!(serde_json::from_value::<RowAssessmentDraft>(encoded).is_err());
    }

    #[test]
    fn blind_request_row_omits_assigned_target_facts() {
        let (plan, row) = plan();
        let evaluator = evaluator(EvaluatorIndependence::Primary);
        let request = request(&plan, &row, 5, &evaluator);
        let encoded = serde_json::to_value(&request).expect("JSON request");
        let row_object = encoded["rows"][0].as_object().expect("request row object");

        assert_eq!(
            row_object.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "plan_item_fingerprint".into(),
                "source_row_fingerprint".into(),
                "source_row_id".into(),
                "text".into(),
            ])
        );
        assert!(!row_object.contains_key("label"));
        assert!(!row_object.contains_key("dimensions"));
        assert!(!row_object.contains_key("cell"));
        assert!(!row_object.contains_key("provenance"));
        assert_eq!(request.allowed_labels, vec!["billing", "fraud"]);
        assert_eq!(
            request.allowed_dimensions["difficulty"],
            vec!["easy", "hard"]
        );
    }

    #[test]
    fn integer_threshold_edges_are_deterministic() {
        let (plan, row) = plan();
        let item = &plan.items[0];

        let qualified = assess(&plan, &row, draft(item, 8_000, 6_800), 6).expect("qualified");
        assert_eq!(qualified.outcomes.assigned_label, CriterionOutcome::Pass);
        assert_eq!(qualified.outcomes.label_margin, CriterionOutcome::Pass);
        assert_eq!(qualified.verdict, QualityVerdict::Qualified);

        let borderline = assess(&plan, &row, draft(item, 7_400, 6_200), 7).expect("borderline");
        assert_eq!(
            borderline.outcomes.assigned_label,
            CriterionOutcome::Borderline
        );
        assert_eq!(borderline.verdict, QualityVerdict::Borderline);

        let quarantined = assess(&plan, &row, draft(item, 7_399, 6_199), 8)
            .expect("valid below-threshold evidence");
        assert_eq!(quarantined.outcomes.assigned_label, CriterionOutcome::Fail);
        assert_eq!(quarantined.verdict, QualityVerdict::Quarantined);
    }

    #[test]
    fn repeated_borderline_evidence_requires_a_distinct_independent_evaluator() {
        let (plan, row) = plan();
        let item = &plan.items[0];
        let primary = assess(&plan, &row, draft(item, 7_400, 6_200), 9).expect("primary");
        assert_eq!(primary.verdict, QualityVerdict::Borderline);

        let same_configuration = EvaluatorIdentity::new(
            "quality-backend",
            "quality-model",
            "quality-evaluator-v1",
            "sha256:quality-configuration",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("identity");
        let same_configuration_request = request(&plan, &row, 10, &same_configuration);
        assert!(
            RowQualityAssessment::create(
                &plan,
                item,
                &same_configuration_request,
                same_configuration,
                draft(item, 8_500, 1_000),
                std::slice::from_ref(&primary),
                Utc.with_ymd_and_hms(2026, 1, 4, 0, 1, 0)
                    .single()
                    .expect("time"),
            )
            .is_err()
        );

        let independent = EvaluatorIdentity::new(
            "independent-backend",
            "independent-model",
            "quality-evaluator-v1",
            "sha256:independent-configuration",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("identity");
        let independent_request = request(&plan, &row, 11, &independent);
        let followup = RowQualityAssessment::create(
            &plan,
            item,
            &independent_request,
            independent,
            draft(item, 8_500, 1_000),
            &[primary],
            Utc.with_ymd_and_hms(2026, 1, 4, 0, 1, 1)
                .single()
                .expect("time"),
        )
        .expect("independent follow-up");
        assert_eq!(followup.verdict, QualityVerdict::Qualified);
    }

    #[test]
    fn integrity_verification_rejects_rehashed_derived_field_tampering() {
        let (plan, row) = plan();
        let item = &plan.items[0];
        let mut assessment =
            assess(&plan, &row, draft(item, 8_500, 1_000), 11).expect("assessment");

        assessment.assigned_label_score = bp(9_999);
        // An attacker able to rewrite storage can also recompute a plain hash;
        // integrity verification must therefore recompute host-derived facts.
        assessment.fingerprint = assessment
            .reproduce_fingerprint()
            .expect("rehashed artifact");

        assert!(assessment.verify_integrity(&plan).is_err());
    }

    #[test]
    fn request_rejects_rehashed_guidance_binding_tampering() {
        let (plan, row) = plan();
        let evaluator = evaluator(EvaluatorIndependence::Primary);
        let mut request = request(&plan, &row, 12, &evaluator);

        request.resolved_guidance_fingerprint = "sha256:forged-guidance".into();
        request.fingerprint = request
            .reproduce_fingerprint()
            .expect("rehashed request artifact");

        assert!(request.verify_integrity(&plan, &evaluator).is_err());
    }

    #[test]
    fn persisted_assessment_rejects_unknown_fields() {
        let (plan, row) = plan();
        let item = &plan.items[0];
        let assessment = assess(&plan, &row, draft(item, 8_500, 1_000), 13).expect("assessment");
        let mut encoded = serde_json::to_value(assessment).expect("assessment JSON");
        encoded
            .as_object_mut()
            .expect("assessment object")
            .insert("replacement_text".into(), Value::String("tampered".into()));

        assert!(serde_json::from_value::<RowQualityAssessment>(encoded).is_err());
    }
}
