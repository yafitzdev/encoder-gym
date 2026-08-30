use research_core::{
    ResearchError,
    brief::ResolvedResearchBrief,
    evidence::{
        FetchRequest, ResearchEvidence, SearchRequest, UntrustedPage, validate_search_request,
    },
    lifecycle::{
        ResearchRun, ResearchStopReason, ResearchToolCall, ResearchToolKind, ResearchUsage,
    },
    ports::{AgentToolRequest, AgentToolResult},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    ExecutionState, PendingFinish, ResearchRunner, RunnerError, ToolExecution,
    inputs::{
        EvidenceInput, FetchInput, FinishInput, FinishReason, InspectEvidenceInput,
        ProfileSubmission, SearchInput, validate_profile_keys,
    },
};

impl ResearchRunner {
    pub(super) async fn handle_tool(
        &self,
        brief: &ResolvedResearchBrief,
        run: &mut ResearchRun,
        state: &mut ExecutionState,
        request: AgentToolRequest,
    ) -> Result<AgentToolResult, RunnerError> {
        let kind = tool_kind(&request.name)?;
        let sequence = self.store.list_tool_calls(run.id).await?.len() as u32 + 1;
        let mut call = ResearchToolCall::start(run, sequence, kind, request.arguments.clone())?;
        self.store.record_tool_call(&call).await?;
        let result = self
            .execute_tool(brief, run, state, &mut call, &request)
            .await;
        match result {
            Ok(result) => {
                if result.usage != ResearchUsage::default() {
                    run.record_usage(brief, result.usage)?;
                }
                call.succeed(result.persisted_response, result.usage)?;
                self.store.record_tool_call(&call).await?;
                self.store.save_run(run).await?;
                Ok(AgentToolResult {
                    content: result.agent_content,
                    details: json!({"tool_call_id": call.id}),
                    terminate: result.terminate,
                })
            }
            Err(error) => {
                let usage = match &error {
                    RunnerError::ToolFailure { usage, .. } => *usage,
                    _ => ResearchUsage::default(),
                };
                if usage != ResearchUsage::default() {
                    run.record_usage(brief, usage)?;
                    self.store.save_run(run).await?;
                }
                call.fail_with_usage(error.to_string(), usage)?;
                self.store.record_tool_call(&call).await?;
                Err(error)
            }
        }
    }

