use benchmark_architect_core::{
    blueprint::{BenchmarkArchitectureProposal, validate_blueprint},
    brief::ResolvedBenchmarkArchitectBrief,
    lifecycle::{
        BenchmarkArchitectRun, BenchmarkArchitectToolCall, BenchmarkArchitectToolKind,
        BenchmarkArchitectUsage,
    },
    ports::{AgentToolRequest, AgentToolResult},
};
use research_core::evidence::{
    FetchRequest, ResearchEvidence, SearchRequest, UntrustedPage, validate_search_request,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    BenchmarkArchitectRunner, ExecutionState, PendingFinish, RunnerError, ToolExecution,
    inputs::{
        BlueprintInput, EmptyInput, EvidenceInput, FetchInput, FinishInput, FinishReason,
        SearchInput,
    },
};

impl BenchmarkArchitectRunner {
    pub(super) async fn handle_tool(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &mut BenchmarkArchitectRun,
        state: &mut ExecutionState,
        request: AgentToolRequest,
    ) -> Result<AgentToolResult, RunnerError> {
        let kind = tool_kind(&request.name)?;
        let sequence = self.store.list_tool_calls(run.id).await?.len() as u32 + 1;
        let mut call =
            BenchmarkArchitectToolCall::start(run, sequence, kind, request.arguments.clone())?;
        self.store.record_tool_call(&call).await?;
        let base = BenchmarkArchitectUsage {
            tool_calls: 1,
            ..BenchmarkArchitectUsage::default()
        };
        preflight(run, brief, base)?;
        let result = self
            .execute_tool(brief, run, state, &mut call, request.arguments)
            .await;
        match result {
            Ok(result) => {
                let usage = base.checked_add(result.usage)?;
                preflight(run, brief, usage)?;
                run.record_usage(brief, usage)?;
                call.succeed(result.persisted_response, usage)?;
                self.store.record_tool_call(&call).await?;
                self.store.save_run(run).await?;
                Ok(AgentToolResult {
                    content: result.agent_content,
                    details: json!({"tool_call_id": call.id}),
                    terminate: result.terminate,
                })
            }
            Err(error) => {
                if run.record_usage(brief, base).is_ok() {
                    call.fail(error.to_string(), base)?;
                    self.store.record_tool_call(&call).await?;
                    self.store.save_run(run).await?;
                } else {
                    call.fail(error.to_string(), BenchmarkArchitectUsage::default())?;
                    self.store.record_tool_call(&call).await?;
                }
                Err(error)
            }
        }
    }

