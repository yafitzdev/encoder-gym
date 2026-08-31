use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use dataset_quality_core::ports::DatasetQualityStore;
use generation_core::jobs::{GenerationAttempt, GenerationAttemptState};
use generation_core::ports::{DatasetStore, GenerationExecutionStore, PlanStore, RowStore};
use generation_supervisor_core::{
    SupervisorError,
    advisor::{
        AdvisorCallState, AdvisorModelCall, AdvisorSession, AdvisorSessionState, AdvisorToolCall,
        GenerationQualityDiagnosisBrief,
    },
    contract::GenerationQualityContract,
    decision::DeterministicQualityDecision,
    lifecycle::{
        ChildOutcome, ChildReservation, SupervisorRun, SupervisorRunEvent, SupervisorRunState,
        SupervisorUsage,
    },
    observation::{
        BatchQualityObservation, QualityEvidenceManifest, QualityScope, RowQualityObservation,
    },
    ports::{
        BoxFuture, GenerationSupervisorStore, SupervisedRowTrace, SupervisorAdvisorStore,
        SupervisorIntegrityReport,
    },
    revision::{
        PromptGuidanceVersion, PromptRevisionActivation, PromptRevisionAuthorization,
        PromptRevisionProposal, PromptRevisionReview, RevisionReviewDecision, SupervisorDiagnosis,
    },
    strategy::StrategyAssignmentSet,
};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{RowRecord, SourceRowRecord, SqliteStore};

