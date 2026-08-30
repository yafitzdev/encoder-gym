use research_core::{
    brief::ResolvedResearchBrief,
    evidence::{ResearchClaim, ResearchEvidence},
    lifecycle::{ResearchRun, ResearchRunState, ResearchToolCall, ToolCallState},
    ports::{BoxFuture, ResearchAdapterError, ResearchStore},
    profile::{
        AuthenticityProfile, GenerationAuthenticityAssignment, ProfileBinding, ProfileReview,
        ProfileReviewDecision, ResolvedAuthenticityContext,
    },
};
use sqlx::Row;
use uuid::Uuid;

use crate::SqliteStore;

impl ResearchStore for SqliteStore {
    fn create_run(
        &self,
        brief: &ResolvedResearchBrief,
        run: &ResearchRun,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
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
                return Err(adapter_error("run and brief integrity mismatch"));
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let existing: Option<String> =
                sqlx::query_scalar("SELECT brief_json FROM research_briefs WHERE id = ?")
                    .bind(brief.id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
            let brief_json = encode(&brief)?;
            match existing {
                Some(existing) if existing != brief_json => {
                    return Err(adapter_error("research brief identity collision"));
                }
                Some(_) => {}
                None => {
                    sqlx::query(
                        "INSERT INTO research_briefs (id, dataset_id, dataset_fingerprint, \
                         fingerprint, brief_json, resolved_at) VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind(brief.id)
                    .bind(brief.dataset.id)
                    .bind(&brief.dataset.fingerprint)
                    .bind(&brief.fingerprint)
                    .bind(&brief_json)
                    .bind(brief.resolved_at)
                    .execute(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
                }
            }
            sqlx::query(
                "INSERT INTO research_runs (id, brief_id, brief_fingerprint, state, \
                 specification_fingerprint, run_json, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
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
    ) -> BoxFuture<'_, Result<Option<ResolvedResearchBrief>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> =
                sqlx::query_scalar("SELECT brief_json FROM research_briefs WHERE id = ?")
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(sql_error)?;
            value.map(|value| decode_checked_brief(&value)).transpose()
        })
    }

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResearchRun>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> =
                sqlx::query_scalar("SELECT run_json FROM research_runs WHERE id = ?")
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(sql_error)?;
            value.map(|value| decode_checked_run(&value)).transpose()
        })
    }

    fn save_run(&self, run: &ResearchRun) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            if run
                .reproduce_specification_fingerprint()
                .map_err(domain_error)?
                != run.specification_fingerprint
            {
                return Err(adapter_error("research run fingerprint mismatch"));
            }
            let updated_at = run.finished_at.or(run.started_at).unwrap_or(run.created_at);
            let result = sqlx::query(
                "UPDATE research_runs SET state = ?, run_json = ?, updated_at = ? WHERE id = ? \
                 AND specification_fingerprint = ? AND (state = ? \
                 OR (state = 'queued' AND ? IN ('running', 'cancelled')) \
                 OR (state = 'running' AND ? IN ('awaiting_review', 'failed', 'cancelled')))",
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
            require_one(result.rows_affected(), "research run")
        })
    }

    fn record_tool_call(
        &self,
        call: &ResearchToolCall,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let existing: Option<String> =
                sqlx::query_scalar("SELECT call_json FROM research_tool_calls WHERE id = ?")
                    .bind(call.id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
            match existing {
                None => {
                    sqlx::query(
                        "INSERT INTO research_tool_calls (id, run_id, sequence, kind, state, \
                         call_json, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
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
                Some(existing) => {
                    let previous: ResearchToolCall = decode(&existing)?;
                    if previous == call {
                        transaction.commit().await.map_err(sql_error)?;
                        return Ok(());
                    }
                    if previous.state != ToolCallState::Started
                        || call.state == ToolCallState::Started
                        || previous.run_id != call.run_id
                        || previous.sequence != call.sequence
                        || previous.kind != call.kind
                        || previous.request != call.request
                        || previous.started_at != call.started_at
                    {
                        return Err(adapter_error(
                            "tool calls are append-only and only a started call may finish",
                        ));
                    }
                    let result = sqlx::query(
                        "UPDATE research_tool_calls SET state = ?, call_json = ?, finished_at = ? \
                         WHERE id = ? AND state = 'started'",
                    )
                    .bind(tool_state(call.state))
                    .bind(encode(&call)?)
                    .bind(call.finished_at)
                    .bind(call.id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(sql_error)?;
                    require_one(result.rows_affected(), "started research tool call")?;
                }
            }
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchToolCall>, ResearchAdapterError>> {
        Box::pin(async move {
            load_json_list(
                self,
                "SELECT call_json FROM research_tool_calls WHERE run_id = ? ORDER BY sequence",
                run_id,
            )
            .await
        })
    }

    fn record_evidence(
        &self,
        evidence: &ResearchEvidence,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let evidence = evidence.clone();
        Box::pin(async move {
            if evidence.reproduce_fingerprint().map_err(domain_error)? != evidence.fingerprint {
                return Err(adapter_error("research evidence fingerprint mismatch"));
            }
            sqlx::query(
                "INSERT INTO research_evidence (id, run_id, tool_call_id, canonical_url, \
                 content_hash, source_class, fingerprint, evidence_json, retrieved_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_evidence(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchEvidence>, ResearchAdapterError>> {
        Box::pin(async move {
            let items: Vec<ResearchEvidence> = load_json_list(
                self,
                "SELECT evidence_json FROM research_evidence WHERE run_id = ? ORDER BY retrieved_at, id",
                run_id,
            )
            .await?;
            for item in &items {
                if item.reproduce_fingerprint().map_err(domain_error)? != item.fingerprint {
                    return Err(adapter_error("research evidence fingerprint mismatch"));
                }
            }
            Ok(items)
        })
    }

    fn record_claim(
        &self,
        claim: &ResearchClaim,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let claim = claim.clone();
        Box::pin(async move {
            if claim.reproduce_fingerprint().map_err(domain_error)? != claim.fingerprint {
                return Err(adapter_error("research claim fingerprint mismatch"));
            }
            sqlx::query(
                "INSERT INTO research_claims (id, run_id, fingerprint, claim_json, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(claim.id)
            .bind(claim.run_id)
            .bind(&claim.fingerprint)
            .bind(encode(&claim)?)
            .bind(claim.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_claims(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchClaim>, ResearchAdapterError>> {
        Box::pin(async move {
            let items: Vec<ResearchClaim> = load_json_list(
                self,
                "SELECT claim_json FROM research_claims WHERE run_id = ? ORDER BY created_at, id",
                run_id,
            )
            .await?;
            for item in &items {
                if item.reproduce_fingerprint().map_err(domain_error)? != item.fingerprint {
                    return Err(adapter_error("research claim fingerprint mismatch"));
                }
            }
            Ok(items)
        })
    }

    fn save_profile(
        &self,
        profile: &AuthenticityProfile,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let profile = profile.clone();
        Box::pin(async move {
            if profile.reproduce_fingerprint().map_err(domain_error)? != profile.fingerprint {
                return Err(adapter_error("authenticity profile fingerprint mismatch"));
            }
            sqlx::query(
                "INSERT INTO authenticity_profiles (id, run_id, dataset_id, version, \
                 predecessor_id, fingerprint, profile_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(profile.id)
            .bind(profile.run_id)
            .bind(profile.dataset_id)
            .bind(i64::from(profile.version))
            .bind(profile.predecessor_id)
            .bind(&profile.fingerprint)
            .bind(encode(&profile)?)
            .bind(profile.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_profile(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuthenticityProfile>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> =
                sqlx::query_scalar("SELECT profile_json FROM authenticity_profiles WHERE id = ?")
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(sql_error)?;
            value
                .map(|value| decode_checked_profile(&value))
                .transpose()
        })
    }

    fn append_review(
        &self,
        review: &ProfileReview,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            if review.reproduce_fingerprint().map_err(domain_error)? != review.fingerprint {
                return Err(adapter_error("profile review fingerprint mismatch"));
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let current: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM authenticity_profile_reviews WHERE profile_id = ? \
                 ORDER BY rowid DESC LIMIT 1",
            )
            .bind(review.profile_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            if current != review.predecessor_id {
                return Err(adapter_error(
                    "review predecessor is stale; reviews are an append-only chain",
                ));
            }
            sqlx::query(
                "INSERT INTO authenticity_profile_reviews (id, profile_id, predecessor_id, \
                 decision, fingerprint, review_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.profile_id)
            .bind(review.predecessor_id)
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
        profile_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProfileReview>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> = sqlx::query_scalar(
                "SELECT review_json FROM authenticity_profile_reviews WHERE profile_id = ? \
                 ORDER BY rowid DESC LIMIT 1",
            )
            .bind(profile_id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)?;
            value.map(|value| decode_checked_review(&value)).transpose()
        })
    }

    fn append_binding(
        &self,
        binding: &ProfileBinding,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let binding = binding.clone();
        Box::pin(async move {
            if binding.reproduce_fingerprint().map_err(domain_error)? != binding.fingerprint {
                return Err(adapter_error("profile binding fingerprint mismatch"));
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let latest_review = latest_review_in(&mut transaction, binding.profile_id).await?;
            let Some(latest_review) = latest_review else {
                return Err(adapter_error("profile has no review"));
            };
            if latest_review.id != binding.approval_id
                || latest_review.decision != ProfileReviewDecision::Approve
                || latest_review.fingerprint != binding.approval_fingerprint
            {
                return Err(adapter_error(
                    "binding requires the latest approval for the exact profile",
                ));
            }
            let current: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM authenticity_profile_bindings WHERE dataset_id = ? \
                 ORDER BY rowid DESC LIMIT 1",
            )
            .bind(binding.dataset_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(sql_error)?;
            if current != binding.predecessor_id {
                return Err(adapter_error(
                    "binding predecessor is stale; bindings are an append-only chain",
                ));
            }
            sqlx::query(
                "INSERT INTO authenticity_profile_bindings (id, dataset_id, profile_id, \
                 approval_id, predecessor_id, fingerprint, binding_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(binding.id)
            .bind(binding.dataset_id)
            .bind(binding.profile_id)
            .bind(binding.approval_id)
            .bind(binding.predecessor_id)
            .bind(&binding.fingerprint)
            .bind(encode(&binding)?)
            .bind(binding.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn resolve_context(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResolvedAuthenticityContext>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> = sqlx::query_scalar(
                "SELECT binding_json FROM authenticity_profile_bindings WHERE dataset_id = ? \
                 ORDER BY rowid DESC LIMIT 1",
            )
            .bind(dataset_id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)?;
            let Some(value) = value else { return Ok(None) };
            let binding: ProfileBinding = decode(&value)?;
            let profile = self
                .get_profile(binding.profile_id)
                .await?
                .ok_or_else(|| adapter_error("bound authenticity profile is missing"))?;
            let review = self
                .latest_review(binding.profile_id)
                .await?
                .ok_or_else(|| adapter_error("bound profile approval is missing"))?;
            ResolvedAuthenticityContext::resolve(&binding, &profile, &review)
                .map(Some)
                .map_err(domain_error)
        })
    }

    fn save_generation_authenticity(
        &self,
        assignment: &GenerationAuthenticityAssignment,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let assignment = assignment.clone();
        Box::pin(async move {
            validate_generation_assignment(&assignment)?;
            sqlx::query(
                "INSERT INTO generation_job_authenticity (job_id, dataset_id, profile_id, \
                 binding_id, context_fingerprint, assignment_fingerprint, assignment_json, \
                 created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(assignment.job_id)
            .bind(assignment.context.dataset_id)
            .bind(assignment.context.profile_id)
            .bind(assignment.context.binding_id)
            .bind(&assignment.context.fingerprint)
            .bind(&assignment.fingerprint)
            .bind(encode(&assignment)?)
            .bind(assignment.created_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_generation_authenticity(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationAuthenticityAssignment>, ResearchAdapterError>> {
        Box::pin(async move {
            let value: Option<String> = sqlx::query_scalar(
                "SELECT assignment_json FROM generation_job_authenticity WHERE job_id = ?",
            )
            .bind(job_id)
            .fetch_optional(self.pool())
            .await
            .map_err(sql_error)?;
            value
                .map(|value| {
                    let assignment: GenerationAuthenticityAssignment = decode(&value)?;
                    validate_generation_assignment(&assignment)?;
                    Ok(assignment)
                })
                .transpose()
        })
    }
}

fn validate_generation_assignment(
    assignment: &GenerationAuthenticityAssignment,
) -> Result<(), ResearchAdapterError> {
    if assignment
        .context
        .reproduce_fingerprint()
        .map_err(domain_error)?
        != assignment.context.fingerprint
        || assignment.reproduce_fingerprint().map_err(domain_error)? != assignment.fingerprint
    {
        return Err(adapter_error(
            "generation authenticity assignment fingerprint mismatch",
        ));
    }
    Ok(())
}

async fn latest_review_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    profile_id: Uuid,
) -> Result<Option<ProfileReview>, ResearchAdapterError> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT review_json FROM authenticity_profile_reviews WHERE profile_id = ? \
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(profile_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(sql_error)?;
    value.map(|value| decode_checked_review(&value)).transpose()
}

async fn load_json_list<T: serde::de::DeserializeOwned>(
    store: &SqliteStore,
    query: &str,
    id: Uuid,
) -> Result<Vec<T>, ResearchAdapterError> {
    sqlx::query(query)
        .bind(id)
        .fetch_all(store.pool())
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| row.try_get::<String, _>(0).map_err(sql_error))
        .map(|value| value.and_then(|value| decode(&value)))
        .collect()
}

fn decode_checked_brief(value: &str) -> Result<ResolvedResearchBrief, ResearchAdapterError> {
    let artifact: ResolvedResearchBrief = decode(value)?;
    if artifact.reproduce_fingerprint().map_err(domain_error)? != artifact.fingerprint {
        return Err(adapter_error("research brief fingerprint mismatch"));
    }
    Ok(artifact)
}

fn decode_checked_run(value: &str) -> Result<ResearchRun, ResearchAdapterError> {
    let artifact: ResearchRun = decode(value)?;
    if artifact
        .reproduce_specification_fingerprint()
        .map_err(domain_error)?
        != artifact.specification_fingerprint
    {
        return Err(adapter_error("research run fingerprint mismatch"));
    }
    Ok(artifact)
}

fn decode_checked_profile(value: &str) -> Result<AuthenticityProfile, ResearchAdapterError> {
    let artifact: AuthenticityProfile = decode(value)?;
    if artifact.reproduce_fingerprint().map_err(domain_error)? != artifact.fingerprint {
        return Err(adapter_error("authenticity profile fingerprint mismatch"));
    }
    Ok(artifact)
}

fn decode_checked_review(value: &str) -> Result<ProfileReview, ResearchAdapterError> {
    let artifact: ProfileReview = decode(value)?;
    if artifact.reproduce_fingerprint().map_err(domain_error)? != artifact.fingerprint {
        return Err(adapter_error("profile review fingerprint mismatch"));
    }
    Ok(artifact)
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String, ResearchAdapterError> {
    serde_json::to_string(value).map_err(|error| adapter_error(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, ResearchAdapterError> {
    serde_json::from_str(value).map_err(|error| adapter_error(error.to_string()))
}

const fn run_state(state: ResearchRunState) -> &'static str {
    match state {
        ResearchRunState::Queued => "queued",
        ResearchRunState::Running => "running",
        ResearchRunState::AwaitingReview => "awaiting_review",
        ResearchRunState::Failed => "failed",
        ResearchRunState::Cancelled => "cancelled",
    }
}

const fn tool_kind(kind: research_core::lifecycle::ResearchToolKind) -> &'static str {
    use research_core::lifecycle::ResearchToolKind;
    match kind {
        ResearchToolKind::SearchWeb => "search_web",
        ResearchToolKind::FetchPage => "fetch_page",
        ResearchToolKind::RecordEvidence => "record_evidence",
        ResearchToolKind::InspectEvidence => "inspect_evidence",
        ResearchToolKind::DraftProfile => "draft_profile",
        ResearchToolKind::FinishResearch => "finish_research",
    }
}

const fn tool_state(state: ToolCallState) -> &'static str {
    match state {
        ToolCallState::Started => "started",
        ToolCallState::Succeeded => "succeeded",
        ToolCallState::Failed => "failed",
        ToolCallState::Interrupted => "interrupted",
    }
}

const fn review_decision(decision: ProfileReviewDecision) -> &'static str {
    match decision {
        ProfileReviewDecision::Approve => "approve",
        ProfileReviewDecision::Reject => "reject",
        ProfileReviewDecision::RequestRevision => "request_revision",
    }
}

fn require_one(rows: u64, artifact: &str) -> Result<(), ResearchAdapterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(adapter_error(format!(
            "{artifact} was not found or changed"
        )))
    }
}

fn sql_error(error: sqlx::Error) -> ResearchAdapterError {
    adapter_error(error.to_string())
}

fn domain_error(error: research_core::ResearchError) -> ResearchAdapterError {
    adapter_error(error.to_string())
}

fn adapter_error(message: impl Into<String>) -> ResearchAdapterError {
    ResearchAdapterError(message.into())
}