    async fn execute_tool(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &BenchmarkArchitectRun,
        state: &mut ExecutionState,
        call: &mut BenchmarkArchitectToolCall,
        arguments: Value,
    ) -> Result<ToolExecution, RunnerError> {
        match call.kind {
            BenchmarkArchitectToolKind::InspectBrief => {
                let _: EmptyInput = parse(arguments)?;
                Ok(ToolExecution::no_usage(json!({
                    "task": brief.task,
                    "labels": brief.labels,
                    "labelSemantics": brief.label_semantics,
                    "deployment": brief.deployment,
                    "risks": brief.risks,
                    "objectives": brief.objectives,
                    "candidateSources": brief.candidate_sources,
                    "briefFingerprint": brief.fingerprint,
                })))
            }
            BenchmarkArchitectToolKind::InspectExistingBenchmark => {
                let _: EmptyInput = parse(arguments)?;
                Ok(ToolExecution::no_usage(serde_json::to_value(
                    &brief.existing_benchmark,
                )?))
            }
            BenchmarkArchitectToolKind::InspectExposureHistory => {
                let _: EmptyInput = parse(arguments)?;
                let exposures = brief
                    .existing_benchmark
                    .iter()
                    .flat_map(|summary| &summary.cohorts)
                    .map(|cohort| {
                        json!({
                            "cohortKey": cohort.key,
                            "suiteKind": cohort.suite_kind,
                            "role": cohort.role,
                            "exposure": cohort.exposure,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(ToolExecution::no_usage(json!(exposures)))
            }
            BenchmarkArchitectToolKind::SearchWeb => {
                let input: SearchInput = parse(arguments)?;
                let request = validate_search_request(
                    &brief.source_policy,
                    SearchRequest {
                        query: input.query,
                        source_classes: input.source_classes,
                        maximum_results: input.maximum_results,
                    },
                )?;
                preflight(
                    run,
                    brief,
                    BenchmarkArchitectUsage {
                        tool_calls: 1,
                        searches: 1,
                        ..BenchmarkArchitectUsage::default()
                    },
                )?;
                let results = self.search.search(request).await?;
                let filtered = results
                    .into_iter()
                    .filter(|item| brief.source_policy.permits_url(&item.url).unwrap_or(false))
                    .collect::<Vec<_>>();
                Ok(ToolExecution {
                    agent_content: serde_json::to_value(&filtered)?,
                    persisted_response: json!({
                        "resultCount": filtered.len(),
                        "urls": filtered.iter().map(|value| &value.url).collect::<Vec<_>>(),
                    }),
                    usage: BenchmarkArchitectUsage {
                        searches: 1,
                        ..BenchmarkArchitectUsage::default()
                    },
                    terminate: false,
                })
            }
            BenchmarkArchitectToolKind::FetchPage => {
                let input: FetchInput = parse(arguments)?;
                if !brief.source_policy.permits_url(&input.url)? {
                    return Err(RunnerError::Validation(
                        "source policy rejected fetch URL".into(),
                    ));
                }
                let remaining = brief
                    .budgets
                    .max_fetched_bytes
                    .saturating_sub(run.usage.fetched_bytes);
                let maximum_bytes = input.maximum_bytes.min(remaining);
                if maximum_bytes == 0 {
                    return Err(RunnerError::Validation(
                        "no fetched-byte budget remains".into(),
                    ));
                }
                preflight(
                    run,
                    brief,
                    BenchmarkArchitectUsage {
                        tool_calls: 1,
                        fetched_pages: 1,
                        ..BenchmarkArchitectUsage::default()
                    },
                )?;
                let fetched = self
                    .fetcher
                    .fetch(FetchRequest {
                        url: input.url,
                        maximum_bytes,
                    })
                    .await?;
                let page = UntrustedPage::create(
                    &brief.source_policy,
                    fetched.url,
                    fetched.title,
                    fetched.media_type,
                    fetched.content,
                )?;
                if page.byte_count > maximum_bytes {
                    return Err(RunnerError::Validation(
                        "fetched page exceeded the authorized byte limit".into(),
                    ));
                }
                let agent_content = json!({
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
                let byte_count = page.byte_count;
                state.pages.insert(page.content_hash.clone(), page);
                Ok(ToolExecution {
                    agent_content,
                    persisted_response,
                    usage: BenchmarkArchitectUsage {
                        fetched_pages: 1,
                        fetched_bytes: byte_count,
                        ..BenchmarkArchitectUsage::default()
                    },
                    terminate: false,
                })
            }
            BenchmarkArchitectToolKind::RecordEvidence => {
                let input: EvidenceInput = parse(arguments)?;
                if state.evidence_keys.contains_key(&input.key) {
                    return Err(RunnerError::Validation(
                        "evidence key was already used".into(),
                    ));
                }
                let key = input.key.clone();
                let draft = input.into_draft();
                let page = state.pages.get(&draft.content_hash).ok_or_else(|| {
                    RunnerError::Validation(
                        "evidence must reference a page fetched in this run".into(),
                    )
                })?;
                if page.url != draft.url || !page.content.contains(&draft.excerpt) {
                    return Err(RunnerError::Validation(
                        "evidence URL, hash, or excerpt differs from fetched content".into(),
                    ));
                }
                if !brief.source_policy.allowed_source_classes.is_empty()
                    && !brief
                        .source_policy
                        .allowed_source_classes
                        .contains(&draft.source_class)
                {
                    return Err(RunnerError::Validation(
                        "evidence source class is outside the brief policy".into(),
                    ));
                }
                let evidence =
                    ResearchEvidence::create(run.id, call.id, &brief.source_policy, draft)?;
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
                Ok(ToolExecution::no_usage(serde_json::to_value(evidence)?))
            }
            BenchmarkArchitectToolKind::InspectEvidence => {
                let _: EmptyInput = parse(arguments)?;
                let evidence = self.store.list_evidence(run.id).await?;
                Ok(ToolExecution::no_usage(serde_json::to_value(evidence)?))
            }
            BenchmarkArchitectToolKind::PreviewBlueprint => {
                let input: BlueprintInput = parse(arguments)?;
                let evidence = self.store.list_evidence(run.id).await?;
                let blueprint = input.resolve(&state.evidence_keys)?;
                let report = validate_blueprint(brief, &evidence, &blueprint)?;
                Ok(ToolExecution {
                    agent_content: serde_json::to_value(&report)?,
                    persisted_response: serde_json::to_value(&report)?,
                    usage: BenchmarkArchitectUsage {
                        blueprint_previews: 1,
                        ..BenchmarkArchitectUsage::default()
                    },
                    terminate: false,
                })
            }
            BenchmarkArchitectToolKind::SubmitProposal => {
                if state.proposal.is_some() {
                    return Err(RunnerError::Validation(
                        "this run already submitted a proposal".into(),
                    ));
                }
                let input: BlueprintInput = parse(arguments)?;
                let evidence = self.store.list_evidence(run.id).await?;
                let blueprint = input.resolve(&state.evidence_keys)?;
                let proposal =
                    BenchmarkArchitectureProposal::create(brief, run, &evidence, blueprint)?;
                let response = json!({
                    "proposalId": proposal.id,
                    "proposalFingerprint": proposal.fingerprint,
                    "validated": true,
                    "acquisitionRequirements": proposal.acquisition_requirements.len(),
                });
                state.proposal = Some(proposal);
                Ok(ToolExecution::no_usage(response))
            }
            BenchmarkArchitectToolKind::FinishArchitecture => {
                let input: FinishInput = parse(arguments)?;
                let proposal = state.proposal.take().ok_or_else(|| {
                    RunnerError::Validation("finish requires a valid submitted proposal".into())
                })?;
                let reason = match input.reason {
                    FinishReason::ProposalSubmitted => benchmark_architect_core::lifecycle::BenchmarkArchitectStopReason::ProposalSubmitted,
                    FinishReason::BudgetExhausted => benchmark_architect_core::lifecycle::BenchmarkArchitectStopReason::BudgetExhausted,
                };
                state.pending_finish = Some(PendingFinish { proposal, reason });
                Ok(ToolExecution {
                    agent_content: json!({
                        "runId": run.id,
                        "finishAccepted": true,
                        "summary": input.summary,
                    }),
                    persisted_response: json!({
                        "run_id": run.id,
                        "finish_accepted": true,
                    }),
                    usage: BenchmarkArchitectUsage::default(),
                    terminate: true,
                })
            }
        }
    }
}

fn tool_kind(name: &str) -> Result<BenchmarkArchitectToolKind, RunnerError> {
    match name {
        "inspect_brief" => Ok(BenchmarkArchitectToolKind::InspectBrief),
        "inspect_existing_benchmark" => Ok(BenchmarkArchitectToolKind::InspectExistingBenchmark),
        "inspect_exposure_history" => Ok(BenchmarkArchitectToolKind::InspectExposureHistory),
        "search_web" => Ok(BenchmarkArchitectToolKind::SearchWeb),
        "fetch_page" => Ok(BenchmarkArchitectToolKind::FetchPage),
        "record_evidence" => Ok(BenchmarkArchitectToolKind::RecordEvidence),
        "inspect_evidence" => Ok(BenchmarkArchitectToolKind::InspectEvidence),
        "preview_blueprint" => Ok(BenchmarkArchitectToolKind::PreviewBlueprint),
        "submit_blueprint" => Ok(BenchmarkArchitectToolKind::SubmitProposal),
        "finish_benchmark_architecture" => Ok(BenchmarkArchitectToolKind::FinishArchitecture),
        _ => Err(RunnerError::Validation(format!("unknown tool {name:?}"))),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, RunnerError> {
    serde_json::from_value(value).map_err(Into::into)
}

fn preflight(
    run: &BenchmarkArchitectRun,
    brief: &ResolvedBenchmarkArchitectBrief,
    delta: BenchmarkArchitectUsage,
) -> Result<(), RunnerError> {
    let mut candidate = run.clone();
    candidate.record_usage(brief, delta)?;
    Ok(())
}
