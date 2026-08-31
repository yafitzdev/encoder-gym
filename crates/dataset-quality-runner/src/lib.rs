//! Durable, host-controlled execution for dataset quality audits.
//!
//! The runner owns local orchestration only. Quality policy, assessment
//! derivation, evidence reconciliation, and report construction remain in
//! `dataset-quality-core`; adapters continue to own candidate loading,
//! evaluator I/O, and persistence.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, TimeDelta, Utc};
use dataset_core::domain::SourceRow;
use dataset_quality_core::{
    QualityError,
    assessment::{
        BlindEvaluatorRequest, EvaluatorGuidance, EvaluatorIdentity, EvaluatorIndependence,
        EvaluatorRequestBudget, RowAssessmentDraft, RowQualityAssessment,
    },
    curation::DatasetQualityReport,
    lifecycle::{
        AuditExecutionLease, EvaluatorAttempt, EvaluatorAttemptState, EvaluatorFailureKind,
        FinishedAttemptEvidence, QualityAuditRun, QualityAuditRunState, QualityAuditStopReason,
    },
    policy::InvalidEvaluatorOutputPolicy,
    population::{AuditPlan, AuditSelection, CheckedAuditPlan},
    ports::{
        BoxFuture, DatasetQualityStore, EvaluatorBatchOutput, QualityAdapterError,
        QualityCandidateSource, QualityEvaluationError, QualityEvaluationErrorKind,
        QualityEvaluator,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_PERSISTED_METADATA_BYTES: usize = 60 * 1024;
const MAX_PERSISTED_ERROR_CHARACTERS: usize = 4_096;

#[cfg(test)]
tokio::task_local! {
    static SCHEDULER_CANDIDATE_VISITS: std::cell::Cell<usize>;
}

#[cfg(test)]
fn note_scheduler_candidate_visits(visited: usize) {
    let _ = SCHEDULER_CANDIDATE_VISITS.try_with(|count| count.set(count.get() + visited));
}

#[cfg(not(test))]
fn note_scheduler_candidate_visits(_visited: usize) {}

#[derive(Debug, Error)]
pub enum QualityRunnerError {
    #[error("dataset quality artifact was not found: {0}")]
    NotFound(String),
    #[error("dataset quality runner rejected the operation: {0}")]
    Validation(String),
    #[error(transparent)]
    Domain(#[from] QualityError),
    #[error(transparent)]
    Adapter(#[from] QualityAdapterError),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QualityAuditOutcome {
    pub run: QualityAuditRun,
    pub report: Option<DatasetQualityReport>,
}

/// Time is injectable because assessment review order is part of immutable
/// evidence. Implementations need not provide unique instants; the runner
/// deterministically advances a review past its predecessor when necessary.
pub trait QualityRunnerClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemQualityRunnerClock;

impl QualityRunnerClock for SystemQualityRunnerClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Bounded retry waiting is injectable so ordinary tests never sleep.
pub trait QualityRetrySleeper: Send + Sync {
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()>;
}

#[derive(Debug, Default)]
pub struct TokioQualityRetrySleeper;

impl QualityRetrySleeper for TokioQualityRetrySleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep(duration).await })
    }
}

pub struct DatasetQualityRunner {
    store: Arc<dyn DatasetQualityStore>,
    candidates: Arc<dyn QualityCandidateSource>,
    evaluators: BTreeMap<String, Arc<dyn QualityEvaluator>>,
    clock: Arc<dyn QualityRunnerClock>,
    sleeper: Arc<dyn QualityRetrySleeper>,
}

impl DatasetQualityRunner {
    pub fn new(
        store: Arc<dyn DatasetQualityStore>,
        candidates: Arc<dyn QualityCandidateSource>,
        evaluators: Vec<Arc<dyn QualityEvaluator>>,
    ) -> Result<Self, QualityRunnerError> {
        Self::with_runtime(
            store,
            candidates,
            evaluators,
            Arc::new(SystemQualityRunnerClock),
            Arc::new(TokioQualityRetrySleeper),
        )
    }

    pub fn with_runtime(
        store: Arc<dyn DatasetQualityStore>,
        candidates: Arc<dyn QualityCandidateSource>,
        evaluators: Vec<Arc<dyn QualityEvaluator>>,
        clock: Arc<dyn QualityRunnerClock>,
        sleeper: Arc<dyn QualityRetrySleeper>,
    ) -> Result<Self, QualityRunnerError> {
        if evaluators.is_empty() {
            return Err(QualityRunnerError::Validation(
                "at least one evaluator adapter is required".into(),
            ));
        }
        let mut by_fingerprint = BTreeMap::new();
        for evaluator in evaluators {
            let identity = evaluator.identity();
            identity.validate()?;
            if by_fingerprint
                .insert(identity.fingerprint.clone(), evaluator)
                .is_some()
            {
                return Err(QualityRunnerError::Validation(format!(
                    "evaluator identity {} is registered more than once",
                    identity.fingerprint
                )));
            }
        }
        Ok(Self {
            store,
            candidates,
            evaluators: by_fingerprint,
            clock,
            sleeper,
        })
    }

    /// Persists a cancellation request. A running call observes it before the
    /// next external request and after an in-flight response is durably closed.
    pub async fn request_cancel(
        &self,
        run_id: Uuid,
    ) -> Result<QualityAuditRun, QualityRunnerError> {
        let mut run = self.load_run(run_id).await?;
        run.request_cancel()?;
        if run.state == QualityAuditRunState::Queued {
            run.cancel()?;
        }
        self.store.save_audit_run(&run).await?;
        Ok(run)
    }

