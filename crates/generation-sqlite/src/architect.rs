use dataset_architect_core::{
    ArchitectError,
    brief::ResolvedArchitectBrief,
    lifecycle::{
        ArchitectRun, ArchitectRunState, ArchitectToolCall, ArchitectToolCallState,
        ArchitectToolKind,
    },
    ports::{ArchitectAdapterError, ArchitectStore, BoxFuture},
    proposal::{
        ArchitectProposalReview, ArchitectReviewDecision, DatasetArchitectureApplication,
        DatasetArchitectureProposal,
    },
};
use generation_core::{domain::GenerationPlan, strategy::ResolvedGenerationStrategyContext};
use sqlx::Row;
use uuid::Uuid;

use crate::{SqliteStore, insert_plan};

impl ArchitectStore for SqliteStore {
    fn create_run(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let brief = brief.clone();
        let run = run.clone();
        Box::pin(async move {
            if brief.reproduce_fingerprint().map_err(domain_error)? != brief.fingerprint
                || run
                    .reproduce_specification_fingerprint()
                    .map_err(domain_error)?
                    != run.specification_fingerprint
                || run.brief_id != brief.id
                || run.brief_fingerprint != brief.fingerprint
            {
                return Err(adapter_error("architect run and brief integrity mismatch"));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let existing: Option<String> =
                sqlx::query_scalar("SELECT brief_json FROM dataset_architect_briefs WHERE id = ?")
                    .bind(brief.id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(sql_error)?;
            let brief_json = encode(&brief)?;
            match existing {
                Some(value) if value != brief_json => {
                    return Err(adapter_error("architect brief identity collision"));
                }
                Some(_) => {}
                None => {
                    sqlx::query("INSERT INTO dataset_architect_briefs (id, dataset_id, dataset_fingerprint, fingerprint, brief_json, resolved_at) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(brief.id).bind(brief.dataset.id).bind(&brief.dataset_fingerprint).bind(&brief.fingerprint)
                    .bind(&brief_json).bind(brief.created_at).execute(&mut *tx).await.map_err(sql_error)?;
                }
            }
            sqlx::query("INSERT INTO dataset_architect_runs (id, brief_id, brief_fingerprint, state, specification_fingerprint, run_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(run.id).bind(run.brief_id).bind(&run.brief_fingerprint).bind(run_state(run.state))
                .bind(&run.specification_fingerprint).bind(encode(&run)?).bind(run.created_at).bind(run.created_at)
                .execute(&mut *tx).await.map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedArchitectBrief>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(
                self,
                "SELECT brief_json FROM dataset_architect_briefs WHERE id = ?",
                id,
            )
            .await?
            .map(check_brief)
            .transpose()
        })
    }

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectRun>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(
                self,
                "SELECT run_json FROM dataset_architect_runs WHERE id = ?",
                id,
            )
            .await?
            .map(check_run)
            .transpose()
        })
    }

    fn save_run(&self, run: &ArchitectRun) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            if run
                .reproduce_specification_fingerprint()
                .map_err(domain_error)?
                != run.specification_fingerprint
            {
                return Err(adapter_error("architect run fingerprint mismatch"));
            }
            let updated = run.finished_at.or(run.started_at).unwrap_or(run.created_at);
            let result = sqlx::query("UPDATE dataset_architect_runs SET state = ?, run_json = ?, updated_at = ? WHERE id = ? AND specification_fingerprint = ? AND (state = ? OR (state = 'queued' AND ? IN ('running', 'cancelled')) OR (state = 'running' AND ? IN ('awaiting_review', 'failed', 'cancelled')))")
                .bind(run_state(run.state)).bind(encode(&run)?).bind(updated).bind(run.id)
                .bind(&run.specification_fingerprint).bind(run_state(run.state)).bind(run_state(run.state)).bind(run_state(run.state))
                .execute(self.pool()).await.map_err(sql_error)?;
            require_one(result.rows_affected(), "architect run")
        })
    }

    fn record_tool_call(
        &self,
        call: &ArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT call_json FROM dataset_architect_tool_calls WHERE id = ?",
            )
            .bind(call.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(sql_error)?;
            match existing {
                None => {
                    sqlx::query("INSERT INTO dataset_architect_tool_calls (id, run_id, sequence, kind, state, call_json, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                    .bind(call.id).bind(call.run_id).bind(i64::from(call.sequence)).bind(tool_kind(call.kind)).bind(tool_state(call.state))
                    .bind(encode(&call)?).bind(call.started_at).bind(call.finished_at).execute(&mut *tx).await.map_err(sql_error)?;
                }
                Some(value) => {
                    let previous: ArchitectToolCall = decode(&value)?;
                    if previous == call {
                        tx.commit().await.map_err(sql_error)?;
                        return Ok(());
                    }
                    if previous.state != ArchitectToolCallState::Started
                        || call.state == ArchitectToolCallState::Started
                        || previous.run_id != call.run_id
                        || previous.sequence != call.sequence
                        || previous.kind != call.kind
                        || previous.request != call.request
                        || previous.started_at != call.started_at
                    {
                        return Err(adapter_error(
                            "architect tool calls are append-only and only a started call may finish",
                        ));
                    }
                    let result = sqlx::query("UPDATE dataset_architect_tool_calls SET state = ?, call_json = ?, finished_at = ? WHERE id = ? AND state = 'started'")
                        .bind(tool_state(call.state)).bind(encode(&call)?).bind(call.finished_at).bind(call.id)
                        .execute(&mut *tx).await.map_err(sql_error)?;
                    require_one(result.rows_affected(), "started architect tool call")?;
                }
            }
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ArchitectToolCall>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_many(self, "SELECT call_json FROM dataset_architect_tool_calls WHERE run_id = ? ORDER BY sequence", run_id).await
        })
    }

    fn save_proposal_and_run(
        &self,
        proposal: &DatasetArchitectureProposal,
        run: &ArchitectRun,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let proposal = proposal.clone();
        let run = run.clone();
        Box::pin(async move {
            if proposal.reproduce_fingerprint().map_err(domain_error)? != proposal.fingerprint
                || run
                    .reproduce_specification_fingerprint()
                    .map_err(domain_error)?
                    != run.specification_fingerprint
                || proposal.run_id != run.id
                || proposal.run_fingerprint != run.specification_fingerprint
                || run.state != ArchitectRunState::AwaitingReview
            {
                return Err(adapter_error(
                    "architect proposal and terminal run integrity mismatch",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            sqlx::query("INSERT INTO dataset_architecture_proposals (id, run_id, brief_id, dataset_id, fingerprint, coverage_fingerprint, proposal_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(proposal.id).bind(proposal.run_id).bind(proposal.brief_id).bind(proposal.dataset_id)
                .bind(&proposal.fingerprint).bind(&proposal.coverage_fingerprint).bind(encode(&proposal)?).bind(proposal.created_at)
                .execute(&mut *tx).await.map_err(sql_error)?;
            let result = sqlx::query("UPDATE dataset_architect_runs SET state = ?, run_json = ?, updated_at = ? WHERE id = ? AND state = 'running' AND specification_fingerprint = ?")
                .bind(run_state(run.state)).bind(encode(&run)?).bind(run.finished_at).bind(run.id).bind(&run.specification_fingerprint)
                .execute(&mut *tx).await.map_err(sql_error)?;
            require_one(result.rows_affected(), "running architect run")?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(
                self,
                "SELECT proposal_json FROM dataset_architecture_proposals WHERE id = ?",
                id,
            )
            .await?
            .map(check_proposal)
            .transpose()
        })
    }

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureProposal>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(self, "SELECT proposal_json FROM dataset_architecture_proposals WHERE run_id = ? ORDER BY created_at DESC, id DESC LIMIT 1", run_id).await?.map(check_proposal).transpose()
        })
    }

    fn append_review(
        &self,
        review: &ArchitectProposalReview,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            if review.reproduce_fingerprint().map_err(domain_error)? != review.fingerprint {
                return Err(adapter_error("architect review fingerprint mismatch"));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let latest: Option<Uuid> = sqlx::query_scalar("SELECT id FROM dataset_architecture_reviews WHERE proposal_id = ? ORDER BY rowid DESC LIMIT 1")
                .bind(review.proposal_id).fetch_optional(&mut *tx).await.map_err(sql_error)?;
            if latest != review.predecessor_id {
                return Err(adapter_error("architect review predecessor is stale"));
            }
            sqlx::query("INSERT INTO dataset_architecture_reviews (id, proposal_id, predecessor_id, decision, fingerprint, review_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(review.id).bind(review.proposal_id).bind(review.predecessor_id).bind(review_decision(review.decision))
                .bind(&review.fingerprint).bind(encode(&review)?).bind(review.created_at).execute(&mut *tx).await.map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ArchitectProposalReview>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(self, "SELECT review_json FROM dataset_architecture_reviews WHERE proposal_id = ? ORDER BY rowid DESC LIMIT 1", proposal_id).await?.map(check_review).transpose()
        })
    }

    fn save_application(
        &self,
        application: &DatasetArchitectureApplication,
        plan: &GenerationPlan,
        strategy: &ResolvedGenerationStrategyContext,
    ) -> BoxFuture<'_, Result<(), ArchitectAdapterError>> {
        let application = application.clone();
        let plan = plan.clone();
        let strategy = strategy.clone();
        Box::pin(async move {
            if application.reproduce_fingerprint().map_err(domain_error)? != application.fingerprint
                || strategy
                    .reproduce_fingerprint()
                    .map_err(|e| adapter_error(e.to_string()))?
                    != strategy.fingerprint
                || application.plan_id != plan.id
                || application.plan_fingerprint
                    != artifact_core::fingerprint(&plan)
                        .map_err(|e| adapter_error(e.to_string()))?
                || application.strategy_context_id != strategy.id
                || application.strategy_context_fingerprint != strategy.fingerprint
                || strategy.plan_id != plan.id
                || strategy.plan_fingerprint != application.plan_fingerprint
            {
                return Err(adapter_error(
                    "architect application bundle integrity mismatch",
                ));
            }
            let mut tx = self.pool().begin().await.map_err(sql_error)?;
            let review_json: Option<String> = sqlx::query_scalar("SELECT review_json FROM dataset_architecture_reviews WHERE proposal_id = ? ORDER BY rowid DESC LIMIT 1")
                .bind(application.proposal_id).fetch_optional(&mut *tx).await.map_err(sql_error)?;
            let review = review_json
                .ok_or_else(|| adapter_error("architect proposal has no review"))
                .and_then(check_review)?;
            if review.id != application.approval_id
                || review.fingerprint != application.approval_fingerprint
                || review.decision != ArchitectReviewDecision::Approve
            {
                return Err(adapter_error(
                    "application requires the latest exact approval",
                ));
            }
            insert_plan(&mut tx, &plan)
                .await
                .map_err(|e| adapter_error(e.to_string()))?;
            sqlx::query("INSERT INTO generation_strategy_contexts (id, dataset_id, plan_id, proposal_id, approval_id, fingerprint, context_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(strategy.id).bind(strategy.dataset_id).bind(strategy.plan_id).bind(strategy.proposal_id).bind(strategy.approval_id)
                .bind(&strategy.fingerprint).bind(encode(&strategy)?).bind(strategy.created_at).execute(&mut *tx).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO dataset_architecture_applications (id, proposal_id, approval_id, plan_id, strategy_context_id, fingerprint, application_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(application.id).bind(application.proposal_id).bind(application.approval_id).bind(application.plan_id).bind(application.strategy_context_id)
                .bind(&application.fingerprint).bind(encode(&application)?).bind(application.created_at).execute(&mut *tx).await.map_err(sql_error)?;
            tx.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetArchitectureApplication>, ArchitectAdapterError>> {
        Box::pin(async move {
            load_one(self, "SELECT application_json FROM dataset_architecture_applications WHERE proposal_id = ?", proposal_id).await?.map(check_application).transpose()
        })
    }
}

impl SqliteStore {
    pub async fn generation_strategy_context_for_plan(
        &self,
        plan_id: Uuid,
    ) -> Result<Option<ResolvedGenerationStrategyContext>, generation_core::ports::StoreError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT context_json FROM generation_strategy_contexts WHERE plan_id = ?",
        )
        .bind(plan_id)
        .fetch_optional(self.pool())
        .await
        .map_err(crate::store_error)?;
        value
            .map(|v| {
                serde_json::from_str::<ResolvedGenerationStrategyContext>(&v)
                    .map_err(crate::store_error)
            })
            .transpose()?
            .map(|context| {
                if context
                    .reproduce_fingerprint()
                    .map_err(crate::store_error)?
                    != context.fingerprint
                {
                    return Err(generation_core::ports::StoreError(
                        "generation strategy context fingerprint mismatch".into(),
                    ));
                }
                Ok(context)
            })
            .transpose()
    }
}

async fn load_one(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Option<String>, ArchitectAdapterError> {
    sqlx::query_scalar(query)
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(sql_error)
}
async fn load_many<T: serde::de::DeserializeOwned>(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Vec<T>, ArchitectAdapterError> {
    sqlx::query(query)
        .bind(id)
        .fetch_all(store.pool())
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            row.try_get::<String, _>(0)
                .map_err(sql_error)
                .and_then(|v| decode(&v))
        })
        .collect()
}
fn check_brief(v: String) -> Result<ResolvedArchitectBrief, ArchitectAdapterError> {
    let a: ResolvedArchitectBrief = decode(&v)?;
    if a.reproduce_fingerprint().map_err(domain_error)? != a.fingerprint {
        Err(adapter_error("architect brief fingerprint mismatch"))
    } else {
        Ok(a)
    }
}
fn check_run(v: String) -> Result<ArchitectRun, ArchitectAdapterError> {
    let a: ArchitectRun = decode(&v)?;
    if a.reproduce_specification_fingerprint()
        .map_err(domain_error)?
        != a.specification_fingerprint
    {
        Err(adapter_error("architect run fingerprint mismatch"))
    } else {
        Ok(a)
    }
}
fn check_proposal(v: String) -> Result<DatasetArchitectureProposal, ArchitectAdapterError> {
    let a: DatasetArchitectureProposal = decode(&v)?;
    if a.reproduce_fingerprint().map_err(domain_error)? != a.fingerprint {
        Err(adapter_error("architect proposal fingerprint mismatch"))
    } else {
        Ok(a)
    }
}
fn check_review(v: String) -> Result<ArchitectProposalReview, ArchitectAdapterError> {
    let a: ArchitectProposalReview = decode(&v)?;
    if a.reproduce_fingerprint().map_err(domain_error)? != a.fingerprint {
        Err(adapter_error("architect review fingerprint mismatch"))
    } else {
        Ok(a)
    }
}
fn check_application(v: String) -> Result<DatasetArchitectureApplication, ArchitectAdapterError> {
    let a: DatasetArchitectureApplication = decode(&v)?;
    if a.reproduce_fingerprint().map_err(domain_error)? != a.fingerprint {
        Err(adapter_error("architect application fingerprint mismatch"))
    } else {
        Ok(a)
    }
}
fn encode<T: serde::Serialize>(v: &T) -> Result<String, ArchitectAdapterError> {
    serde_json::to_string(v).map_err(|e| adapter_error(e.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(v: &str) -> Result<T, ArchitectAdapterError> {
    serde_json::from_str(v).map_err(|e| adapter_error(e.to_string()))
}
const fn run_state(v: ArchitectRunState) -> &'static str {
    match v {
        ArchitectRunState::Queued => "queued",
        ArchitectRunState::Running => "running",
        ArchitectRunState::AwaitingReview => "awaiting_review",
        ArchitectRunState::Failed => "failed",
        ArchitectRunState::Cancelled => "cancelled",
    }
}
const fn tool_state(v: ArchitectToolCallState) -> &'static str {
    match v {
        ArchitectToolCallState::Started => "started",
        ArchitectToolCallState::Succeeded => "succeeded",
        ArchitectToolCallState::Failed => "failed",
        ArchitectToolCallState::Interrupted => "interrupted",
    }
}
const fn tool_kind(v: ArchitectToolKind) -> &'static str {
    match v {
        ArchitectToolKind::InspectDataset => "inspect_dataset",
        ArchitectToolKind::InspectSemantics => "inspect_semantics",
        ArchitectToolKind::InspectAuthenticity => "inspect_authenticity",
        ArchitectToolKind::InspectCoverage => "inspect_coverage",
        ArchitectToolKind::InspectDevelopmentEvidence => "inspect_development_evidence",
        ArchitectToolKind::PreviewAllocation => "preview_allocation",
        ArchitectToolKind::EstimateCost => "estimate_cost",
        ArchitectToolKind::SubmitProposal => "submit_proposal",
        ArchitectToolKind::FinishArchitecture => "finish_architecture",
    }
}
const fn review_decision(v: ArchitectReviewDecision) -> &'static str {
    match v {
        ArchitectReviewDecision::Approve => "approve",
        ArchitectReviewDecision::Reject => "reject",
        ArchitectReviewDecision::RequestRevision => "request_revision",
    }
}
fn require_one(rows: u64, name: &str) -> Result<(), ArchitectAdapterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(adapter_error(format!("{name} was not found or changed")))
    }
}
fn sql_error(e: sqlx::Error) -> ArchitectAdapterError {
    adapter_error(e.to_string())
}
fn domain_error(e: ArchitectError) -> ArchitectAdapterError {
    adapter_error(e.to_string())
}
fn adapter_error(v: impl Into<String>) -> ArchitectAdapterError {
    ArchitectAdapterError(v.into())
}
