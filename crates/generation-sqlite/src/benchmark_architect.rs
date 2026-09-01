use benchmark_architect_core::{
    BenchmarkArchitectError,
    blueprint::{
        BenchmarkAcquisitionHandoff, BenchmarkArchitectureProposal, BenchmarkArchitectureReview,
        BenchmarkArchitectureReviewDecision,
    },
    brief::ResolvedBenchmarkArchitectBrief,
    lifecycle::{
        BenchmarkArchitectRun, BenchmarkArchitectRunState, BenchmarkArchitectToolCall,
        BenchmarkArchitectToolCallState, BenchmarkArchitectToolKind,
    },
    ports::{BenchmarkArchitectAdapterError, BenchmarkArchitectStore, BoxFuture},
};
use research_core::evidence::ResearchEvidence;
use sqlx::Row;
use uuid::Uuid;

use crate::SqliteStore;

impl BenchmarkArchitectStore for SqliteStore {
    fn create_run(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let brief = brief.clone();
        let run = run.clone();
        Box::pin(async move {
            check_run_inputs(&brief, &run)?;
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let brief_json = encode(&brief)?;
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT brief_json FROM benchmark_architect_briefs WHERE id = ?",
            )
            .bind(brief.id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            match existing {
                Some(value) if value != brief_json => {
                    return Err(adapter_error(
                        "benchmark architect brief identity collision",
                    ));
                }
                Some(_) => {}
                None => {
                    sqlx::query(
                        "INSERT INTO benchmark_architect_briefs \
                         (id, schema_version, fingerprint, brief_json, created_at) \
                         VALUES (?, ?, ?, ?, ?)",
                    )
                    .bind(brief.id)
                    .bind(i64::from(brief.schema_version))
                    .bind(&brief.fingerprint)
                    .bind(brief_json)
                    .bind(brief.created_at)
                    .execute(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
                }
            }
            sqlx::query(
                "INSERT INTO benchmark_architect_runs \
                 (id, brief_id, brief_fingerprint, state, specification_fingerprint, run_json, \
                  created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(run.id)
            .bind(run.brief_id)
            .bind(&run.brief_fingerprint)
            .bind(run_state(run.state))
            .bind(&run.specification_fingerprint)
            .bind(encode(&run)?)
            .bind(run.created_at)
            .bind(run.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<
        '_,
        Result<Option<ResolvedBenchmarkArchitectBrief>, BenchmarkArchitectAdapterError>,
    > {
        Box::pin(async move { load_brief(self, id).await })
    }

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectRun>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            load_one(
                self,
                "SELECT run_json FROM benchmark_architect_runs WHERE id = ?",
                id,
            )
            .await?
            .map(check_run)
            .transpose()
        })
    }

    fn save_run(
        &self,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            check_run_value(&run)?;
            let updated_at = run.finished_at.or(run.started_at).unwrap_or(run.created_at);
            let result = sqlx::query(
                "UPDATE benchmark_architect_runs SET state = ?, run_json = ?, updated_at = ? \
                 WHERE id = ? AND specification_fingerprint = ? AND \
                 (state = ? OR (state = 'queued' AND ? IN ('running', 'cancelled')) OR \
                  (state = 'running' AND ? IN ('awaiting_review', 'failed', 'cancelled')))",
            )
            .bind(run_state(run.state))
            .bind(encode(&run)?)
            .bind(updated_at)
            .bind(run.id)
            .bind(&run.specification_fingerprint)
            .bind(run_state(run.state))
            .bind(run_state(run.state))
            .bind(run_state(run.state))
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            require_one(result.rows_affected(), "benchmark architect run")
        })
    }

    fn record_tool_call(
        &self,
        call: &BenchmarkArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT call_json FROM benchmark_architect_tool_calls WHERE id = ?",
            )
            .bind(call.id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            match existing {
                None => {
                    sqlx::query(
                        "INSERT INTO benchmark_architect_tool_calls \
                         (id, run_id, sequence, kind, state, call_json, started_at, finished_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(call.id)
                    .bind(call.run_id)
                    .bind(i64::from(call.sequence))
                    .bind(tool_kind(call.kind))
                    .bind(tool_state(call.state))
                    .bind(encode(&call)?)
                    .bind(call.started_at)
                    .bind(call.finished_at)
                    .execute(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
                }
                Some(value) => {
                    let previous: BenchmarkArchitectToolCall = decode(&value)?;
                    if previous == call {
                        transaction.commit().await.map_err(sql_error)?;
                        return Ok(());
                    }
                    if previous.state != BenchmarkArchitectToolCallState::Started
                        || call.state == BenchmarkArchitectToolCallState::Started
                        || previous.run_id != call.run_id
                        || previous.sequence != call.sequence
                        || previous.kind != call.kind
                        || previous.request != call.request
                        || previous.started_at != call.started_at
                    {
                        return Err(adapter_error(
                            "benchmark architect tool calls are append-only and only a started call may finish",
                        ));
                    }
                    let result = sqlx::query(
                        "UPDATE benchmark_architect_tool_calls SET state = ?, call_json = ?, \
                         finished_at = ? WHERE id = ? AND state = 'started'",
                    )
                    .bind(tool_state(call.state))
                    .bind(encode(&call)?)
                    .bind(call.finished_at)
                    .bind(call.id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
                    require_one(
                        result.rows_affected(),
                        "started benchmark architect tool call",
                    )?;
                }
            }
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkArchitectToolCall>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            load_many(
                self,
                "SELECT call_json FROM benchmark_architect_tool_calls \
                 WHERE run_id = ? ORDER BY sequence",
                run_id,
            )
            .await
        })
    }

    fn record_evidence(
        &self,
        evidence: &ResearchEvidence,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let evidence = evidence.clone();
        Box::pin(async move {
            check_evidence(&evidence)?;
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let call_json: Option<String> = sqlx::query_scalar(
                "SELECT call_json FROM benchmark_architect_tool_calls WHERE id = ?",
            )
            .bind(evidence.tool_call_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            let call: BenchmarkArchitectToolCall = call_json
                .ok_or_else(|| adapter_error("benchmark architect evidence tool call not found"))
                .and_then(|value| decode(&value))?;
            if call.run_id != evidence.run_id
                || call.kind != BenchmarkArchitectToolKind::RecordEvidence
                || call.state != BenchmarkArchitectToolCallState::Started
            {
                return Err(adapter_error(
                    "benchmark architect evidence must belong to its active record-evidence call",
                ));
            }
            sqlx::query(
                "INSERT INTO benchmark_architect_evidence \
                 (id, run_id, tool_call_id, canonical_url, content_hash, source_class, \
                  fingerprint, evidence_json, retrieved_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(evidence.id)
            .bind(evidence.run_id)
            .bind(evidence.tool_call_id)
            .bind(&evidence.canonical_url)
            .bind(&evidence.content_hash)
            .bind(&evidence.source_class)
            .bind(&evidence.fingerprint)
            .bind(encode(&evidence)?)
            .bind(evidence.retrieved_at)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_evidence(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchEvidence>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move { load_evidence(self, run_id).await })
    }

    fn get_evidence(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResearchEvidence>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            let run_id: Option<Uuid> =
                sqlx::query_scalar("SELECT run_id FROM benchmark_architect_evidence WHERE id = ?")
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(sql_error)?;
            match run_id {
                Some(run_id) => Ok(load_evidence(self, run_id)
                    .await?
                    .into_iter()
                    .find(|value| value.id == id)),
                None => Ok(None),
            }
        })
    }

    fn save_proposal_and_run(
        &self,
        proposal: &BenchmarkArchitectureProposal,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let proposal = proposal.clone();
        let run = run.clone();
        Box::pin(async move {
            check_run_value(&run)?;
            let brief = load_brief(self, proposal.brief_id)
                .await?
                .ok_or_else(|| adapter_error("benchmark architect proposal brief not found"))?;
            let evidence = load_evidence(self, proposal.run_id).await?;
            proposal
                .validate_integrity(&brief, &evidence)
                .map_err(domain_error)?;
            if proposal.run_id != run.id
                || proposal.run_fingerprint != run.specification_fingerprint
                || run.state != BenchmarkArchitectRunState::AwaitingReview
            {
                return Err(adapter_error(
                    "benchmark architecture proposal and terminal run do not belong together",
                ));
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            sqlx::query(
                "INSERT INTO benchmark_architecture_proposals \
                 (id, run_id, brief_id, fingerprint, validation_fingerprint, proposal_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(proposal.id)
            .bind(proposal.run_id)
            .bind(proposal.brief_id)
            .bind(&proposal.fingerprint)
            .bind(&proposal.validation.fingerprint)
            .bind(encode(&proposal)?)
            .bind(proposal.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            let result = sqlx::query(
                "UPDATE benchmark_architect_runs SET state = ?, run_json = ?, updated_at = ? \
                 WHERE id = ? AND state = 'running' AND specification_fingerprint = ?",
            )
            .bind(run_state(run.state))
            .bind(encode(&run)?)
            .bind(run.finished_at)
            .bind(run.id)
            .bind(&run.specification_fingerprint)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            require_one(result.rows_affected(), "running benchmark architect run")?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move { load_proposal(self, id).await })
    }

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM benchmark_architecture_proposals WHERE run_id = ?",
            )
            .bind(run_id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)?;
            match id {
                Some(id) => load_proposal(self, id).await,
                None => Ok(None),
            }
        })
    }

    fn append_review(
        &self,
        review: &BenchmarkArchitectureReview,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            check_review(&review)?;
            let proposal = load_proposal(self, review.proposal_id)
                .await?
                .ok_or_else(|| adapter_error("benchmark architecture proposal not found"))?;
            if review.proposal_fingerprint != proposal.fingerprint {
                return Err(adapter_error(
                    "benchmark architecture review proposal pin mismatch",
                ));
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let handed_off: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM benchmark_acquisition_handoffs WHERE proposal_id = ?)",
            )
            .bind(review.proposal_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(sql_error)?;
            if handed_off {
                return Err(adapter_error(
                    "benchmark architecture review chain is closed by its acquisition handoff",
                ));
            }
            let latest_json: Option<String> = sqlx::query_scalar(
                "SELECT review_json FROM benchmark_architecture_reviews \
                 WHERE proposal_id = ? ORDER BY rowid DESC LIMIT 1",
            )
            .bind(review.proposal_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            let latest = latest_json
                .map(|value| decode::<BenchmarkArchitectureReview>(&value))
                .transpose()?;
            if review.predecessor_id != latest.as_ref().map(|value| value.id)
                || review.predecessor_fingerprint
                    != latest.as_ref().map(|value| value.fingerprint.clone())
            {
                return Err(adapter_error(
                    "benchmark architecture review predecessor is stale",
                ));
            }
            sqlx::query(
                "INSERT INTO benchmark_architecture_reviews \
                 (id, proposal_id, predecessor_id, predecessor_fingerprint, decision, fingerprint, \
                  review_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.proposal_id)
            .bind(review.predecessor_id)
            .bind(&review.predecessor_fingerprint)
            .bind(review_decision(review.decision))
            .bind(&review.fingerprint)
            .bind(encode(&review)?)
            .bind(review.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move { load_latest_review(self, proposal_id).await })
    }

    fn get_review(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            let proposal_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT proposal_id FROM benchmark_architecture_reviews WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)?;
            let Some(proposal_id) = proposal_id else {
                return Ok(None);
            };
            let values: Vec<BenchmarkArchitectureReview> = load_many(
                self,
                "SELECT review_json FROM benchmark_architecture_reviews \
                 WHERE proposal_id = ? ORDER BY rowid",
                proposal_id,
            )
            .await?;
            load_latest_review(self, proposal_id).await?;
            Ok(values.into_iter().find(|value| value.id == id))
        })
    }

    fn save_handoff(
        &self,
        handoff: &BenchmarkAcquisitionHandoff,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let handoff = handoff.clone();
        Box::pin(async move {
            let proposal = load_proposal(self, handoff.proposal_id)
                .await?
                .ok_or_else(|| adapter_error("benchmark acquisition proposal not found"))?;
            let brief = load_brief(self, proposal.brief_id)
                .await?
                .ok_or_else(|| adapter_error("benchmark acquisition brief not found"))?;
            let evidence = load_evidence(self, proposal.run_id).await?;
            let approval = load_latest_review(self, proposal.id)
                .await?
                .ok_or_else(|| adapter_error("benchmark acquisition approval not found"))?;
            handoff
                .validate_integrity(&brief, &proposal, &approval, &evidence)
                .map_err(domain_error)?;
            sqlx::query(
                "INSERT INTO benchmark_acquisition_handoffs \
                 (id, proposal_id, approval_id, fingerprint, handoff_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(handoff.id)
            .bind(handoff.proposal_id)
            .bind(handoff.approval_id)
            .bind(&handoff.fingerprint)
            .bind(encode(&handoff)?)
            .bind(handoff.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_handoff(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move { load_handoff(self, "proposal_id", proposal_id).await })
    }

    fn get_handoff_by_id(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move { load_handoff(self, "id", id).await })
    }
}

async fn load_brief(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<ResolvedBenchmarkArchitectBrief>, BenchmarkArchitectAdapterError> {
    load_one(
        store,
        "SELECT brief_json FROM benchmark_architect_briefs WHERE id = ?",
        id,
    )
    .await?
    .map(check_brief)
    .transpose()
}

async fn load_evidence(
    store: &SqliteStore,
    run_id: Uuid,
) -> Result<Vec<ResearchEvidence>, BenchmarkArchitectAdapterError> {
    let values: Vec<ResearchEvidence> = load_many(
        store,
        "SELECT evidence_json FROM benchmark_architect_evidence \
         WHERE run_id = ? ORDER BY retrieved_at, id",
        run_id,
    )
    .await?;
    for value in &values {
        check_evidence(value)?;
        if value.run_id != run_id {
            return Err(adapter_error("benchmark architect evidence run mismatch"));
        }
    }
    Ok(values)
}

async fn load_proposal(
    store: &SqliteStore,
    id: Uuid,
) -> Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError> {
    let Some(json) = load_one(
        store,
        "SELECT proposal_json FROM benchmark_architecture_proposals WHERE id = ?",
        id,
    )
    .await?
    else {
        return Ok(None);
    };
    let proposal: BenchmarkArchitectureProposal = decode(&json)?;
    let brief = load_brief(store, proposal.brief_id)
        .await?
        .ok_or_else(|| adapter_error("benchmark architecture proposal brief not found"))?;
    let evidence = load_evidence(store, proposal.run_id).await?;
    proposal
        .validate_integrity(&brief, &evidence)
        .map_err(domain_error)?;
    Ok(Some(proposal))
}

async fn load_latest_review(
    store: &SqliteStore,
    proposal_id: Uuid,
) -> Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError> {
    let proposal = load_proposal(store, proposal_id).await?;
    let values: Vec<BenchmarkArchitectureReview> = load_many(
        store,
        "SELECT review_json FROM benchmark_architecture_reviews \
         WHERE proposal_id = ? ORDER BY rowid",
        proposal_id,
    )
    .await?;
    let mut previous: Option<&BenchmarkArchitectureReview> = None;
    for value in &values {
        check_review(value)?;
        let proposal = proposal
            .as_ref()
            .ok_or_else(|| adapter_error("benchmark architecture review proposal not found"))?;
        if value.proposal_id != proposal.id
            || value.proposal_fingerprint != proposal.fingerprint
            || value.predecessor_id != previous.map(|item| item.id)
            || value.predecessor_fingerprint != previous.map(|item| item.fingerprint.clone())
        {
            return Err(adapter_error(
                "benchmark architecture review chain is invalid",
            ));
        }
        previous = Some(value);
    }
    Ok(values.last().cloned())
}

async fn load_handoff(
    store: &SqliteStore,
    column: &'static str,
    id: Uuid,
) -> Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError> {
    let query =
        format!("SELECT handoff_json FROM benchmark_acquisition_handoffs WHERE {column} = ?");
    let Some(json): Option<String> = sqlx::query_scalar(&query)
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(sql_error)?
    else {
        return Ok(None);
    };
    let handoff: BenchmarkAcquisitionHandoff = decode(&json)?;
    let proposal = load_proposal(store, handoff.proposal_id)
        .await?
        .ok_or_else(|| adapter_error("benchmark acquisition proposal not found"))?;
    let brief = load_brief(store, proposal.brief_id)
        .await?
        .ok_or_else(|| adapter_error("benchmark acquisition brief not found"))?;
    let evidence = load_evidence(store, proposal.run_id).await?;
    let approval = load_latest_review(store, proposal.id)
        .await?
        .ok_or_else(|| adapter_error("benchmark acquisition approval not found"))?;
    handoff
        .validate_integrity(&brief, &proposal, &approval, &evidence)
        .map_err(domain_error)?;
    Ok(Some(handoff))
}

async fn load_one(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Option<String>, BenchmarkArchitectAdapterError> {
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
) -> Result<Vec<T>, BenchmarkArchitectAdapterError> {
    sqlx::query(query)
        .bind(id)
        .fetch_all(store.pool())
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            row.try_get::<String, _>(0)
                .map_err(sql_error)
                .and_then(|value| decode(&value))
        })
        .collect()
}

fn check_run_inputs(
    brief: &ResolvedBenchmarkArchitectBrief,
    run: &BenchmarkArchitectRun,
) -> Result<(), BenchmarkArchitectAdapterError> {
    check_brief(encode(brief)?)?;
    check_run_value(run)?;
    if run.brief_id != brief.id || run.brief_fingerprint != brief.fingerprint {
        return Err(adapter_error(
            "benchmark architect run and brief integrity mismatch",
        ));
    }
    Ok(())
}

fn check_brief(
    json: String,
) -> Result<ResolvedBenchmarkArchitectBrief, BenchmarkArchitectAdapterError> {
    let value: ResolvedBenchmarkArchitectBrief = decode(&json)?;
    if value.reproduce_fingerprint().map_err(domain_error)? != value.fingerprint {
        Err(adapter_error(
            "benchmark architect brief fingerprint mismatch",
        ))
    } else {
        Ok(value)
    }
}

fn check_run(json: String) -> Result<BenchmarkArchitectRun, BenchmarkArchitectAdapterError> {
    let value: BenchmarkArchitectRun = decode(&json)?;
    check_run_value(&value)?;
    Ok(value)
}

fn check_run_value(value: &BenchmarkArchitectRun) -> Result<(), BenchmarkArchitectAdapterError> {
    if value
        .reproduce_specification_fingerprint()
        .map_err(domain_error)?
        != value.specification_fingerprint
    {
        Err(adapter_error(
            "benchmark architect run fingerprint mismatch",
        ))
    } else {
        Ok(())
    }
}

fn check_evidence(value: &ResearchEvidence) -> Result<(), BenchmarkArchitectAdapterError> {
    if value
        .reproduce_fingerprint()
        .map_err(|error| adapter_error(error.to_string()))?
        != value.fingerprint
    {
        Err(adapter_error(
            "benchmark architect evidence fingerprint mismatch",
        ))
    } else {
        Ok(())
    }
}

fn check_review(value: &BenchmarkArchitectureReview) -> Result<(), BenchmarkArchitectAdapterError> {
    if value.reproduce_fingerprint().map_err(domain_error)? != value.fingerprint {
        Err(adapter_error(
            "benchmark architecture review fingerprint mismatch",
        ))
    } else {
        Ok(())
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String, BenchmarkArchitectAdapterError> {
    serde_json::to_string(value).map_err(|error| adapter_error(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, BenchmarkArchitectAdapterError> {
    serde_json::from_str(value).map_err(|error| adapter_error(error.to_string()))
}

const fn run_state(value: BenchmarkArchitectRunState) -> &'static str {
    match value {
        BenchmarkArchitectRunState::Queued => "queued",
        BenchmarkArchitectRunState::Running => "running",
        BenchmarkArchitectRunState::AwaitingReview => "awaiting_review",
        BenchmarkArchitectRunState::Failed => "failed",
        BenchmarkArchitectRunState::Cancelled => "cancelled",
    }
}

const fn tool_state(value: BenchmarkArchitectToolCallState) -> &'static str {
    match value {
        BenchmarkArchitectToolCallState::Started => "started",
        BenchmarkArchitectToolCallState::Succeeded => "succeeded",
        BenchmarkArchitectToolCallState::Failed => "failed",
        BenchmarkArchitectToolCallState::Interrupted => "interrupted",
    }
}

const fn tool_kind(value: BenchmarkArchitectToolKind) -> &'static str {
    match value {
        BenchmarkArchitectToolKind::InspectBrief => "inspect_brief",
        BenchmarkArchitectToolKind::InspectExistingBenchmark => "inspect_existing_benchmark",
        BenchmarkArchitectToolKind::InspectExposureHistory => "inspect_exposure_history",
        BenchmarkArchitectToolKind::SearchWeb => "search_web",
        BenchmarkArchitectToolKind::FetchPage => "fetch_page",
        BenchmarkArchitectToolKind::RecordEvidence => "record_evidence",
        BenchmarkArchitectToolKind::InspectEvidence => "inspect_evidence",
        BenchmarkArchitectToolKind::PreviewBlueprint => "preview_blueprint",
        BenchmarkArchitectToolKind::SubmitProposal => "submit_proposal",
        BenchmarkArchitectToolKind::FinishArchitecture => "finish_architecture",
    }
}

const fn review_decision(value: BenchmarkArchitectureReviewDecision) -> &'static str {
    match value {
        BenchmarkArchitectureReviewDecision::Approve => "approve",
        BenchmarkArchitectureReviewDecision::Reject => "reject",
        BenchmarkArchitectureReviewDecision::RequestRevision => "request_revision",
    }
}

fn require_one(rows: u64, name: &str) -> Result<(), BenchmarkArchitectAdapterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(adapter_error(format!("{name} was not found or changed")))
    }
}

fn sql_error(error: sqlx::Error) -> BenchmarkArchitectAdapterError {
    adapter_error(error.to_string())
}

fn domain_error(error: BenchmarkArchitectError) -> BenchmarkArchitectAdapterError {
    adapter_error(error.to_string())
}

fn adapter_error(value: impl Into<String>) -> BenchmarkArchitectAdapterError {
    BenchmarkArchitectAdapterError(value.into())
}