    /// Starts or resumes one local audit. Reinvocation is idempotent for a
    /// terminal run and uses persisted requests to resume interrupted work.
    pub async fn execute(&self, run_id: Uuid) -> Result<QualityAuditOutcome, QualityRunnerError> {
        let lease = self
            .store
            .acquire_audit_execution_lease(run_id, Uuid::new_v4())
            .await?;
        let outcome = self.execute_with_lease(run_id, &lease).await;
        let release = self.store.release_audit_execution_lease(&lease).await;
        match (outcome, release) {
            (Ok(outcome), Ok(())) => Ok(outcome),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    async fn execute_with_lease(
        &self,
        run_id: Uuid,
        lease: &AuditExecutionLease,
    ) -> Result<QualityAuditOutcome, QualityRunnerError> {
        let mut run = self.load_run(run_id).await?;
        let plan = self.load_plan(run.plan_id).await?;
        let checked_plan = CheckedAuditPlan::new(&plan)?;
        let guidance = self
            .store
            .get_audit_guidance(plan.id)
            .await?
            .ok_or_else(|| {
                QualityRunnerError::NotFound(format!(
                    "pinned evaluator guidance for audit plan {}",
                    plan.id
                ))
            })?;
        self.verify_execution_specification(&plan, &run, &guidance)?;

        if is_terminal(run.state) {
            return self.terminal_outcome(&plan, run).await;
        }
        if run.state == QualityAuditRunState::Queued {
            if run.cancel_requested {
                run.cancel()?;
                self.store.save_audit_run(&run).await?;
                return self.terminal_outcome(&plan, run).await;
            }
            run.start(&plan)?;
            self.store.save_audit_run(&run).await?;
        }

        let mut evidence = self
            .recover_started_attempts(&checked_plan, lease, &mut run)
            .await?;
        if self.refresh_cancellation(&mut run).await? {
            return self.terminal_outcome(&plan, run).await;
        }
        run.reconcile_from_evidence(
            &plan,
            &evidence.requests,
            &evidence.attempts,
            &evidence.assessments,
        )?;

        loop {
            if let Some(pending) = unresolved_logical_request(&evidence)? {
                match pending.disposition(&plan) {
                    PendingDisposition::Retry => {
                        self.retry_request(
                            &checked_plan,
                            lease,
                            &mut run,
                            &guidance,
                            &pending,
                            &mut evidence,
                        )
                        .await?;
                        if is_terminal(run.state) {
                            return self.terminal_outcome(&plan, run).await;
                        }
                        continue;
                    }
                    PendingDisposition::Exhausted => {
                        run.fail(
                            QualityAuditStopReason::ProviderFailure,
                            "evaluator request exhausted its finite attempt budget",
                        )?;
                        self.store.save_audit_run(&run).await?;
                        return self.terminal_outcome(&plan, run).await;
                    }
                    PendingDisposition::Permanent => {
                        run.fail(
                            QualityAuditStopReason::ProviderFailure,
                            "evaluator request ended with a permanent provider failure",
                        )?;
                        self.store.save_audit_run(&run).await?;
                        return self.terminal_outcome(&plan, run).await;
                    }
                }
            }

            if self.refresh_cancellation(&mut run).await? {
                return self.terminal_outcome(&plan, run).await;
            }

            if let Some(work) = next_logical_work(&run, &mut evidence)? {
                self.start_logical_request(
                    &checked_plan,
                    lease,
                    &mut run,
                    guidance.clone(),
                    work,
                    &mut evidence,
                )
                .await?;
                if is_terminal(run.state) {
                    return self.terminal_outcome(&plan, run).await;
                }
                continue;
            }

            let overrun = run.reconcile_from_evidence(
                &plan,
                &evidence.requests,
                &evidence.attempts,
                &evidence.assessments,
            )?;
            if overrun {
                run.fail(
                    QualityAuditStopReason::BudgetExhausted,
                    "observed provider usage exceeded a persisted audit budget",
                )?;
                self.store.save_audit_run(&run).await?;
                return self.terminal_outcome(&plan, run).await;
            }
            run.complete(&plan)?;
            run.verify_against_evidence(
                &plan,
                &evidence.requests,
                &evidence.attempts,
                &evidence.assessments,
            )?;
            self.store.save_audit_run(&run).await?;
            return self.completed_outcome(&plan, run, evidence).await;
        }
    }

    async fn terminal_outcome(
        &self,
        plan: &AuditPlan,
        run: QualityAuditRun,
    ) -> Result<QualityAuditOutcome, QualityRunnerError> {
        let evidence = self.load_evidence(plan, &run).await?;
        run.verify_against_evidence(
            plan,
            &evidence.requests,
            &evidence.attempts,
            &evidence.assessments,
        )?;
        if run.state == QualityAuditRunState::Completed {
            self.completed_outcome(plan, run, evidence).await
        } else {
            Ok(QualityAuditOutcome { run, report: None })
        }
    }

    async fn completed_outcome(
        &self,
        plan: &AuditPlan,
        run: QualityAuditRun,
        evidence: AuditEvidence,
    ) -> Result<QualityAuditOutcome, QualityRunnerError> {
        let report = match self.store.report_for_run(run.id).await? {
            Some(report) => {
                report.verify_against(
                    plan,
                    &run,
                    &evidence.requests,
                    &evidence.attempts,
                    &evidence.assessments,
                )?;
                report
            }
            None => {
                let report = DatasetQualityReport::create(
                    plan,
                    &run,
                    &evidence.requests,
                    &evidence.attempts,
                    &evidence.assessments,
                )?;
                report.verify_against(
                    plan,
                    &run,
                    &evidence.requests,
                    &evidence.attempts,
                    &evidence.assessments,
                )?;
                self.store.save_report(&report).await?;
                report
            }
        };
        Ok(QualityAuditOutcome {
            run,
            report: Some(report),
        })
    }

    async fn recover_started_attempts(
        &self,
        checked: &CheckedAuditPlan<'_>,
        lease: &AuditExecutionLease,
        run: &mut QualityAuditRun,
    ) -> Result<AuditEvidence, QualityRunnerError> {
        let plan = checked.plan();
        let mut evidence = self.load_evidence(plan, run).await?;
        let request_by_id = evidence
            .requests
            .iter()
            .map(|request| (request.id, request.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut started = evidence
            .attempts
            .iter()
            .filter(|attempt| attempt.state == EvaluatorAttemptState::Started)
            .cloned()
            .collect::<Vec<_>>();
        started.sort_by_key(|attempt| (attempt.request_sequence, attempt.attempt_number));
        for mut attempt in started {
            let request = request_by_id.get(&attempt.request_id).ok_or_else(|| {
                QualityRunnerError::Validation(format!(
                    "started attempt {} has no durable request",
                    attempt.id
                ))
            })?;
            attempt.interrupt()?;
            attempt.verify_with_context(checked, run, request)?;
            self.store
                .finish_attempt(checked, lease, &attempt, &[], run)
                .await?;
            evidence.replace_attempt(attempt)?;
        }
        run.reconcile_from_evidence(
            plan,
            &evidence.requests,
            &evidence.attempts,
            &evidence.assessments,
        )?;
        Ok(evidence)
    }

    async fn retry_request(
        &self,
        checked: &CheckedAuditPlan<'_>,
        lease: &AuditExecutionLease,
        run: &mut QualityAuditRun,
        guidance: &EvaluatorGuidance,
        pending: &PendingRequest,
        evidence: &mut AuditEvidence,
    ) -> Result<(), QualityRunnerError> {
        let plan = checked.plan();
        let delay = retry_delay(pending.last_error.as_ref());
        if !delay.is_zero() {
            self.sleeper.sleep(delay).await;
        }
        if self.refresh_cancellation(run).await? {
            return Ok(());
        }
        let evaluator = self.evaluator(&pending.evaluator)?;
        let fixed_budget = match fixed_request_budget(plan) {
            Ok(budget) => budget,
            Err(error) => {
                run.fail(QualityAuditStopReason::BudgetExhausted, error.to_string())?;
                self.store.save_audit_run(run).await?;
                return Ok(());
            }
        };
        if pending.budget != fixed_budget
            || !request_budget_fits_remaining(plan, run, pending.budget)
        {
            run.fail(
                QualityAuditStopReason::BudgetExhausted,
                "retry request does not fit the deterministic remaining audit budget",
            )?;
            self.store.save_audit_run(run).await?;
            return Ok(());
        }
        let rows = match self
            .load_exact_rows(plan, pending.source_row_ids.clone())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                run.fail(
                    QualityAuditStopReason::ProviderFailure,
                    format!("candidate rows could not be reloaded for retry: {error}"),
                )?;
                self.store.save_audit_run(run).await?;
                return Ok(());
            }
        };
        let request = BlindEvaluatorRequest::create_with_context(
            checked,
            Uuid::new_v4(),
            run.id,
            Uuid::new_v4(),
            pending.request_sequence,
            pending.next_attempt_number,
            &pending.evaluator,
            rows,
            guidance.clone(),
            plan.resolved_guidance_fingerprint.clone(),
            pending.budget,
        )?;
        if request.reproduce_retry_payload_fingerprint()? != pending.retry_payload_fingerprint {
            return Err(QualityRunnerError::Validation(
                "retry payload drifted from its durable logical request".into(),
            ));
        }
        self.perform_attempt(checked, lease, run, request, evaluator, evidence)
            .await
    }

    async fn start_logical_request(
        &self,
        checked: &CheckedAuditPlan<'_>,
        lease: &AuditExecutionLease,
        run: &mut QualityAuditRun,
        guidance: EvaluatorGuidance,
        work: LogicalWork,
        evidence: &mut AuditEvidence,
    ) -> Result<(), QualityRunnerError> {
        let plan = checked.plan();
        let evaluator = self.evaluator(&work.evaluator)?;
        let rows = match self.load_exact_rows(plan, work.source_row_ids).await {
            Ok(rows) => rows,
            Err(error) => {
                run.fail(
                    QualityAuditStopReason::ProviderFailure,
                    format!("candidate rows could not be loaded: {error}"),
                )?;
                self.store.save_audit_run(run).await?;
                return Ok(());
            }
        };
        let budget = match fixed_request_budget(plan) {
            Ok(budget) => budget,
            Err(error) => {
                run.fail(QualityAuditStopReason::BudgetExhausted, error.to_string())?;
                self.store.save_audit_run(run).await?;
                return Ok(());
            }
        };
        if !request_budget_fits_remaining(plan, run, budget) {
            run.fail(
                QualityAuditStopReason::BudgetExhausted,
                "next evaluator request does not fit the remaining audit budget",
            )?;
            self.store.save_audit_run(run).await?;
            return Ok(());
        }
        let request = BlindEvaluatorRequest::create_with_context(
            checked,
            Uuid::new_v4(),
            run.id,
            Uuid::new_v4(),
            work.request_sequence,
            1,
            &work.evaluator,
            rows,
            guidance,
            plan.resolved_guidance_fingerprint.clone(),
            budget,
        )?;
        self.perform_attempt(checked, lease, run, request, evaluator, evidence)
            .await
    }

    async fn perform_attempt(
        &self,
        checked: &CheckedAuditPlan<'_>,
        lease: &AuditExecutionLease,
        run: &mut QualityAuditRun,
        request: BlindEvaluatorRequest,
        evaluator: Arc<dyn QualityEvaluator>,
        evidence: &mut AuditEvidence,
    ) -> Result<(), QualityRunnerError> {
        let plan = checked.plan();
        let identity = evaluator.identity();
        let mut attempt =
            EvaluatorAttempt::start_with_context(checked, run, &request, identity.clone())?;
        run.reserve_attempt_with_context(checked, &request, &identity)?;
        self.store
            .record_attempt(checked, lease, &attempt, &request, run)
            .await?;
        evidence.push_attempt(request.clone(), attempt.clone())?;

        let result = evaluator.evaluate(request.clone()).await;
        let (assessments, terminal_message) = match result {
            Ok(output) => self.normalize_success(
                NormalizationContext {
                    plan,
                    checked,
                    run,
                    request: &request,
                    identity: &identity,
                    evidence,
                },
                &mut attempt,
                output,
            )?,
            Err(error) => self.normalize_failure(plan, run, &request, &mut attempt, error)?,
        };

        let prior_assessments = evidence.assessments_for_rows(&attempt.source_row_ids);
        let prior_invalid_attempts = evidence.invalid_attempts_for(&attempt.source_row_ids);
        let overrun = run.reconcile_finished_attempt_with_context(
            checked,
            FinishedAttemptEvidence {
                request: &request,
                previous_attempt: evidence.attempt(request.attempt_id)?,
                finished_attempt: &attempt,
                prior_assessments: &prior_assessments,
                prior_invalid_attempts: &prior_invalid_attempts,
                new_assessments: &assessments,
            },
        )?;
        evidence.replace_attempt(attempt.clone())?;
        evidence.extend_assessments(&assessments);

        let cancellation_requested = self.persisted_cancel_requested(run.id).await?;
        if cancellation_requested {
            run.request_cancel()?;
            run.cancel()?;
        } else if overrun {
            run.fail(
                QualityAuditStopReason::BudgetExhausted,
                "provider usage exceeded the request or aggregate audit budget",
            )?;
        } else if terminal_message.is_some()
            && plan.policy.invalid_output_policy == InvalidEvaluatorOutputPolicy::FailAudit
            && attempt.state == EvaluatorAttemptState::InvalidResponse
        {
            run.fail(
                QualityAuditStopReason::InvalidResponse,
                terminal_message
                    .clone()
                    .expect("invalid response has a terminal message"),
            )?;
        }
        self.store
            .finish_attempt(checked, lease, &attempt, &assessments, run)
            .await?;
        Ok(())
    }

    fn normalize_success(
        &self,
        context: NormalizationContext<'_>,
        attempt: &mut EvaluatorAttempt,
        output: EvaluatorBatchOutput,
    ) -> Result<(Vec<RowQualityAssessment>, Option<String>), QualityRunnerError> {
        let NormalizationContext {
            plan,
            checked,
            run,
            request,
            identity,
            evidence,
        } = context;
        let EvaluatorBatchOutput {
            assessments: drafts,
            usage,
            metadata,
        } = output;
        let (metadata, metadata_was_oversized) = bounded_metadata(metadata);
        if let Err(error) = usage.validate() {
            let message = format!("provider usage was internally inconsistent: {error}");
            attempt.invalidate(
                Default::default(),
                request_source_ids(request),
                message.clone(),
                json!({
                    "provider_metadata": metadata,
                    "reported_usage": usage,
                }),
            )?;
            return Ok((Vec::new(), Some(message)));
        }
        if provider_usage_overruns(plan, run, request, usage)? {
            let message = "provider usage exceeded the request or audit budget".to_owned();
            attempt.fail(
                usage,
                EvaluatorFailureKind::BudgetExceeded,
                message.clone(),
                metadata,
            )?;
            return Ok((Vec::new(), Some(message)));
        }
        if metadata_was_oversized {
            let message = "provider response metadata exceeded the persisted evidence limit";
            attempt.invalidate(usage, request_source_ids(request), message, metadata)?;
            return Ok((Vec::new(), Some(message.into())));
        }
        let malformed = validate_output_shape(request, &drafts)
            .err()
            .map(|error| error.to_string());
        if let Some(message) = malformed {
            attempt.invalidate(
                usage,
                request_source_ids(request),
                message.clone(),
                metadata,
            )?;
            return Ok((Vec::new(), Some(message)));
        }

        let drafts = drafts
            .into_iter()
            .map(|draft| (draft.source_row_id, draft))
            .collect::<BTreeMap<_, _>>();
        let mut assessments = Vec::with_capacity(request.rows.len());
        for row in &request.rows {
            let item = checked.item(row.source_row_id).ok_or_else(|| {
                QualityRunnerError::Validation(
                    "request row disappeared from its immutable plan".into(),
                )
            })?;
            let row_prior = evidence.assessments_for(row.source_row_id);
            let created_at = next_assessment_time(self.clock.now(), &row_prior);
            let draft = drafts
                .get(&row.source_row_id)
                .expect("shape validation guarantees every draft")
                .clone();
            match RowQualityAssessment::create_with_context(
                checked,
                item,
                request,
                identity.clone(),
                draft,
                &row_prior,
                created_at,
            ) {
                Ok(assessment) => assessments.push(assessment),
                Err(error) => {
                    let message = format!("evaluator output failed strict normalization: {error}");
                    attempt.invalidate(
                        usage,
                        request_source_ids(request),
                        message.clone(),
                        metadata,
                    )?;
                    return Ok((Vec::new(), Some(message)));
                }
            }
        }
        attempt.succeed(usage, metadata)?;
        Ok((assessments, None))
    }

    fn normalize_failure(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        request: &BlindEvaluatorRequest,
        attempt: &mut EvaluatorAttempt,
        error: QualityEvaluationError,
    ) -> Result<(Vec<RowQualityAssessment>, Option<String>), QualityRunnerError> {
        let QualityEvaluationError {
            kind,
            message,
            retry_after_millis,
            observed_usage,
            metadata: provider_metadata,
        } = error;
        let (provider_metadata, _) = bounded_metadata(provider_metadata);
        let message = bounded_error_message(message);
        let metadata = json!({
            "retry_after_millis": retry_after_millis,
            "provider_metadata": provider_metadata,
        });
        if let Err(usage_error) = observed_usage.validate() {
            let message = format!("provider usage was internally inconsistent: {usage_error}");
            attempt.invalidate(
                Default::default(),
                request_source_ids(request),
                message.clone(),
                json!({
                    "provider_metadata": metadata,
                    "reported_usage": observed_usage,
                }),
            )?;
            return Ok((Vec::new(), Some(message)));
        }
        if provider_usage_overruns(plan, run, request, observed_usage)? {
            let message = "provider usage exceeded the request or audit budget".to_owned();
            attempt.fail(
                observed_usage,
                EvaluatorFailureKind::BudgetExceeded,
                message.clone(),
                metadata,
            )?;
            return Ok((Vec::new(), Some(message)));
        }
        if kind == QualityEvaluationErrorKind::InvalidResponse {
            attempt.invalidate(
                observed_usage,
                request_source_ids(request),
                message.clone(),
                metadata,
            )?;
            return Ok((Vec::new(), Some(message)));
        }
        let kind = kind.attempt_failure_kind().ok_or_else(|| {
            QualityRunnerError::Validation(
                "invalid-response errors must use the invalid attempt path".into(),
            )
        })?;
        attempt.fail(observed_usage, kind, message, metadata)?;
        Ok((Vec::new(), None))
    }

    async fn refresh_cancellation(
        &self,
        run: &mut QualityAuditRun,
    ) -> Result<bool, QualityRunnerError> {
        if self.persisted_cancel_requested(run.id).await? {
            if !run.cancel_requested {
                run.request_cancel()?;
            }
            run.cancel()?;
            self.store.save_audit_run(run).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn persisted_cancel_requested(&self, run_id: Uuid) -> Result<bool, QualityRunnerError> {
        self.store
            .audit_cancel_requested(run_id)
            .await?
            .ok_or_else(|| QualityRunnerError::NotFound(format!("audit run {run_id}")))
    }

    async fn load_exact_rows(
        &self,
        plan: &AuditPlan,
        mut source_row_ids: Vec<Uuid>,
    ) -> Result<Vec<SourceRow>, QualityRunnerError> {
        source_row_ids.sort_unstable();
        source_row_ids.dedup();
        let rows = self
            .candidates
            .get_source_rows(
                plan.dataset_schema.dataset_definition_id,
                source_row_ids.clone(),
            )
            .await?;
        let returned_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
        if returned_ids != source_row_ids {
            return Err(QualityRunnerError::Validation(
                "candidate source returned a partial, duplicate, or non-canonical row set".into(),
            ));
        }
        Ok(rows)
    }

    fn evaluator(
        &self,
        identity: &EvaluatorIdentity,
    ) -> Result<Arc<dyn QualityEvaluator>, QualityRunnerError> {
        let evaluator = self
            .evaluators
            .get(&identity.fingerprint)
            .cloned()
            .ok_or_else(|| {
                QualityRunnerError::Validation(format!(
                    "no adapter is registered for evaluator {}",
                    identity.fingerprint
                ))
            })?;
        if evaluator.identity() != *identity {
            return Err(QualityRunnerError::Validation(
                "registered evaluator identity changed after runner construction".into(),
            ));
        }
        Ok(evaluator)
    }

    fn verify_execution_specification(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        guidance: &EvaluatorGuidance,
    ) -> Result<(), QualityRunnerError> {
        run.verify_integrity(plan)?;
        guidance.verify_against(plan)?;
        for identity in
            std::iter::once(&run.primary_evaluator).chain(run.independent_reviewers.iter())
        {
            self.evaluator(identity)?;
        }
        Ok(())
    }

    async fn load_plan(&self, plan_id: Uuid) -> Result<AuditPlan, QualityRunnerError> {
        self.store
            .get_audit_plan(plan_id)
            .await?
            .ok_or_else(|| QualityRunnerError::NotFound(format!("audit plan {plan_id}")))
    }

    async fn load_run(&self, run_id: Uuid) -> Result<QualityAuditRun, QualityRunnerError> {
        self.store
            .get_audit_run(run_id)
            .await?
            .ok_or_else(|| QualityRunnerError::NotFound(format!("audit run {run_id}")))
    }

    async fn load_evidence(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
    ) -> Result<AuditEvidence, QualityRunnerError> {
        AuditEvidence::new(
            plan,
            run,
            self.store.list_evaluator_requests(run.id).await?,
            self.store.list_attempts(run.id).await?,
            self.store.list_assessments(run.id).await?,
        )
    }
}

fn is_terminal(state: QualityAuditRunState) -> bool {
    matches!(
        state,
        QualityAuditRunState::Completed
            | QualityAuditRunState::Failed
            | QualityAuditRunState::Cancelled
    )
}

#[derive(Debug)]
struct AuditEvidence {
    requests: Vec<BlindEvaluatorRequest>,
    attempts: Vec<EvaluatorAttempt>,
    assessments: Vec<RowQualityAssessment>,
    request_indices_by_id: BTreeMap<Uuid, usize>,
    attempt_indices_by_id: BTreeMap<Uuid, usize>,
    latest_attempt_by_sequence: BTreeMap<u32, usize>,
    unresolved_sequences: BTreeSet<u32>,
    assessment_indices_by_row: BTreeMap<Uuid, Vec<usize>>,
    invalid_attempt_indices_by_row: BTreeMap<Uuid, Vec<usize>>,
    any_invalid_rows: BTreeSet<Uuid>,
    primary_pending: BTreeSet<Uuid>,
    reviewer_pending: BTreeMap<String, BTreeSet<Uuid>>,
    reviewer_order: Vec<String>,
    batch_size: usize,
    next_request_sequence: u32,
}

impl AuditEvidence {
    fn new(
        plan: &AuditPlan,
        run: &QualityAuditRun,
        requests: Vec<BlindEvaluatorRequest>,
        attempts: Vec<EvaluatorAttempt>,
        assessments: Vec<RowQualityAssessment>,
    ) -> Result<Self, QualityRunnerError> {
        let mut request_indices_by_id = BTreeMap::new();
        for (index, request) in requests.iter().enumerate() {
            if request_indices_by_id.insert(request.id, index).is_some() {
                return Err(QualityRunnerError::Validation(
                    "durable evaluator request identity is duplicated".into(),
                ));
            }
        }
        let mut attempt_indices_by_id = BTreeMap::new();
        let mut latest_attempt_by_sequence = BTreeMap::<u32, usize>::new();
        let mut unresolved_sequences = BTreeSet::new();
        let mut maximum_sequence = 0_u32;
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt_indices_by_id.insert(attempt.id, index).is_some() {
                return Err(QualityRunnerError::Validation(
                    "durable evaluator attempt identity is duplicated".into(),
                ));
            }
            maximum_sequence = maximum_sequence.max(attempt.request_sequence);
            match latest_attempt_by_sequence.get(&attempt.request_sequence) {
                Some(previous) if attempts[*previous].attempt_number >= attempt.attempt_number => {}
                _ => {
                    latest_attempt_by_sequence.insert(attempt.request_sequence, index);
                }
            }
        }
        for (sequence, index) in &latest_attempt_by_sequence {
            if !matches!(
                attempts[*index].state,
                EvaluatorAttemptState::Succeeded | EvaluatorAttemptState::InvalidResponse
            ) {
                unresolved_sequences.insert(*sequence);
            }
        }
        let mut assessment_indices_by_row = BTreeMap::<Uuid, Vec<usize>>::new();
        for (index, assessment) in assessments.iter().enumerate() {
            assessment_indices_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(index);
        }
        let mut invalid_attempt_indices_by_row = BTreeMap::<Uuid, Vec<usize>>::new();
        let mut any_invalid_rows = BTreeSet::new();
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt.state == EvaluatorAttemptState::InvalidResponse {
                for source_row_id in &attempt.invalid_source_row_ids {
                    any_invalid_rows.insert(*source_row_id);
                    invalid_attempt_indices_by_row
                        .entry(*source_row_id)
                        .or_default()
                        .push(index);
                }
            }
        }
        let selected = plan
            .items
            .iter()
            .filter(|item| item.selection == AuditSelection::Selected)
            .map(|item| item.source_row_id)
            .collect::<BTreeSet<_>>();
        let owned_by_evaluator = attempts.iter().fold(
            BTreeMap::<String, BTreeSet<Uuid>>::new(),
            |mut owned, attempt| {
                owned
                    .entry(attempt.evaluator.fingerprint.clone())
                    .or_default()
                    .extend(attempt.source_row_ids.iter().copied());
                owned
            },
        );
        let primary_owned = owned_by_evaluator
            .get(&run.primary_evaluator.fingerprint)
            .cloned()
            .unwrap_or_default();
        let primary_pending = selected
            .difference(&primary_owned)
            .copied()
            .collect::<BTreeSet<_>>();
        let primary_borderline = assessments
            .iter()
            .filter(|assessment| {
                assessment.evaluator == run.primary_evaluator
                    && assessment.verdict
                        == dataset_quality_core::assessment::QualityVerdict::Borderline
            })
            .map(|assessment| assessment.source_row_id)
            .collect::<BTreeSet<_>>();
        let reviewer_order = run
            .independent_reviewers
            .iter()
            .map(|reviewer| reviewer.fingerprint.clone())
            .collect::<Vec<_>>();
        let reviewer_pending = reviewer_order
            .iter()
            .map(|fingerprint| {
                let owned = owned_by_evaluator
                    .get(fingerprint)
                    .cloned()
                    .unwrap_or_default();
                let pending = primary_borderline
                    .difference(&owned)
                    .copied()
                    .filter(|source_row_id| !any_invalid_rows.contains(source_row_id))
                    .collect();
                (fingerprint.clone(), pending)
            })
            .collect();
        let next_request_sequence = maximum_sequence
            .checked_add(1)
            .ok_or_else(|| QualityRunnerError::Validation("request sequence overflowed".into()))?;
        Ok(Self {
            requests,
            attempts,
            assessments,
            request_indices_by_id,
            attempt_indices_by_id,
            latest_attempt_by_sequence,
            unresolved_sequences,
            assessment_indices_by_row,
            invalid_attempt_indices_by_row,
            any_invalid_rows,
            primary_pending,
            reviewer_pending,
            reviewer_order,
            batch_size: plan.policy.budgets.maximum_rows_per_batch as usize,
            next_request_sequence,
        })
    }

    fn assessments_for(&self, source_row_id: Uuid) -> Vec<RowQualityAssessment> {
        self.assessment_indices_by_row
            .get(&source_row_id)
            .into_iter()
            .flatten()
            .map(|index| self.assessments[*index].clone())
            .collect()
    }

    fn extend_assessments(&mut self, values: &[RowQualityAssessment]) {
        for assessment in values {
            let index = self.assessments.len();
            self.assessments.push(assessment.clone());
            self.assessment_indices_by_row
                .entry(assessment.source_row_id)
                .or_default()
                .push(index);
            if assessment.evaluator.independence == EvaluatorIndependence::Primary
                && assessment.verdict
                    == dataset_quality_core::assessment::QualityVerdict::Borderline
                && !self.any_invalid_rows.contains(&assessment.source_row_id)
            {
                for pending in self.reviewer_pending.values_mut() {
                    pending.insert(assessment.source_row_id);
                }
            }
        }
    }

    fn assessments_for_rows(&self, source_row_ids: &[Uuid]) -> Vec<RowQualityAssessment> {
        source_row_ids
            .iter()
            .flat_map(|source_row_id| self.assessments_for(*source_row_id))
            .collect()
    }

    fn invalid_attempts_for(&self, source_row_ids: &[Uuid]) -> Vec<EvaluatorAttempt> {
        let indices = source_row_ids
            .iter()
            .filter_map(|source_row_id| self.invalid_attempt_indices_by_row.get(source_row_id))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        indices
            .into_iter()
            .map(|index| self.attempts[index].clone())
            .collect()
    }

    fn attempt(&self, id: Uuid) -> Result<&EvaluatorAttempt, QualityRunnerError> {
        self.attempt_indices_by_id
            .get(&id)
            .map(|index| &self.attempts[*index])
            .ok_or_else(|| {
                QualityRunnerError::Validation(
                    "reserved evaluator attempt was not durably visible before provider I/O".into(),
                )
            })
    }

    fn push_attempt(
        &mut self,
        request: BlindEvaluatorRequest,
        attempt: EvaluatorAttempt,
    ) -> Result<(), QualityRunnerError> {
        if self.request_indices_by_id.contains_key(&request.id)
            || self.attempt_indices_by_id.contains_key(&attempt.id)
        {
            return Err(QualityRunnerError::Validation(
                "new evaluator request or attempt identity is duplicated".into(),
            ));
        }
        if attempt.request_sequence >= self.next_request_sequence {
            self.next_request_sequence =
                attempt.request_sequence.checked_add(1).ok_or_else(|| {
                    QualityRunnerError::Validation("request sequence overflowed".into())
                })?;
        }
        match attempt.evaluator.independence {
            EvaluatorIndependence::Primary => {
                for source_row_id in &attempt.source_row_ids {
                    self.primary_pending.remove(source_row_id);
                }
            }
            EvaluatorIndependence::IndependentReview => {
                let pending = self
                    .reviewer_pending
                    .get_mut(&attempt.evaluator.fingerprint)
                    .ok_or_else(|| {
                        QualityRunnerError::Validation(
                            "attempt uses an unpinned independent reviewer".into(),
                        )
                    })?;
                for source_row_id in &attempt.source_row_ids {
                    pending.remove(source_row_id);
                }
            }
        }
        let request_index = self.requests.len();
        self.request_indices_by_id.insert(request.id, request_index);
        self.requests.push(request);
        let attempt_index = self.attempts.len();
        self.attempt_indices_by_id.insert(attempt.id, attempt_index);
        self.latest_attempt_by_sequence
            .insert(attempt.request_sequence, attempt_index);
        self.unresolved_sequences.insert(attempt.request_sequence);
        self.attempts.push(attempt);
        Ok(())
    }

    fn replace_attempt(&mut self, replacement: EvaluatorAttempt) -> Result<(), QualityRunnerError> {
        let index = self
            .attempt_indices_by_id
            .get(&replacement.id)
            .copied()
            .ok_or_else(|| {
                QualityRunnerError::Validation(
                    "reserved evaluator attempt was not durably visible before provider I/O".into(),
                )
            })?;
        let previous = &self.attempts[index];
        for source_row_id in &previous.invalid_source_row_ids {
            if let Some(indices) = self.invalid_attempt_indices_by_row.get_mut(source_row_id) {
                indices.retain(|value| *value != index);
            }
        }
        self.invalid_attempt_indices_by_row
            .retain(|_, indices| !indices.is_empty());
        self.attempts[index] = replacement;
        self.latest_attempt_by_sequence
            .insert(self.attempts[index].request_sequence, index);
        if matches!(
            self.attempts[index].state,
            EvaluatorAttemptState::Succeeded | EvaluatorAttemptState::InvalidResponse
        ) {
            self.unresolved_sequences
                .remove(&self.attempts[index].request_sequence);
        } else {
            self.unresolved_sequences
                .insert(self.attempts[index].request_sequence);
        }
        if self.attempts[index].state == EvaluatorAttemptState::InvalidResponse {
            for source_row_id in &self.attempts[index].invalid_source_row_ids {
                self.any_invalid_rows.insert(*source_row_id);
                self.primary_pending.remove(source_row_id);
                for pending in self.reviewer_pending.values_mut() {
                    pending.remove(source_row_id);
                }
                self.invalid_attempt_indices_by_row
                    .entry(*source_row_id)
                    .or_default()
                    .push(index);
            }
        }
        Ok(())
    }

    fn unresolved_logical_request(&self) -> Result<Option<PendingRequest>, QualityRunnerError> {
        let Some(sequence) = self.unresolved_sequences.first().copied() else {
            return Ok(None);
        };
        let attempt_index = self
            .latest_attempt_by_sequence
            .get(&sequence)
            .copied()
            .ok_or_else(|| {
                QualityRunnerError::Validation(
                    "unresolved request has no durable evaluator attempt".into(),
                )
            })?;
        let last = &self.attempts[attempt_index];
        if last.state == EvaluatorAttemptState::Started {
            return Err(QualityRunnerError::Validation(
                "started evaluator attempt must be recovered before scheduling".into(),
            ));
        }
        let request = self
            .request_indices_by_id
            .get(&last.request_id)
            .map(|index| &self.requests[*index])
            .ok_or_else(|| {
                QualityRunnerError::Validation(format!(
                    "attempt {} has no durable evaluator request",
                    last.id
                ))
            })?;
        Ok(Some(PendingRequest {
            request_sequence: sequence,
            next_attempt_number: last.attempt_number.saturating_add(1),
            evaluator: last.evaluator.clone(),
            source_row_ids: last.source_row_ids.clone(),
            budget: request.budget,
            retry_payload_fingerprint: last.retry_payload_fingerprint.clone(),
            last_state: last.state,
            last_failure_kind: last.failure_kind,
            last_error: last
                .error_message
                .as_ref()
                .map(|message| QualityEvaluationError {
                    kind: failure_error_kind(last.failure_kind),
                    message: message.clone(),
                    retry_after_millis: request_retry_after(last),
                    observed_usage: Default::default(),
                    metadata: last.response_metadata.clone(),
                }),
        }))
    }

    fn next_logical_work(&mut self, run: &QualityAuditRun) -> Option<LogicalWork> {
        let primary = self
            .primary_pending
            .iter()
            .take(self.batch_size)
            .copied()
            .collect::<Vec<_>>();
        note_scheduler_candidate_visits(primary.len());
        if !primary.is_empty() {
            return Some(LogicalWork {
                request_sequence: self.next_request_sequence,
                evaluator: run.primary_evaluator.clone(),
                source_row_ids: primary,
            });
        }
        for (reviewer, fingerprint) in run.independent_reviewers.iter().zip(&self.reviewer_order) {
            let rows = self
                .reviewer_pending
                .get(fingerprint)
                .into_iter()
                .flatten()
                .take(self.batch_size)
                .copied()
                .collect::<Vec<_>>();
            note_scheduler_candidate_visits(rows.len());
            if !rows.is_empty() {
                return Some(LogicalWork {
                    request_sequence: self.next_request_sequence,
                    evaluator: reviewer.clone(),
                    source_row_ids: rows,
                });
            }
        }
        None
    }
}

struct NormalizationContext<'a> {
    plan: &'a AuditPlan,
    checked: &'a CheckedAuditPlan<'a>,
    run: &'a QualityAuditRun,
    request: &'a BlindEvaluatorRequest,
    identity: &'a EvaluatorIdentity,
    evidence: &'a AuditEvidence,
}

#[derive(Debug)]
struct LogicalWork {
    request_sequence: u32,
    evaluator: EvaluatorIdentity,
    source_row_ids: Vec<Uuid>,
}

#[derive(Debug)]
struct PendingRequest {
    request_sequence: u32,
    next_attempt_number: u32,
    evaluator: EvaluatorIdentity,
    source_row_ids: Vec<Uuid>,
    budget: EvaluatorRequestBudget,
    retry_payload_fingerprint: String,
    last_state: EvaluatorAttemptState,
    last_failure_kind: Option<EvaluatorFailureKind>,
    last_error: Option<QualityEvaluationError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingDisposition {
    Retry,
    Exhausted,
    Permanent,
}

impl PendingRequest {
    fn disposition(&self, plan: &AuditPlan) -> PendingDisposition {
        if self.next_attempt_number > plan.policy.budgets.maximum_attempts_per_request {
            return PendingDisposition::Exhausted;
        }
        match self.last_state {
            EvaluatorAttemptState::Interrupted => PendingDisposition::Retry,
            EvaluatorAttemptState::Failed
                if matches!(
                    self.last_failure_kind,
                    Some(
                        EvaluatorFailureKind::RateLimit
                            | EvaluatorFailureKind::Transport
                            | EvaluatorFailureKind::Provider
                    )
                ) =>
            {
                PendingDisposition::Retry
            }
            EvaluatorAttemptState::Failed => PendingDisposition::Permanent,
            _ => PendingDisposition::Permanent,
        }
    }
}

fn unresolved_logical_request(
    evidence: &AuditEvidence,
) -> Result<Option<PendingRequest>, QualityRunnerError> {
    evidence.unresolved_logical_request()
}

fn next_logical_work(
    run: &QualityAuditRun,
    evidence: &mut AuditEvidence,
) -> Result<Option<LogicalWork>, QualityRunnerError> {
    Ok(evidence.next_logical_work(run))
}

fn validate_output_shape(
    request: &BlindEvaluatorRequest,
    drafts: &[RowAssessmentDraft],
) -> Result<(), QualityRunnerError> {
    let requested = request_source_ids(request)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let returned = drafts
        .iter()
        .map(|draft| draft.source_row_id)
        .collect::<Vec<_>>();
    let unique = returned.iter().copied().collect::<BTreeSet<_>>();
    if returned.len() != requested.len() || unique.len() != returned.len() || unique != requested {
        return Err(QualityRunnerError::Validation(
            "evaluator response must contain exactly one draft for every requested row".into(),
        ));
    }
    for draft in drafts {
        let expected = request
            .rows
            .iter()
            .find(|row| row.source_row_id == draft.source_row_id)
            .expect("source IDs were validated");
        if draft.source_row_fingerprint != expected.source_row_fingerprint {
            return Err(QualityRunnerError::Validation(
                "evaluator draft changed a source-row fingerprint".into(),
            ));
        }
    }
    Ok(())
}

/// Divides every aggregate provider ceiling across the plan's worst-case
/// number of transport attempts. Integer remainders remain deliberately
/// unallocated, which is deterministic and ensures retry attempts keep the
/// exact same payload authorization.
fn fixed_request_budget(plan: &AuditPlan) -> Result<EvaluatorRequestBudget, QualityRunnerError> {
    let budgets = &plan.policy.budgets;
    let rounds = 1_u64
        + u64::from(
            plan.policy
                .borderline_review_policy
                .required_additional_assessments(),
        );
    let logical_requests = budgets.required_requests_for_rows(plan.selected_count(), rounds)?;
    let worst_case_attempts = logical_requests
        .checked_mul(u64::from(budgets.maximum_attempts_per_request))
        .ok_or_else(|| {
            QualityRunnerError::Domain(QualityError::BudgetExhausted(
                "worst-case evaluator attempt count overflowed".into(),
            ))
        })?;
    if worst_case_attempts == 0 {
        return Err(QualityRunnerError::Domain(QualityError::BudgetExhausted(
            "audit has no finite evaluator attempt allocation".into(),
        )));
    }
    let input = budgets.maximum_input_tokens / worst_case_attempts;
    let output = budgets.maximum_output_tokens / worst_case_attempts;
    let total =
        (budgets.maximum_total_tokens / worst_case_attempts).min(input.saturating_add(output));
    let cost = budgets
        .maximum_cost_microusd
        .map(|maximum| maximum / worst_case_attempts);
    if input == 0 || output == 0 || total == 0 || cost == Some(0) {
        return Err(QualityRunnerError::Domain(QualityError::BudgetExhausted(
            "aggregate provider budget cannot authorize every worst-case attempt".into(),
        )));
    }
    Ok(EvaluatorRequestBudget {
        maximum_input_tokens: input,
        maximum_output_tokens: output,
        maximum_total_tokens: total,
        maximum_cost_microusd: cost,
    })
}

fn request_budget_fits_remaining(
    plan: &AuditPlan,
    run: &QualityAuditRun,
    budget: EvaluatorRequestBudget,
) -> bool {
    let aggregate = &plan.policy.budgets;
    budget.maximum_input_tokens
        <= aggregate
            .maximum_input_tokens
            .saturating_sub(run.usage.input_tokens)
        && budget.maximum_output_tokens
            <= aggregate
                .maximum_output_tokens
                .saturating_sub(run.usage.output_tokens)
        && budget.maximum_total_tokens
            <= aggregate
                .maximum_total_tokens
                .saturating_sub(run.usage.total_tokens)
        && match (
            budget.maximum_cost_microusd,
            aggregate.maximum_cost_microusd,
        ) {
            (Some(request), Some(maximum)) => {
                request <= maximum.saturating_sub(run.usage.cost_microusd)
            }
            (None, None) => true,
            _ => false,
        }
}

fn provider_usage_overruns(
    plan: &AuditPlan,
    run: &QualityAuditRun,
    request: &BlindEvaluatorRequest,
    usage: dataset_quality_core::lifecycle::ProviderUsage,
) -> Result<bool, QualityRunnerError> {
    usage.validate()?;
    let request_overrun = usage.input_tokens > request.budget.maximum_input_tokens
        || usage.output_tokens > request.budget.maximum_output_tokens
        || usage.total_tokens > request.budget.maximum_total_tokens
        || request
            .budget
            .maximum_cost_microusd
            .is_some_and(|maximum| usage.cost_microusd > maximum);
    let aggregate = run.usage.checked_add_provider(usage)?;
    Ok(request_overrun || aggregate.exceeds(&plan.policy.budgets))
}

fn request_source_ids(request: &BlindEvaluatorRequest) -> Vec<Uuid> {
    request.rows.iter().map(|row| row.source_row_id).collect()
}

fn next_assessment_time(proposed: DateTime<Utc>, prior: &[RowQualityAssessment]) -> DateTime<Utc> {
    prior
        .iter()
        .map(|assessment| assessment.created_at)
        .max()
        .and_then(|latest| latest.checked_add_signed(TimeDelta::nanoseconds(1)))
        .map_or(proposed, |minimum| proposed.max(minimum))
}

fn retry_delay(error: Option<&QualityEvaluationError>) -> Duration {
    error
        .and_then(|error| error.retry_after_millis)
        .map(Duration::from_millis)
        .unwrap_or_default()
        .min(MAX_RETRY_DELAY)
}

fn failure_error_kind(kind: Option<EvaluatorFailureKind>) -> QualityEvaluationErrorKind {
    match kind {
        Some(EvaluatorFailureKind::Configuration) => QualityEvaluationErrorKind::Configuration,
        Some(EvaluatorFailureKind::Authentication) => QualityEvaluationErrorKind::Authentication,
        Some(EvaluatorFailureKind::RateLimit) => QualityEvaluationErrorKind::RateLimit,
        Some(EvaluatorFailureKind::Transport) => QualityEvaluationErrorKind::Transport,
        Some(EvaluatorFailureKind::Provider | EvaluatorFailureKind::BudgetExceeded) | None => {
            QualityEvaluationErrorKind::Provider
        }
    }
}

fn request_retry_after(attempt: &EvaluatorAttempt) -> Option<u64> {
    attempt
        .response_metadata
        .get("retry_after_millis")
        .and_then(Value::as_u64)
}

fn bounded_metadata(metadata: Value) -> (Value, bool) {
    if serde_json::to_vec(&metadata)
        .is_ok_and(|encoded| encoded.len() <= MAX_PERSISTED_METADATA_BYTES)
    {
        (metadata, false)
    } else {
        (
            json!({
                "metadata_discarded": "provider metadata exceeded the runner evidence limit"
            }),
            true,
        )
    }
}

fn bounded_error_message(message: String) -> String {
    if message.chars().count() <= MAX_PERSISTED_ERROR_CHARACTERS {
        message
    } else {
        let mut value = message
            .chars()
            .take(MAX_PERSISTED_ERROR_CHARACTERS)
            .collect::<String>();
        value.push_str(" [truncated]");
        value
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::{Arc, Mutex},
    };

    use chrono::{TimeZone, Utc};
    use dataset_core::domain::{DatasetSnapshot, SnapshotMember, SourceProvenance, SourceRow};
    use dataset_quality_core::{
        assessment::{
            EvaluatorExecutionLocation, EvaluatorGuidance, EvaluatorIdentity,
            EvaluatorIndependence, RowAssessmentDraft,
        },
        curation::{
            ApprovedCurationManifest, CurationApplication, CurationManifestReview,
            CurationProposal, DatasetQualityReport, RowQualityReview,
        },
        lifecycle::{
            AuditPlanStatusProjection, AuditStatusProjection, EvaluatorAttempt,
            EvaluatorAttemptState, ProviderUsage, QualityAuditRun, QualityAuditRunState,
            QualityAuditStopReason,
        },
        policy::{
            AuditBudgets, AuditMode, BasisPoints, BorderlineReviewPolicy, EvaluatorEgressPolicy,
            InvalidEvaluatorOutputPolicy, QualityPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, GuidanceReferences},
        ports::{
            DatasetQualityStore, EvaluatorBatchOutput, QualityAdapterError, QualityCandidateSource,
            QualityEvaluationError, QualityEvaluationErrorKind, QualityEvaluator,
        },
    };
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};

