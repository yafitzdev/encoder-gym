//! Durable audit-run and evaluator-attempt state machines.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    QualityError,
    assessment::{
        BlindEvaluatorRequest, EvaluatorExecutionLocation, EvaluatorIdentity,
        EvaluatorIndependence, QualityVerdict, RowQualityAssessment,
        verify_assessment_sequence_with_context,
    },
    bounded_required, fingerprint,
    policy::{AuditBudgets, EvaluatorEgressPolicy, InvalidEvaluatorOutputPolicy},
    population::{AUDIT_PLAN_SCHEMA_VERSION, AuditPlan, AuditSelection, CheckedAuditPlan},
};

pub const QUALITY_AUDIT_RUN_SCHEMA_VERSION: u32 = 1;
pub const EVALUATOR_ATTEMPT_SCHEMA_VERSION: u32 = 1;
const MAX_RESPONSE_METADATA_BYTES: usize = 64 * 1024;
const MAX_ERROR_MESSAGE_CHARACTERS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityAuditRunState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityAuditStopReason {
    Completed,
    Cancelled,
    BudgetExhausted,
    ProviderFailure,
    InvalidResponse,
    Interrupted,
}

/// Durable ownership of one local audit execution. The process identity makes
/// abandoned work detectable after a crash, while the invocation token fences
/// concurrent tasks inside the same live process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditExecutionLease {
    pub run_id: Uuid,
    pub invocation_token: Uuid,
    pub process_id: u32,
    pub process_started_at: u64,
    pub acquired_at: DateTime<Utc>,
}

impl AuditExecutionLease {
    pub fn create(
        run_id: Uuid,
        invocation_token: Uuid,
        process_id: u32,
        process_started_at: u64,
        acquired_at: DateTime<Utc>,
    ) -> Result<Self, QualityError> {
        let value = Self {
            run_id,
            invocation_token,
            process_id,
            process_started_at,
            acquired_at,
        };
        value.verify()?;
        Ok(value)
    }