    async fn execute_tool(
        &self,
        brief: &ResolvedResearchBrief,
        run: &mut ResearchRun,
        state: &mut ExecutionState,
        call: &mut ResearchToolCall,
        request: &AgentToolRequest,
    ) -> Result<ToolExecution, RunnerError> {
        match call.kind {
            ResearchToolKind::SearchWeb => {
                let input: SearchInput = parse(request.arguments.clone())?;
                let search_request = validate_search_request(
                    &brief.source_policy,
                    SearchRequest {
                        query: input.query,
                        source_classes: input.source_classes,
                        maximum_results: input.maximum_results,
                    },
                )?;
                let mut attempts = 0_u32;
                let results = loop {
                    let next_attempts = attempts + 1;
                    if let Err(error) = preflight_usage(
                        run,
                        brief,
                        ResearchUsage {
                            searches: next_attempts,
                            ..ResearchUsage::default()
                        },
                    ) {
                        return Err(RunnerError::ToolFailure {
                            message: error.to_string(),
                            usage: ResearchUsage {
                                searches: attempts,
                                ..ResearchUsage::default()
                            },
                        });
                    }
                    attempts = next_attempts;
                    match self.search.search(search_request.clone()).await {
                        Ok(results) => break results,
                        Err(_) if attempts <= brief.budgets.max_retries_per_call => continue,
                        Err(error) => {
                            return Err(RunnerError::ToolFailure {
                                message: error.to_string(),
                                usage: ResearchUsage {
                                    searches: attempts,
                                    ..ResearchUsage::default()
                                },
                            });
                        }
                    }
                };
                let filtered = results
                    .into_iter()
                    .filter(|item| brief.source_policy.permits_url(&item.url).unwrap_or(false))
                    .collect::<Vec<_>>();
                let content = serde_json::to_value(filtered)?;
                Ok(ToolExecution {
                    agent_content: content.clone(),
                    persisted_response: content,
                    usage: ResearchUsage {
                        searches: attempts,
                        ..ResearchUsage::default()
                    },
                    terminate: false,
                })
            }
            ResearchToolKind::FetchPage => {
                let input: FetchInput = parse(request.arguments.clone())?;
                let mut fetch_request = FetchRequest {
                    url: input.url,
                    maximum_bytes: input.maximum_bytes,
                };
                if !brief.source_policy.permits_url(&fetch_request.url)? {
                    return Err(RunnerError::Validation(
                        "source policy rejected fetch URL".into(),
                    ));
                }
                let remaining = brief
                    .budgets
                    .max_fetched_bytes
                    .saturating_sub(run.usage.fetched_bytes);
                fetch_request.maximum_bytes = fetch_request.maximum_bytes.min(remaining);
                if fetch_request.maximum_bytes == 0 {
                    return Err(ResearchError::BudgetExhausted(
                        "no fetched-byte budget remains".into(),
                    )
                    .into());
                }
                let mut attempts = 0_u32;
                let fetched = loop {
                    let next_attempts = attempts + 1;
                    if let Err(error) = preflight_usage(
                        run,
                        brief,
                        ResearchUsage {
                            fetched_pages: next_attempts,
                            ..ResearchUsage::default()
                        },
                    ) {
                        return Err(RunnerError::ToolFailure {
                            message: error.to_string(),
                            usage: ResearchUsage {
                                fetched_pages: attempts,
                                ..ResearchUsage::default()
                            },
                        });
                    }
                    attempts = next_attempts;
                    match self.fetcher.fetch(fetch_request.clone()).await {
                        Ok(page) => break page,
                        Err(_) if attempts <= brief.budgets.max_retries_per_call => continue,
                        Err(error) => {
                            return Err(RunnerError::ToolFailure {
                                message: error.to_string(),
                                usage: ResearchUsage {
                                    fetched_pages: attempts,
                                    ..ResearchUsage::default()
                                },
                            });
                        }
                    }
                };
                let page = UntrustedPage::create(
                    &brief.source_policy,
                    fetched.url,
                    fetched.title,
                    fetched.media_type,
                    fetched.content,
                )?;
                if page.byte_count > remaining {
                    return Err(ResearchError::BudgetExhausted(
                        "fetched page exceeded remaining byte budget".into(),
                    )
                    .into());
                }
                let content = json!({
                    "url": page.url,
                    "title": page.title,
                    "contentHash": page.content_hash,
                    "byteCount": page.byte_count,
                    "content": page.delimited_for_agent(),
                });
                let persisted_response = json!({
                    "url": page.url,
                    "title": page.title,
                    "content_hash": page.content_hash,
                    "byte_count": page.byte_count,
                    "media_type": page.media_type,
                });
                state.pages.insert(page.content_hash.clone(), page.clone());
                Ok(ToolExecution {
                    agent_content: content,
                    persisted_response,
                    usage: ResearchUsage {
                        fetched_pages: attempts,
                        fetched_bytes: page.byte_count,
                        ..ResearchUsage::default()
                    },
                    terminate: false,
                })
            }
            ResearchToolKind::RecordEvidence => {
                let input: EvidenceInput = parse(request.arguments.clone())?;
                let key = input.key.clone();
                if state.evidence_keys.contains_key(&key) {
                    return Err(RunnerError::Validation(
                        "evidence key was already used".into(),
                    ));
                }
                let evidence_draft = input.into_draft();
                let page = state
                    .pages
                    .get(&evidence_draft.content_hash)
                    .ok_or_else(|| {
                        RunnerError::Validation(
                            "evidence must reference a page fetched in this run".into(),
                        )
                    })?;
                if page.url != evidence_draft.url || !page.content.contains(&evidence_draft.excerpt)
                {
                    return Err(RunnerError::Validation(
                        "evidence URL, hash, or excerpt does not match fetched content".into(),
                    ));
                }
                if !brief.source_policy.allowed_source_classes.is_empty()
                    && !brief
                        .source_policy
                        .allowed_source_classes
                        .contains(&evidence_draft.source_class)
                {
                    return Err(RunnerError::Validation(
                        "evidence source class is outside the brief policy".into(),
                    ));
                }
                let evidence = ResearchEvidence::create(
                    run.id,
                    call.id,
                    &brief.source_policy,
                    evidence_draft,
                )?;
                let existing = self.store.list_evidence(run.id).await?;
                if existing.iter().any(|item| {
                    item.content_hash == evidence.content_hash
                        && item.excerpt == evidence.excerpt
                        && item.observation == evidence.observation
                }) {
                    return Err(RunnerError::Validation(
                        "duplicate research evidence was rejected".into(),
                    ));
                }
                self.store.record_evidence(&evidence).await?;
                state.evidence_keys.insert(key, evidence.id);
                let content = serde_json::to_value(&evidence)?;
                Ok(ToolExecution::no_usage(content))
            }
            ResearchToolKind::InspectEvidence => {
                let _: InspectEvidenceInput = parse(request.arguments.clone())?;
                let evidence = self.store.list_evidence(run.id).await?;
                Ok(ToolExecution::no_usage(serde_json::to_value(evidence)?))
            }
            ResearchToolKind::DraftProfile => {
                let draft: ProfileSubmission = parse(request.arguments.clone())?;
                if state.profile_draft.is_some() {
                    return Err(RunnerError::Validation(
                        "this run already has a profile draft".into(),
                    ));
                }
                validate_profile_keys(&draft, &state.evidence_keys)?;
                state.profile_draft = Some(draft);
                Ok(ToolExecution::no_usage(
                    json!({"accepted_as_review_candidate": true}),
                ))
            }
            ResearchToolKind::FinishResearch => {
                let input: FinishInput = parse(request.arguments.clone())?;
                let submission = state.profile_draft.take().ok_or_else(|| {
                    RunnerError::Validation("finish_research requires a profile draft".into())
                })?;
                let reason = match input.reason {
                    FinishReason::SufficientEvidence => ResearchStopReason::SufficientEvidence,
                    FinishReason::BudgetExhausted => ResearchStopReason::BudgetExhausted,
                };
                state.pending_finish = Some(PendingFinish { submission, reason });
                Ok(ToolExecution {
                    agent_content: json!({"run_id": run.id, "finish_accepted": true}),
                    persisted_response: json!({"run_id": run.id, "finish_accepted": true}),
                    usage: ResearchUsage::default(),
                    terminate: true,
                })
            }
        }
    }
}

fn tool_kind(name: &str) -> Result<ResearchToolKind, RunnerError> {
    match name {
        "search_web" => Ok(ResearchToolKind::SearchWeb),
        "fetch_page" => Ok(ResearchToolKind::FetchPage),
        "record_evidence" => Ok(ResearchToolKind::RecordEvidence),
        "inspect_evidence" => Ok(ResearchToolKind::InspectEvidence),
        "draft_profile" => Ok(ResearchToolKind::DraftProfile),
        "finish_research" => Ok(ResearchToolKind::FinishResearch),
        _ => Err(RunnerError::Validation(format!(
            "unknown research tool {name:?}"
        ))),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, RunnerError> {
    serde_json::from_value(value).map_err(Into::into)
}

fn preflight_usage(
    run: &ResearchRun,
    brief: &ResolvedResearchBrief,
    delta: ResearchUsage,
) -> Result<(), RunnerError> {
    let mut candidate = run.clone();
    candidate.record_usage(brief, delta)?;
    Ok(())
}