impl SupervisorAdvisorStore for SqliteStore {
    fn create_session(
        &self,
        brief: &GenerationQualityDiagnosisBrief,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let brief = brief.clone();
        let session = session.clone();
        Box::pin(async move {
            brief.validate()?;
            session.validate()?;
            if session.brief_id != brief.id
                || session.brief_fingerprint != brief.fingerprint
                || session.supervisor_run_id != brief.supervisor_run_id
                || session.state != AdvisorSessionState::Queued
            {
                return Err(integrity(
                    "advisor session does not bind its diagnosis brief",
                ));
            }
            let decision = load_decision(self, brief.decision.id)
                .await?
                .ok_or_else(|| integrity("diagnosis decision is not persisted"))?;
            let window = load_window(self, brief.window.id)
                .await?
                .ok_or_else(|| integrity("diagnosis window is not persisted"))?;
            let prompt = load_prompt(self, brief.current_prompt.id)
                .await?
                .ok_or_else(|| integrity("diagnosis prompt version is not persisted"))?;
            if decision != brief.decision
                || window != brief.window
                || prompt != brief.current_prompt
            {
                return Err(integrity(
                    "diagnosis brief does not reproduce persisted pause artifacts",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            sqlx::query(
                "INSERT INTO generation_supervisor_diagnosis_briefs \
                 (id, run_id, contract_id, decision_id, window_id, prompt_version_id, fingerprint, brief_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(brief.id)
            .bind(brief.supervisor_run_id)
            .bind(brief.contract.id)
            .bind(brief.decision.id)
            .bind(brief.window.id)
            .bind(brief.current_prompt.id)
            .bind(&brief.fingerprint)
            .bind(encode(&brief)?)
            .bind(brief.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            sqlx::query(
                "INSERT INTO generation_supervisor_advisor_sessions \
                 (id, run_id, brief_id, state, fingerprint, session_json, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session.id)
            .bind(session.supervisor_run_id)
            .bind(session.brief_id)
            .bind(advisor_state(session.state))
            .bind(&session.fingerprint)
            .bind(encode(&session)?)
            .bind(session.created_at)
            .bind(session.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_brief(
        &self,
        brief_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationQualityDiagnosisBrief>, SupervisorError>> {
        Box::pin(async move {
            load_json_optional(
                self,
                "SELECT brief_json FROM generation_supervisor_diagnosis_briefs WHERE id = ?",
                brief_id,
            )
            .await?
            .map(check_brief)
            .transpose()
        })
    }

    fn get_session(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AdvisorSession>, SupervisorError>> {
        Box::pin(async move { load_session(self, session_id).await })
    }

    fn save_session(&self, session: &AdvisorSession) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let session = session.clone();
        Box::pin(async move {
            session.validate()?;
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            save_session_tx(&mut tx, &session).await?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn reserve_model_calls(
        &self,
        calls: &[AdvisorModelCall],
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let calls = calls.to_vec();
        Box::pin(async move {
            let Some(first) = calls.first() else {
                return Err(validation("model call reservation set must not be empty"));
            };
            let session = load_session(self, first.session_id)
                .await?
                .ok_or_else(|| integrity("advisor session is not persisted"))?;
            if session.state != AdvisorSessionState::Queued
                || calls.iter().any(|call| {
                    call.session_id != session.id
                        || call.state != AdvisorCallState::Reserved
                        || call.reproduce_fingerprint().ok().as_deref()
                            != Some(call.fingerprint.as_str())
                })
            {
                return Err(integrity(
                    "model call reservations must be complete pre-I/O facts for one queued session",
                ));
            }
            let sequences = calls
                .iter()
                .map(|call| call.sequence)
                .collect::<BTreeSet<_>>();
            if sequences.len() != calls.len()
                || sequences
                    .iter()
                    .copied()
                    .ne(1..=u32::try_from(calls.len()).unwrap_or(u32::MAX))
            {
                return Err(validation(
                    "model call reservations must be contiguous from one",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            for call in calls {
                sqlx::query(
                    "INSERT INTO generation_supervisor_model_calls \
                     (id, session_id, sequence, state, fingerprint, call_json, reserved_at, finished_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(call.id)
                .bind(call.session_id)
                .bind(i64::from(call.sequence))
                .bind(advisor_call_state(call.state))
                .bind(&call.fingerprint)
                .bind(encode(&call)?)
                .bind(call.reserved_at)
                .bind(call.finished_at)
                .execute(&mut *tx)
                .await
                .map_err(sql_error)?;
            }
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_model_calls(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<AdvisorModelCall>, SupervisorError>> {
        Box::pin(async move {
            let calls: Vec<AdvisorModelCall> = load_json_many(
                self,
                "SELECT call_json FROM generation_supervisor_model_calls WHERE session_id = ? ORDER BY sequence",
                session_id,
            )
            .await?;
            for call in &calls {
                if call.reproduce_fingerprint()? != call.fingerprint {
                    return Err(integrity("advisor model call fingerprint mismatch"));
                }
            }
            Ok(calls)
        })
    }

    fn save_model_call_and_session(
        &self,
        call: &AdvisorModelCall,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        let session = session.clone();
        Box::pin(async move {
            session.validate()?;
            if call.session_id != session.id || call.reproduce_fingerprint()? != call.fingerprint {
                return Err(integrity("model call and advisor session do not match"));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            save_model_call_tx(&mut tx, &call).await?;
            save_session_tx(&mut tx, &session).await?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn save_tool_call(&self, call: &AdvisorToolCall) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        Box::pin(async move {
            if call.reproduce_fingerprint()? != call.fingerprint {
                return Err(integrity("advisor tool call fingerprint mismatch"));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            save_tool_call_tx(&mut tx, &call).await?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn save_tool_call_and_session(
        &self,
        call: &AdvisorToolCall,
        session: &AdvisorSession,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let call = call.clone();
        let session = session.clone();
        Box::pin(async move {
            session.validate()?;
            if call.session_id != session.id || call.reproduce_fingerprint()? != call.fingerprint {
                return Err(integrity("tool call and advisor session do not match"));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            save_tool_call_tx(&mut tx, &call).await?;
            save_session_tx(&mut tx, &session).await?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<AdvisorToolCall>, SupervisorError>> {
        Box::pin(async move {
            let calls: Vec<AdvisorToolCall> = load_json_many(
                self,
                "SELECT call_json FROM generation_supervisor_tool_calls WHERE session_id = ? ORDER BY started_at, id",
                session_id,
            )
            .await?;
            for call in &calls {
                if call.reproduce_fingerprint()? != call.fingerprint {
                    return Err(integrity("advisor tool call fingerprint mismatch"));
                }
            }
            Ok(calls)
        })
    }

    fn save_diagnosis(
        &self,
        session_id: Uuid,
        diagnosis: &SupervisorDiagnosis,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let diagnosis = diagnosis.clone();
        Box::pin(async move {
            if diagnosis.reproduce_fingerprint()? != diagnosis.fingerprint {
                return Err(integrity("supervisor diagnosis fingerprint mismatch"));
            }
            let session = load_session(self, session_id)
                .await?
                .ok_or_else(|| integrity("advisor session is not persisted"))?;
            if session.supervisor_run_id != diagnosis.supervisor_run_id {
                return Err(integrity("diagnosis belongs to another supervisor run"));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_diagnoses \
                 (id, session_id, decision_id, fingerprint, diagnosis_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(diagnosis.id)
            .bind(session_id)
            .bind(diagnosis.triggering_decision_id)
            .bind(&diagnosis.fingerprint)
            .bind(encode(&diagnosis)?)
            .bind(diagnosis.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn save_proposal(
        &self,
        session_id: Uuid,
        proposal: &PromptRevisionProposal,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let proposal = proposal.clone();
        Box::pin(async move {
            let session = load_session(self, session_id)
                .await?
                .ok_or_else(|| integrity("advisor session is not persisted"))?;
            let brief = <Self as SupervisorAdvisorStore>::get_brief(self, session.brief_id)
                .await?
                .ok_or_else(|| integrity("diagnosis brief is not persisted"))?;
            proposal.validate(&brief.contract, &brief.current_prompt)?;
            let diagnosis: SupervisorDiagnosis = load_json_required(
                self,
                "SELECT diagnosis_json FROM generation_supervisor_diagnoses WHERE id = ? AND session_id = ?",
                proposal.diagnosis_id,
                Some(session_id),
            )
            .await?;
            if diagnosis.fingerprint != proposal.diagnosis_fingerprint {
                return Err(integrity("proposal diagnosis binding mismatch"));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_revision_proposals \
                 (id, session_id, run_id, diagnosis_id, parent_prompt_version_id, fingerprint, proposal_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(proposal.id)
            .bind(session_id)
            .bind(proposal.supervisor_run_id)
            .bind(proposal.diagnosis_id)
            .bind(proposal.parent_prompt_version_id)
            .bind(&proposal.fingerprint)
            .bind(encode(&proposal)?)
            .bind(proposal.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn latest_diagnosis(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisorDiagnosis>, SupervisorError>> {
        Box::pin(async move {
            load_json_optional(
                self,
                "SELECT diagnosis_json FROM generation_supervisor_diagnoses WHERE session_id = ?",
                session_id,
            )
            .await?
            .map(check_diagnosis)
            .transpose()
        })
    }

    fn latest_proposal(
        &self,
        session_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionProposal>, SupervisorError>> {
        Box::pin(async move {
            load_json_optional(
                self,
                "SELECT proposal_json FROM generation_supervisor_revision_proposals WHERE session_id = ?",
                session_id,
            )
            .await?
            .map(check_proposal)
            .transpose()
        })
    }
}

async fn load_session(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<AdvisorSession>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT session_json FROM generation_supervisor_advisor_sessions WHERE id = ?",
        id,
    )
    .await?
    .map(|session: AdvisorSession| {
        session.validate()?;
        Ok(session)
    })
    .transpose()
}

async fn save_session_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session: &AdvisorSession,
) -> Result<(), SupervisorError> {
    let prior_json: String = sqlx::query_scalar(
        "SELECT session_json FROM generation_supervisor_advisor_sessions WHERE id = ?",
    )
    .bind(session.id)
    .fetch_one(&mut **tx)
    .await
    .map_err(sql_error)?;
    let prior: AdvisorSession = decode(&prior_json)?;
    prior.validate()?;
    if prior == *session {
        return Ok(());
    }
    if !same_session_identity(&prior, session)
        || !advisor_transition_allowed(prior.state, session.state)
    {
        return Err(integrity(
            "advisor session update is not a legal immutable transition",
        ));
    }
    let result = sqlx::query(
        "UPDATE generation_supervisor_advisor_sessions \
         SET state = ?, fingerprint = ?, session_json = ?, updated_at = ? \
         WHERE id = ? AND fingerprint = ?",
    )
    .bind(advisor_state(session.state))
    .bind(&session.fingerprint)
    .bind(encode(session)?)
    .bind(session.updated_at)
    .bind(session.id)
    .bind(&prior.fingerprint)
    .execute(&mut **tx)
    .await
    .map_err(sql_error)?;
    require_one(result.rows_affected(), "advisor session optimistic update")
}

async fn save_model_call_tx(
    tx: &mut Transaction<'_, Sqlite>,
    call: &AdvisorModelCall,
) -> Result<(), SupervisorError> {
    let prior_json: String =
        sqlx::query_scalar("SELECT call_json FROM generation_supervisor_model_calls WHERE id = ?")
            .bind(call.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(sql_error)?;
    let prior: AdvisorModelCall = decode(&prior_json)?;
    if prior == *call {
        return Ok(());
    }
    if prior.session_id != call.session_id
        || prior.sequence != call.sequence
        || prior.input_fingerprint != call.input_fingerprint
        || prior.reserved_at != call.reserved_at
        || !call_transition_allowed(prior.state, call.state)
    {
        return Err(integrity("model call is not a legal terminal transition"));
    }
    let result = sqlx::query(
        "UPDATE generation_supervisor_model_calls SET state = ?, fingerprint = ?, call_json = ?, finished_at = ? WHERE id = ? AND fingerprint = ?",
    )
    .bind(advisor_call_state(call.state))
    .bind(&call.fingerprint)
    .bind(encode(call)?)
    .bind(call.finished_at)
    .bind(call.id)
    .bind(&prior.fingerprint)
    .execute(&mut **tx)
    .await
    .map_err(sql_error)?;
    require_one(
        result.rows_affected(),
        "advisor model call optimistic update",
    )
}

async fn save_tool_call_tx(
    tx: &mut Transaction<'_, Sqlite>,
    call: &AdvisorToolCall,
) -> Result<(), SupervisorError> {
    let prior_json: Option<String> =
        sqlx::query_scalar("SELECT call_json FROM generation_supervisor_tool_calls WHERE id = ?")
            .bind(call.id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(sql_error)?;
    let Some(prior_json) = prior_json else {
        if call.state != AdvisorCallState::Started {
            return Err(integrity("tool call must be persisted before it finishes"));
        }
        sqlx::query(
            "INSERT INTO generation_supervisor_tool_calls \
             (id, session_id, external_call_id, name, state, fingerprint, call_json, started_at, finished_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(call.id)
        .bind(call.session_id)
        .bind(&call.external_call_id)
        .bind(&call.name)
        .bind(advisor_call_state(call.state))
        .bind(&call.fingerprint)
        .bind(encode(call)?)
        .bind(call.started_at)
        .bind(call.finished_at)
        .execute(&mut **tx)
        .await
        .map_err(sql_error)?;
        return Ok(());
    };
    let prior: AdvisorToolCall = decode(&prior_json)?;
    if prior == *call {
        return Ok(());
    }
    if prior.session_id != call.session_id
        || prior.external_call_id != call.external_call_id
        || prior.name != call.name
        || prior.arguments != call.arguments
        || prior.arguments_fingerprint != call.arguments_fingerprint
        || prior.started_at != call.started_at
        || !call_transition_allowed(prior.state, call.state)
    {
        return Err(integrity("tool call is not a legal terminal transition"));
    }
    let result = sqlx::query(
        "UPDATE generation_supervisor_tool_calls SET state = ?, fingerprint = ?, call_json = ?, finished_at = ? WHERE id = ? AND fingerprint = ?",
    )
    .bind(advisor_call_state(call.state))
    .bind(&call.fingerprint)
    .bind(encode(call)?)
    .bind(call.finished_at)
    .bind(call.id)
    .bind(&prior.fingerprint)
    .execute(&mut **tx)
    .await
    .map_err(sql_error)?;
    require_one(
        result.rows_affected(),
        "advisor tool call optimistic update",
    )
}

fn same_session_identity(left: &AdvisorSession, right: &AdvisorSession) -> bool {
    left.id == right.id
        && left.supervisor_run_id == right.supervisor_run_id
        && left.brief_id == right.brief_id
        && left.brief_fingerprint == right.brief_fingerprint
        && left.protocol_version == right.protocol_version
        && left.protocol_fingerprint == right.protocol_fingerprint
        && left.created_at == right.created_at
}

fn advisor_transition_allowed(from: AdvisorSessionState, to: AdvisorSessionState) -> bool {
    from == to
        || matches!(
            (from, to),
            (AdvisorSessionState::Queued, AdvisorSessionState::Running)
                | (AdvisorSessionState::Queued, AdvisorSessionState::Cancelled)
                | (AdvisorSessionState::Queued, AdvisorSessionState::Failed)
                | (AdvisorSessionState::Running, AdvisorSessionState::Running)
                | (
                    AdvisorSessionState::Running,
                    AdvisorSessionState::AwaitingReview
                )
                | (AdvisorSessionState::Running, AdvisorSessionState::Escalated)
                | (AdvisorSessionState::Running, AdvisorSessionState::Failed)
                | (AdvisorSessionState::Running, AdvisorSessionState::Cancelled)
        )
}

fn call_transition_allowed(from: AdvisorCallState, to: AdvisorCallState) -> bool {
    from == to
        || matches!(
            (from, to),
            (AdvisorCallState::Reserved, AdvisorCallState::Started)
                | (AdvisorCallState::Reserved, AdvisorCallState::Succeeded)
                | (AdvisorCallState::Reserved, AdvisorCallState::Interrupted)
                | (AdvisorCallState::Started, AdvisorCallState::Succeeded)
                | (AdvisorCallState::Started, AdvisorCallState::Failed)
                | (AdvisorCallState::Started, AdvisorCallState::Interrupted)
        )
}

fn advisor_state(value: AdvisorSessionState) -> &'static str {
    match value {
        AdvisorSessionState::Queued => "queued",
        AdvisorSessionState::Running => "running",
        AdvisorSessionState::AwaitingReview => "awaiting_review",
        AdvisorSessionState::Escalated => "escalated",
        AdvisorSessionState::Failed => "failed",
        AdvisorSessionState::Cancelled => "cancelled",
    }
}

fn advisor_call_state(value: AdvisorCallState) -> &'static str {
    match value {
        AdvisorCallState::Reserved => "reserved",
        AdvisorCallState::Started => "started",
        AdvisorCallState::Succeeded => "succeeded",
        AdvisorCallState::Failed => "failed",
        AdvisorCallState::Interrupted => "interrupted",
    }
}

fn check_brief(
    value: GenerationQualityDiagnosisBrief,
) -> Result<GenerationQualityDiagnosisBrief, SupervisorError> {
    value.validate()?;
    Ok(value)
}

fn check_diagnosis(value: SupervisorDiagnosis) -> Result<SupervisorDiagnosis, SupervisorError> {
    if value.reproduce_fingerprint()? != value.fingerprint {
        return Err(integrity("supervisor diagnosis fingerprint mismatch"));
    }
    Ok(value)
}

fn check_proposal(
    value: PromptRevisionProposal,
) -> Result<PromptRevisionProposal, SupervisorError> {
    if value.reproduce_fingerprint()? != value.fingerprint {
        return Err(integrity("prompt revision proposal fingerprint mismatch"));
    }
    Ok(value)
}

impl GenerationSupervisorStore for SqliteStore {
    fn create_contract(
        &self,
        contract: &GenerationQualityContract,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let contract = contract.clone();
        Box::pin(async move {
            contract.validate()?;
            let dataset = self
                .get_dataset(contract.dataset.id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("contract dataset does not exist"))?;
            let plan = self
                .get_plan(contract.plan.id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("contract plan does not exist"))?;
            if plan.dataset_id != dataset.id
                || artifact_core::fingerprint(&dataset).map_err(fingerprint_error)?
                    != contract.dataset.fingerprint
                || artifact_core::fingerprint(&plan).map_err(fingerprint_error)?
                    != contract.plan.fingerprint
            {
                return Err(integrity(
                    "contract dataset or plan fingerprint does not match persisted facts",
                ));
            }
            let counts = self
                .dataset_cell_counts(dataset.id)
                .await
                .map_err(generation_error)?;
            if generation_supervisor_core::contract::AcceptedCoverageBinding::from_counts(&counts)?
                != contract.starting_coverage
            {
                return Err(integrity(
                    "contract starting coverage is stale or does not reproduce",
                ));
            }
            sqlx::query(
                "INSERT INTO generation_quality_contracts \
                 (id, dataset_id, plan_id, fingerprint, contract_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(contract.id)
            .bind(contract.dataset.id)
            .bind(contract.plan.id)
            .bind(&contract.fingerprint)
            .bind(encode(&contract)?)
            .bind(contract.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_contract(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationQualityContract>, SupervisorError>> {
        Box::pin(async move { load_contract(self, id).await })
    }

    fn create_run(
        &self,
        run: &SupervisorRun,
        initial_prompt: &PromptGuidanceVersion,
        assignments: Option<&StrategyAssignmentSet>,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let run = run.clone();
        let prompt = initial_prompt.clone();
        let assignments = assignments.cloned();
        Box::pin(async move {
            let contract = load_contract(self, run.contract_id)
                .await?
                .ok_or_else(|| integrity("supervisor contract is not persisted"))?;
            if run.reproduce_fingerprint()? != run.fingerprint
                || run.contract_fingerprint != contract.fingerprint
                || run.initial_prompt_version_id != prompt.id
                || run.initial_prompt_version_fingerprint != prompt.fingerprint
                || prompt.supervisor_run_id != run.id
                || prompt.sequence != 0
            {
                return Err(integrity("supervisor run bundle does not reproduce"));
            }
            prompt.validate()?;
            let plan = self
                .get_plan(contract.plan.id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("supervisor generation plan is missing"))?;
            match (&contract.strategy_context, &assignments) {
                (None, None) => {}
                (Some(binding), Some(set)) => {
                    set.validate(&plan)?;
                    if binding.id != set.strategy_context_id
                        || binding.fingerprint != set.strategy_context_fingerprint
                    {
                        return Err(integrity(
                            "strategy assignment set is outside the quality contract",
                        ));
                    }
                }
                _ => {
                    return Err(integrity(
                        "strategy assignment set presence must match the contract context",
                    ));
                }
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            sqlx::query(
                "INSERT INTO generation_supervisor_runs \
                 (id, contract_id, initial_prompt_version_id, fingerprint, state, cancel_requested, run_json, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 'queued', 0, ?, ?, ?)",
            )
            .bind(run.id)
            .bind(run.contract_id)
            .bind(run.initial_prompt_version_id)
            .bind(&run.fingerprint)
            .bind(encode(&run)?)
            .bind(run.created_at)
            .bind(run.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            insert_prompt_tx(&mut tx, &prompt).await?;
            if let Some(set) = assignments {
                insert_strategy_set_tx(&mut tx, run.id, &set).await?;
            }
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_supervisor_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisorRun>, SupervisorError>> {
        Box::pin(async move { load_run(self, id).await })
    }

    fn request_supervisor_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, SupervisorError>> {
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE generation_supervisor_runs SET cancel_requested = 1, updated_at = ? \
                 WHERE id = ? AND state NOT IN ('completed', 'failed', 'cancelled')",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(result.rows_affected() == 1)
        })
    }

    fn supervisor_cancel_requested(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<bool>, SupervisorError>> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT cancel_requested FROM generation_supervisor_runs WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)
        })
    }

    fn append_run_event(
        &self,
        event: &SupervisorRunEvent,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let event = event.clone();
        Box::pin(async move {
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let run_json: String =
                sqlx::query_scalar("SELECT run_json FROM generation_supervisor_runs WHERE id = ?")
                    .bind(event.run_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(sql_error)?;
            let run: SupervisorRun = decode(&run_json)?;
            let event_jsons: Vec<String> = sqlx::query_scalar(
                "SELECT event_json FROM generation_supervisor_run_events WHERE run_id = ? ORDER BY sequence",
            )
            .bind(event.run_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(sql_error)?;
            let mut events = event_jsons
                .into_iter()
                .map(|value| decode(&value))
                .collect::<Result<Vec<SupervisorRunEvent>, _>>()?;
            let current = SupervisorRunEvent::verify_chain(run.id, &events)?;
            if current != event.from_state
                || event.sequence != u32::try_from(events.len()).unwrap_or(u32::MAX)
                || event.reproduce_fingerprint()? != event.fingerprint
            {
                return Err(integrity("run event does not extend the durable chain"));
            }
            events.push(event.clone());
            SupervisorRunEvent::verify_chain(run.id, &events)?;
            sqlx::query(
                "INSERT INTO generation_supervisor_run_events \
                 (id, run_id, sequence, previous_event_fingerprint, from_state, to_state, fingerprint, event_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(event.id)
            .bind(event.run_id)
            .bind(i64::from(event.sequence))
            .bind(&event.previous_event_fingerprint)
            .bind(run_state(event.from_state))
            .bind(run_state(event.to_state))
            .bind(&event.fingerprint)
            .bind(encode(&event)?)
            .bind(event.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            let result = sqlx::query(
                "UPDATE generation_supervisor_runs SET state = ?, updated_at = ? WHERE id = ? AND state = ?",
            )
            .bind(run_state(event.to_state))
            .bind(event.created_at)
            .bind(event.run_id)
            .bind(run_state(event.from_state))
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            require_one(result.rows_affected(), "supervisor run state transition")?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_run_events(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SupervisorRunEvent>, SupervisorError>> {
        Box::pin(async move {
            let events: Vec<SupervisorRunEvent> = load_json_many(
                self,
                "SELECT event_json FROM generation_supervisor_run_events WHERE run_id = ? ORDER BY sequence",
                run_id,
            )
            .await?;
            SupervisorRunEvent::verify_chain(run_id, &events)?;
            Ok(events)
        })
    }

    fn reserve_child(
        &self,
        reservation: &ChildReservation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let reservation = reservation.clone();
        Box::pin(async move {
            if reservation.reproduce_fingerprint()? != reservation.fingerprint {
                return Err(integrity("child reservation fingerprint mismatch"));
            }
            reservation
                .reserved_usage
                .validate_for_child(reservation.kind)?;
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let run_json: Option<String> =
                sqlx::query_scalar("SELECT run_json FROM generation_supervisor_runs WHERE id = ?")
                    .bind(reservation.run_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(sql_error)?;
            let run: SupervisorRun = run_json
                .ok_or_else(|| integrity("child reservation supervisor run is missing"))
                .and_then(|value| decode(&value))?;
            if run.reproduce_fingerprint()? != run.fingerprint {
                return Err(integrity("supervisor run fingerprint mismatch"));
            }
            let contract_json: Option<String> = sqlx::query_scalar(
                "SELECT contract_json FROM generation_quality_contracts WHERE id = ?",
            )
            .bind(run.contract_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(sql_error)?;
            let contract: GenerationQualityContract = contract_json
                .ok_or_else(|| integrity("child reservation contract is missing"))
                .and_then(|value| decode(&value))?;
            contract.validate()?;
            if run.contract_fingerprint != contract.fingerprint {
                return Err(integrity("supervisor run contract binding mismatch"));
            }
            let existing_json = sqlx::query_scalar::<_, String>(
                "SELECT reservation_json FROM generation_supervisor_child_reservations WHERE run_id = ? ORDER BY reserved_at, id",
            )
            .bind(reservation.run_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(sql_error)?;
            let mut usage = SupervisorUsage::default();
            for value in existing_json {
                let existing: ChildReservation = decode(&value)?;
                if existing.reproduce_fingerprint()? != existing.fingerprint {
                    return Err(integrity("child reservation fingerprint mismatch"));
                }
                usage = usage.checked_add(&existing.reserved_usage)?;
            }
            usage.reserve(&reservation.reserved_usage, &contract)?;
            if let Some(replaces) = reservation.replaces_reservation_id {
                let prior_json: Option<String> = sqlx::query_scalar(
                    "SELECT reservation_json FROM generation_supervisor_child_reservations WHERE id = ?",
                )
                .bind(replaces)
                .fetch_optional(&mut *tx)
                .await
                .map_err(sql_error)?;
                let prior: ChildReservation = prior_json
                    .ok_or_else(|| integrity("replacement reservation is missing"))
                    .and_then(|value| decode(&value))?;
                let outcome_json: Option<String> = sqlx::query_scalar(
                    "SELECT outcome_json FROM generation_supervisor_child_outcomes WHERE reservation_id = ?",
                )
                .bind(replaces)
                .fetch_optional(&mut *tx)
                .await
                .map_err(sql_error)?;
                let outcome: ChildOutcome = outcome_json
                    .ok_or_else(|| integrity("replacement outcome is missing"))
                    .and_then(|value| decode(&value))?;
                if prior.run_id != reservation.run_id
                    || prior.kind != reservation.kind
                    || prior.logical_input_key != reservation.logical_input_key
                    || outcome.state
                        != generation_supervisor_core::lifecycle::ChildOutcomeState::Interrupted
                {
                    return Err(integrity(
                        "replacement child must explicitly replace an interrupted logical child",
                    ));
                }
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_child_reservations \
                 (id, run_id, kind, logical_input_key, child_id, attempt, replaces_reservation_id, fingerprint, reservation_json, reserved_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(reservation.id)
            .bind(reservation.run_id)
            .bind(child_kind(reservation.kind))
            .bind(&reservation.logical_input_key)
            .bind(reservation.child_id)
            .bind(i64::from(reservation.attempt))
            .bind(reservation.replaces_reservation_id)
            .bind(&reservation.fingerprint)
            .bind(encode(&reservation)?)
            .bind(reservation.reserved_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn finish_child(&self, outcome: &ChildOutcome) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let outcome = outcome.clone();
        Box::pin(async move {
            if outcome.reproduce_fingerprint()? != outcome.fingerprint {
                return Err(integrity("child outcome fingerprint mismatch"));
            }
            let reservation: ChildReservation = load_json_required(
                self,
                "SELECT reservation_json FROM generation_supervisor_child_reservations WHERE id = ?",
                outcome.reservation_id,
                None,
            )
            .await?;
            if outcome.reservation_fingerprint != reservation.fingerprint {
                return Err(integrity("child outcome reservation binding mismatch"));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_child_outcomes \
                 (id, reservation_id, state, fingerprint, outcome_json, finished_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(outcome.id)
            .bind(outcome.reservation_id)
            .bind(child_outcome_state(outcome.state))
            .bind(&outcome.fingerprint)
            .bind(encode(&outcome)?)
            .bind(outcome.finished_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_child_reservations(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ChildReservation>, SupervisorError>> {
        Box::pin(async move {
            let values: Vec<ChildReservation> = load_json_many(
                self,
                "SELECT reservation_json FROM generation_supervisor_child_reservations WHERE run_id = ? ORDER BY reserved_at, id",
                run_id,
            )
            .await?;
            for value in &values {
                if value.reproduce_fingerprint()? != value.fingerprint {
                    return Err(integrity("child reservation fingerprint mismatch"));
                }
            }
            Ok(values)
        })
    }

    fn list_child_outcomes(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ChildOutcome>, SupervisorError>> {
        Box::pin(async move {
            let values = sqlx::query_scalar::<_, String>(
                "SELECT o.outcome_json FROM generation_supervisor_child_outcomes o \
                 JOIN generation_supervisor_child_reservations r ON r.id = o.reservation_id \
                 WHERE r.run_id = ? ORDER BY o.finished_at, o.id",
            )
            .bind(run_id)
            .fetch_all(self.pool())
            .await
            .map_err(sql_error)?
            .into_iter()
            .map(|value| decode(&value))
            .collect::<Result<Vec<ChildOutcome>, _>>()?;
            for value in &values {
                if value.reproduce_fingerprint()? != value.fingerprint {
                    return Err(integrity("child outcome fingerprint mismatch"));
                }
            }
            Ok(values)
        })
    }

    fn get_strategy_assignments(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<StrategyAssignmentSet>, SupervisorError>> {
        Box::pin(async move {
            let Some(set): Option<StrategyAssignmentSet> = load_json_optional(
                self,
                "SELECT set_json FROM generation_supervisor_strategy_sets WHERE run_id = ?",
                run_id,
            )
            .await?
            else {
                return Ok(None);
            };
            let plan = self
                .get_plan(set.plan_id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("strategy assignment plan is missing"))?;
            set.validate(&plan)?;
            let assignment_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM generation_supervisor_strategy_assignments WHERE assignment_set_id = ?",
            )
            .bind(set.id)
            .fetch_one(self.pool())
            .await
            .map_err(sql_error)?;
            if u64::try_from(assignment_count).ok() != Some(set.assignments.len() as u64) {
                return Err(integrity("strategy assignment rows do not match their set"));
            }
            Ok(Some(set))
        })
    }

    fn save_prompt_version(
        &self,
        version: &PromptGuidanceVersion,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let version = version.clone();
        Box::pin(async move {
            version.validate()?;
            if version.sequence == 0 {
                return Err(validation("initial prompt version is created with its run"));
            }
            let prior = load_prompt(
                self,
                version
                    .parent_version_id
                    .ok_or_else(|| integrity("revised prompt has no parent"))?,
            )
            .await?
            .ok_or_else(|| integrity("prompt parent is not persisted"))?;
            if prior.supervisor_run_id != version.supervisor_run_id
                || prior.sequence.checked_add(1) != Some(version.sequence)
                || prior.base_prompt_fingerprint != version.base_prompt_fingerprint
                || prior.protected_fields_fingerprint != version.protected_fields_fingerprint
            {
                return Err(integrity(
                    "prompt version chain or protected fields changed",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            insert_prompt_tx(&mut tx, &version).await?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_prompt_version(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptGuidanceVersion>, SupervisorError>> {
        Box::pin(async move { load_prompt(self, id).await })
    }

    fn list_prompt_versions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<PromptGuidanceVersion>, SupervisorError>> {
        Box::pin(async move {
            let versions: Vec<PromptGuidanceVersion> = load_json_many(
                self,
                "SELECT version_json FROM generation_supervisor_prompt_versions WHERE run_id = ? ORDER BY sequence",
                run_id,
            )
            .await?;
            validate_prompt_chain(&versions)?;
            Ok(versions)
        })
    }

    fn save_row_observations(
        &self,
        rows: &[RowQualityObservation],
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let rows = rows.to_vec();
        Box::pin(async move {
            if rows.is_empty() {
                return Err(validation("row observation batch must not be empty"));
            }
            let run_id = rows[0].supervisor_run_id;
            let run = load_run(self, run_id)
                .await?
                .ok_or_else(|| integrity("row observation run is not persisted"))?;
            let contract = load_contract(self, run.contract_id)
                .await?
                .ok_or_else(|| integrity("row observation contract is missing"))?;
            let mut ids = BTreeSet::new();
            for row in &rows {
                if row.supervisor_run_id != run_id || !ids.insert(row.generated_row_id) {
                    return Err(integrity(
                        "row observation batch mixes runs or duplicates a generated row",
                    ));
                }
                verify_row_binding(self, row, &contract, run_id).await?;
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            for row in rows {
                sqlx::query(
                    "INSERT INTO generation_supervisor_row_observations \
                     (id, run_id, contract_id, prompt_version_id, generated_row_id, generation_job_id, generation_attempt_id, cell_key, directive_id, fingerprint, observation_json, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(row.id)
                .bind(row.supervisor_run_id)
                .bind(row.contract_id)
                .bind(row.prompt_version_id)
                .bind(row.generated_row_id)
                .bind(row.generation_job_id)
                .bind(row.generation_attempt_id)
                .bind(&row.cell_key)
                .bind(row.strategy_directive_id)
                .bind(&row.fingerprint)
                .bind(encode(&row)?)
                .bind(row.created_at)
                .execute(&mut *tx)
                .await
                .map_err(sql_error)?;
            }
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_row_observations(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityObservation>, SupervisorError>> {
        Box::pin(async move {
            let run = load_run(self, run_id)
                .await?
                .ok_or_else(|| integrity("supervisor run is not persisted"))?;
            let contract = load_contract(self, run.contract_id)
                .await?
                .ok_or_else(|| integrity("supervisor contract is not persisted"))?;
            let rows: Vec<RowQualityObservation> = load_json_many(
                self,
                "SELECT observation_json FROM generation_supervisor_row_observations WHERE run_id = ? ORDER BY created_at, id",
                run_id,
            )
            .await?;
            for row in &rows {
                verify_row_binding(self, row, &contract, run_id).await?;
            }
            Ok(rows)
        })
    }

    fn save_quality_window(
        &self,
        manifest: &QualityEvidenceManifest,
        observation: &BatchQualityObservation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let manifest = manifest.clone();
        let observation = observation.clone();
        Box::pin(async move {
            let contract = load_contract(self, observation.contract_id)
                .await?
                .ok_or_else(|| integrity("quality window contract is missing"))?;
            observation.validate(&contract, &manifest)?;
            let persisted: BTreeMap<Uuid, String> = sqlx::query(
                "SELECT id, fingerprint FROM generation_supervisor_row_observations WHERE run_id = ?",
            )
            .bind(observation.supervisor_run_id)
            .fetch_all(self.pool())
            .await
            .map_err(sql_error)?
            .into_iter()
            .map(|row| Ok((row.try_get("id").map_err(sql_error)?, row.try_get("fingerprint").map_err(sql_error)?)))
            .collect::<Result<_, SupervisorError>>()?;
            if manifest.members.iter().any(|member| {
                persisted.get(&member.observation_id) != Some(&member.observation_fingerprint)
            }) {
                return Err(integrity(
                    "quality manifest references missing or changed row evidence",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            sqlx::query(
                "INSERT INTO generation_supervisor_quality_manifests \
                 (id, run_id, window_id, fingerprint, manifest_json, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(manifest.id)
            .bind(manifest.supervisor_run_id)
            .bind(manifest.window_id)
            .bind(&manifest.fingerprint)
            .bind(encode(&manifest)?)
            .bind(manifest.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            for member in &manifest.members {
                sqlx::query(
                    "INSERT INTO generation_supervisor_manifest_members \
                     (manifest_id, observation_id, observation_fingerprint, generated_row_id) VALUES (?, ?, ?, ?)",
                )
                .bind(manifest.id)
                .bind(member.observation_id)
                .bind(&member.observation_fingerprint)
                .bind(member.generated_row_id)
                .execute(&mut *tx)
                .await
                .map_err(sql_error)?;
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_quality_windows \
                 (id, run_id, contract_id, prompt_version_id, manifest_id, scope_key, kind, sequence, fingerprint, observation_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(observation.id)
            .bind(observation.supervisor_run_id)
            .bind(observation.contract_id)
            .bind(observation.prompt_version_id)
            .bind(observation.manifest_id)
            .bind(scope_key(&observation.scope)?)
            .bind(window_kind(observation.kind))
            .bind(i64::from(observation.sequence))
            .bind(&observation.fingerprint)
            .bind(encode(&observation)?)
            .bind(observation.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_quality_window(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BatchQualityObservation>, SupervisorError>> {
        Box::pin(async move { load_window(self, id).await })
    }

    fn list_quality_windows(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BatchQualityObservation>, SupervisorError>> {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM generation_supervisor_quality_windows WHERE run_id = ? ORDER BY created_at, id",
            )
            .bind(run_id)
            .fetch_all(self.pool())
            .await
            .map_err(sql_error)?;
            let mut windows = Vec::with_capacity(ids.len());
            for id in ids {
                windows.push(
                    load_window(self, id)
                        .await?
                        .ok_or_else(|| integrity("quality window disappeared"))?,
                );
            }
            Ok(windows)
        })
    }

    fn save_decision(
        &self,
        decision: &DeterministicQualityDecision,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let decision = decision.clone();
        Box::pin(async move {
            let contract = load_contract(self, decision.contract_id)
                .await?
                .ok_or_else(|| integrity("decision contract is missing"))?;
            decision.validate(&contract)?;
            let window = load_window(self, decision.window_id)
                .await?
                .ok_or_else(|| integrity("decision window is missing"))?;
            let baseline = match decision.baseline_window_id {
                Some(id) => Some(
                    load_window(self, id)
                        .await?
                        .ok_or_else(|| integrity("decision baseline window is missing"))?,
                ),
                None => None,
            };
            let reproduced = DeterministicQualityDecision::evaluate(
                decision.id,
                &contract,
                &window,
                baseline.as_ref(),
                decision.created_at,
            )?;
            if reproduced != decision {
                return Err(integrity(
                    "deterministic quality decision does not recompute",
                ));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_decisions \
                 (id, run_id, contract_id, window_id, state, fingerprint, decision_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(decision.id)
            .bind(decision.supervisor_run_id)
            .bind(decision.contract_id)
            .bind(decision.window_id)
            .bind(decision_state(decision.state))
            .bind(&decision.fingerprint)
            .bind(encode(&decision)?)
            .bind(decision.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_decisions(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<DeterministicQualityDecision>, SupervisorError>> {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM generation_supervisor_decisions WHERE run_id = ? ORDER BY created_at, id",
            )
            .bind(run_id)
            .fetch_all(self.pool())
            .await
            .map_err(sql_error)?;
            let mut decisions = Vec::with_capacity(ids.len());
            for id in ids {
                decisions.push(
                    load_decision(self, id)
                        .await?
                        .ok_or_else(|| integrity("quality decision disappeared"))?,
                );
            }
            Ok(decisions)
        })
    }

    fn append_revision_review(
        &self,
        review: &PromptRevisionReview,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let review = review.clone();
        Box::pin(async move {
            if review.reproduce_fingerprint()? != review.fingerprint {
                return Err(integrity("revision review fingerprint mismatch"));
            }
            let proposal = load_proposal(self, review.proposal_id)
                .await?
                .ok_or_else(|| integrity("review proposal is missing"))?;
            if review.proposal_fingerprint != proposal.fingerprint {
                return Err(integrity("revision review proposal fingerprint mismatch"));
            }
            let latest = load_latest_review(self, review.proposal_id).await?;
            if review.predecessor_id != latest.as_ref().map(|value| value.id)
                || review.predecessor_fingerprint
                    != latest.as_ref().map(|value| value.fingerprint.clone())
            {
                return Err(integrity("revision review predecessor is stale"));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_revision_reviews \
                 (id, proposal_id, predecessor_id, predecessor_fingerprint, decision, fingerprint, review_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.proposal_id)
            .bind(review.predecessor_id)
            .bind(&review.predecessor_fingerprint)
            .bind(review_decision(review.decision))
            .bind(&review.fingerprint)
            .bind(encode(&review)?)
            .bind(review.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn latest_revision_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionReview>, SupervisorError>> {
        Box::pin(async move { load_latest_review(self, proposal_id).await })
    }

    fn save_revision_authorization(
        &self,
        authorization: &PromptRevisionAuthorization,
        candidate_version: &PromptGuidanceVersion,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let authorization = authorization.clone();
        let version = candidate_version.clone();
        Box::pin(async move {
            if authorization.reproduce_fingerprint()? != authorization.fingerprint
                || version.source_proposal_id != Some(authorization.proposal_id)
            {
                return Err(integrity(
                    "revision authorization bundle fingerprint mismatch",
                ));
            }
            let proposal = load_proposal(self, authorization.proposal_id)
                .await?
                .ok_or_else(|| integrity("authorized proposal is missing"))?;
            if proposal.fingerprint != authorization.proposal_fingerprint {
                return Err(integrity("authorization proposal fingerprint mismatch"));
            }
            let parent = load_prompt(self, proposal.parent_prompt_version_id)
                .await?
                .ok_or_else(|| integrity("authorized proposal parent prompt is missing"))?;
            let contract_id: Uuid = sqlx::query_scalar(
                "SELECT contract_id FROM generation_supervisor_runs WHERE id = ?",
            )
            .bind(proposal.supervisor_run_id)
            .fetch_one(self.pool())
            .await
            .map_err(sql_error)?;
            let contract = load_contract(self, contract_id)
                .await?
                .ok_or_else(|| integrity("authorization contract is missing"))?;
            proposal.validate(&contract, &parent)?;
            let review = match authorization.review_id {
                Some(id) => Some(
                    load_review(self, id)
                        .await?
                        .ok_or_else(|| integrity("authorization review is missing"))?,
                ),
                None => None,
            };
            let (expected, mut expected_version) = PromptRevisionAuthorization::authorize(
                &contract,
                &proposal,
                &parent,
                review.as_ref(),
                authorization.authorized_at,
            )?;
            // The constructor allocates a fresh identity. All other derived
            // fields must match the caller-persisted candidate exactly.
            expected_version.id = version.id;
            expected_version.fingerprint = expected_version.reproduce_fingerprint()?;
            if expected != authorization || expected_version != version {
                return Err(integrity(
                    "revision authorization does not deterministically reproduce",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            insert_prompt_tx(&mut tx, &version).await?;
            sqlx::query(
                "INSERT INTO generation_supervisor_revision_authorizations \
                 (proposal_id, review_id, prompt_version_id, fingerprint, authorization_json, authorized_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(authorization.proposal_id)
            .bind(authorization.review_id)
            .bind(version.id)
            .bind(&authorization.fingerprint)
            .bind(encode(&authorization)?)
            .bind(authorization.authorized_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn save_revision_activation(
        &self,
        activation: &PromptRevisionActivation,
    ) -> BoxFuture<'_, Result<(), SupervisorError>> {
        let activation = activation.clone();
        Box::pin(async move {
            if activation.reproduce_fingerprint()? != activation.fingerprint {
                return Err(integrity("revision activation fingerprint mismatch"));
            }
            let version = load_prompt(self, activation.prompt_version_id)
                .await?
                .ok_or_else(|| integrity("activated prompt version is missing"))?;
            let authorization: PromptRevisionAuthorization = load_json_required(
                self,
                "SELECT authorization_json FROM generation_supervisor_revision_authorizations WHERE prompt_version_id = ?",
                version.id,
                None,
            )
            .await?;
            let decision = load_decision(self, activation.canary_decision_id)
                .await?
                .ok_or_else(|| integrity("activation canary decision is missing"))?;
            let expected = PromptRevisionActivation::create(
                activation.id,
                &version,
                &authorization,
                &decision,
                activation.activated_at,
            )?;
            if expected != activation {
                return Err(integrity(
                    "revision activation does not deterministically reproduce",
                ));
            }
            sqlx::query(
                "INSERT INTO generation_supervisor_revision_activations \
                 (id, run_id, prompt_version_id, canary_decision_id, fingerprint, activation_json, activated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(activation.id)
            .bind(activation.supervisor_run_id)
            .bind(activation.prompt_version_id)
            .bind(activation.canary_decision_id)
            .bind(&activation.fingerprint)
            .bind(encode(&activation)?)
            .bind(activation.activated_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_revision_activation(
        &self,
        prompt_version_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PromptRevisionActivation>, SupervisorError>> {
        Box::pin(async move {
            load_json_optional(
                self,
                "SELECT activation_json FROM generation_supervisor_revision_activations WHERE prompt_version_id = ?",
                prompt_version_id,
            )
            .await?
            .map(|value: PromptRevisionActivation| {
                if value.reproduce_fingerprint()? != value.fingerprint {
                    return Err(integrity("revision activation fingerprint mismatch"));
                }
                Ok(value)
            })
            .transpose()
        })
    }

    fn trace_supervised_row(
        &self,
        generated_row_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SupervisedRowTrace>, SupervisorError>> {
        Box::pin(async move {
            let Some(row): Option<RowQualityObservation> = load_json_optional(
                self,
                "SELECT observation_json FROM generation_supervisor_row_observations WHERE generated_row_id = ?",
                generated_row_id,
            )
            .await?
            else {
                return Ok(None);
            };
            let run = load_run(self, row.supervisor_run_id)
                .await?
                .ok_or_else(|| integrity("trace supervisor run is missing"))?;
            let contract = load_contract(self, row.contract_id)
                .await?
                .ok_or_else(|| integrity("trace quality contract is missing"))?;
            row.validate(&contract)?;
            let prompt_version = load_prompt(self, row.prompt_version_id)
                .await?
                .ok_or_else(|| integrity("trace prompt version is missing"))?;
            let window_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT m.window_id FROM generation_supervisor_manifest_members mm \
                 JOIN generation_supervisor_quality_manifests m ON m.id = mm.manifest_id \
                 WHERE mm.observation_id = ? ORDER BY m.created_at, m.window_id",
            )
            .bind(row.id)
            .fetch_all(self.pool())
            .await
            .map_err(sql_error)?;
            let mut windows = Vec::with_capacity(window_ids.len());
            for id in window_ids {
                windows.push(
                    load_window(self, id)
                        .await?
                        .ok_or_else(|| integrity("trace quality window is missing"))?,
                );
            }
            let window_set = windows
                .iter()
                .map(|value| value.id)
                .collect::<BTreeSet<_>>();
            let decisions = <Self as GenerationSupervisorStore>::list_decisions(self, run.id)
                .await?
                .into_iter()
                .filter(|value| window_set.contains(&value.window_id))
                .collect();
            Ok(Some(SupervisedRowTrace {
                contract,
                run,
                prompt_version,
                strategy_assignment_set:
                    <Self as GenerationSupervisorStore>::get_strategy_assignments(
                        self,
                        row.supervisor_run_id,
                    )
                    .await?,
                row_observation: row,
                windows,
                decisions,
            }))
        })
    }

    fn verify_supervisor_integrity(
        &self,
    ) -> BoxFuture<'_, Result<SupervisorIntegrityReport, SupervisorError>> {
        Box::pin(async move { verify_all(self).await })
    }
}

async fn insert_prompt_tx(
    tx: &mut Transaction<'_, Sqlite>,
    version: &PromptGuidanceVersion,
) -> Result<(), SupervisorError> {
    version.validate()?;
    sqlx::query(
        "INSERT INTO generation_supervisor_prompt_versions \
         (id, run_id, sequence, parent_version_id, source_proposal_id, fingerprint, version_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(version.id)
    .bind(version.supervisor_run_id)
    .bind(i64::from(version.sequence))
    .bind(version.parent_version_id)
    .bind(version.source_proposal_id)
    .bind(&version.fingerprint)
    .bind(encode(version)?)
    .bind(version.created_at)
    .execute(&mut **tx)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn insert_strategy_set_tx(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: Uuid,
    set: &StrategyAssignmentSet,
) -> Result<(), SupervisorError> {
    sqlx::query(
        "INSERT INTO generation_supervisor_strategy_sets \
         (id, run_id, plan_id, context_id, fingerprint, set_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(set.id)
    .bind(run_id)
    .bind(set.plan_id)
    .bind(set.strategy_context_id)
    .bind(&set.fingerprint)
    .bind(encode(set)?)
    .bind(set.created_at)
    .execute(&mut **tx)
    .await
    .map_err(sql_error)?;
    for assignment in &set.assignments {
        sqlx::query(
            "INSERT INTO generation_supervisor_strategy_assignments \
             (assignment_set_id, cell_key, row_sequence, directive_id, fingerprint, assignment_json) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(set.id)
        .bind(&assignment.cell_key)
        .bind(i64::from(assignment.row_sequence))
        .bind(assignment.directive_id)
        .bind(&assignment.fingerprint)
        .bind(encode(assignment)?)
        .execute(&mut **tx)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn load_contract(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<GenerationQualityContract>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT contract_json FROM generation_quality_contracts WHERE id = ?",
        id,
    )
    .await?
    .map(|contract: GenerationQualityContract| {
        contract.validate()?;
        Ok(contract)
    })
    .transpose()
}

async fn load_run(store: &SqliteStore, id: Uuid) -> Result<Option<SupervisorRun>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT run_json FROM generation_supervisor_runs WHERE id = ?",
        id,
    )
    .await?
    .map(|run: SupervisorRun| {
        if run.reproduce_fingerprint()? != run.fingerprint {
            return Err(integrity("supervisor run fingerprint mismatch"));
        }
        Ok(run)
    })
    .transpose()
}

async fn load_prompt(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<PromptGuidanceVersion>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT version_json FROM generation_supervisor_prompt_versions WHERE id = ?",
        id,
    )
    .await?
    .map(|value: PromptGuidanceVersion| {
        value.validate()?;
        Ok(value)
    })
    .transpose()
}

async fn load_window(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<BatchQualityObservation>, SupervisorError> {
    let Some(value): Option<BatchQualityObservation> = load_json_optional(
        store,
        "SELECT observation_json FROM generation_supervisor_quality_windows WHERE id = ?",
        id,
    )
    .await?
    else {
        return Ok(None);
    };
    let contract = load_contract(store, value.contract_id)
        .await?
        .ok_or_else(|| integrity("quality window contract is missing"))?;
    let manifest: QualityEvidenceManifest = load_json_required(
        store,
        "SELECT manifest_json FROM generation_supervisor_quality_manifests WHERE id = ?",
        value.manifest_id,
        None,
    )
    .await?;
    value.validate(&contract, &manifest)?;
    let member_rows = sqlx::query(
        "SELECT observation_id, observation_fingerprint, generated_row_id \
         FROM generation_supervisor_manifest_members WHERE manifest_id = ?",
    )
    .bind(manifest.id)
    .fetch_all(store.pool())
    .await
    .map_err(sql_error)?;
    let persisted = member_rows
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<Uuid, _>("observation_id")
                    .map_err(sql_error)?,
                row.try_get::<String, _>("observation_fingerprint")
                    .map_err(sql_error)?,
                row.try_get::<Uuid, _>("generated_row_id")
                    .map_err(sql_error)?,
            ))
        })
        .collect::<Result<BTreeSet<_>, SupervisorError>>()?;
    let expected = manifest
        .members
        .iter()
        .map(|member| {
            (
                member.observation_id,
                member.observation_fingerprint.clone(),
                member.generated_row_id,
            )
        })
        .collect::<BTreeSet<_>>();
    if persisted != expected {
        return Err(integrity("quality manifest member rows do not reproduce"));
    }
    let mut observations = Vec::with_capacity(manifest.members.len());
    for member in &manifest.members {
        let observation: RowQualityObservation = load_json_required(
            store,
            "SELECT observation_json FROM generation_supervisor_row_observations WHERE id = ?",
            member.observation_id,
            None,
        )
        .await?;
        verify_row_binding(store, &observation, &contract, value.supervisor_run_id).await?;
        observations.push(observation);
    }
    let reproduced = BatchQualityObservation::aggregate(
        value.id,
        manifest.id,
        &contract,
        value.supervisor_run_id,
        value.prompt_version_id,
        value.prompt_version_fingerprint.clone(),
        value.kind,
        value.sequence,
        value.scope.clone(),
        &observations,
        value.created_at,
    )?;
    if reproduced.manifest != manifest || reproduced.observation != value {
        return Err(integrity(
            "quality window does not reproduce from its immutable row evidence",
        ));
    }
    Ok(Some(value))
}

async fn load_decision(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<DeterministicQualityDecision>, SupervisorError> {
    let Some(value): Option<DeterministicQualityDecision> = load_json_optional(
        store,
        "SELECT decision_json FROM generation_supervisor_decisions WHERE id = ?",
        id,
    )
    .await?
    else {
        return Ok(None);
    };
    if value.reproduce_fingerprint()? != value.fingerprint {
        return Err(integrity("quality decision fingerprint mismatch"));
    }
    let contract = load_contract(store, value.contract_id)
        .await?
        .ok_or_else(|| integrity("quality decision contract is missing"))?;
    let window = load_window(store, value.window_id)
        .await?
        .ok_or_else(|| integrity("quality decision window is missing"))?;
    let baseline = match value.baseline_window_id {
        Some(id) => Some(
            load_window(store, id)
                .await?
                .ok_or_else(|| integrity("quality decision baseline is missing"))?,
        ),
        None => None,
    };
    let reproduced = DeterministicQualityDecision::evaluate(
        value.id,
        &contract,
        &window,
        baseline.as_ref(),
        value.created_at,
    )?;
    if reproduced != value {
        return Err(integrity(
            "quality decision does not reproduce from its immutable window evidence",
        ));
    }
    Ok(Some(value))
}

async fn load_proposal(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<PromptRevisionProposal>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT proposal_json FROM generation_supervisor_revision_proposals WHERE id = ?",
        id,
    )
    .await?
    .map(check_proposal)
    .transpose()
}

async fn load_review(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<PromptRevisionReview>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT review_json FROM generation_supervisor_revision_reviews WHERE id = ?",
        id,
    )
    .await?
    .map(|value: PromptRevisionReview| {
        if value.reproduce_fingerprint()? != value.fingerprint {
            return Err(integrity("revision review fingerprint mismatch"));
        }
        Ok(value)
    })
    .transpose()
}

async fn load_latest_review(
    store: &SqliteStore,
    proposal_id: Uuid,
) -> Result<Option<PromptRevisionReview>, SupervisorError> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT review_json FROM generation_supervisor_revision_reviews WHERE proposal_id = ? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(proposal_id)
    .fetch_optional(store.pool())
    .await
    .map_err(sql_error)?;
    value
        .map(|value| decode(&value))
        .transpose()?
        .map(|review: PromptRevisionReview| {
            if review.reproduce_fingerprint()? != review.fingerprint {
                return Err(integrity("revision review fingerprint mismatch"));
            }
            Ok(review)
        })
        .transpose()
}

async fn load_child_reservation_by_child(
    store: &SqliteStore,
    run_id: Uuid,
    child_id: Uuid,
) -> Result<Option<ChildReservation>, SupervisorError> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT reservation_json FROM generation_supervisor_child_reservations WHERE run_id = ? AND child_id = ?",
    )
    .bind(run_id)
    .bind(child_id)
    .fetch_optional(store.pool())
    .await
    .map_err(sql_error)?;
    value.map(|value| decode(&value)).transpose()
}

async fn load_child_outcome(
    store: &SqliteStore,
    reservation_id: Uuid,
) -> Result<Option<ChildOutcome>, SupervisorError> {
    load_json_optional(
        store,
        "SELECT outcome_json FROM generation_supervisor_child_outcomes WHERE reservation_id = ?",
        reservation_id,
    )
    .await
}

async fn load_generated_row(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<generation_core::domain::GeneratedRow>, SupervisorError> {
    let record = sqlx::query_as::<_, RowRecord>(
        "SELECT id, dataset_id, plan_id, generation_job_id, cell_key, text, normalized_text, \
         label, dimensions_json, generator_backend, generator_model, created_at, \
         validation_status, validation_errors_json, generation_metadata_json, fields_json, construction_json \
         FROM generated_rows WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(sql_error)?;
    record
        .map(RowRecord::into_domain)
        .transpose()
        .map_err(generation_error)
}

async fn load_source_row(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<dataset_core::domain::SourceRow>, SupervisorError> {
    let record = sqlx::query_as::<_, SourceRowRecord>(
        "SELECT id, dataset_id, text, label, dimensions_json, fields_json, provenance_json, created_at \
         FROM dataset_source_rows WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(sql_error)?;
    record
        .map(SourceRowRecord::into_domain)
        .transpose()
        .map_err(|error| integrity(error.to_string()))
}

async fn load_generation_attempt(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<GenerationAttempt>, SupervisorError> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT artifact_json FROM generation_request_attempts WHERE id = ?")
            .bind(id)
            .fetch_optional(store.pool())
            .await
            .map_err(sql_error)?;
    value.map(|value| decode(&value)).transpose()
}

async fn verify_row_binding(
    store: &SqliteStore,
    row: &RowQualityObservation,
    contract: &GenerationQualityContract,
    run_id: Uuid,
) -> Result<(), SupervisorError> {
    row.validate(contract)?;
    if row.supervisor_run_id != run_id {
        return Err(integrity("row observation is outside its supervisor run"));
    }
    let segment = load_child_reservation_by_child(store, run_id, row.segment_id)
        .await?
        .ok_or_else(|| integrity("row generation segment is not reserved"))?;
    if !matches!(
        segment.kind,
        generation_supervisor_core::lifecycle::ChildKind::GenerationSegment
            | generation_supervisor_core::lifecycle::ChildKind::RevisionCanary
    ) {
        return Err(integrity("row segment has the wrong child kind"));
    }
    let outcome = load_child_outcome(store, segment.id)
        .await?
        .ok_or_else(|| integrity("row generation segment has no durable outcome"))?;
    if outcome.state != generation_supervisor_core::lifecycle::ChildOutcomeState::Succeeded
        || outcome.output_id != Some(row.generation_job_id)
    {
        return Err(integrity(
            "row generation segment does not bind its generation job",
        ));
    }
    let persisted = load_generated_row(store, row.generated_row_id)
        .await?
        .ok_or_else(|| integrity("observed generated row is not persisted"))?;
    if artifact_core::fingerprint(&persisted).map_err(fingerprint_error)?
        != row.generated_row_fingerprint
        || persisted.generation_job_id != row.generation_job_id
    {
        return Err(integrity("observed generated row fingerprint changed"));
    }
    let segment_plan = store
        .get_plan(persisted.plan_id)
        .await
        .map_err(generation_error)?
        .ok_or_else(|| integrity("row generation segment plan is missing"))?;
    let master_plan = store
        .get_plan(contract.plan.id)
        .await
        .map_err(generation_error)?
        .ok_or_else(|| integrity("supervisor master plan is missing"))?;
    if segment_plan.dataset_id != master_plan.dataset_id
        || segment_plan.cells.iter().any(|segment| {
            master_plan
                .cells
                .iter()
                .find(|planned| planned.cell == segment.cell)
                .is_none_or(|planned| segment.target_count > planned.target_count)
        })
    {
        return Err(integrity(
            "row generation segment plan exceeds the supervisor master plan",
        ));
    }
    let execution = store
        .get_generation_execution_spec(row.generation_job_id)
        .await
        .map_err(generation_error)?
        .ok_or_else(|| integrity("row generation execution specification is missing"))?;
    if execution.plan_id != segment_plan.id
        || execution
            .supervision_schedule_fingerprint
            .as_deref()
            .is_none()
    {
        return Err(integrity(
            "row generation execution is not pinned to a supervisor schedule",
        ));
    }
    let attempt = load_generation_attempt(store, row.generation_attempt_id)
        .await?
        .ok_or_else(|| integrity("row generation attempt is not persisted"))?;
    if attempt.job_id != row.generation_job_id
        || attempt.state != GenerationAttemptState::Succeeded
        || attempt.cell.key() != row.cell_key
    {
        return Err(integrity(
            "row observation generation attempt binding is invalid",
        ));
    }
    let prompt = load_prompt(store, row.prompt_version_id)
        .await?
        .ok_or_else(|| integrity("row prompt version is not persisted"))?;
    if prompt.supervisor_run_id != run_id || prompt.fingerprint != row.prompt_version_fingerprint {
        return Err(integrity("row prompt version binding mismatch"));
    }
    if let Some(evidence) = &row.assessment {
        let assessment = store
            .get_assessment(evidence.assessment_id)
            .await
            .map_err(|error| integrity(error.to_string()))?
            .ok_or_else(|| integrity("row quality assessment is not persisted"))?;
        if assessment.fingerprint != evidence.assessment_fingerprint
            || row.source_row_id != Some(assessment.source_row_id)
            || row.source_row_fingerprint.as_deref().is_none()
        {
            return Err(integrity(
                "row observation assessment provenance does not reproduce",
            ));
        }
        let audit = load_child_reservation_by_child(store, run_id, assessment.audit_run_id)
            .await?
            .ok_or_else(|| integrity("row quality audit is not a reserved supervisor child"))?;
        if audit.kind != generation_supervisor_core::lifecycle::ChildKind::QualityAudit {
            return Err(integrity("row assessment child has the wrong kind"));
        }
        let audit_outcome = load_child_outcome(store, audit.id)
            .await?
            .ok_or_else(|| integrity("row quality audit has no durable outcome"))?;
        if audit_outcome.state
            != generation_supervisor_core::lifecycle::ChildOutcomeState::Succeeded
            || audit_outcome.output_id != Some(assessment.audit_run_id)
        {
            return Err(integrity(
                "row assessment is not bound to a successful quality child",
            ));
        }
        let source = load_source_row(store, assessment.source_row_id)
            .await?
            .ok_or_else(|| integrity("quality assessment source row is missing"))?;
        let source_fingerprint = artifact_core::fingerprint(&source).map_err(fingerprint_error)?;
        if row.source_row_fingerprint.as_deref() != Some(source_fingerprint.as_str()) {
            return Err(integrity(
                "row observation source-row fingerprint does not reproduce",
            ));
        }
    } else if row.source_row_id.is_some() || row.source_row_fingerprint.is_some() {
        return Err(integrity(
            "unassessed row observation cannot bind orphaned source evidence",
        ));
    }
    Ok(())
}

fn validate_prompt_chain(versions: &[PromptGuidanceVersion]) -> Result<(), SupervisorError> {
    for (index, version) in versions.iter().enumerate() {
        version.validate()?;
        if version.sequence != u32::try_from(index).unwrap_or(u32::MAX)
            || (index == 0 && version.parent_version_id.is_some())
            || (index > 0 && version.parent_version_id != Some(versions[index - 1].id))
            || (index > 0
                && (version.base_prompt_fingerprint != versions[0].base_prompt_fingerprint
                    || version.protected_fields_fingerprint
                        != versions[0].protected_fields_fingerprint))
        {
            return Err(integrity("prompt guidance version chain is invalid"));
        }
    }
    Ok(())
}

async fn verify_all(store: &SqliteStore) -> Result<SupervisorIntegrityReport, SupervisorError> {
    let mut report = SupervisorIntegrityReport::default();
    let contract_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM generation_quality_contracts ORDER BY created_at, id")
            .fetch_all(store.pool())
            .await
            .map_err(sql_error)?;
    report.contracts = contract_ids.len() as u64;
    for id in contract_ids {
        if let Err(error) = async {
            let contract = load_contract(store, id)
                .await?
                .ok_or_else(|| integrity("contract disappeared"))?;
            let dataset = store
                .get_dataset(contract.dataset.id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("contract dataset is missing"))?;
            let plan = store
                .get_plan(contract.plan.id)
                .await
                .map_err(generation_error)?
                .ok_or_else(|| integrity("contract plan is missing"))?;
            if artifact_core::fingerprint(&dataset).map_err(fingerprint_error)?
                != contract.dataset.fingerprint
                || artifact_core::fingerprint(&plan).map_err(fingerprint_error)?
                    != contract.plan.fingerprint
            {
                return Err(integrity("contract source artifact changed"));
            }
            Ok::<_, SupervisorError>(())
        }
        .await
        {
            report.errors.push(format!("contract {id}: {error}"));
        }
    }

    let run_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM generation_supervisor_runs ORDER BY created_at, id")
            .fetch_all(store.pool())
            .await
            .map_err(sql_error)?;
    report.runs = run_ids.len() as u64;
    for id in run_ids {
        if let Err(error) = async {
            let run = load_run(store, id)
                .await?
                .ok_or_else(|| integrity("run disappeared"))?;
            let events =
                <SqliteStore as GenerationSupervisorStore>::list_run_events(store, id).await?;
            let projected = SupervisorRunEvent::verify_chain(id, &events)?;
            let persisted: String =
                sqlx::query_scalar("SELECT state FROM generation_supervisor_runs WHERE id = ?")
                    .bind(id)
                    .fetch_one(store.pool())
                    .await
                    .map_err(sql_error)?;
            if persisted != run_state(projected) {
                return Err(integrity("run state projection differs from event chain"));
            }
            let versions =
                <SqliteStore as GenerationSupervisorStore>::list_prompt_versions(store, id).await?;
            if versions
                .first()
                .map(|value| (value.id, value.fingerprint.as_str()))
                != Some((
                    run.initial_prompt_version_id,
                    run.initial_prompt_version_fingerprint.as_str(),
                ))
            {
                return Err(integrity("run initial prompt binding mismatch"));
            }
            if let Some(set) =
                <SqliteStore as GenerationSupervisorStore>::get_strategy_assignments(store, id)
                    .await?
            {
                report.strategy_assignments += set.assignments.len() as u64;
            }
            report.prompt_versions += versions.len() as u64;
            let rows = <SqliteStore as GenerationSupervisorStore>::list_row_observations(store, id)
                .await?;
            report.row_observations += rows.len() as u64;
            let windows =
                <SqliteStore as GenerationSupervisorStore>::list_quality_windows(store, id).await?;
            for window in &windows {
                load_window(store, window.id).await?;
            }
            report.quality_windows += windows.len() as u64;
            let decisions =
                <SqliteStore as GenerationSupervisorStore>::list_decisions(store, id).await?;
            report.decisions += decisions.len() as u64;
            Ok::<_, SupervisorError>(())
        }
        .await
        {
            report.errors.push(format!("run {id}: {error}"));
        }
    }

    let session_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM generation_supervisor_advisor_sessions ORDER BY created_at, id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(sql_error)?;
    report.advisor_sessions = session_ids.len() as u64;
    for id in session_ids {
        if let Err(error) = async {
            load_session(store, id)
                .await?
                .ok_or_else(|| integrity("advisor session disappeared"))?;
            <SqliteStore as SupervisorAdvisorStore>::list_model_calls(store, id).await?;
            <SqliteStore as SupervisorAdvisorStore>::list_tool_calls(store, id).await?;
            if let Some(proposal) =
                <SqliteStore as SupervisorAdvisorStore>::latest_proposal(store, id).await?
            {
                let session = load_session(store, id)
                    .await?
                    .ok_or_else(|| integrity("session missing"))?;
                let brief =
                    <SqliteStore as SupervisorAdvisorStore>::get_brief(store, session.brief_id)
                        .await?
                        .ok_or_else(|| integrity("brief missing"))?;
                proposal.validate(&brief.contract, &brief.current_prompt)?;
                report.revision_proposals += 1;
            }
            Ok::<_, SupervisorError>(())
        }
        .await
        {
            report.errors.push(format!("advisor session {id}: {error}"));
        }
    }
    let activation_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT prompt_version_id FROM generation_supervisor_revision_activations ORDER BY activated_at, id",
    )
    .fetch_all(store.pool())
    .await
    .map_err(sql_error)?;
    report.activations = activation_ids.len() as u64;
    for prompt_id in activation_ids {
        if let Err(error) =
            <SqliteStore as GenerationSupervisorStore>::get_revision_activation(store, prompt_id)
                .await
        {
            report
                .errors
                .push(format!("activation {prompt_id}: {error}"));
        }
    }
    Ok(report)
}

async fn load_json_optional<T: DeserializeOwned>(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Option<T>, SupervisorError> {
    let value: Option<String> = sqlx::query_scalar(query)
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(sql_error)?;
    value.map(|value| decode(&value)).transpose()
}

async fn load_json_many<T: DeserializeOwned>(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Vec<T>, SupervisorError> {
    sqlx::query_scalar::<_, String>(query)
        .bind(id)
        .fetch_all(store.pool())
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|value| decode(&value))
        .collect()
}

async fn load_json_required<T: DeserializeOwned>(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
    second: Option<Uuid>,
) -> Result<T, SupervisorError> {
    let value: Option<String> = match second {
        Some(second) => sqlx::query_scalar(query)
            .bind(id)
            .bind(second)
            .fetch_optional(store.pool())
            .await
            .map_err(sql_error)?,
        None => sqlx::query_scalar(query)
            .bind(id)
            .fetch_optional(store.pool())
            .await
            .map_err(sql_error)?,
    };
    value
        .ok_or_else(|| integrity("required supervisor artifact is missing"))
        .and_then(|value| decode(&value))
}

fn encode<T: Serialize>(value: &T) -> Result<String, SupervisorError> {
    serde_json::to_string(value).map_err(|error| validation(error.to_string()))
}

fn decode<T: DeserializeOwned>(value: &str) -> Result<T, SupervisorError> {
    serde_json::from_str(value).map_err(|error| integrity(error.to_string()))
}

fn scope_key(scope: &QualityScope) -> Result<String, SupervisorError> {
    encode(scope)
}

fn window_kind(value: generation_supervisor_core::observation::QualityWindowKind) -> &'static str {
    use generation_supervisor_core::observation::QualityWindowKind as Kind;
    match value {
        Kind::InitialCanary => "initial_canary",
        Kind::Rolling => "rolling",
        Kind::RevisionCanary => "revision_canary",
    }
}

fn run_state(value: SupervisorRunState) -> &'static str {
    match value {
        SupervisorRunState::Queued => "queued",
        SupervisorRunState::Running => "running",
        SupervisorRunState::Generating => "generating",
        SupervisorRunState::Assessing => "assessing",
        SupervisorRunState::Paused => "paused",
        SupervisorRunState::Diagnosing => "diagnosing",
        SupervisorRunState::AwaitingReview => "awaiting_review",
        SupervisorRunState::Canary => "canary",
        SupervisorRunState::Completed => "completed",
        SupervisorRunState::Failed => "failed",
        SupervisorRunState::Cancelled => "cancelled",
    }
}

fn child_kind(value: generation_supervisor_core::lifecycle::ChildKind) -> &'static str {
    use generation_supervisor_core::lifecycle::ChildKind as Kind;
    match value {
        Kind::GenerationSegment => "generation_segment",
        Kind::QualityAudit => "quality_audit",
        Kind::PiModelTurn => "pi_model_turn",
        Kind::PiToolCall => "pi_tool_call",
        Kind::RevisionCanary => "revision_canary",
    }
}

fn child_outcome_state(
    value: generation_supervisor_core::lifecycle::ChildOutcomeState,
) -> &'static str {
    use generation_supervisor_core::lifecycle::ChildOutcomeState as State;
    match value {
        State::Succeeded => "succeeded",
        State::Failed => "failed",
        State::Interrupted => "interrupted",
        State::Cancelled => "cancelled",
    }
}

fn decision_state(
    value: generation_supervisor_core::decision::SupervisorDecisionState,
) -> &'static str {
    use generation_supervisor_core::decision::SupervisorDecisionState as State;
    match value {
        State::Healthy => "healthy",
        State::InsufficientEvidence => "insufficient_evidence",
        State::PauseForDiagnosis => "pause_for_diagnosis",
        State::AwaitingReview => "awaiting_review",
        State::RevisionCanaryRequired => "revision_canary_required",
        State::RevisionPassed => "revision_passed",
        State::RevisionFailed => "revision_failed",
        State::Escalate => "escalate",
        State::Completed => "completed",
        State::Failed => "failed",
        State::Cancelled => "cancelled",
    }
}

fn review_decision(value: RevisionReviewDecision) -> &'static str {
    match value {
        RevisionReviewDecision::Approve => "approve",
        RevisionReviewDecision::Reject => "reject",
        RevisionReviewDecision::RequestRevision => "request_revision",
    }
}

fn require_one(rows: u64, context: &str) -> Result<(), SupervisorError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(integrity(format!("{context} affected {rows} rows")))
    }
}

fn generation_error(error: generation_core::ports::StoreError) -> SupervisorError {
    validation(error.to_string())
}

fn fingerprint_error(error: artifact_core::FingerprintError) -> SupervisorError {
    SupervisorError::Fingerprint(error.to_string())
}

fn sql_error(error: sqlx::Error) -> SupervisorError {
    validation(format!("SQLite supervisor adapter failed: {error}"))
}

fn validation(message: impl Into<String>) -> SupervisorError {
    SupervisorError::Validation(message.into())
}

fn integrity(message: impl Into<String>) -> SupervisorError {
    SupervisorError::Integrity(message.into())
}
