use dataset_architect_core::{
    brief::ResolvedArchitectBrief,
    lifecycle::{ArchitectRun, ArchitectToolCall, ArchitectToolKind, ArchitectUsage},
    ports::{AgentToolRequest, AgentToolResult},
    proposal::{DatasetArchitectureProposal, estimate_cost},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use workflow_core::allocation::{
    InitialAllocationPolicy, InitialAllocationRequest, allocate_initial_budget,
    explain_initial_allocation,
};

use crate::{
    ArchitectRunner, ExecutionState, PendingFinish, RunnerError,
    inputs::{
        EstimateCostInput, FinishInput, FinishReason, PreviewAllocationInput, ProposalSubmission,
    },
};

impl ArchitectRunner {
    pub(super) async fn handle_tool(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &mut ArchitectRun,
        state: &mut ExecutionState,
        request: AgentToolRequest,
    ) -> Result<AgentToolResult, RunnerError> {
        let kind = tool_kind(&request.name)?;
        let sequence = self.store.list_tool_calls(run.id).await?.len() as u32 + 1;
        let mut call = ArchitectToolCall::start(run, sequence, kind, request.arguments.clone())?;
        self.store.record_tool_call(&call).await?;
        let base_usage = ArchitectUsage {
            tool_calls: 1,
            ..ArchitectUsage::default()
        };
        let expected_usage = if kind == ArchitectToolKind::PreviewAllocation {
            base_usage.checked_add(ArchitectUsage {
                allocation_previews: 1,
                ..ArchitectUsage::default()
            })?
        } else {
            base_usage
        };
        let mut preflight = run.clone();
        if let Err(error) = preflight.record_usage(brief, expected_usage) {
            call.fail(error.to_string(), ArchitectUsage::default())?;
            self.store.record_tool_call(&call).await?;
            return Err(error.into());
        }
        let result = self
            .execute_tool(brief, run, state, kind, request.arguments)
            .await;
        match result {
            Ok((content, usage, terminate)) => {
                let usage = base_usage.checked_add(usage)?;
                debug_assert_eq!(usage, expected_usage);
                run.record_usage(brief, usage)?;
                call.succeed(content.clone(), usage)?;
                self.store.record_tool_call(&call).await?;
                self.store.save_run(run).await?;
                Ok(AgentToolResult {
                    content,
                    details: json!({"tool_call_id": call.id}),
                    terminate,
                })
            }
            Err(error) => {
                if run.record_usage(brief, base_usage).is_ok() {
                    call.fail(error.to_string(), base_usage)?;
                    self.store.record_tool_call(&call).await?;
                    self.store.save_run(run).await?;
                } else {
                    call.fail(error.to_string(), ArchitectUsage::default())?;
                    self.store.record_tool_call(&call).await?;
                }
                Err(error)
            }
        }
    }

    async fn execute_tool(
        &self,
        brief: &ResolvedArchitectBrief,
        run: &ArchitectRun,
        state: &mut ExecutionState,
        kind: ArchitectToolKind,
        arguments: Value,
    ) -> Result<(Value, ArchitectUsage, bool), RunnerError> {
        let none = ArchitectUsage::default();
        match kind {
            ArchitectToolKind::InspectDataset => Ok((
                json!({
                    "dataset": brief.dataset,
                    "datasetFingerprint": brief.dataset_fingerprint,
                    "targetTotalRows": brief.target_total_rows,
                    "reservedRows": brief.reserved_rows,
                    "priorities": brief.priorities,
                }),
                none,
                false,
            )),
            ArchitectToolKind::InspectSemantics => {
                Ok((serde_json::to_value(&brief.semantic_context)?, none, false))
            }
            ArchitectToolKind::InspectAuthenticity => Ok((
                serde_json::to_value(&brief.authenticity_context)?,
                none,
                false,
            )),
            ArchitectToolKind::InspectCoverage => Ok((
                json!({
                    "currentCoverage": brief.current_coverage,
                    "constraints": brief.constraints,
                    "coverageFingerprint": artifact_core::fingerprint(&brief.current_coverage)
                        .map_err(|error| RunnerError::Validation(error.to_string()))?,
                }),
                none,
                false,
            )),
            ArchitectToolKind::InspectDevelopmentEvidence => Ok((
                serde_json::to_value(&brief.development_evidence)?,
                none,
                false,
            )),
            ArchitectToolKind::PreviewAllocation => {
                let input: PreviewAllocationInput = parse(arguments)?;
                let result = allocate_initial_budget(
                    &brief.dataset,
                    InitialAllocationRequest {
                        total_rows: brief.target_total_rows,
                        reserved_rows: brief.reserved_rows,
                        policy: InitialAllocationPolicy::Explicit {
                            targets: input.into_targets(),
                        },
                        current_coverage: brief.current_coverage.clone(),
                        constraints: brief.constraints.clone(),
                    },
                )?;
                let explanation = explain_initial_allocation(&result);
                Ok((
                    json!({"result": result, "explanation": explanation}),
                    ArchitectUsage {
                        allocation_previews: 1,
                        ..ArchitectUsage::default()
                    },
                    false,
                ))
            }
            ArchitectToolKind::EstimateCost => {
                let input: EstimateCostInput = parse(arguments)?;
                Ok((
                    serde_json::to_value(estimate_cost(input.additional_rows, &brief.cost_model)?)?,
                    none,
                    false,
                ))
            }
            ArchitectToolKind::SubmitProposal => {
                if state.proposal.is_some() {
                    return Err(RunnerError::Validation(
                        "this run already submitted a proposal".into(),
                    ));
                }
                let input: ProposalSubmission = parse(arguments)?;
                let proposal = DatasetArchitectureProposal::create(brief, run, input.into_draft())?;
                let response = json!({
                    "proposalId": proposal.id,
                    "proposalFingerprint": proposal.fingerprint,
                    "validated": true,
                    "costEstimate": proposal.cost_estimate,
                });
                state.proposal = Some(proposal);
                Ok((response, none, false))
            }
            ArchitectToolKind::FinishArchitecture => {
                let input: FinishInput = parse(arguments)?;
                let proposal = state.proposal.take().ok_or_else(|| {
                    RunnerError::Validation(
                        "finish_architecture requires a valid submitted proposal".into(),
                    )
                })?;
                let reason = match input.reason {
                    FinishReason::ProposalSubmitted => {
                        dataset_architect_core::lifecycle::ArchitectStopReason::ProposalSubmitted
                    }
                    FinishReason::BudgetExhausted => {
                        dataset_architect_core::lifecycle::ArchitectStopReason::BudgetExhausted
                    }
                };
                state.pending_finish = Some(PendingFinish { proposal, reason });
                Ok((
                    json!({
                        "runId": run.id,
                        "finishAccepted": true,
                        "summary": input.summary,
                        "confidence": input.confidence,
                    }),
                    none,
                    true,
                ))
            }
        }
    }
}

fn tool_kind(name: &str) -> Result<ArchitectToolKind, RunnerError> {
    match name {
        "inspect_dataset" => Ok(ArchitectToolKind::InspectDataset),
        "inspect_semantics" => Ok(ArchitectToolKind::InspectSemantics),
        "inspect_authenticity" => Ok(ArchitectToolKind::InspectAuthenticity),
        "inspect_coverage" => Ok(ArchitectToolKind::InspectCoverage),
        "inspect_development_evidence" => Ok(ArchitectToolKind::InspectDevelopmentEvidence),
        "preview_allocation" => Ok(ArchitectToolKind::PreviewAllocation),
        "estimate_cost" => Ok(ArchitectToolKind::EstimateCost),
        "submit_proposal" => Ok(ArchitectToolKind::SubmitProposal),
        "finish_architecture" => Ok(ArchitectToolKind::FinishArchitecture),
        _ => Err(RunnerError::Validation(format!(
            "unknown architect tool {name:?}"
        ))),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, RunnerError> {
    serde_json::from_value(value).map_err(Into::into)
}