    use super::*;

    #[derive(Default)]
    struct MemoryState {
        plan: Option<AuditPlan>,
        guidance: Option<EvaluatorGuidance>,
        run: Option<QualityAuditRun>,
        requests: Vec<BlindEvaluatorRequest>,
        attempts: Vec<EvaluatorAttempt>,
        assessments: Vec<RowQualityAssessment>,
        reports: Vec<DatasetQualityReport>,
        lease: Option<AuditExecutionLease>,
        evidence_list_calls: EvidenceListCalls,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct EvidenceListCalls {
        requests: usize,
        attempts: usize,
        assessments: usize,
    }

    #[derive(Default)]
    struct MemoryStore {
        state: Mutex<MemoryState>,
    }

    impl MemoryStore {
        fn seed(&self, plan: AuditPlan, run: QualityAuditRun) {
            let mut state = self.state.lock().expect("memory store");
            state.plan = Some(plan);
            state.guidance = Some(EvaluatorGuidance::default());
            state.run = Some(run);
        }

        fn snapshot(&self) -> MemorySnapshot {
            let state = self.state.lock().expect("memory store");
            MemorySnapshot {
                requests: state.requests.clone(),
                attempts: state.attempts.clone(),
                assessments: state.assessments.clone(),
                reports: state.reports.clone(),
            }
        }