    pub fn verify(&self) -> Result<(), QualityError> {
        if self.run_id.is_nil()
            || self.invocation_token.is_nil()
            || self.process_id == 0
            || self.process_started_at == 0
        {
            return Err(QualityError::Integrity(
                "audit execution lease identity is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditProgress {
    pub population_rows: u64,
    pub selected_rows: u64,
    pub assessed_rows: u64,
    pub qualified_rows: u64,
    pub borderline_rows: u64,
    pub quarantined_rows: u64,
    pub invalid_rows: u64,
    /// Rows whose primary verdict requires configured independent evidence.
    pub pending_review_rows: u64,
}

impl AuditProgress {
    pub fn validate(self) -> Result<(), QualityError> {
        if self.selected_rows > self.population_rows
            || self.assessed_rows.saturating_add(self.invalid_rows) > self.selected_rows
            || self
                .qualified_rows
                .saturating_add(self.borderline_rows)
                .saturating_add(self.quarantined_rows)
                != self.assessed_rows
            || self.pending_review_rows > self.assessed_rows
        {
            return Err(QualityError::Integrity(
                "audit progress counters do not reconcile".into(),
            ));
        }
        Ok(())
    }

    fn can_follow(self, previous: Self) -> bool {
        self.population_rows == previous.population_rows
            && self.selected_rows == previous.selected_rows
            && self.invalid_rows >= previous.invalid_rows
            && self.assessed_rows.saturating_add(self.invalid_rows)
                >= previous.assessed_rows.saturating_add(previous.invalid_rows)
    }

    pub fn is_complete(self) -> bool {
        self.assessed_rows.saturating_add(self.invalid_rows) == self.selected_rows
            && self.pending_review_rows == 0
    }
}

/// Usage reported by a provider. Logical request and transport-attempt counts
/// are deliberately absent: the host derives those from persisted attempts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: u64,
}

impl ProviderUsage {
    pub fn validate(self) -> Result<(), QualityError> {
        if self.total_tokens != self.input_tokens.saturating_add(self.output_tokens) {
            return Err(QualityError::Integrity(
                "provider token usage does not reconcile".into(),
            ));
        }
        Ok(())
    }

    pub fn exceeds_request(self, budget: crate::assessment::EvaluatorRequestBudget) -> bool {
        self.input_tokens > budget.maximum_input_tokens
            || self.output_tokens > budget.maximum_output_tokens
            || self.total_tokens > budget.maximum_total_tokens
            || budget
                .maximum_cost_microusd
                .is_some_and(|maximum| self.cost_microusd > maximum)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditUsage {
    pub evaluator_requests: u32,
    pub request_attempts: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: u64,
}

impl AuditUsage {
    pub fn checked_add_provider(self, provider: ProviderUsage) -> Result<Self, QualityError> {
        provider.validate()?;
        Ok(Self {
            evaluator_requests: self.evaluator_requests,
            request_attempts: self.request_attempts,
            input_tokens: self
                .input_tokens
                .checked_add(provider.input_tokens)
                .ok_or_else(|| QualityError::BudgetExhausted("input tokens overflowed".into()))?,
            output_tokens: self
                .output_tokens
                .checked_add(provider.output_tokens)
                .ok_or_else(|| QualityError::BudgetExhausted("output tokens overflowed".into()))?,
            total_tokens: self
                .total_tokens
                .checked_add(provider.total_tokens)
                .ok_or_else(|| QualityError::BudgetExhausted("total tokens overflowed".into()))?,
            cost_microusd: self
                .cost_microusd
                .checked_add(provider.cost_microusd)
                .ok_or_else(|| QualityError::BudgetExhausted("cost overflowed".into()))?,
        })
    }

    fn reserve_attempt(self, first_attempt: bool) -> Result<Self, QualityError> {
        Ok(Self {
            evaluator_requests: if first_attempt {
                self.evaluator_requests.checked_add(1).ok_or_else(|| {
                    QualityError::BudgetExhausted("request count overflowed".into())
                })?
            } else {
                self.evaluator_requests
            },
            request_attempts: self
                .request_attempts
                .checked_add(1)
                .ok_or_else(|| QualityError::BudgetExhausted("attempt count overflowed".into()))?,
            ..self
        })
    }

    pub fn validate_against(self, budgets: &AuditBudgets) -> Result<(), QualityError> {
        self.validate_shape()?;
        if self.exceeds(budgets) {
            return Err(QualityError::BudgetExhausted(
                "audit usage exceeds its persisted finite limits".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_shape(self) -> Result<(), QualityError> {
        if self.total_tokens != self.input_tokens.saturating_add(self.output_tokens) {
            return Err(QualityError::Integrity(
                "audit token usage does not reconcile".into(),
            ));
        }
        Ok(())
    }

    pub fn exceeds(self, budgets: &AuditBudgets) -> bool {
        let maximum_attempts = budgets.maximum_request_attempts();
        self.evaluator_requests > budgets.maximum_evaluator_requests
            || u64::from(self.request_attempts) > maximum_attempts
            || self.input_tokens > budgets.maximum_input_tokens
            || self.output_tokens > budgets.maximum_output_tokens
            || self.total_tokens > budgets.maximum_total_tokens
            || budgets
                .maximum_cost_microusd
                .is_some_and(|maximum| self.cost_microusd > maximum)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityAuditRun {
    pub id: Uuid,
    pub schema_version: u32,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub source_set_fingerprint: String,
    pub policy_fingerprint: String,
    pub primary_evaluator: EvaluatorIdentity,
    pub independent_reviewers: Vec<EvaluatorIdentity>,
    pub state: QualityAuditRunState,
    pub progress: AuditProgress,
    pub usage: AuditUsage,
    pub cancel_requested: bool,
    pub stop_reason: Option<QualityAuditStopReason>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub specification_fingerprint: String,
}

/// The bounded evidence needed to reproduce one durable attempt completion.
///
/// `prior_assessments` and `prior_invalid_attempts` contain only evidence for
/// rows in `request`; callers remain responsible for loading that complete
/// row-local history from their checked persistence boundary.
pub struct FinishedAttemptEvidence<'a> {
    pub request: &'a BlindEvaluatorRequest,
    pub previous_attempt: &'a EvaluatorAttempt,
    pub finished_attempt: &'a EvaluatorAttempt,
    pub prior_assessments: &'a [RowQualityAssessment],
    pub prior_invalid_attempts: &'a [EvaluatorAttempt],
    pub new_assessments: &'a [RowQualityAssessment],
}

/// Bounded immutable plan facts needed by status/watch surfaces. Unlike an
/// [`AuditPlan`], this projection never carries the full population manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPlanStatusProjection {
    pub id: Uuid,
    pub schema_version: u32,
    pub dataset_definition_id: Uuid,
    pub dataset_definition_fingerprint: String,
    pub policy_fingerprint: String,
    pub resolved_guidance_fingerprint: String,
    pub evaluator_protocol_version: String,
    pub population_rows: u64,
    pub selected_rows: u64,
    pub source_set_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Checked bounded audit facts for high-frequency operator polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditStatusProjection {
    pub run: QualityAuditRun,
    pub plan: AuditPlanStatusProjection,
    pub evaluator_requests: u64,
    pub evaluator_attempts: u64,
    pub assessments: u64,
    pub report_id: Option<Uuid>,
}

impl AuditStatusProjection {
    pub fn verify(&self) -> Result<(), QualityError> {
        self.run.progress.validate()?;
        self.run.usage.validate_shape()?;
        self.run.primary_evaluator.validate()?;
        for reviewer in &self.run.independent_reviewers {
            reviewer.validate()?;
        }
        let plan = &self.plan;
        let required_plan_strings = [
            plan.dataset_definition_fingerprint.as_str(),
            plan.policy_fingerprint.as_str(),
            plan.resolved_guidance_fingerprint.as_str(),
            plan.evaluator_protocol_version.as_str(),
            plan.source_set_fingerprint.as_str(),
            plan.fingerprint.as_str(),
        ];
        if plan.id.is_nil()
            || plan.schema_version != AUDIT_PLAN_SCHEMA_VERSION
            || plan.dataset_definition_id.is_nil()
            || required_plan_strings
                .iter()
                .any(|value| value.trim().is_empty())
            || plan.population_rows == 0
            || plan.selected_rows == 0
            || plan.selected_rows > plan.population_rows
            || self.run.plan_id != plan.id
            || self.run.plan_fingerprint != plan.fingerprint
            || self.run.policy_fingerprint != plan.policy_fingerprint
            || self.run.source_set_fingerprint != plan.source_set_fingerprint
            || self.run.progress.population_rows != plan.population_rows
            || self.run.progress.selected_rows != plan.selected_rows
            || self.run.schema_version != QUALITY_AUDIT_RUN_SCHEMA_VERSION
            || self.run.primary_evaluator.protocol_version != plan.evaluator_protocol_version
            || self
                .run
                .independent_reviewers
                .iter()
                .any(|reviewer| reviewer.protocol_version != plan.evaluator_protocol_version)
            || self.run.created_at < plan.created_at
            || self.run.specification_fingerprint.is_empty()
            || self.run.reproduce_specification_fingerprint()? != self.run.specification_fingerprint
        {
            return Err(QualityError::Integrity(
                "audit status run and plan projection do not reproduce".into(),
            ));
        }
        let timestamps_valid = match self.run.state {
            QualityAuditRunState::Queued => {
                self.run.started_at.is_none() && self.run.finished_at.is_none()
            }
            QualityAuditRunState::Running => {
                self.run.started_at.is_some() && self.run.finished_at.is_none()
            }
            QualityAuditRunState::Completed
            | QualityAuditRunState::Failed
            | QualityAuditRunState::Cancelled => {
                self.run.started_at.is_some() && self.run.finished_at.is_some()
            }
        };
        let terminal_valid = match self.run.state {
            QualityAuditRunState::Completed => {
                self.run.stop_reason == Some(QualityAuditStopReason::Completed)
                    && self.run.error_message.is_none()
                    && !self.run.cancel_requested
                    && self.run.progress.is_complete()
            }
            QualityAuditRunState::Cancelled => {
                self.run.stop_reason == Some(QualityAuditStopReason::Cancelled)
                    && self.run.cancel_requested
                    && self.run.error_message.is_none()
            }
            QualityAuditRunState::Failed => {
                self.run.stop_reason.is_some() && self.run.error_message.is_some()
            }
            QualityAuditRunState::Queued | QualityAuditRunState::Running => {
                self.run.stop_reason.is_none() && self.run.error_message.is_none()
            }
        };
        if !timestamps_valid || !terminal_valid {
            return Err(QualityError::Integrity(
                "audit status state and timestamps are inconsistent".into(),
            ));
        }
        if self.evaluator_requests != u64::from(self.run.usage.evaluator_requests)
            || self.evaluator_attempts != u64::from(self.run.usage.request_attempts)
        {
            return Err(QualityError::Integrity(
                "audit status attempt counts disagree with durable run usage".into(),
            ));
        }
        let maximum_assessments = plan.selected_rows.checked_mul(
            u64::try_from(1 + self.run.independent_reviewers.len())
                .map_err(|_| QualityError::Integrity("reviewer count overflowed".into()))?,
        );
        if maximum_assessments.is_none_or(|maximum| self.assessments > maximum)
            || self.report_id.is_some() && self.run.state != QualityAuditRunState::Completed
        {
            return Err(QualityError::Integrity(
                "audit status assessment or report facts are impossible".into(),
            ));
        }
        Ok(())
    }
}

impl QualityAuditRun {
    pub fn queue(
        plan: &AuditPlan,
        primary_evaluator: EvaluatorIdentity,
        independent_reviewers: Vec<EvaluatorIdentity>,
    ) -> Result<Self, QualityError> {
        plan.verify_integrity()?;
        validate_evaluators(plan, &primary_evaluator, &independent_reviewers)?;
        let progress = AuditProgress {
            population_rows: plan.population_count(),
            selected_rows: plan.selected_count(),
            ..AuditProgress::default()
        };
        let mut value = Self {
            id: Uuid::new_v4(),
            schema_version: QUALITY_AUDIT_RUN_SCHEMA_VERSION,
            plan_id: plan.id,
            plan_fingerprint: plan.fingerprint.clone(),
            source_set_fingerprint: plan.source_set_fingerprint.clone(),
            policy_fingerprint: plan.policy.fingerprint.clone(),
            primary_evaluator,
            independent_reviewers,
            state: QualityAuditRunState::Queued,
            progress,
            usage: AuditUsage::default(),
            cancel_requested: false,
            stop_reason: None,
            error_message: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            specification_fingerprint: String::new(),
        };
        value.specification_fingerprint = value.reproduce_specification_fingerprint()?;
        value.verify_integrity(plan)?;
        Ok(value)
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, QualityError> {
        fingerprint(&(
            self.id,
            self.schema_version,
            self.plan_id,
            &self.plan_fingerprint,
            &self.source_set_fingerprint,
            &self.policy_fingerprint,
            &self.primary_evaluator,
            &self.independent_reviewers,
            self.progress.population_rows,
            self.progress.selected_rows,
            self.created_at,
        ))
    }

    pub fn verify_integrity(&self, plan: &AuditPlan) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.verify_with_context(&checked)
    }

    pub fn verify_with_context(&self, checked: &CheckedAuditPlan<'_>) -> Result<(), QualityError> {
        let plan = checked.plan();
        validate_evaluators(plan, &self.primary_evaluator, &self.independent_reviewers)?;
        self.progress.validate()?;
        self.usage.validate_shape()?;
        if self.usage.exceeds(&plan.policy.budgets)
            && !(self.state == QualityAuditRunState::Failed
                && self.stop_reason == Some(QualityAuditStopReason::BudgetExhausted))
        {
            return Err(QualityError::Integrity(
                "over-budget observed usage requires a terminal budget-exhausted audit".into(),
            ));
        }
        let timestamps_valid = match self.state {
            QualityAuditRunState::Queued => self.started_at.is_none() && self.finished_at.is_none(),
            QualityAuditRunState::Running => {
                self.started_at.is_some() && self.finished_at.is_none()
            }
            QualityAuditRunState::Completed
            | QualityAuditRunState::Failed
            | QualityAuditRunState::Cancelled => {
                self.started_at.is_some() && self.finished_at.is_some()
            }
        };
        let terminal_valid = match self.state {
            QualityAuditRunState::Completed => {
                self.stop_reason == Some(QualityAuditStopReason::Completed)
                    && self.error_message.is_none()
                    && !self.cancel_requested
                    && self.progress.is_complete()
                    && !(plan.policy.invalid_output_policy
                        == InvalidEvaluatorOutputPolicy::FailAudit
                        && self.progress.invalid_rows > 0)
            }
            QualityAuditRunState::Cancelled => {
                self.stop_reason == Some(QualityAuditStopReason::Cancelled)
                    && self.cancel_requested
                    && self.error_message.is_none()
            }
            QualityAuditRunState::Failed => {
                self.stop_reason.is_some() && self.error_message.is_some()
            }
            QualityAuditRunState::Queued | QualityAuditRunState::Running => {
                self.stop_reason.is_none() && self.error_message.is_none()
            }
        };
        if self.id.is_nil()
            || self.schema_version != QUALITY_AUDIT_RUN_SCHEMA_VERSION
            || self.plan_id != plan.id
            || self.plan_fingerprint != plan.fingerprint
            || self.source_set_fingerprint != plan.source_set_fingerprint
            || self.policy_fingerprint != plan.policy.fingerprint
            || !timestamps_valid
            || !terminal_valid
            || self.specification_fingerprint.is_empty()
            || self.reproduce_specification_fingerprint()? != self.specification_fingerprint
        {
            return Err(QualityError::Integrity(
                "quality audit run identity, state, or specification does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn start(&mut self, plan: &AuditPlan) -> Result<(), QualityError> {
        self.verify_integrity(plan)?;
        if self.state != QualityAuditRunState::Queued || self.cancel_requested {
            return Err(QualityError::InvalidTransition(
                "only a non-cancelled queued audit may start".into(),
            ));
        }
        self.state = QualityAuditRunState::Running;
        self.started_at = Some(Utc::now());
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), QualityError> {
        if !matches!(
            self.state,
            QualityAuditRunState::Queued | QualityAuditRunState::Running
        ) {
            return Err(QualityError::InvalidTransition(
                "terminal audit run cannot request cancellation".into(),
            ));
        }
        self.cancel_requested = true;
        Ok(())
    }

    pub fn evaluator_for_fingerprint(&self, value: &str) -> Option<&EvaluatorIdentity> {
        std::iter::once(&self.primary_evaluator)
            .chain(self.independent_reviewers.iter())
            .find(|evaluator| evaluator.fingerprint == value)
    }

    pub fn reserve_attempt(
        &mut self,
        plan: &AuditPlan,
        request: &BlindEvaluatorRequest,
        evaluator: &EvaluatorIdentity,
    ) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.reserve_attempt_with_context(&checked, request, evaluator)
    }

    pub fn reserve_attempt_with_context(
        &mut self,
        checked: &CheckedAuditPlan<'_>,
        request: &BlindEvaluatorRequest,
        evaluator: &EvaluatorIdentity,
    ) -> Result<(), QualityError> {
        let plan = checked.plan();
        self.verify_with_context(checked)?;
        request.verify_with_context(checked, evaluator)?;
        if self.state != QualityAuditRunState::Running
            || self.cancel_requested
            || request.audit_run_id != self.id
            || self.evaluator_for_fingerprint(&evaluator.fingerprint) != Some(evaluator)
        {
            return Err(QualityError::InvalidTransition(
                "attempt reservation violates run, request, or evaluator identity".into(),
            ));
        }
        let next = self.usage.reserve_attempt(request.attempt_number == 1)?;
        next.validate_against(&plan.policy.budgets)?;
        self.usage = next;
        Ok(())
    }

    pub fn record_provider_usage(
        &mut self,
        plan: &AuditPlan,
        usage: ProviderUsage,
    ) -> Result<bool, QualityError> {
        if self.state != QualityAuditRunState::Running {
            return Err(QualityError::InvalidTransition(
                "provider usage belongs only to a running audit".into(),
            ));
        }
        let next = self.usage.checked_add_provider(usage)?;
        next.validate_shape()?;
        self.usage = next;
        Ok(next.exceeds(&plan.policy.budgets))
    }

    /// Advances counters from one exact attempt delta without rescanning the
    /// audit's full population evidence. This is the hot-path counterpart to
    /// `reconcile_from_evidence`, which remains the exhaustive audit boundary.
    pub fn reconcile_finished_attempt(
        &mut self,
        plan: &AuditPlan,
        evidence: FinishedAttemptEvidence<'_>,
    ) -> Result<bool, QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.reconcile_finished_attempt_with_context(&checked, evidence)
    }

    pub fn reconcile_finished_attempt_with_context(
        &mut self,
        checked: &CheckedAuditPlan<'_>,
        evidence: FinishedAttemptEvidence<'_>,
    ) -> Result<bool, QualityError> {
        let plan = checked.plan();
        self.verify_with_context(checked)?;
        let FinishedAttemptEvidence {
            request,
            previous_attempt,
            finished_attempt,
            prior_assessments,
            prior_invalid_attempts,
            new_assessments,
        } = evidence;
        let evaluator = self
            .evaluator_for_fingerprint(&request.evaluator_identity_fingerprint)
            .ok_or_else(|| {
                QualityError::Integrity("attempt request uses an evaluator outside its run".into())
            })?;
        request.verify_with_context(checked, evaluator)?;
        previous_attempt.verify_with_context(checked, self, request)?;
        finished_attempt.verify_with_context(checked, self, request)?;
        verify_attempt_finish(previous_attempt, finished_attempt)?;
        verify_new_assessments(checked, request, finished_attempt, new_assessments)?;

        let requested_rows = request
            .rows
            .iter()
            .map(|row| row.source_row_id)
            .collect::<BTreeSet<_>>();
        let mut prior_by_row = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
        for assessment in prior_assessments {
            if assessment.audit_run_id != self.id
                || !requested_rows.contains(&assessment.source_row_id)
                || assessment.attempt_id == finished_attempt.id
            {
                return Err(QualityError::Integrity(
                    "prior assessment evidence is not the requested row-local history".into(),
                ));
            }
            prior_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(assessment);
        }
        let mut new_by_row = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
        for assessment in new_assessments {
            new_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(assessment);
        }
        let prior_invalid_by_row = checked_invalid_attempts_by_row(
            self,
            &requested_rows,
            prior_invalid_attempts,
            Some(finished_attempt.id),
        )?;
        let current_invalid_by_row = checked_invalid_attempts_by_row(
            self,
            &requested_rows,
            std::slice::from_ref(finished_attempt),
            None,
        )?;

        let mut progress = self.progress;
        for source_row_id in requested_rows {
            let prior = prior_by_row
                .get(&source_row_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let before = row_progress_contribution(
                checked,
                self,
                prior,
                prior_invalid_by_row.get(&source_row_id).copied(),
            )?;
            let mut after_assessments = prior.to_vec();
            after_assessments.extend(
                new_by_row
                    .get(&source_row_id)
                    .into_iter()
                    .flatten()
                    .copied(),
            );
            let after_invalid = current_invalid_by_row
                .get(&source_row_id)
                .copied()
                .or_else(|| prior_invalid_by_row.get(&source_row_id).copied());
            let after =
                row_progress_contribution(checked, self, &after_assessments, after_invalid)?;
            apply_row_progress_delta(&mut progress, before, after)?;
        }
        progress.validate()?;
        if !progress.can_follow(self.progress) {
            return Err(QualityError::Integrity(
                "finished attempt evidence would lose durable row progress".into(),
            ));
        }

        let next_usage = self
            .usage
            .checked_add_provider(finished_attempt.provider_usage)?;
        let overrun = finished_attempt
            .provider_usage
            .exceeds_request(request.budget)
            || next_usage.exceeds(&plan.policy.budgets);
        let budget_failure =
            finished_attempt.failure_kind == Some(EvaluatorFailureKind::BudgetExceeded);
        if overrun != budget_failure {
            return Err(QualityError::Integrity(
                "attempt budget evidence does not match its terminal failure kind".into(),
            ));
        }
        self.progress = progress;
        self.usage = next_usage;
        Ok(overrun)
    }

    pub fn reconcile_progress(&mut self, progress: AuditProgress) -> Result<(), QualityError> {
        if self.state != QualityAuditRunState::Running || self.cancel_requested {
            return Err(QualityError::InvalidTransition(
                "progress can be reconciled only on a live audit".into(),
            ));
        }
        progress.validate()?;
        if !progress.can_follow(self.progress) {
            return Err(QualityError::Integrity(
                "audit progress cannot lose processed rows or change its population".into(),
            ));
        }
        self.progress = progress;
        Ok(())
    }

    pub fn complete(&mut self, plan: &AuditPlan) -> Result<(), QualityError> {
        if self.state != QualityAuditRunState::Running
            || self.cancel_requested
            || !self.progress.is_complete()
            || (plan.policy.invalid_output_policy == InvalidEvaluatorOutputPolicy::FailAudit
                && self.progress.invalid_rows > 0)
        {
            return Err(QualityError::InvalidTransition(
                "audit completion requires complete evidence and no forbidden invalid output"
                    .into(),
            ));
        }
        self.state = QualityAuditRunState::Completed;
        self.stop_reason = Some(QualityAuditStopReason::Completed);
        self.finished_at = Some(Utc::now());
        self.verify_integrity(plan)
    }

    pub fn cancel(&mut self) -> Result<(), QualityError> {
        if !matches!(
            self.state,
            QualityAuditRunState::Queued | QualityAuditRunState::Running
        ) {
            return Err(QualityError::InvalidTransition(
                "only a queued or running audit may be cancelled".into(),
            ));
        }
        if self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }
        self.cancel_requested = true;
        self.state = QualityAuditRunState::Cancelled;
        self.stop_reason = Some(QualityAuditStopReason::Cancelled);
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn fail(
        &mut self,
        reason: QualityAuditStopReason,
        message: impl Into<String>,
    ) -> Result<(), QualityError> {
        if self.state != QualityAuditRunState::Running
            || matches!(
                reason,
                QualityAuditStopReason::Completed | QualityAuditStopReason::Cancelled
            )
        {
            return Err(QualityError::InvalidTransition(
                "invalid audit failure transition".into(),
            ));
        }
        self.state = QualityAuditRunState::Failed;
        self.stop_reason = Some(reason);
        self.error_message = Some(bounded_required(
            message,
            "audit failure message",
            MAX_ERROR_MESSAGE_CHARACTERS,
        )?);
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    pub fn verify_against_evidence(
        &self,
        plan: &AuditPlan,
        requests: &[BlindEvaluatorRequest],
        attempts: &[EvaluatorAttempt],
        assessments: &[RowQualityAssessment],
    ) -> Result<(), QualityError> {
        self.verify_integrity(plan)?;
        let (progress, usage, budget_overrun) =
            derive_audit_evidence(plan, self, requests, attempts, assessments)?;
        if progress != self.progress || usage != self.usage {
            return Err(QualityError::Integrity(
                "audit run counters do not reproduce from durable request evidence".into(),
            ));
        }
        if self.state == QualityAuditRunState::Completed && !progress.is_complete() {
            return Err(QualityError::Integrity(
                "completed audit has unresolved row evidence".into(),
            ));
        }
        if budget_overrun
            && !(self.state == QualityAuditRunState::Failed
                && self.stop_reason == Some(QualityAuditStopReason::BudgetExhausted))
        {
            return Err(QualityError::Integrity(
                "observed provider usage exceeded a request or audit budget without a budget-exhausted terminal run"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Rebuilds mutable counters exclusively from durable request, attempt,
    /// and assessment facts. The returned flag tells the runner to terminate
    /// the run as `BudgetExhausted` before persisting when a provider exceeded
    /// either its per-request allowance or the aggregate audit budget.
    pub fn reconcile_from_evidence(
        &mut self,
        plan: &AuditPlan,
        requests: &[BlindEvaluatorRequest],
        attempts: &[EvaluatorAttempt],
        assessments: &[RowQualityAssessment],
    ) -> Result<bool, QualityError> {
        if self.state != QualityAuditRunState::Running {
            return Err(QualityError::InvalidTransition(
                "only a running audit can reconcile execution evidence".into(),
            ));
        }
        let (progress, usage, budget_overrun) =
            derive_audit_evidence(plan, self, requests, attempts, assessments)?;
        if !progress.can_follow(self.progress)
            || usage.request_attempts < self.usage.request_attempts
        {
            return Err(QualityError::Integrity(
                "reconciled audit evidence would lose durable progress or attempt reservations"
                    .into(),
            ));
        }
        self.progress = progress;
        self.usage = usage;
        Ok(budget_overrun)
    }
}

fn validate_evaluators(
    plan: &AuditPlan,
    primary: &EvaluatorIdentity,
    reviewers: &[EvaluatorIdentity],
) -> Result<(), QualityError> {
    primary.validate()?;
    let required = usize::from(
        plan.policy
            .borderline_review_policy
            .required_additional_assessments(),
    );
    if primary.independence != EvaluatorIndependence::Primary
        || primary.protocol_version != plan.evaluator_protocol_version
        || reviewers.len() != required
        || !egress_allowed(plan.policy.egress_policy, primary.execution_location)
    {
        return Err(QualityError::Validation(
            "audit run primary evaluator or reviewer count violates the plan".into(),
        ));
    }
    let mut fingerprints = BTreeSet::from([primary.configuration_fingerprint.clone()]);
    let mut backend_models = BTreeSet::from([(primary.backend.clone(), primary.model.clone())]);
    for reviewer in reviewers {
        reviewer.validate()?;
        if reviewer.independence != EvaluatorIndependence::IndependentReview
            || reviewer.protocol_version != plan.evaluator_protocol_version
            || !egress_allowed(plan.policy.egress_policy, reviewer.execution_location)
            || !fingerprints.insert(reviewer.configuration_fingerprint.clone())
            || !backend_models.insert((reviewer.backend.clone(), reviewer.model.clone()))
        {
            return Err(QualityError::Validation(
                "independent reviewers must be distinct, compatible, and correctly classified"
                    .into(),
            ));
        }
    }
    Ok(())
}

const fn egress_allowed(
    policy: EvaluatorEgressPolicy,
    location: EvaluatorExecutionLocation,
) -> bool {
    match policy {
        EvaluatorEgressPolicy::LocalOnly => {
            matches!(location, EvaluatorExecutionLocation::LocalProcess)
        }
        EvaluatorEgressPolicy::ExternalCandidateText => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorAttemptState {
    Started,
    Succeeded,
    Failed,
    InvalidResponse,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorFailureKind {
    Configuration,
    Authentication,
    BudgetExceeded,
    RateLimit,
    Transport,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorAttempt {
    pub id: Uuid,
    pub schema_version: u32,
    pub run_id: Uuid,
    pub request_id: Uuid,
    pub request_sequence: u32,
    pub attempt_number: u32,
    pub evaluator: EvaluatorIdentity,
    pub source_row_ids: Vec<Uuid>,
    pub request_fingerprint: String,
    pub retry_payload_fingerprint: String,
    pub state: EvaluatorAttemptState,
    pub provider_usage: ProviderUsage,
    pub response_metadata: Value,
    pub invalid_source_row_ids: Vec<Uuid>,
    pub failure_kind: Option<EvaluatorFailureKind>,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
}

impl EvaluatorAttempt {
    pub fn start(
        plan: &AuditPlan,
        run: &QualityAuditRun,
        request: &BlindEvaluatorRequest,
        evaluator: EvaluatorIdentity,
    ) -> Result<Self, QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        Self::start_with_context(&checked, run, request, evaluator)
    }

    pub fn start_with_context(
        checked: &CheckedAuditPlan<'_>,
        run: &QualityAuditRun,
        request: &BlindEvaluatorRequest,
        evaluator: EvaluatorIdentity,
    ) -> Result<Self, QualityError> {
        run.verify_with_context(checked)?;
        request.verify_with_context(checked, &evaluator)?;
        if run.state != QualityAuditRunState::Running
            || run.cancel_requested
            || request.audit_run_id != run.id
            || run.evaluator_for_fingerprint(&evaluator.fingerprint) != Some(&evaluator)
        {
            return Err(QualityError::InvalidTransition(
                "new evaluator attempt violates run, request, or evaluator identity".into(),
            ));
        }
        let mut source_row_ids = request
            .rows
            .iter()
            .map(|row| row.source_row_id)
            .collect::<Vec<_>>();
        source_row_ids.sort_unstable();
        let mut value = Self {
            id: request.attempt_id,
            schema_version: EVALUATOR_ATTEMPT_SCHEMA_VERSION,
            run_id: run.id,
            request_id: request.id,
            request_sequence: request.request_sequence,
            attempt_number: request.attempt_number,
            evaluator,
            source_row_ids,
            request_fingerprint: request.fingerprint.clone(),
            retry_payload_fingerprint: request.reproduce_retry_payload_fingerprint()?,
            state: EvaluatorAttemptState::Started,
            provider_usage: ProviderUsage::default(),
            response_metadata: Value::Null,
            invalid_source_row_ids: Vec::new(),
            failure_kind: None,
            error_message: None,
            started_at: Utc::now(),
            finished_at: None,
            fingerprint: String::new(),
        };
        value.refresh_fingerprint()?;
        value.verify_with_context(checked, run, request)?;
        Ok(value)
    }

    pub fn succeed(&mut self, usage: ProviderUsage, metadata: Value) -> Result<(), QualityError> {
        self.finish(
            EvaluatorAttemptState::Succeeded,
            usage,
            metadata,
            Vec::new(),
            None,
            None,
        )
    }

    pub fn fail(
        &mut self,
        usage: ProviderUsage,
        kind: EvaluatorFailureKind,
        message: impl Into<String>,
        metadata: Value,
    ) -> Result<(), QualityError> {
        self.finish(
            EvaluatorAttemptState::Failed,
            usage,
            metadata,
            Vec::new(),
            Some(kind),
            Some(bounded_required(
                message,
                "evaluator failure message",
                MAX_ERROR_MESSAGE_CHARACTERS,
            )?),
        )
    }

    pub fn invalidate(
        &mut self,
        usage: ProviderUsage,
        mut source_row_ids: Vec<Uuid>,
        message: impl Into<String>,
        metadata: Value,
    ) -> Result<(), QualityError> {
        source_row_ids.sort_unstable();
        source_row_ids.dedup();
        if source_row_ids.is_empty()
            || source_row_ids
                .iter()
                .any(|id| self.source_row_ids.binary_search(id).is_err())
        {
            return Err(QualityError::Validation(
                "invalid response rows must be a non-empty subset of the attempted batch".into(),
            ));
        }
        self.finish(
            EvaluatorAttemptState::InvalidResponse,
            usage,
            metadata,
            source_row_ids,
            None,
            Some(bounded_required(
                message,
                "invalid evaluator response message",
                MAX_ERROR_MESSAGE_CHARACTERS,
            )?),
        )
    }

    pub fn interrupt(&mut self) -> Result<(), QualityError> {
        self.finish(
            EvaluatorAttemptState::Interrupted,
            ProviderUsage::default(),
            Value::Null,
            Vec::new(),
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self,
        state: EvaluatorAttemptState,
        usage: ProviderUsage,
        metadata: Value,
        invalid_source_row_ids: Vec<Uuid>,
        failure_kind: Option<EvaluatorFailureKind>,
        error_message: Option<String>,
    ) -> Result<(), QualityError> {
        if self.state != EvaluatorAttemptState::Started {
            return Err(QualityError::InvalidTransition(
                "terminal evaluator attempt cannot change".into(),
            ));
        }
        usage.validate()?;
        if serde_json::to_vec(&metadata)
            .map_err(|error| QualityError::Validation(error.to_string()))?
            .len()
            > MAX_RESPONSE_METADATA_BYTES
        {
            return Err(QualityError::Validation(
                "evaluator response metadata exceeds its bounded size".into(),
            ));
        }
        self.state = state;
        self.provider_usage = usage;
        self.response_metadata = metadata;
        self.invalid_source_row_ids = invalid_source_row_ids;
        self.failure_kind = failure_kind;
        self.error_message = error_message;
        self.finished_at = Some(Utc::now());
        self.refresh_fingerprint()
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    fn refresh_fingerprint(&mut self) -> Result<(), QualityError> {
        self.fingerprint = self.reproduce_fingerprint()?;
        Ok(())
    }

    pub fn verify_integrity(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        request: &BlindEvaluatorRequest,
    ) -> Result<(), QualityError> {
        let checked = CheckedAuditPlan::new(plan)?;
        self.verify_with_context(&checked, run, request)
    }

    pub fn verify_with_context(
        &self,
        checked: &CheckedAuditPlan<'_>,
        run: &QualityAuditRun,
        request: &BlindEvaluatorRequest,
    ) -> Result<(), QualityError> {
        request.verify_with_context(checked, &self.evaluator)?;
        self.provider_usage.validate()?;
        let terminal_shape = match self.state {
            EvaluatorAttemptState::Started => {
                self.finished_at.is_none()
                    && self.provider_usage == ProviderUsage::default()
                    && self.response_metadata == Value::Null
                    && self.invalid_source_row_ids.is_empty()
                    && self.failure_kind.is_none()
                    && self.error_message.is_none()
            }
            EvaluatorAttemptState::Succeeded => {
                self.finished_at.is_some()
                    && self.invalid_source_row_ids.is_empty()
                    && self.failure_kind.is_none()
                    && self.error_message.is_none()
            }
            EvaluatorAttemptState::Failed => {
                self.finished_at.is_some()
                    && self.invalid_source_row_ids.is_empty()
                    && self.failure_kind.is_some()
                    && self.error_message.is_some()
            }
            EvaluatorAttemptState::InvalidResponse => {
                self.finished_at.is_some()
                    && !self.invalid_source_row_ids.is_empty()
                    && self.failure_kind.is_none()
                    && self.error_message.is_some()
            }
            EvaluatorAttemptState::Interrupted => {
                self.finished_at.is_some()
                    && self.provider_usage == ProviderUsage::default()
                    && self.response_metadata == Value::Null
                    && self.invalid_source_row_ids.is_empty()
                    && self.failure_kind.is_none()
                    && self.error_message.is_none()
            }
        };
        if self.id.is_nil()
            || self.schema_version != EVALUATOR_ATTEMPT_SCHEMA_VERSION
            || self.run_id != run.id
            || self.request_id != request.id
            || self.id != request.attempt_id
            || self.request_sequence != request.request_sequence
            || self.attempt_number != request.attempt_number
            || self.request_fingerprint != request.fingerprint
            || self.retry_payload_fingerprint != request.reproduce_retry_payload_fingerprint()?
            || self.evaluator.fingerprint != request.evaluator_identity_fingerprint
            || self.source_row_ids
                != request
                    .rows
                    .iter()
                    .map(|row| row.source_row_id)
                    .collect::<Vec<_>>()
            || !terminal_shape
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(QualityError::Integrity(
                "evaluator attempt does not reproduce from its exact durable request".into(),
            ));
        }
        Ok(())
    }
}

fn verify_attempt_finish(
    previous: &EvaluatorAttempt,
    finished: &EvaluatorAttempt,
) -> Result<(), QualityError> {
    if previous.state != EvaluatorAttemptState::Started
        || finished.state == EvaluatorAttemptState::Started
        || previous.id != finished.id
        || previous.run_id != finished.run_id
        || previous.request_id != finished.request_id
        || previous.request_sequence != finished.request_sequence
        || previous.attempt_number != finished.attempt_number
        || previous.evaluator != finished.evaluator
        || previous.source_row_ids != finished.source_row_ids
        || previous.request_fingerprint != finished.request_fingerprint
        || previous.retry_payload_fingerprint != finished.retry_payload_fingerprint
        || previous.started_at != finished.started_at
    {
        return Err(QualityError::Integrity(
            "only the exact durable started evaluator attempt may finish".into(),
        ));
    }
    Ok(())
}

fn verify_new_assessments(
    checked: &CheckedAuditPlan<'_>,
    request: &BlindEvaluatorRequest,
    attempt: &EvaluatorAttempt,
    assessments: &[RowQualityAssessment],
) -> Result<(), QualityError> {
    if attempt.state != EvaluatorAttemptState::Succeeded && !assessments.is_empty() {
        return Err(QualityError::Integrity(
            "only a successful attempt may add assessment evidence".into(),
        ));
    }
    if attempt.state == EvaluatorAttemptState::Succeeded
        && assessments.len() != attempt.source_row_ids.len()
    {
        return Err(QualityError::Integrity(
            "successful attempt evidence must assess every requested row exactly once".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut rows = BTreeSet::new();
    for assessment in assessments {
        assessment.verify_request_binding_with_context(checked, request)?;
        if !ids.insert(assessment.id)
            || !rows.insert(assessment.source_row_id)
            || assessment.audit_run_id != attempt.run_id
            || assessment.attempt_id != attempt.id
            || assessment.request_id != attempt.request_id
            || assessment.request_sequence != attempt.request_sequence
            || assessment.attempt_number != attempt.attempt_number
            || assessment.evaluator != attempt.evaluator
        {
            return Err(QualityError::Integrity(
                "new assessment evidence does not bind the finished attempt exactly".into(),
            ));
        }
    }
    Ok(())
}

fn checked_invalid_attempts_by_row<'a>(
    run: &QualityAuditRun,
    requested_rows: &BTreeSet<Uuid>,
    attempts: &'a [EvaluatorAttempt],
    excluded_attempt_id: Option<Uuid>,
) -> Result<BTreeMap<Uuid, &'a EvaluatorAttempt>, QualityError> {
    let mut by_row = BTreeMap::new();
    for attempt in attempts {
        if Some(attempt.id) == excluded_attempt_id {
            return Err(QualityError::Integrity(
                "prior invalid evidence contains the attempt being finished".into(),
            ));
        }
        if attempt.state != EvaluatorAttemptState::InvalidResponse {
            continue;
        }
        if attempt.run_id != run.id
            || run.evaluator_for_fingerprint(&attempt.evaluator.fingerprint)
                != Some(&attempt.evaluator)
            || attempt.fingerprint.is_empty()
            || attempt.reproduce_fingerprint()? != attempt.fingerprint
        {
            return Err(QualityError::Integrity(
                "invalid-output evidence does not belong to the audit run".into(),
            ));
        }
        for source_row_id in &attempt.invalid_source_row_ids {
            if !requested_rows.contains(source_row_id) {
                continue;
            }
            if by_row.insert(*source_row_id, attempt).is_some() {
                return Err(QualityError::Integrity(
                    "a source row has more than one terminal invalid-output marker".into(),
                ));
            }
        }
    }
    Ok(by_row)
}

#[derive(Debug, Clone, Copy, Default)]
struct RowProgressContribution {
    assessed: u64,
    qualified: u64,
    borderline: u64,
    quarantined: u64,
    invalid: u64,
    pending_review: u64,
}

fn row_progress_contribution(
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    assessments: &[&RowQualityAssessment],
    invalid_attempt: Option<&EvaluatorAttempt>,
) -> Result<RowProgressContribution, QualityError> {
    if assessments.is_empty() {
        return match invalid_attempt {
            Some(attempt) if attempt.evaluator == run.primary_evaluator => {
                Ok(RowProgressContribution {
                    invalid: 1,
                    ..RowProgressContribution::default()
                })
            }
            Some(_) => Err(QualityError::Integrity(
                "initial invalid output must belong to the run's primary evaluator".into(),
            )),
            None => Ok(RowProgressContribution::default()),
        };
    }

    let summary = verify_assessment_sequence_with_context(
        checked,
        run.id,
        &run.primary_evaluator,
        &run.independent_reviewers,
        assessments,
    )?;
    if let Some(attempt) = invalid_attempt {
        let last_sequence = assessments
            .iter()
            .map(|assessment| assessment.request_sequence)
            .max()
            .expect("non-empty assessment sequence");
        if summary.is_complete()
            || attempt.evaluator != run.independent_reviewers[summary.completed_required_reviews]
            || attempt.request_sequence <= last_sequence
        {
            return Err(QualityError::Integrity(
                "invalid reviewer output does not occupy the next required review position".into(),
            ));
        }
        return Ok(RowProgressContribution {
            invalid: 1,
            ..RowProgressContribution::default()
        });
    }

    let mut contribution = RowProgressContribution {
        assessed: 1,
        pending_review: u64::from(!summary.is_complete()),
        ..RowProgressContribution::default()
    };
    match summary.effective_verdict {
        QualityVerdict::Qualified => contribution.qualified = 1,
        QualityVerdict::Borderline => contribution.borderline = 1,
        QualityVerdict::Quarantined => contribution.quarantined = 1,
    }
    Ok(contribution)
}

fn apply_row_progress_delta(
    progress: &mut AuditProgress,
    previous: RowProgressContribution,
    next: RowProgressContribution,
) -> Result<(), QualityError> {
    progress.assessed_rows =
        replace_counter(progress.assessed_rows, previous.assessed, next.assessed)?;
    progress.qualified_rows =
        replace_counter(progress.qualified_rows, previous.qualified, next.qualified)?;
    progress.borderline_rows = replace_counter(
        progress.borderline_rows,
        previous.borderline,
        next.borderline,
    )?;
    progress.quarantined_rows = replace_counter(
        progress.quarantined_rows,
        previous.quarantined,
        next.quarantined,
    )?;
    progress.invalid_rows = replace_counter(progress.invalid_rows, previous.invalid, next.invalid)?;
    progress.pending_review_rows = replace_counter(
        progress.pending_review_rows,
        previous.pending_review,
        next.pending_review,
    )?;
    Ok(())
}

fn replace_counter(value: u64, previous: u64, next: u64) -> Result<u64, QualityError> {
    value
        .checked_sub(previous)
        .and_then(|value| value.checked_add(next))
        .ok_or_else(|| QualityError::Integrity("audit progress counter overflowed".into()))
}

fn derive_audit_evidence(
    plan: &AuditPlan,
    run: &QualityAuditRun,
    requests: &[BlindEvaluatorRequest],
    attempts: &[EvaluatorAttempt],
    assessments: &[RowQualityAssessment],
) -> Result<(AuditProgress, AuditUsage, bool), QualityError> {
    let checked = CheckedAuditPlan::new(plan)?;
    let mut requests_by_id = BTreeMap::new();
    for request in requests {
        let evaluator = run
            .evaluator_for_fingerprint(&request.evaluator_identity_fingerprint)
            .ok_or_else(|| {
                QualityError::Integrity("request uses an evaluator outside its run".into())
            })?;
        request.verify_with_context(&checked, evaluator)?;
        if request.audit_run_id != run.id || requests_by_id.insert(request.id, request).is_some() {
            return Err(QualityError::Integrity(
                "durable evaluator requests contain a foreign or duplicate identity".into(),
            ));
        }
    }
    let mut attempts_by_sequence = BTreeMap::<u32, Vec<&EvaluatorAttempt>>::new();
    let mut attempts_by_id = BTreeMap::new();
    let mut provider_usage = ProviderUsage::default();
    let mut request_budget_overrun = false;
    for attempt in attempts {
        let request = requests_by_id.get(&attempt.request_id).ok_or_else(|| {
            QualityError::Integrity("evaluator attempt has no durable request".into())
        })?;
        attempt.verify_with_context(&checked, run, request)?;
        let attempt_budget_overrun = attempt.provider_usage.exceeds_request(request.budget);
        if attempt_budget_overrun
            && !(attempt.state == EvaluatorAttemptState::Failed
                && attempt.failure_kind == Some(EvaluatorFailureKind::BudgetExceeded))
        {
            return Err(QualityError::Integrity(
                "provider usage above a request budget must be preserved as a budget-exceeded attempt"
                    .into(),
            ));
        }
        request_budget_overrun |= attempt_budget_overrun;
        if attempts_by_id.insert(attempt.id, attempt).is_some() {
            return Err(QualityError::Integrity(
                "evaluator attempt identity is duplicated".into(),
            ));
        }
        attempts_by_sequence
            .entry(attempt.request_sequence)
            .or_default()
            .push(attempt);
        provider_usage.input_tokens = provider_usage
            .input_tokens
            .checked_add(attempt.provider_usage.input_tokens)
            .ok_or_else(|| QualityError::Integrity("provider usage overflowed".into()))?;
        provider_usage.output_tokens = provider_usage
            .output_tokens
            .checked_add(attempt.provider_usage.output_tokens)
            .ok_or_else(|| QualityError::Integrity("provider usage overflowed".into()))?;
        provider_usage.total_tokens = provider_usage
            .total_tokens
            .checked_add(attempt.provider_usage.total_tokens)
            .ok_or_else(|| QualityError::Integrity("provider usage overflowed".into()))?;
        provider_usage.cost_microusd = provider_usage
            .cost_microusd
            .checked_add(attempt.provider_usage.cost_microusd)
            .ok_or_else(|| QualityError::Integrity("provider usage overflowed".into()))?;
    }
    if requests_by_id.len() != attempts_by_id.len()
        || requests_by_id
            .values()
            .any(|request| !attempts_by_id.contains_key(&request.attempt_id))
    {
        return Err(QualityError::Integrity(
            "durable evaluator requests and attempts are not one-to-one".into(),
        ));
    }
    if matches!(
        run.state,
        QualityAuditRunState::Completed
            | QualityAuditRunState::Failed
            | QualityAuditRunState::Cancelled
    ) && attempts
        .iter()
        .any(|attempt| attempt.state == EvaluatorAttemptState::Started)
    {
        return Err(QualityError::Integrity(
            "a terminal audit cannot retain a started evaluator attempt".into(),
        ));
    }
    let mut logical_row_owners = BTreeMap::<(String, Uuid), u32>::new();
    for (index, (request_sequence, sequence)) in attempts_by_sequence.iter_mut().enumerate() {
        if *request_sequence as usize != index + 1
            || sequence.len() > plan.policy.budgets.maximum_attempts_per_request as usize
        {
            return Err(QualityError::Integrity(
                "logical evaluator request sequences are non-canonical or exceed retry limits"
                    .into(),
            ));
        }
        sequence.sort_by_key(|attempt| attempt.attempt_number);
        let expected_retry = sequence[0].retry_payload_fingerprint.clone();
        for (index, attempt) in sequence.iter().enumerate() {
            if attempt.attempt_number as usize != index + 1
                || attempt.retry_payload_fingerprint != expected_retry
                || (index + 1 < sequence.len()
                    && !matches!(
                        attempt.state,
                        EvaluatorAttemptState::Failed | EvaluatorAttemptState::Interrupted
                    ))
            {
                return Err(QualityError::Integrity(
                    "logical evaluator request retries are non-consecutive or changed payload"
                        .into(),
                ));
            }
        }
        if run.state == QualityAuditRunState::Completed
            && !matches!(
                sequence.last().expect("non-empty attempt sequence").state,
                EvaluatorAttemptState::Succeeded | EvaluatorAttemptState::InvalidResponse
            )
        {
            return Err(QualityError::Integrity(
                "a completed audit cannot retain an unresolved logical evaluator request".into(),
            ));
        }
        for source_row_id in &sequence[0].source_row_ids {
            let key = (sequence[0].evaluator.fingerprint.clone(), *source_row_id);
            if logical_row_owners.insert(key, *request_sequence).is_some() {
                return Err(QualityError::Integrity(
                    "the same evaluator and row appear in more than one logical request; retries must retain their request sequence"
                        .into(),
                ));
            }
        }
    }

    let mut assessment_ids = BTreeSet::new();
    let mut assessments_by_attempt = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
    let mut assessments_by_row = BTreeMap::<Uuid, Vec<&RowQualityAssessment>>::new();
    for assessment in assessments {
        let attempt = attempts_by_id.get(&assessment.attempt_id).ok_or_else(|| {
            QualityError::Integrity("assessment has no durable evaluator attempt".into())
        })?;
        let request = requests_by_id
            .get(&assessment.request_id)
            .ok_or_else(|| QualityError::Integrity("assessment has no durable request".into()))?;
        assessment.verify_request_binding_with_context(&checked, request)?;
        if attempt.state != EvaluatorAttemptState::Succeeded
            || attempt.request_id != assessment.request_id
            || !assessment_ids.insert(assessment.id)
        {
            return Err(QualityError::Integrity(
                "assessment is duplicated or belongs to a non-successful attempt".into(),
            ));
        }
        assessments_by_attempt
            .entry(attempt.id)
            .or_default()
            .push(assessment);
        assessments_by_row
            .entry(assessment.source_row_id)
            .or_default()
            .push(assessment);
    }
    for attempt in attempts {
        let evidence = assessments_by_attempt
            .get(&attempt.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if attempt.state == EvaluatorAttemptState::Succeeded {
            let evidence_rows = evidence
                .iter()
                .map(|assessment| assessment.source_row_id)
                .collect::<BTreeSet<_>>();
            if evidence_rows.len() != evidence.len()
                || evidence_rows != attempt.source_row_ids.iter().copied().collect()
            {
                return Err(QualityError::Integrity(
                    "successful attempt does not have exactly one assessment per requested row"
                        .into(),
                ));
            }
        } else if !evidence.is_empty() {
            return Err(QualityError::Integrity(
                "non-successful evaluator attempt contains assessments".into(),
            ));
        }
    }
    let mut invalid_attempt_by_row = BTreeMap::<Uuid, &EvaluatorAttempt>::new();
    for attempt in attempts
        .iter()
        .filter(|attempt| attempt.state == EvaluatorAttemptState::InvalidResponse)
    {
        for source_row_id in &attempt.invalid_source_row_ids {
            if invalid_attempt_by_row
                .insert(*source_row_id, attempt)
                .is_some()
            {
                return Err(QualityError::Integrity(
                    "a source row has more than one terminal invalid-output marker".into(),
                ));
            }
        }
    }
    let invalid_rows = invalid_attempt_by_row
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let selected = plan
        .items
        .iter()
        .filter(|item| item.selection == AuditSelection::Selected)
        .map(|item| item.source_row_id)
        .collect::<BTreeSet<_>>();
    if invalid_rows.iter().any(|row_id| !selected.contains(row_id))
        || assessments_by_row
            .keys()
            .any(|row_id| !selected.contains(row_id))
    {
        return Err(QualityError::Integrity(
            "audit evidence contains an unselected source row".into(),
        ));
    }
    let mut progress = AuditProgress {
        population_rows: plan.population_count(),
        selected_rows: plan.selected_count(),
        invalid_rows: invalid_rows.len() as u64,
        ..AuditProgress::default()
    };
    for (source_row_id, sequence) in &assessments_by_row {
        let summary = verify_assessment_sequence_with_context(
            &checked,
            run.id,
            &run.primary_evaluator,
            &run.independent_reviewers,
            sequence,
        )?;
        if let Some(invalid_attempt) = invalid_attempt_by_row.get(source_row_id) {
            if summary.is_complete()
                || invalid_attempt.evaluator
                    != run.independent_reviewers[summary.completed_required_reviews]
                || invalid_attempt.request_sequence
                    <= sequence
                        .iter()
                        .map(|assessment| assessment.request_sequence)
                        .max()
                        .expect("non-empty assessment sequence")
            {
                return Err(QualityError::Integrity(
                    "invalid reviewer output does not occupy the next required review position"
                        .into(),
                ));
            }
            continue;
        }
        progress.assessed_rows += 1;
        progress.pending_review_rows += u64::from(!summary.is_complete());
        match summary.effective_verdict {
            QualityVerdict::Qualified => progress.qualified_rows += 1,
            QualityVerdict::Borderline => progress.borderline_rows += 1,
            QualityVerdict::Quarantined => progress.quarantined_rows += 1,
        }
    }
    for (source_row_id, invalid_attempt) in &invalid_attempt_by_row {
        if !assessments_by_row.contains_key(source_row_id)
            && invalid_attempt.evaluator != run.primary_evaluator
        {
            return Err(QualityError::Integrity(
                "initial invalid output must belong to the run's primary evaluator".into(),
            ));
        }
    }
    progress.validate()?;
    provider_usage.validate()?;
    let usage = AuditUsage {
        evaluator_requests: u32::try_from(attempts_by_sequence.len())
            .map_err(|_| QualityError::Integrity("request count overflowed".into()))?,
        request_attempts: u32::try_from(attempts.len())
            .map_err(|_| QualityError::Integrity("attempt count overflowed".into()))?,
        input_tokens: provider_usage.input_tokens,
        output_tokens: provider_usage.output_tokens,
        total_tokens: provider_usage.total_tokens,
        cost_microusd: provider_usage.cost_microusd,
    };
    usage.validate_shape()?;
    let budget_overrun = request_budget_overrun || usage.exceeds(&plan.policy.budgets);
    let budget_failure_attempts = attempts
        .iter()
        .filter(|attempt| attempt.failure_kind == Some(EvaluatorFailureKind::BudgetExceeded))
        .count();
    if (budget_overrun && budget_failure_attempts != 1)
        || (!budget_overrun && budget_failure_attempts != 0)
    {
        return Err(QualityError::Integrity(
            "observed budget overrun must have exactly one matching budget-exceeded attempt".into(),
        ));
    }
    Ok((progress, usage, budget_overrun))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::domain::{SourceProvenance, SourceRow};
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use serde_json::{Value, json};

    use crate::{
        assessment::{EvaluatorGuidance, EvaluatorRequestBudget, RowAssessmentDraft},
        policy::{
            AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, GuidanceReferences},
    };

    use super::*;

    const EVALUATOR_PROTOCOL: &str = "quality-evaluator-v1";

    fn bp(value: u16) -> BasisPoints {
        BasisPoints::new(value).expect("valid basis points")
    }

    fn fixture_plan(
        preset: QualityPreset,
        egress_policy: EvaluatorEgressPolicy,
    ) -> (AuditPlan, SourceRow) {
        let dataset = DatasetDefinition::with_identity(
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
        .expect("dataset");
        let row = SourceRow {
            id: Uuid::parse_str("20000000-0000-4000-8000-000000000001").expect("row ID"),
            dataset_id: dataset.id,
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
        };
        let policy = preset
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("policy");
        let guidance = EvaluatorGuidance::default();
        let plan = AuditPlan::with_identity(
            Uuid::parse_str("40000000-0000-4000-8000-000000000001").expect("plan ID"),
            &dataset,
            policy,
            GuidanceReferences::default(),
            guidance
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            EVALUATOR_PROTOCOL,
            vec![row.clone()],
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("plan");
        (plan, row)
    }

    fn evaluator(
        suffix: &str,
        independence: EvaluatorIndependence,
        execution_location: EvaluatorExecutionLocation,
    ) -> EvaluatorIdentity {
        EvaluatorIdentity::new(
            format!("quality-backend-{suffix}"),
            format!("quality-model-{suffix}"),
            EVALUATOR_PROTOCOL,
            format!("sha256:quality-configuration-{suffix}"),
            independence,
            execution_location,
        )
        .expect("evaluator")
    }

    fn primary() -> EvaluatorIdentity {
        evaluator(
            "primary",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
    }

    fn reviewer() -> EvaluatorIdentity {
        evaluator(
            "reviewer",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::LocalProcess,
        )
    }

    fn running_run(
        plan: &AuditPlan,
        primary_evaluator: EvaluatorIdentity,
        reviewers: Vec<EvaluatorIdentity>,
    ) -> QualityAuditRun {
        let mut run = QualityAuditRun::queue(plan, primary_evaluator, reviewers).expect("run");
        run.start(plan).expect("start run");
        run
    }

    fn request_budget() -> EvaluatorRequestBudget {
        EvaluatorRequestBudget {
            maximum_input_tokens: 1_000,
            maximum_output_tokens: 1_000,
            maximum_total_tokens: 2_000,
            maximum_cost_microusd: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn request(
        plan: &AuditPlan,
        run_id: Uuid,
        row: &SourceRow,
        evaluator: &EvaluatorIdentity,
        request_sequence: u32,
        attempt_number: u32,
        discriminator: u128,
        budget: EvaluatorRequestBudget,
    ) -> BlindEvaluatorRequest {
        BlindEvaluatorRequest::create(
            plan,
            Uuid::from_u128(0x50000000000040008000000000000000 + discriminator),
            run_id,
            Uuid::from_u128(0x60000000000040008000000000000000 + discriminator),
            request_sequence,
            attempt_number,
            evaluator,
            vec![row.clone()],
            EvaluatorGuidance::default(),
            plan.resolved_guidance_fingerprint.clone(),
            budget,
        )
        .expect("request")
    }

    fn provider_usage() -> ProviderUsage {
        ProviderUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cost_microusd: 7,
        }
    }

    fn over_request_usage() -> ProviderUsage {
        ProviderUsage {
            input_tokens: 1_001,
            output_tokens: 1,
            total_tokens: 1_002,
            cost_microusd: 0,
        }
    }

    fn complete_invalid_progress(plan: &AuditPlan) -> AuditProgress {
        AuditProgress {
            population_rows: plan.population_count(),
            selected_rows: plan.selected_count(),
            invalid_rows: plan.selected_count(),
            ..AuditProgress::default()
        }
    }

    fn assert_integrity_contains(result: Result<(), QualityError>, expected: &str) {
        match result {
            Err(QualityError::Integrity(message)) => assert!(
                message.contains(expected),
                "expected integrity error containing {expected:?}, got {message:?}"
            ),
            other => panic!("expected integrity error containing {expected:?}, got {other:?}"),
        }
    }

    fn borderline_draft(plan: &AuditPlan) -> RowAssessmentDraft {
        let item = &plan.items[0];
        RowAssessmentDraft {
            source_row_id: item.source_row_id,
            source_row_fingerprint: item.source_row_fingerprint.clone(),
            label_scores: BTreeMap::from([
                ("billing".into(), bp(7_400)),
                ("fraud".into(), bp(6_200)),
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
            rationale: "The assigned label is plausible but close to the policy threshold.".into(),
        }
    }

    fn strict_borderline_draft(plan: &AuditPlan) -> RowAssessmentDraft {
        let item = &plan.items[0];
        RowAssessmentDraft {
            source_row_id: item.source_row_id,
            source_row_fingerprint: item.source_row_fingerprint.clone(),
            label_scores: BTreeMap::from([
                ("billing".into(), bp(8_500)),
                ("fraud".into(), bp(7_000)),
            ]),
            dimension_scores: BTreeMap::from([(
                "difficulty".into(),
                BTreeMap::from([("easy".into(), bp(8_000)), ("hard".into(), bp(2_000))]),
            )]),
            authenticity_score: None,
            label_leakage_risk: bp(1_000),
            shortcut_risk: bp(1_000),
            confidence: bp(8_000),
            issue_codes: vec![],
            rationale: "The row is close enough to every strict threshold to require review."
                .into(),
        }
    }

    fn quarantined_draft(plan: &AuditPlan) -> RowAssessmentDraft {
        let mut draft = strict_borderline_draft(plan);
        draft.label_scores.insert("billing".into(), bp(6_000));
        draft.rationale = "The assigned label is not supported strongly enough.".into();
        draft
    }

    #[test]
    fn host_reservations_count_failed_and_interrupted_calls() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);

        let first_request = request(&plan, run.id, &row, &primary, 1, 1, 1, request_budget());
        run.reserve_attempt(&plan, &first_request, &primary)
            .expect("reserve failed call");
        let mut failed = EvaluatorAttempt::start(&plan, &run, &first_request, primary.clone())
            .expect("failed attempt");
        failed
            .fail(
                provider_usage(),
                EvaluatorFailureKind::RateLimit,
                "rate limited",
                json!({"retry_after_ms": 5}),
            )
            .expect("record failure");
        run.record_provider_usage(&plan, provider_usage())
            .expect("record provider usage");

        let retry_request = request(&plan, run.id, &row, &primary, 1, 2, 2, request_budget());
        run.reserve_attempt(&plan, &retry_request, &primary)
            .expect("reserve interrupted call");
        let mut interrupted = EvaluatorAttempt::start(&plan, &run, &retry_request, primary.clone())
            .expect("interrupted attempt");
        interrupted.interrupt().expect("record interruption");

        assert_eq!(run.usage.evaluator_requests, 1);
        assert_eq!(run.usage.request_attempts, 2);
        assert_eq!(run.usage.input_tokens, provider_usage().input_tokens);
        assert!(
            run.verify_against_evidence(
                &plan,
                &[first_request, retry_request],
                &[failed, interrupted],
                &[],
            )
            .is_ok()
        );
    }

    #[test]
    fn contextual_evidence_rejects_orphan_requests() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let run = running_run(&plan, primary.clone(), vec![]);
        let orphan = request(&plan, run.id, &row, &primary, 1, 1, 3, request_budget());

        assert_integrity_contains(
            run.verify_against_evidence(&plan, &[orphan], &[], &[]),
            "one-to-one",
        );
    }

    #[test]
    fn contextual_evidence_rejects_skipped_and_repeated_sequences() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();

        let mut skipped_run = running_run(&plan, primary.clone(), vec![]);
        let skipped_request = request(
            &plan,
            skipped_run.id,
            &row,
            &primary,
            2,
            1,
            4,
            request_budget(),
        );
        skipped_run
            .reserve_attempt(&plan, &skipped_request, &primary)
            .expect("reserve skipped sequence");
        let mut skipped_attempt =
            EvaluatorAttempt::start(&plan, &skipped_run, &skipped_request, primary.clone())
                .expect("skipped attempt");
        skipped_attempt.interrupt().expect("interrupt attempt");
        assert_integrity_contains(
            skipped_run.verify_against_evidence(&plan, &[skipped_request], &[skipped_attempt], &[]),
            "non-canonical",
        );

        let mut repeated_run = running_run(&plan, primary.clone(), vec![]);
        let first = request(
            &plan,
            repeated_run.id,
            &row,
            &primary,
            1,
            1,
            5,
            request_budget(),
        );
        let repeated = request(
            &plan,
            repeated_run.id,
            &row,
            &primary,
            1,
            1,
            6,
            request_budget(),
        );
        repeated_run
            .reserve_attempt(&plan, &first, &primary)
            .expect("reserve first sequence");
        let mut first_attempt =
            EvaluatorAttempt::start(&plan, &repeated_run, &first, primary.clone())
                .expect("first attempt");
        first_attempt.interrupt().expect("interrupt first");
        repeated_run
            .reserve_attempt(&plan, &repeated, &primary)
            .expect("reserve repeated sequence");
        let mut repeated_attempt =
            EvaluatorAttempt::start(&plan, &repeated_run, &repeated, primary.clone())
                .expect("repeated attempt");
        repeated_attempt.interrupt().expect("interrupt repeated");
        assert_integrity_contains(
            repeated_run.verify_against_evidence(
                &plan,
                &[first, repeated],
                &[first_attempt, repeated_attempt],
                &[],
            ),
            "non-consecutive",
        );
    }

    #[test]
    fn contextual_evidence_rejects_changed_retry_payload() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);
        let first_request = request(&plan, run.id, &row, &primary, 1, 1, 7, request_budget());
        run.reserve_attempt(&plan, &first_request, &primary)
            .expect("reserve first attempt");
        let mut first_attempt =
            EvaluatorAttempt::start(&plan, &run, &first_request, primary.clone())
                .expect("first attempt");
        first_attempt
            .fail(
                ProviderUsage::default(),
                EvaluatorFailureKind::Transport,
                "connection reset",
                Value::Null,
            )
            .expect("record failure");

        let changed_budget = EvaluatorRequestBudget {
            maximum_input_tokens: 1_001,
            ..request_budget()
        };
        let changed_retry = request(&plan, run.id, &row, &primary, 1, 2, 8, changed_budget);
        run.reserve_attempt(&plan, &changed_retry, &primary)
            .expect("reserve changed retry");
        let mut changed_attempt =
            EvaluatorAttempt::start(&plan, &run, &changed_retry, primary.clone())
                .expect("changed retry attempt");
        changed_attempt.interrupt().expect("interrupt retry");

        assert_integrity_contains(
            run.verify_against_evidence(
                &plan,
                &[first_request, changed_retry],
                &[first_attempt, changed_attempt],
                &[],
            ),
            "changed payload",
        );
    }

    #[test]
    fn completed_run_rejects_dangling_failed_requests() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);
        let request = request(&plan, run.id, &row, &primary, 1, 1, 9, request_budget());
        run.reserve_attempt(&plan, &request, &primary)
            .expect("reserve failed request");
        let mut attempt =
            EvaluatorAttempt::start(&plan, &run, &request, primary.clone()).expect("attempt");
        attempt
            .fail(
                provider_usage(),
                EvaluatorFailureKind::Provider,
                "provider unavailable",
                Value::Null,
            )
            .expect("record failure");
        run.record_provider_usage(&plan, provider_usage())
            .expect("record usage");
        run.reconcile_progress(AuditProgress {
            population_rows: plan.population_count(),
            selected_rows: plan.selected_count(),
            assessed_rows: 1,
            qualified_rows: 1,
            ..AuditProgress::default()
        })
        .expect("forge superficially complete counters");
        run.complete(&plan).expect("counter-only completion");

        assert_integrity_contains(
            run.verify_against_evidence(&plan, &[request], &[attempt], &[]),
            "unresolved logical evaluator request",
        );
    }

    #[test]
    fn invalid_primary_output_is_durable_terminal_row_evidence() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);
        let request = request(&plan, run.id, &row, &primary, 1, 1, 10, request_budget());
        run.reserve_attempt(&plan, &request, &primary)
            .expect("reserve invalid response");
        let mut attempt =
            EvaluatorAttempt::start(&plan, &run, &request, primary).expect("primary attempt");
        attempt
            .invalidate(
                ProviderUsage::default(),
                vec![row.id],
                "malformed primary output",
                json!({"finish_reason": "invalid_json"}),
            )
            .expect("record invalid output");
        run.reconcile_progress(complete_invalid_progress(&plan))
            .expect("reconcile invalid row");
        run.complete(&plan).expect("quarantine-policy completion");

        assert_eq!(run.progress.invalid_rows, 1);
        assert_eq!(run.progress.assessed_rows, 0);
        assert!(
            run.verify_against_evidence(&plan, &[request], &[attempt], &[])
                .is_ok()
        );
    }

    #[test]
    fn invalid_independent_review_preserves_primary_evidence_and_marks_row_invalid() {
        let (plan, row) = fixture_plan(QualityPreset::Balanced, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let reviewer = reviewer();
        let mut run = running_run(&plan, primary.clone(), vec![reviewer.clone()]);

        let primary_request = request(&plan, run.id, &row, &primary, 1, 1, 11, request_budget());
        run.reserve_attempt(&plan, &primary_request, &primary)
            .expect("reserve primary");
        let mut primary_attempt =
            EvaluatorAttempt::start(&plan, &run, &primary_request, primary.clone())
                .expect("primary attempt");
        primary_attempt
            .succeed(ProviderUsage::default(), json!({"finish_reason": "stop"}))
            .expect("primary success");
        let primary_assessment = RowQualityAssessment::create(
            &plan,
            &plan.items[0],
            &primary_request,
            primary,
            borderline_draft(&plan),
            &[],
            Utc.with_ymd_and_hms(2026, 1, 4, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("borderline primary evidence");
        assert_eq!(primary_assessment.verdict, QualityVerdict::Borderline);
        assert!(
            !run.reconcile_from_evidence(
                &plan,
                std::slice::from_ref(&primary_request),
                std::slice::from_ref(&primary_attempt),
                std::slice::from_ref(&primary_assessment),
            )
            .expect("reconcile pending borderline review")
        );
        assert_eq!(run.progress.assessed_rows, 1);
        assert_eq!(run.progress.invalid_rows, 0);
        assert_eq!(run.progress.pending_review_rows, 1);

        let review_request = request(&plan, run.id, &row, &reviewer, 2, 1, 12, request_budget());
        run.reserve_attempt(&plan, &review_request, &reviewer)
            .expect("reserve review");
        let mut review_attempt = EvaluatorAttempt::start(&plan, &run, &review_request, reviewer)
            .expect("review attempt");
        review_attempt
            .invalidate(
                ProviderUsage::default(),
                vec![row.id],
                "malformed independent review",
                Value::Null,
            )
            .expect("record invalid review");
        assert!(
            !run.reconcile_from_evidence(
                &plan,
                &[primary_request.clone(), review_request.clone()],
                &[primary_attempt.clone(), review_attempt.clone()],
                std::slice::from_ref(&primary_assessment),
            )
            .expect("reconcile assessed-to-invalid transition")
        );
        run.complete(&plan).expect("quarantine-policy completion");

        assert_eq!(run.progress.invalid_rows, 1);
        assert_eq!(run.progress.assessed_rows, 0);
        assert!(
            run.verify_against_evidence(
                &plan,
                &[primary_request, review_request],
                &[primary_attempt, review_attempt],
                &[primary_assessment],
            )
            .is_ok()
        );
    }

    #[test]
    fn quarantining_first_review_can_remain_pending_for_second_pinned_reviewer() {
        let (plan, row) = fixture_plan(QualityPreset::Strict, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let first_reviewer = evaluator(
            "reviewer-one",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::LocalProcess,
        );
        let second_reviewer = evaluator(
            "reviewer-two",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::LocalProcess,
        );
        let mut run = running_run(
            &plan,
            primary.clone(),
            vec![first_reviewer.clone(), second_reviewer],
        );

        let primary_request = request(&plan, run.id, &row, &primary, 1, 1, 18, request_budget());
        run.reserve_attempt(&plan, &primary_request, &primary)
            .expect("reserve primary");
        let mut primary_attempt =
            EvaluatorAttempt::start(&plan, &run, &primary_request, primary.clone())
                .expect("primary attempt");
        primary_attempt
            .succeed(ProviderUsage::default(), Value::Null)
            .expect("primary success");
        let primary_assessment = RowQualityAssessment::create(
            &plan,
            &plan.items[0],
            &primary_request,
            primary,
            strict_borderline_draft(&plan),
            &[],
            Utc.with_ymd_and_hms(2026, 1, 4, 0, 0, 0)
                .single()
                .expect("primary time"),
        )
        .expect("strict borderline primary");
        assert_eq!(primary_assessment.verdict, QualityVerdict::Borderline);

        let review_request = request(
            &plan,
            run.id,
            &row,
            &first_reviewer,
            2,
            1,
            19,
            request_budget(),
        );
        run.reserve_attempt(&plan, &review_request, &first_reviewer)
            .expect("reserve first review");
        let mut review_attempt =
            EvaluatorAttempt::start(&plan, &run, &review_request, first_reviewer.clone())
                .expect("first review attempt");
        review_attempt
            .succeed(ProviderUsage::default(), Value::Null)
            .expect("first review success");
        let review_assessment = RowQualityAssessment::create(
            &plan,
            &plan.items[0],
            &review_request,
            first_reviewer,
            quarantined_draft(&plan),
            std::slice::from_ref(&primary_assessment),
            Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
                .single()
                .expect("review time"),
        )
        .expect("quarantining first review");
        assert_eq!(review_assessment.verdict, QualityVerdict::Quarantined);

        assert!(
            !run.reconcile_from_evidence(
                &plan,
                &[primary_request.clone(), review_request.clone()],
                &[primary_attempt.clone(), review_attempt.clone()],
                &[primary_assessment.clone(), review_assessment.clone()],
            )
            .expect("reconcile partial strict review")
        );
        assert_eq!(run.progress.assessed_rows, 1);
        assert_eq!(run.progress.quarantined_rows, 1);
        assert_eq!(run.progress.pending_review_rows, 1);
        assert!(
            run.verify_against_evidence(
                &plan,
                &[primary_request, review_request],
                &[primary_attempt, review_attempt],
                &[primary_assessment, review_assessment],
            )
            .is_ok()
        );
    }

    #[test]
    fn persisted_failure_messages_are_bounded() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);
        let request = request(&plan, run.id, &row, &primary, 1, 1, 20, request_budget());
        run.reserve_attempt(&plan, &request, &primary)
            .expect("reserve attempt");
        let mut attempt = EvaluatorAttempt::start(&plan, &run, &request, primary).expect("attempt");
        let long_message = "failure".repeat(1_000);
        attempt
            .fail(
                ProviderUsage::default(),
                EvaluatorFailureKind::Provider,
                long_message.clone(),
                Value::Null,
            )
            .expect("bounded attempt failure");
        run.fail(QualityAuditStopReason::ProviderFailure, long_message)
            .expect("bounded run failure");

        assert_eq!(
            attempt
                .error_message
                .as_deref()
                .expect("attempt error")
                .chars()
                .count(),
            MAX_ERROR_MESSAGE_CHARACTERS
        );
        assert_eq!(
            run.error_message
                .as_deref()
                .expect("run error")
                .chars()
                .count(),
            MAX_ERROR_MESSAGE_CHARACTERS
        );
    }

    #[test]
    fn request_budget_overrun_preserves_actual_usage_in_terminal_failure_evidence() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut run = running_run(&plan, primary.clone(), vec![]);
        let request = request(&plan, run.id, &row, &primary, 1, 1, 13, request_budget());
        run.reserve_attempt(&plan, &request, &primary)
            .expect("reserve over-budget attempt");
        let mut attempt =
            EvaluatorAttempt::start(&plan, &run, &request, primary).expect("over-budget attempt");
        attempt
            .fail(
                over_request_usage(),
                EvaluatorFailureKind::BudgetExceeded,
                "provider exceeded the request input-token budget",
                json!({"observed_input_tokens": 1_001}),
            )
            .expect("record actual over-budget usage");

        assert!(
            run.reconcile_from_evidence(
                &plan,
                std::slice::from_ref(&request),
                std::slice::from_ref(&attempt),
                &[],
            )
            .expect("reconcile overrun")
        );
        assert_eq!(run.usage.input_tokens, 1_001);
        assert_eq!(run.usage.total_tokens, 1_002);
        run.fail(
            QualityAuditStopReason::BudgetExhausted,
            "provider exceeded the request budget",
        )
        .expect("terminate budget-exhausted run");

        assert!(
            run.verify_against_evidence(&plan, &[request], &[attempt], &[])
                .is_ok()
        );
    }

    #[test]
    fn request_budget_overrun_rejects_live_or_inconsistently_failed_evidence() {
        let (plan, row) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let primary = primary();
        let mut live_run = running_run(&plan, primary.clone(), vec![]);
        let request = request(
            &plan,
            live_run.id,
            &row,
            &primary,
            1,
            1,
            14,
            request_budget(),
        );
        live_run
            .reserve_attempt(&plan, &request, &primary)
            .expect("reserve over-budget attempt");
        let mut budget_attempt =
            EvaluatorAttempt::start(&plan, &live_run, &request, primary.clone())
                .expect("budget attempt");
        budget_attempt
            .fail(
                over_request_usage(),
                EvaluatorFailureKind::BudgetExceeded,
                "request budget exceeded",
                Value::Null,
            )
            .expect("record overrun");
        assert!(
            live_run
                .reconcile_from_evidence(
                    &plan,
                    std::slice::from_ref(&request),
                    std::slice::from_ref(&budget_attempt),
                    &[],
                )
                .expect("reconcile overrun")
        );

        assert_integrity_contains(
            live_run.verify_against_evidence(
                &plan,
                std::slice::from_ref(&request),
                std::slice::from_ref(&budget_attempt),
                &[],
            ),
            "budget-exhausted terminal run",
        );

        let mut wrong_run_reason = live_run.clone();
        wrong_run_reason
            .fail(
                QualityAuditStopReason::ProviderFailure,
                "incorrect terminal reason",
            )
            .expect("fail with wrong reason");
        assert_integrity_contains(
            wrong_run_reason.verify_against_evidence(
                &plan,
                std::slice::from_ref(&request),
                std::slice::from_ref(&budget_attempt),
                &[],
            ),
            "budget-exhausted terminal run",
        );

        let mut wrong_attempt_kind = EvaluatorAttempt::start(&plan, &live_run, &request, primary)
            .expect("wrong-kind attempt");
        wrong_attempt_kind
            .fail(
                over_request_usage(),
                EvaluatorFailureKind::Provider,
                "misclassified provider failure",
                Value::Null,
            )
            .expect("record wrong failure kind");
        let mut terminal_run = live_run;
        terminal_run
            .fail(
                QualityAuditStopReason::BudgetExhausted,
                "provider exceeded the request budget",
            )
            .expect("terminate budget-exhausted run");
        assert!(matches!(
            terminal_run.verify_against_evidence(&plan, &[request], &[wrong_attempt_kind], &[],),
            Err(QualityError::Integrity(_))
        ));
    }

    #[test]
    fn local_only_policy_rejects_external_primary_and_review_evaluators() {
        let (fast_plan, _) = fixture_plan(QualityPreset::Fast, EvaluatorEgressPolicy::LocalOnly);
        let external_primary = evaluator(
            "external-primary",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::ExternalService,
        );
        assert!(matches!(
            QualityAuditRun::queue(&fast_plan, external_primary, vec![]),
            Err(QualityError::Validation(_))
        ));

        let (balanced_plan, _) =
            fixture_plan(QualityPreset::Balanced, EvaluatorEgressPolicy::LocalOnly);
        let external_reviewer = evaluator(
            "external-reviewer",
            EvaluatorIndependence::IndependentReview,
            EvaluatorExecutionLocation::ExternalService,
        );
        assert!(matches!(
            QualityAuditRun::queue(&balanced_plan, primary(), vec![external_reviewer]),
            Err(QualityError::Validation(_))
        ));
    }
}
