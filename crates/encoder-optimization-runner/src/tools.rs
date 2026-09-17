use std::collections::BTreeSet;

use agent_runtime_core::AgentToolRequest;
use encoder_optimization_core::{
    OptimizationError,
    agent::{AgentAnalysisScope, AgentTurnRecord, DatasetEditProposal, InspectionPage},
    ports::OptimizationInspection,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageRequest {
    offset: u64,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainingRequest {
    offset: u64,
    limit: u32,
    query: Option<String>,
}

pub(super) async fn execute(
    inspection: &dyn OptimizationInspection,
    scope: &AgentAnalysisScope,
    request: &AgentToolRequest,
    rows: &mut BTreeSet<String>,
    evidence: &mut BTreeSet<String>,
    submitted: bool,
    proposal_only: bool,
) -> Result<(Value, Option<DatasetEditProposal>), OptimizationError> {
    if submitted {
        return Err(OptimizationError::Validation(
            "An edit proposal was already submitted".into(),
        ));
    }
    if serde_json::to_vec(&request.arguments)?.len() > 65536 {
        return Err(OptimizationError::Validation(
            "Tool arguments exceed 64 KiB".into(),
        ));
    }
    let invalid =
        |_| OptimizationError::Validation("Tool arguments do not match the declared schema".into());
    if proposal_only && request.name != "propose_dataset_edits" {
        return Err(OptimizationError::Validation(
            "Inspection is closed; this turn must submit propose_dataset_edits".into(),
        ));
    }
    match request.name.as_str() {
        "inspect_development_failures" => {
            let input: PageRequest =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            page_limit(input.limit)?;
            let page = inspection
                .development_failures(scope.clone(), input.offset, input.limit)
                .await?;
            page.validate(input.limit)?;
            evidence.extend(page.items.iter().map(|item| item.id.clone()));
            Ok((serde_json::to_value(page)?, None))
        }
        "inspect_training_rows" => {
            let input: TrainingRequest =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            page_limit(input.limit)?;
            if input
                .query
                .as_ref()
                .is_some_and(|v| v.chars().count() > 200)
            {
                return Err(OptimizationError::Validation(
                    "Training query exceeds 200 characters".into(),
                ));
            }
            let page = inspection
                .training_rows(scope.clone(), input.offset, input.limit, input.query)
                .await?;
            page.validate(input.limit)?;
            rows.extend(page.items.iter().map(|item| item.id.clone()));
            Ok((serde_json::to_value(page)?, None))
        }
        "propose_dataset_edits" => {
            let proposal: DatasetEditProposal =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            proposal.validate(scope, rows, evidence)?;
            Ok((json!({"accepted":true}), Some(proposal)))
        }
        _ => Err(OptimizationError::Validation(
            "Tool is not permitted for encoder optimization".into(),
        )),
    }
}

fn page_limit(limit: u32) -> Result<(), OptimizationError> {
    if (1..=20).contains(&limit) {
        Ok(())
    } else {
        Err(OptimizationError::Validation(
            "Inspection page size must be 1–20".into(),
        ))
    }
}

pub(super) fn restore_inspections(
    record: &AgentTurnRecord,
    rows: &mut BTreeSet<String>,
    evidence: &mut BTreeSet<String>,
) -> Result<(), OptimizationError> {
    for tool in &record.tools {
        if tool.failed {
            continue;
        }
        let target = match tool.name.as_str() {
            "inspect_training_rows" => &mut *rows,
            "inspect_development_failures" => &mut *evidence,
            "propose_dataset_edits" => continue,
            _ => {
                return Err(OptimizationError::Validation(
                    "Persisted Agent tool is not permitted".into(),
                ));
            }
        };
        let page: InspectionPage = serde_json::from_value(tool.result.clone())?;
        page.validate(20)?;
        target.extend(page.items.into_iter().map(|item| item.id));
    }
    Ok(())
}