        fn request_cancel_directly(&self, run_id: Uuid) {
            let mut state = self.state.lock().expect("memory store");
            let run = state.run.as_mut().expect("seeded run");
            assert_eq!(run.id, run_id);
            run.request_cancel().expect("request cancellation");
        }

        fn reservation_is_durable(&self, request: &BlindEvaluatorRequest) -> bool {
            let state = self.state.lock().expect("memory store");
            state.requests.iter().any(|saved| saved == request)
                && state.attempts.iter().any(|attempt| {
                    attempt.id == request.attempt_id
                        && attempt.state == EvaluatorAttemptState::Started
                })
                && state
                    .run
                    .as_ref()
                    .is_some_and(|run| run.usage.request_attempts >= request.attempt_number)
        }

        fn evidence_list_calls(&self) -> EvidenceListCalls {
            self.state.lock().expect("memory store").evidence_list_calls
        }
    }

    #[derive(Clone)]
    struct MemorySnapshot {
        requests: Vec<BlindEvaluatorRequest>,
        attempts: Vec<EvaluatorAttempt>,
        assessments: Vec<RowQualityAssessment>,
        reports: Vec<DatasetQualityReport>,
    }

    impl DatasetQualityStore for MemoryStore {
        fn create_audit(
            &self,
            plan: &AuditPlan,
            run: &QualityAuditRun,
            guidance: &EvaluatorGuidance,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            let plan = plan.clone();
            let run = run.clone();
            let guidance = guidance.clone();
            Box::pin(async move {
                self.seed(plan, run);
                self.state.lock().expect("memory store").guidance = Some(guidance);
                Ok(())
            })
        }

        fn get_audit_plan(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<AuditPlan>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .plan
                    .clone()
                    .filter(|plan| plan.id == id))
            })
        }

        fn get_audit_guidance(
            &self,
            plan_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<EvaluatorGuidance>, QualityAdapterError>> {
            Box::pin(async move {
                let state = self.state.lock().expect("memory store");
                Ok(state
                    .plan
                    .as_ref()
                    .filter(|plan| plan.id == plan_id)
                    .and(state.guidance.clone()))
            })
        }

        fn get_audit_run(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<QualityAuditRun>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .run
                    .clone()
                    .filter(|run| run.id == id))
            })
        }

        fn audit_cancel_requested(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<bool>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .run
                    .as_ref()
                    .filter(|run| run.id == id)
                    .map(|run| run.cancel_requested))
            })
        }

        fn get_audit_status(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<AuditStatusProjection>, QualityAdapterError>> {
            Box::pin(async move {
                let state = self.state.lock().expect("memory store");
                let Some(run) = state.run.as_ref().filter(|run| run.id == id).cloned() else {
                    return Ok(None);
                };
                let plan = state
                    .plan
                    .as_ref()
                    .filter(|plan| plan.id == run.plan_id)
                    .ok_or_else(|| QualityAdapterError("audit plan is missing".into()))?;
                let projection = AuditStatusProjection {
                    evaluator_requests: state
                        .attempts
                        .iter()
                        .filter(|attempt| attempt.run_id == id)
                        .map(|attempt| attempt.request_sequence)
                        .collect::<BTreeSet<_>>()
                        .len() as u64,
                    evaluator_attempts: state
                        .attempts
                        .iter()
                        .filter(|attempt| attempt.run_id == id)
                        .count() as u64,
                    assessments: state
                        .assessments
                        .iter()
                        .filter(|assessment| assessment.audit_run_id == id)
                        .count() as u64,
                    report_id: state
                        .reports
                        .iter()
                        .find(|report| report.run_id == id)
                        .map(|report| report.id),
                    plan: AuditPlanStatusProjection {
                        id: plan.id,
                        schema_version: plan.schema_version,
                        dataset_definition_id: plan.dataset_schema.dataset_definition_id,
                        dataset_definition_fingerprint: plan
                            .dataset_schema
                            .dataset_definition_fingerprint
                            .clone(),
                        policy_fingerprint: plan.policy.fingerprint.clone(),
                        resolved_guidance_fingerprint: plan.resolved_guidance_fingerprint.clone(),
                        evaluator_protocol_version: plan.evaluator_protocol_version.clone(),
                        population_rows: plan.population_count(),
                        selected_rows: plan.selected_count(),
                        source_set_fingerprint: plan.source_set_fingerprint.clone(),
                        created_at: plan.created_at,
                        fingerprint: plan.fingerprint.clone(),
                    },
                    run,
                };
                projection
                    .verify()
                    .map_err(|error| QualityAdapterError(error.to_string()))?;
                Ok(Some(projection))
            })
        }

        fn save_audit_run(
            &self,
            run: &QualityAuditRun,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            let run = run.clone();
            Box::pin(async move {
                self.state.lock().expect("memory store").run = Some(run);
                Ok(())
            })
        }

        fn acquire_audit_execution_lease(
            &self,
            run_id: Uuid,
            invocation_token: Uuid,
        ) -> BoxFuture<'_, Result<AuditExecutionLease, QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                if state.run.as_ref().is_none_or(|run| run.id != run_id) {
                    return Err(QualityAdapterError("audit run is missing".into()));
                }
                if state.lease.is_some() {
                    return Err(QualityAdapterError(
                        "audit is owned by another live execution".into(),
                    ));
                }
                let lease = AuditExecutionLease::create(
                    run_id,
                    invocation_token,
                    std::process::id(),
                    1,
                    Utc::now(),
                )
                .map_err(|error| QualityAdapterError(error.to_string()))?;
                state.lease = Some(lease.clone());
                Ok(lease)
            })
        }

        fn release_audit_execution_lease(
            &self,
            lease: &AuditExecutionLease,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            let lease = lease.clone();
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                if state.lease.as_ref() != Some(&lease) {
                    return Err(QualityAdapterError(
                        "audit execution lease changed concurrently".into(),
                    ));
                }
                state.lease = None;
                Ok(())
            })
        }

        fn record_attempt<'a>(
            &'a self,
            _checked: &'a CheckedAuditPlan<'a>,
            lease: &'a AuditExecutionLease,
            attempt: &'a EvaluatorAttempt,
            request: &'a BlindEvaluatorRequest,
            run: &'a QualityAuditRun,
        ) -> BoxFuture<'a, Result<(), QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                if state.lease.as_ref() != Some(lease) {
                    return Err(QualityAdapterError(
                        "audit execution lease changed concurrently".into(),
                    ));
                }
                if state.attempts.iter().any(|value| value.id == attempt.id)
                    || state.requests.iter().any(|value| value.id == request.id)
                {
                    return Err(QualityAdapterError("duplicate attempt reservation".into()));
                }
                state.requests.push(request.clone());
                state.attempts.push(attempt.clone());
                state.run = Some(run.clone());
                Ok(())
            })
        }

        fn get_evaluator_request(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<BlindEvaluatorRequest>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .requests
                    .iter()
                    .find(|request| request.id == id)
                    .cloned())
            })
        }

        fn list_evaluator_requests(
            &self,
            run_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<BlindEvaluatorRequest>, QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                state.evidence_list_calls.requests += 1;
                let mut values = state
                    .requests
                    .iter()
                    .filter(|request| request.audit_run_id == run_id)
                    .cloned()
                    .collect::<Vec<_>>();
                values.sort_by_key(|request| (request.request_sequence, request.attempt_number));
                Ok(values)
            })
        }

        fn finish_attempt<'a>(
            &'a self,
            _checked: &'a CheckedAuditPlan<'a>,
            lease: &'a AuditExecutionLease,
            attempt: &'a EvaluatorAttempt,
            assessments: &'a [RowQualityAssessment],
            run: &'a QualityAuditRun,
        ) -> BoxFuture<'a, Result<(), QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                if state.lease.as_ref() != Some(lease) {
                    return Err(QualityAdapterError(
                        "audit execution lease changed concurrently".into(),
                    ));
                }
                let slot = state
                    .attempts
                    .iter_mut()
                    .find(|value| value.id == attempt.id)
                    .ok_or_else(|| QualityAdapterError("attempt was not reserved".into()))?;
                if slot.state != EvaluatorAttemptState::Started {
                    return Err(QualityAdapterError("attempt is already terminal".into()));
                }
                *slot = attempt.clone();
                state.assessments.extend_from_slice(assessments);
                state.run = Some(run.clone());
                Ok(())
            })
        }

        fn list_attempts(
            &self,
            run_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<EvaluatorAttempt>, QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                state.evidence_list_calls.attempts += 1;
                let mut values = state
                    .attempts
                    .iter()
                    .filter(|attempt| attempt.run_id == run_id)
                    .cloned()
                    .collect::<Vec<_>>();
                values.sort_by_key(|attempt| (attempt.request_sequence, attempt.attempt_number));
                Ok(values)
            })
        }

        fn list_assessments(
            &self,
            run_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<RowQualityAssessment>, QualityAdapterError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                state.evidence_list_calls.assessments += 1;
                let mut values = state
                    .assessments
                    .iter()
                    .filter(|assessment| assessment.audit_run_id == run_id)
                    .cloned()
                    .collect::<Vec<_>>();
                values.sort_by_key(|assessment| {
                    (assessment.request_sequence, assessment.attempt_number)
                });
                Ok(values)
            })
        }

        fn get_assessment(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<RowQualityAssessment>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .assessments
                    .iter()
                    .find(|assessment| assessment.id == id)
                    .cloned())
            })
        }

        fn save_report(
            &self,
            report: &DatasetQualityReport,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            let report = report.clone();
            Box::pin(async move {
                let mut state = self.state.lock().expect("memory store");
                if state
                    .reports
                    .iter()
                    .any(|value| value.run_id == report.run_id)
                {
                    return Err(QualityAdapterError("duplicate report".into()));
                }
                state.reports.push(report);
                Ok(())
            })
        }

        fn get_report(
            &self,
            id: Uuid,
        ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .reports
                    .iter()
                    .find(|report| report.id == id)
                    .cloned())
            })
        }

        fn report_for_run(
            &self,
            run_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .expect("memory store")
                    .reports
                    .iter()
                    .find(|report| report.run_id == run_id)
                    .cloned())
            })
        }

        fn append_row_review(
            &self,
            _review: &RowQualityReview,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            unsupported()
        }

        fn list_row_reviews(
            &self,
            _report_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<RowQualityReview>, QualityAdapterError>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn save_curation_proposal(
            &self,
            _proposal: &CurationProposal,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            unsupported()
        }

        fn get_curation_proposal(
            &self,
            _id: Uuid,
        ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }

        fn latest_curation_proposal(
            &self,
            _report_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }

        fn append_manifest_review(
            &self,
            _review: &CurationManifestReview,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            unsupported()
        }

        fn list_manifest_reviews(
            &self,
            _proposal_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<CurationManifestReview>, QualityAdapterError>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn save_manifest(
            &self,
            _manifest: &ApprovedCurationManifest,
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            unsupported()
        }

        fn get_manifest(
            &self,
            _id: Uuid,
        ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }

        fn manifest_for_proposal(
            &self,
            _proposal_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }

        fn apply_manifest(
            &self,
            _application: &CurationApplication,
            _snapshot: &DatasetSnapshot,
            _members: &[SnapshotMember],
        ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
            unsupported()
        }

        fn curation_application_for_snapshot(
            &self,
            _snapshot_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }

        fn curation_application_for_manifest(
            &self,
            _manifest_id: Uuid,
        ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>> {
            Box::pin(async { Ok(None) })
        }
    }

    fn unsupported<'a, T>() -> BoxFuture<'a, Result<T, QualityAdapterError>> {
        Box::pin(async { Err(QualityAdapterError("not used by runner tests".into())) })
    }

    struct MemoryCandidates {
        rows: Vec<SourceRow>,
    }

    impl QualityCandidateSource for MemoryCandidates {
        fn list_source_rows(
            &self,
            dataset_id: Uuid,
        ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>> {
            Box::pin(async move {
                let mut rows = self
                    .rows
                    .iter()
                    .filter(|row| row.dataset_id == dataset_id)
                    .cloned()
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.id);
                Ok(rows)
            })
        }

        fn get_source_rows(
            &self,
            dataset_id: Uuid,
            source_row_ids: Vec<Uuid>,
        ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>> {
            Box::pin(async move {
                let requested = source_row_ids.into_iter().collect::<BTreeSet<_>>();
                let mut rows = self
                    .rows
                    .iter()
                    .filter(|row| row.dataset_id == dataset_id && requested.contains(&row.id))
                    .cloned()
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.id);
                Ok(rows)
            })
        }
    }

    #[derive(Debug, Clone)]
    enum EvalStep {
        Qualified,
        Borderline,
        Empty,
        Error(QualityEvaluationError),
        CancelAndQualify,
        OverBudget,
    }

    struct ScriptedEvaluator {
        identity: EvaluatorIdentity,
        steps: Mutex<VecDeque<EvalStep>>,
        store: Arc<MemoryStore>,
        calls: Mutex<Vec<BlindEvaluatorRequest>>,
    }

    impl ScriptedEvaluator {
        fn new(identity: EvaluatorIdentity, steps: Vec<EvalStep>, store: Arc<MemoryStore>) -> Self {
            Self {
                identity,
                steps: Mutex::new(steps.into()),
                store,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<BlindEvaluatorRequest> {
            self.calls.lock().expect("scripted evaluator").clone()
        }
    }

    impl QualityEvaluator for ScriptedEvaluator {
        fn identity(&self) -> EvaluatorIdentity {
            self.identity.clone()
        }

        fn evaluate(
            &self,
            request: BlindEvaluatorRequest,
        ) -> BoxFuture<'_, Result<EvaluatorBatchOutput, QualityEvaluationError>> {
            Box::pin(async move {
                assert!(
                    self.store.reservation_is_durable(&request),
                    "request and started attempt must be committed before evaluator I/O"
                );
                self.calls
                    .lock()
                    .expect("scripted evaluator")
                    .push(request.clone());
                let step = self
                    .steps
                    .lock()
                    .expect("scripted evaluator")
                    .pop_front()
                    .unwrap_or(EvalStep::Qualified);
                match step {
                    EvalStep::Qualified => Ok(batch(&request, 9_200, provider_usage())),
                    EvalStep::Borderline => Ok(batch(&request, 7_800, provider_usage())),
                    EvalStep::Empty => Ok(EvaluatorBatchOutput {
                        assessments: Vec::new(),
                        usage: provider_usage(),
                        metadata: json!({"shape": "empty"}),
                    }),
                    EvalStep::Error(error) => Err(error),
                    EvalStep::CancelAndQualify => {
                        self.store.request_cancel_directly(request.audit_run_id);
                        Ok(batch(&request, 9_200, provider_usage()))
                    }
                    EvalStep::OverBudget => Ok(batch(
                        &request,
                        9_200,
                        ProviderUsage {
                            input_tokens: request.budget.maximum_input_tokens + 1,
                            output_tokens: 0,
                            total_tokens: request.budget.maximum_input_tokens + 1,
                            cost_microusd: 0,
                        },
                    )),
                }
            })
        }
    }

    #[derive(Default)]
    struct FixedClock;

    impl QualityRunnerClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
                .single()
                .expect("fixed time")
        }
    }

    #[derive(Default)]
    struct RecordingSleeper {
        durations: Mutex<Vec<Duration>>,
    }

    impl QualityRetrySleeper for RecordingSleeper {
        fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                self.durations
                    .lock()
                    .expect("recording sleeper")
                    .push(duration);
            })
        }
    }

    struct Fixture {
        plan: AuditPlan,
        run: QualityAuditRun,
        rows: Vec<SourceRow>,
        store: Arc<MemoryStore>,
        primary: Arc<ScriptedEvaluator>,
        reviewers: Vec<Arc<ScriptedEvaluator>>,
        sleeper: Arc<RecordingSleeper>,
    }

    fn fixture(
        row_count: usize,
        policy: QualityPolicy,
        primary_steps: Vec<EvalStep>,
        reviewer_steps: Vec<Vec<EvalStep>>,
    ) -> Fixture {
        let dataset = dataset();
        let rows = rows(dataset.id, row_count);
        let guidance = EvaluatorGuidance::default();
        let plan = AuditPlan::with_identity(
            Uuid::from_u128(0x40000000000040008000000000000001),
            &dataset,
            policy,
            GuidanceReferences::default(),
            guidance
                .reproduce_fingerprint()
                .expect("guidance fingerprint"),
            "quality-evaluator-v1",
            rows.clone(),
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("plan time"),
        )
        .expect("audit plan");
        let store = Arc::new(MemoryStore::default());
        let primary_identity = evaluator_identity(0, EvaluatorIndependence::Primary);
        let primary = Arc::new(ScriptedEvaluator::new(
            primary_identity.clone(),
            primary_steps,
            store.clone(),
        ));
        let reviewers = reviewer_steps
            .into_iter()
            .enumerate()
            .map(|(index, steps)| {
                Arc::new(ScriptedEvaluator::new(
                    evaluator_identity(index + 1, EvaluatorIndependence::IndependentReview),
                    steps,
                    store.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let run = QualityAuditRun::queue(
            &plan,
            primary_identity,
            reviewers.iter().map(|value| value.identity()).collect(),
        )
        .expect("audit run");
        store.seed(plan.clone(), run.clone());
        Fixture {
            plan,
            run,
            rows,
            store,
            primary,
            reviewers,
            sleeper: Arc::new(RecordingSleeper::default()),
        }
    }

    fn runner(fixture: &Fixture) -> DatasetQualityRunner {
        let mut evaluators = vec![fixture.primary.clone() as Arc<dyn QualityEvaluator>];
        evaluators.extend(
            fixture
                .reviewers
                .iter()
                .cloned()
                .map(|value| value as Arc<dyn QualityEvaluator>),
        );
        DatasetQualityRunner::with_runtime(
            fixture.store.clone(),
            Arc::new(MemoryCandidates {
                rows: fixture.rows.clone(),
            }),
            evaluators,
            Arc::new(FixedClock),
            fixture.sleeper.clone(),
        )
        .expect("runner")
    }

    #[tokio::test]
    async fn success_batches_canonical_rows_and_completed_resume_is_idempotent() {
        let policy = policy_with(
            QualityPreset::Fast,
            InvalidEvaluatorOutputPolicy::Quarantine,
            BorderlineReviewPolicy::None,
            AuditBudgets {
                maximum_rows_per_batch: 2,
                maximum_evaluator_requests: 10,
                maximum_attempts_per_request: 2,
                maximum_input_tokens: 1_000,
                maximum_output_tokens: 1_000,
                maximum_total_tokens: 2_000,
                maximum_cost_microusd: None,
            },
        );
        let fixture = fixture(
            3,
            policy,
            vec![EvalStep::Qualified, EvalStep::Qualified],
            vec![],
        );
        let runner = runner(&fixture);

        let first = runner
            .execute(fixture.run.id)
            .await
            .expect("successful audit");
        assert_eq!(first.run.state, QualityAuditRunState::Completed);
        assert_eq!(first.run.progress.qualified_rows, 3);
        assert!(first.report.is_some());
        let calls = fixture.primary.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].request_sequence, 1);
        assert_eq!(calls[1].request_sequence, 2);
        assert!(calls.iter().all(|request| {
            request
                .rows
                .windows(2)
                .all(|pair| pair[0].source_row_id < pair[1].source_row_id)
        }));

        let second = runner
            .execute(fixture.run.id)
            .await
            .expect("idempotent completed resume");
        assert_eq!(second.report, first.report);
        assert_eq!(fixture.store.snapshot().requests.len(), 2);
    }

    #[tokio::test]
    async fn scheduling_hydrates_complete_evidence_once_across_many_batches() {
        const ROW_COUNT: usize = 8;
        let policy = policy_with(
            QualityPreset::Fast,
            InvalidEvaluatorOutputPolicy::Quarantine,
            BorderlineReviewPolicy::None,
            AuditBudgets {
                maximum_rows_per_batch: 1,
                maximum_evaluator_requests: ROW_COUNT as u32,
                maximum_attempts_per_request: 1,
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 10_000,
                maximum_total_tokens: 20_000,
                maximum_cost_microusd: None,
            },
        );
        let fixture = fixture(
            ROW_COUNT,
            policy,
            vec![EvalStep::Qualified; ROW_COUNT],
            vec![],
        );

        let outcome = SCHEDULER_CANDIDATE_VISITS
            .scope(std::cell::Cell::new(0), async {
                let outcome = runner(&fixture)
                    .execute(fixture.run.id)
                    .await
                    .expect("many-batch audit");
                assert_eq!(
                    SCHEDULER_CANDIDATE_VISITS.with(std::cell::Cell::get),
                    ROW_COUNT,
                    "scheduler work must grow with emitted rows, not batch-count times population",
                );
                outcome
            })
            .await;

        assert_eq!(outcome.run.state, QualityAuditRunState::Completed);
        assert_eq!(fixture.primary.calls().len(), ROW_COUNT);
        assert_eq!(
            fixture.store.evidence_list_calls(),
            EvidenceListCalls {
                requests: 1,
                attempts: 1,
                assessments: 1,
            },
            "full evidence hydration must stay bounded as request count grows",
        );
    }

    #[tokio::test]
    async fn retryable_error_uses_same_logical_request_and_bounded_retry_delay() {
        let error = QualityEvaluationError::new(
            QualityEvaluationErrorKind::Transport,
            "temporary connection loss",
        )
        .with_retry_after_millis(7)
        .with_observed_evidence(provider_usage(), json!({"request_id": "redacted"}));
        let fixture = fixture(
            1,
            fast_policy(),
            vec![EvalStep::Error(error), EvalStep::Qualified],
            vec![],
        );
        let runner = runner(&fixture);

        let outcome = runner.execute(fixture.run.id).await.expect("retried audit");

        assert_eq!(outcome.run.state, QualityAuditRunState::Completed);
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.attempts.len(), 2);
        assert_eq!(snapshot.attempts[0].request_sequence, 1);
        assert_eq!(snapshot.attempts[1].request_sequence, 1);
        assert_eq!(snapshot.attempts[0].attempt_number, 1);
        assert_eq!(snapshot.attempts[1].attempt_number, 2);
        assert_eq!(outcome.run.usage.total_tokens, 16);
        assert_eq!(snapshot.attempts[0].provider_usage, provider_usage());
        assert_eq!(
            snapshot.attempts[0].retry_payload_fingerprint,
            snapshot.attempts[1].retry_payload_fingerprint
        );
        assert_eq!(
            fixture
                .sleeper
                .durations
                .lock()
                .expect("durations")
                .as_slice(),
            &[Duration::from_millis(7)]
        );
    }

    #[tokio::test]
    async fn nonretryable_failure_terminates_after_one_durable_attempt() {
        let fixture = fixture(
            1,
            fast_policy(),
            vec![EvalStep::Error(QualityEvaluationError::new(
                QualityEvaluationErrorKind::Authentication,
                "bad credential",
            ))],
            vec![],
        );

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("durable failure");

        assert_eq!(outcome.run.state, QualityAuditRunState::Failed);
        assert_eq!(
            outcome.run.stop_reason,
            Some(QualityAuditStopReason::ProviderFailure)
        );
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.attempts.len(), 1);
        assert_eq!(snapshot.attempts[0].state, EvaluatorAttemptState::Failed);
        assert!(snapshot.assessments.is_empty());
        assert!(snapshot.reports.is_empty());
    }

    #[tokio::test]
    async fn retryable_failure_cannot_exceed_the_pinned_attempt_ceiling() {
        let policy = policy_with(
            QualityPreset::Fast,
            InvalidEvaluatorOutputPolicy::Quarantine,
            BorderlineReviewPolicy::None,
            AuditBudgets {
                maximum_attempts_per_request: 2,
                ..default_budgets()
            },
        );
        let transient = || {
            EvalStep::Error(QualityEvaluationError::new(
                QualityEvaluationErrorKind::RateLimit,
                "provider remains rate limited",
            ))
        };
        let fixture = fixture(
            1,
            policy,
            vec![transient(), transient(), EvalStep::Qualified],
            vec![],
        );

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("bounded retry exhaustion");

        assert_eq!(outcome.run.state, QualityAuditRunState::Failed);
        assert_eq!(fixture.primary.calls().len(), 2);
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.attempts.len(), 2);
        assert!(
            snapshot
                .attempts
                .iter()
                .all(|attempt| attempt.state == EvaluatorAttemptState::Failed)
        );
        assert_eq!(outcome.run.usage.request_attempts, 2);
        assert_eq!(outcome.run.usage.evaluator_requests, 1);
    }

    #[tokio::test]
    async fn malformed_output_is_quarantined_or_fails_according_to_policy() {
        let quarantine = fixture(1, fast_policy(), vec![EvalStep::Empty], vec![]);
        let quarantine_outcome = runner(&quarantine)
            .execute(quarantine.run.id)
            .await
            .expect("quarantined invalid response");
        assert_eq!(
            quarantine_outcome.run.state,
            QualityAuditRunState::Completed
        );
        assert_eq!(quarantine_outcome.run.progress.invalid_rows, 1);
        assert!(quarantine_outcome.report.is_some());

        let strict = policy_with(
            QualityPreset::Fast,
            InvalidEvaluatorOutputPolicy::FailAudit,
            BorderlineReviewPolicy::None,
            default_budgets(),
        );
        let fail = fixture(1, strict, vec![EvalStep::Empty], vec![]);
        let fail_outcome = runner(&fail)
            .execute(fail.run.id)
            .await
            .expect("failed invalid response");
        assert_eq!(fail_outcome.run.state, QualityAuditRunState::Failed);
        assert_eq!(
            fail_outcome.run.stop_reason,
            Some(QualityAuditStopReason::InvalidResponse)
        );
        assert!(fail_outcome.report.is_none());
    }

    #[tokio::test]
    async fn cancellation_after_inflight_response_keeps_terminal_attempt_evidence() {
        let fixture = fixture(1, fast_policy(), vec![EvalStep::CancelAndQualify], vec![]);

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("cancelled audit");

        assert_eq!(outcome.run.state, QualityAuditRunState::Cancelled);
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.attempts[0].state, EvaluatorAttemptState::Succeeded);
        assert_eq!(snapshot.assessments.len(), 1);
        assert!(snapshot.reports.is_empty());
    }

    #[tokio::test]
    async fn cancelling_queued_audit_is_immediately_terminal_without_provider_io() {
        let fixture = fixture(1, fast_policy(), vec![EvalStep::Qualified], vec![]);
        let runner = runner(&fixture);

        let cancelled = runner
            .request_cancel(fixture.run.id)
            .await
            .expect("cancel queued audit");

        assert_eq!(cancelled.state, QualityAuditRunState::Cancelled);
        assert!(cancelled.cancel_requested);
        assert!(fixture.primary.calls().is_empty());
        assert!(fixture.store.snapshot().attempts.is_empty());
    }

    #[tokio::test]
    async fn execution_requires_the_exact_guidance_persisted_with_the_plan() {
        let fixture = fixture(1, fast_policy(), vec![EvalStep::Qualified], vec![]);
        fixture.store.state.lock().expect("memory store").guidance = None;

        let error = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect_err("missing pinned guidance must stop before provider I/O");

        assert!(error.to_string().contains("pinned evaluator guidance"));
        assert!(fixture.primary.calls().is_empty());
        assert!(fixture.store.snapshot().attempts.is_empty());

        fixture.store.state.lock().expect("memory store").guidance =
            Some(EvaluatorGuidance::default());
        let resumed = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("execution error released its invocation lease");
        assert_eq!(resumed.run.state, QualityAuditRunState::Completed);
    }

    #[tokio::test]
    async fn recovery_interrupts_started_attempt_then_retries_without_new_logical_request() {
        let fixture = fixture(1, fast_policy(), vec![EvalStep::Qualified], vec![]);
        let guidance = EvaluatorGuidance::default();
        let mut running = fixture.run.clone();
        running.start(&fixture.plan).expect("start run");
        fixture.store.state.lock().expect("memory store").run = Some(running.clone());
        let request = BlindEvaluatorRequest::create(
            &fixture.plan,
            Uuid::new_v4(),
            running.id,
            Uuid::new_v4(),
            1,
            1,
            &running.primary_evaluator,
            fixture.rows.clone(),
            guidance.clone(),
            fixture.plan.resolved_guidance_fingerprint.clone(),
            fixed_request_budget(&fixture.plan).expect("fixed request budget"),
        )
        .expect("request");
        let attempt = EvaluatorAttempt::start(
            &fixture.plan,
            &running,
            &request,
            running.primary_evaluator.clone(),
        )
        .expect("attempt");
        let primary_identity = running.primary_evaluator.clone();
        running
            .reserve_attempt(&fixture.plan, &request, &primary_identity)
            .expect("reserve attempt");
        let checked = CheckedAuditPlan::new(&fixture.plan).expect("checked plan");
        let lease = fixture
            .store
            .acquire_audit_execution_lease(running.id, Uuid::new_v4())
            .await
            .expect("acquire setup lease");
        fixture
            .store
            .record_attempt(&checked, &lease, &attempt, &request, &running)
            .await
            .expect("persist started attempt");
        fixture
            .store
            .release_audit_execution_lease(&lease)
            .await
            .expect("release setup lease");

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("recovered audit");

        assert_eq!(outcome.run.state, QualityAuditRunState::Completed);
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.attempts.len(), 2);
        assert_eq!(
            snapshot.attempts[0].state,
            EvaluatorAttemptState::Interrupted
        );
        assert_eq!(snapshot.attempts[1].state, EvaluatorAttemptState::Succeeded);
        assert_eq!(snapshot.attempts[0].request_sequence, 1);
        assert_eq!(snapshot.attempts[1].request_sequence, 1);
        assert_eq!(fixture.primary.calls().len(), 1);
    }

    #[tokio::test]
    async fn borderline_rows_receive_the_pinned_independent_review_round() {
        let fixture = fixture(
            1,
            balanced_policy(),
            vec![EvalStep::Borderline],
            vec![vec![EvalStep::Qualified]],
        );

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("reviewed audit");

        assert_eq!(outcome.run.state, QualityAuditRunState::Completed);
        assert_eq!(fixture.primary.calls().len(), 1);
        assert_eq!(fixture.reviewers[0].calls().len(), 1);
        let snapshot = fixture.store.snapshot();
        assert_eq!(snapshot.assessments.len(), 2);
        assert_eq!(snapshot.attempts[0].request_sequence, 1);
        assert_eq!(snapshot.attempts[1].request_sequence, 2);
        assert_eq!(outcome.run.progress.pending_review_rows, 0);
        assert_eq!(outcome.run.progress.borderline_rows, 1);
    }

    #[tokio::test]
    async fn observed_provider_overrun_is_preserved_and_fails_the_run() {
        let fixture = fixture(1, fast_policy(), vec![EvalStep::OverBudget], vec![]);

        let outcome = runner(&fixture)
            .execute(fixture.run.id)
            .await
            .expect("budget-exhausted audit");

        assert_eq!(outcome.run.state, QualityAuditRunState::Failed);
        assert_eq!(
            outcome.run.stop_reason,
            Some(QualityAuditStopReason::BudgetExhausted)
        );
        let snapshot = fixture.store.snapshot();
        assert!(
            snapshot.attempts[0].provider_usage.input_tokens
                > snapshot.requests[0].budget.maximum_input_tokens
        );
        assert_eq!(
            outcome.run.usage.input_tokens,
            snapshot.attempts[0].provider_usage.input_tokens
        );
        assert_eq!(snapshot.attempts[0].state, EvaluatorAttemptState::Failed);
        assert_eq!(
            snapshot.attempts[0].failure_kind,
            Some(EvaluatorFailureKind::BudgetExceeded)
        );
    }

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::with_identity(
            Uuid::from_u128(0x10000000000040008000000000000001),
            "support",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("dataset time"),
        )
        .expect("dataset")
    }

    fn rows(dataset_id: Uuid, count: usize) -> Vec<SourceRow> {
        (0..count)
            .map(|index| SourceRow {
                id: Uuid::from_u128(0x20000000000040008000000000000001 + index as u128),
                dataset_id,
                text: format!("billing request {index}"),
                label: "billing".into(),
                dimensions: BTreeMap::from([("difficulty".into(), "easy".into())]),
                fields: BTreeMap::new(),
                provenance: SourceProvenance::Generated {
                    generation_job_id: Uuid::from_u128(
                        0x30000000000040008000000000000001 + index as u128,
                    ),
                    backend: "fake-generator".into(),
                    model: "fake-model".into(),
                    construction_plan_fingerprint: None,
                },
                created_at: Utc
                    .with_ymd_and_hms(2026, 1, 2, 0, 0, (index % 60) as u32)
                    .single()
                    .expect("row time"),
            })
            .collect()
    }

    fn evaluator_identity(index: usize, independence: EvaluatorIndependence) -> EvaluatorIdentity {
        EvaluatorIdentity::new(
            format!("quality-backend-{index}"),
            format!("quality-model-{index}"),
            "quality-evaluator-v1",
            format!("sha256:quality-config-{index}"),
            independence,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator identity")
    }

    fn bp(value: u16) -> BasisPoints {
        BasisPoints::new(value).expect("basis points")
    }

    fn batch(
        request: &BlindEvaluatorRequest,
        assigned_score: u16,
        usage: ProviderUsage,
    ) -> EvaluatorBatchOutput {
        EvaluatorBatchOutput {
            assessments: request
                .rows
                .iter()
                .map(|row| RowAssessmentDraft {
                    source_row_id: row.source_row_id,
                    source_row_fingerprint: row.source_row_fingerprint.clone(),
                    label_scores: BTreeMap::from([
                        ("billing".into(), bp(assigned_score)),
                        ("fraud".into(), bp(1_000)),
                    ]),
                    dimension_scores: BTreeMap::from([(
                        "difficulty".into(),
                        BTreeMap::from([("easy".into(), bp(9_000)), ("hard".into(), bp(1_000))]),
                    )]),
                    authenticity_score: None,
                    label_leakage_risk: bp(500),
                    shortcut_risk: bp(500),
                    confidence: bp(9_000),
                    issue_codes: Vec::new(),
                    rationale: "The blind evidence supports the scored concepts.".into(),
                })
                .collect(),
            usage,
            metadata: json!({"fixture": true}),
        }
    }

    fn provider_usage() -> ProviderUsage {
        ProviderUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            cost_microusd: 0,
        }
    }

    fn default_budgets() -> AuditBudgets {
        AuditBudgets {
            maximum_rows_per_batch: 2,
            maximum_evaluator_requests: 10,
            maximum_attempts_per_request: 3,
            maximum_input_tokens: 1_000,
            maximum_output_tokens: 1_000,
            maximum_total_tokens: 2_000,
            maximum_cost_microusd: None,
        }
    }

    fn fast_policy() -> QualityPolicy {
        policy_with(
            QualityPreset::Fast,
            InvalidEvaluatorOutputPolicy::Quarantine,
            BorderlineReviewPolicy::None,
            default_budgets(),
        )
    }

    fn balanced_policy() -> QualityPolicy {
        policy_with(
            QualityPreset::Balanced,
            InvalidEvaluatorOutputPolicy::Quarantine,
            BorderlineReviewPolicy::Independent {
                maximum_additional_assessments: 1,
            },
            default_budgets(),
        )
    }

    fn policy_with(
        preset: QualityPreset,
        invalid_output_policy: InvalidEvaluatorOutputPolicy,
        borderline_review_policy: BorderlineReviewPolicy,
        budgets: AuditBudgets,
    ) -> QualityPolicy {
        let base = preset
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("base policy");
        QualityPolicy::new(
            Some(preset),
            base.thresholds,
            invalid_output_policy,
            borderline_review_policy,
            budgets,
            EvaluatorEgressPolicy::LocalOnly,
            AuditMode::FullPopulation,
        )
        .expect("policy")
    }
}
