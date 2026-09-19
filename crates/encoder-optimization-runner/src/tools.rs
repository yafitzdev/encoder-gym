use std::collections::BTreeSet;

use agent_runtime_core::AgentToolRequest;
use encoder_optimization_core::{
    OptimizationError,
    agent::{AgentAnalysisScope, DatasetEditProposal, InspectionPage},
    ports::OptimizationInspection,
    repair_strategy::{
        AnchorAllocation, RepairOperation, RepairPlan, verify_repair_plan_submission,
    },
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClusterRowsRequest {
    cluster_ids: Vec<String>,
    examples_per_cluster: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvestigationRowsRequest {
    cluster_ids: Vec<String>,
    cursor: u64,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitRepairPlanRequest {
    plan: RepairPlan,
    preview_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum V3ToolStage {
    Landscape,
    Examples,
    Planning,
    Submission,
}

pub(super) struct ToolExecution<'a> {
    pub rows: &'a mut BTreeSet<String>,
    pub evidence: &'a mut BTreeSet<String>,
    pub preview_fingerprints: &'a mut BTreeSet<String>,
    pub proposal_only: bool,
    pub v3_stage: Option<V3ToolStage>,
}

pub(super) fn proposal_requirements(
    scope: &AgentAnalysisScope,
    rows: &BTreeSet<String>,
    evidence: &BTreeSet<String>,
) -> Value {
    let reference_rule = if scope.analysis_protocol >= 2 {
        "Every evidenceIds entry must be an exact outer item.id from inspect_dataset_landscape. rowId and templateRowId must be exact outer item.id values returned by inspect_dataset_clusters. Nested report, source-row and fingerprint values are never valid references."
    } else {
        "Every evidenceIds entry must be an exact outer item.id from inspect_development_failures. Never use reportId, source row IDs, fingerprints or training-row IDs found inside content. rowId and templateRowId instead use inspect_training_rows item.id values."
    };
    json!({
        "maximumRowChanges": scope.maximum_row_changes,
        "maximumSummaryCharacters": 400,
        "maximumEvidenceIdsPerEdit": 20,
        "rowChangeRule": "removals.length + sum(additions[*].count) must not exceed maximumRowChanges",
        "referenceRule": reference_rule,
        "trainingRowIds": rows.iter().take(20).collect::<Vec<_>>(),
        "trainingRowIdsComplete": rows.len() <= 20,
        "developmentEvidenceIds": evidence.iter().take(20).collect::<Vec<_>>(),
        "developmentEvidenceIdsComplete": evidence.len() <= 20,
        "repairPlanProtocol": (scope.analysis_protocol == 3).then_some("repair-plan.v3"),
        "repairPlanLimits": (scope.analysis_protocol == 3).then_some(json!({
            "maximumTargets": 4,
            "maximumAnchorsPerTarget": 8,
            "maximumInspectedRows": 32,
            "maximumAdditionsPerAnchor": 8,
            "countBases": ["absolute_rows", "relative_cluster_growth"],
        })),
    })
}

pub(super) async fn execute(
    inspection: &dyn OptimizationInspection,
    scope: &AgentAnalysisScope,
    request: &AgentToolRequest,
    context: &mut ToolExecution<'_>,
    submitted: bool,
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
    if context.proposal_only
        && !matches!(
            request.name.as_str(),
            "propose_dataset_edits" | "submit_repair_plan"
        )
    {
        let required = if scope.analysis_protocol == 3 {
            "submit_repair_plan"
        } else {
            "propose_dataset_edits"
        };
        return Err(OptimizationError::Validation(format!(
            "Inspection is closed; this turn must submit {required}"
        )));
    }
    let permitted = match scope.analysis_protocol {
        1 => matches!(
            request.name.as_str(),
            "inspect_development_failures" | "inspect_training_rows" | "propose_dataset_edits"
        ),
        2 => matches!(
            request.name.as_str(),
            "inspect_dataset_landscape" | "inspect_dataset_clusters" | "propose_dataset_edits"
        ),
        3 => matches!(
            request.name.as_str(),
            "inspect_dataset_landscape"
                | "inspect_dataset_clusters"
                | "preview_repair_plan"
                | "submit_repair_plan"
        ),
        _ => false,
    };
    if !permitted {
        return Err(OptimizationError::Validation(
            "Tool is not permitted by this analysis protocol".into(),
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
            let page = context_page(page, input.offset, input.limit)?;
            context
                .evidence
                .extend(page.items.iter().map(|item| item.id.clone()));
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
            let page = context_page(page, input.offset, input.limit)?;
            context
                .rows
                .extend(page.items.iter().map(|item| item.id.clone()));
            Ok((serde_json::to_value(page)?, None))
        }
        "inspect_dataset_landscape" => {
            if scope.analysis_protocol == 3
                && !matches!(
                    context.v3_stage,
                    Some(V3ToolStage::Landscape | V3ToolStage::Examples | V3ToolStage::Planning)
                )
            {
                return Err(OptimizationError::Validation(
                    "Landscape inspection is not available in this V3 stage".into(),
                ));
            }
            let input: PageRequest =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            page_limit(input.limit)?;
            let page = inspection
                .dataset_landscape(scope.clone(), input.offset, input.limit)
                .await?;
            let page = context_page(page, input.offset, input.limit)?;
            context
                .evidence
                .extend(page.items.iter().map(|item| item.id.clone()));
            Ok((serde_json::to_value(page)?, None))
        }
        "inspect_dataset_clusters" => {
            if scope.analysis_protocol == 3 {
                if !matches!(
                    context.v3_stage,
                    Some(V3ToolStage::Examples | V3ToolStage::Planning)
                ) {
                    return Err(OptimizationError::Validation(
                        "Example inspection requires a completed earlier landscape turn".into(),
                    ));
                }
                let input: InvestigationRowsRequest =
                    serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
                if input.cluster_ids.is_empty()
                    || input.cluster_ids.len() > 4
                    || !(1..=32).contains(&input.limit)
                    || input
                        .cluster_ids
                        .iter()
                        .any(|id| !context.evidence.contains(id))
                {
                    return Err(OptimizationError::Validation(
                        "V3 example inspection requires 1–4 returned clusters, a 1–32 page and at most 32 inspected rows per iteration"
                            .into(),
                    ));
                }
                let page = inspection
                    .dataset_investigation_rows(
                        scope.clone(),
                        input.cluster_ids,
                        input.cursor,
                        input.limit,
                    )
                    .await?;
                let page = context_page(page, input.cursor, input.limit)?;
                let returned = page
                    .items
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<BTreeSet<_>>();
                let projected = context.rows.union(&returned).count();
                if projected > 32 {
                    return Err(OptimizationError::Validation(
                        "V3 inspection would exceed 32 distinct training rows".into(),
                    ));
                }
                context
                    .rows
                    .extend(page.items.iter().map(|item| item.id.clone()));
                return Ok((serde_json::to_value(page)?, None));
            }
            let input: ClusterRowsRequest =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            if input.cluster_ids.is_empty()
                || input.cluster_ids.len() > 4
                || !(1..=4).contains(&input.examples_per_cluster)
                || input
                    .cluster_ids
                    .iter()
                    .any(|id| !context.evidence.contains(id))
            {
                return Err(OptimizationError::Validation(
                    "Cluster inspection requires 1–4 landscape IDs already returned to this Agent and 1–4 examples per cluster"
                        .into(),
                ));
            }
            let page = inspection
                .dataset_cluster_rows(scope.clone(), input.cluster_ids, input.examples_per_cluster)
                .await?;
            let page = context_page(page, 0, 20)?;
            context
                .rows
                .extend(page.items.iter().map(|item| item.id.clone()));
            Ok((serde_json::to_value(page)?, None))
        }
        "propose_dataset_edits" => {
            if scope.analysis_protocol == 2 && !context.proposal_only {
                return Err(OptimizationError::Validation(
                    "Protocol V2 proposals are accepted only after landscape and cluster inspection complete"
                        .into(),
                ));
            }
            let proposal: DatasetEditProposal =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            proposal.validate(scope, context.rows, context.evidence)?;
            Ok((json!({"accepted":true}), Some(proposal)))
        }
        "preview_repair_plan" => {
            if scope.analysis_protocol != 3 || context.v3_stage != Some(V3ToolStage::Planning) {
                return Err(OptimizationError::Validation(
                    "Repair preview requires completed earlier landscape and example turns".into(),
                ));
            }
            let plan: RepairPlan =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            let planning_context = inspection
                .repair_planning_context(
                    scope.clone(),
                    context.evidence.iter().cloned().collect(),
                    context.rows.iter().cloned().collect(),
                )
                .await?;
            let compilation = encoder_optimization_core::repair_strategy::compile_repair_plan(
                &plan,
                &planning_context,
            )?;
            if let Some(preview) = &compilation.preview {
                context
                    .preview_fingerprints
                    .insert(preview.fingerprint.clone());
            }
            Ok((serde_json::to_value(compilation)?, None))
        }
        "submit_repair_plan" => {
            if scope.analysis_protocol != 3
                || !context.proposal_only
                || context.v3_stage != Some(V3ToolStage::Submission)
            {
                return Err(OptimizationError::Validation(
                    "Repair submission is accepted only in the reserved final V3 turn".into(),
                ));
            }
            let input: SubmitRepairPlanRequest =
                serde_json::from_value(request.arguments.clone()).map_err(invalid)?;
            if !context
                .preview_fingerprints
                .contains(&input.preview_fingerprint)
            {
                return Err(OptimizationError::Validation(
                    "Repair submission does not reference a preview from an earlier turn".into(),
                ));
            }
            let planning_context = inspection
                .repair_planning_context(
                    scope.clone(),
                    context.evidence.iter().cloned().collect(),
                    context.rows.iter().cloned().collect(),
                )
                .await?;
            let preview = verify_repair_plan_submission(
                &input.plan,
                &planning_context,
                &input.preview_fingerprint,
            )?;
            let proposal = compile_legacy_proposal(&input.plan)?;
            proposal.validate(scope, context.rows, context.evidence)?;
            Ok((
                json!({
                    "accepted": true,
                    "previewFingerprint": preview.fingerprint,
                    "planFingerprint": preview.plan_fingerprint,
                    "compiledProposal": proposal,
                }),
                Some(proposal),
            ))
        }
        _ => Err(OptimizationError::Validation(
            "Tool is not permitted for encoder optimization".into(),
        )),
    }
}

fn compile_legacy_proposal(plan: &RepairPlan) -> Result<DatasetEditProposal, OptimizationError> {
    let mut removals = Vec::new();
    let mut additions = Vec::new();
    for target in &plan.targets {
        match &target.operation {
            RepairOperation::LabelPreservingVariants { anchors, .. } => {
                for anchor in anchors {
                    additions.push(legacy_addition(
                        target,
                        anchor,
                        "label_preserving_variants",
                    )?);
                }
                continue;
            }
            RepairOperation::ExistingAnchorContrast { pairs, .. } => {
                for pair in pairs {
                    for anchor in [&pair.left, &pair.right] {
                        additions.push(encoder_optimization_core::agent::GenerationTarget {
                            template_row_id: anchor.row_id.clone(),
                            instruction: abstract_brief(target, "existing_anchor_contrast")?,
                            count: pair.additions_per_side,
                            evidence_ids: target.evidence_ids.clone(),
                        });
                    }
                }
                continue;
            }
            RepairOperation::ProvenRedundantRowRemoval {
                removals: target_rows,
            } => {
                for row in target_rows {
                    removals.push(encoder_optimization_core::agent::RowRemoval {
                        row_id: row.row_id.clone(),
                        reason: row.reason.clone(),
                        evidence_ids: target.evidence_ids.clone(),
                    });
                }
            }
        }
    }
    Ok(DatasetEditProposal {
        summary: plan.summary.clone(),
        stop: plan.stop,
        stop_reason: plan.stop_reason,
        removals,
        additions,
    })
}

fn legacy_addition(
    target: &encoder_optimization_core::repair_strategy::RepairTarget,
    anchor: &AnchorAllocation,
    strategy: &str,
) -> Result<encoder_optimization_core::agent::GenerationTarget, OptimizationError> {
    Ok(encoder_optimization_core::agent::GenerationTarget {
        template_row_id: anchor.row_id.clone(),
        instruction: abstract_brief(target, strategy)?,
        count: anchor.additions,
        evidence_ids: target.evidence_ids.clone(),
    })
}

fn abstract_brief(
    target: &encoder_optimization_core::repair_strategy::RepairTarget,
    strategy: &str,
) -> Result<String, OptimizationError> {
    let value = json!({
        "strategy": strategy,
        "clusterKeys": target.cluster_keys,
        "targetMetric": target.target_metric.name,
        "desiredDirection": target.target_metric.direction,
    });
    let brief = serde_json::to_string(&value)?;
    if brief.chars().count() > 2_000 {
        return Err(OptimizationError::Validation(
            "Compiled repair brief exceeds 2000 characters".into(),
        ));
    }
    Ok(brief)
}

// A native item may contain up to 32 KiB of task-visible content. Leave space
// for its bounded identity/fingerprint and page framing, but do not replay twenty
// such items on every proposal/correction call. Historical pages retain their
// original, larger validation bound and are never rewritten on recovery.
const CONTEXT_PAGE_BYTES: usize = 32 * 1024 + 512;

fn context_page(
    mut page: InspectionPage,
    offset: u64,
    limit: u32,
) -> Result<InspectionPage, OptimizationError> {
    page.validate(limit)?;
    while serde_json::to_vec(&page)?.len() > CONTEXT_PAGE_BYTES {
        if page.items.len() <= 1 {
            return Err(OptimizationError::Validation(
                "Inspection item exceeds the Agent context page limit; its content was not truncated"
                    .into(),
            ));
        }
        page.items.pop();
        page.next_offset = Some(offset.checked_add(page.items.len() as u64).ok_or_else(|| {
            OptimizationError::Validation("Inspection continuation offset overflow".into())
        })?);
    }
    if let Some(selection) = &mut page.selection {
        selection.cursor = offset;
        selection.selected_count = page.items.len() as u32;
    }
    page.validate(limit)?;
    Ok(page)
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

pub(super) use encoder_optimization_core::agent::restore_inspections;

#[cfg(test)]
mod tests {
    use super::*;
    use encoder_optimization_core::{agent::InspectionItem, fingerprint};

    fn item(index: usize, text: String) -> InspectionItem {
        let content = json!({"text":text});
        InspectionItem {
            id: format!("row-{index}"),
            fingerprint: fingerprint(&content).unwrap(),
            content,
        }
    }

    #[test]
    fn context_pages_preserve_exact_items_and_advance_without_skipping() {
        // Quotes and multibyte text exercise serialized UTF-8 bytes, not chars.
        let items: Vec<_> = (0..20).map(|i| item(i, "界\"".repeat(1000))).collect();
        let original = InspectionPage {
            items: items.clone(),
            next_offset: None,
            selection: None,
        };
        assert!(serde_json::to_vec(&original).unwrap().len() > 90_000);
        let mut offset = 0;
        let mut seen = Vec::new();
        loop {
            let remaining = InspectionPage {
                items: items[offset as usize..].to_vec(),
                next_offset: None,
                selection: None,
            };
            let page = context_page(remaining, offset, 20).unwrap();
            page.validate(20).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= CONTEXT_PAGE_BYTES);
            seen.extend(page.items);
            match page.next_offset {
                Some(next) => {
                    assert_eq!(next as usize, seen.len());
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(seen, items);
        original.validate(20).unwrap(); // Historical full pages stay readable.
    }

    #[test]
    fn preserves_small_pages_and_full_native_items_but_never_clips_content() {
        let page = InspectionPage {
            items: vec![item(0, "x".repeat(32_740))],
            next_offset: Some(91),
            selection: None,
        };
        assert_eq!(context_page(page.clone(), 90, 20).unwrap(), page);
        let empty = InspectionPage {
            items: vec![],
            next_offset: None,
            selection: None,
        };
        assert_eq!(context_page(empty.clone(), 0, 20).unwrap(), empty);
        let oversized = InspectionPage {
            items: vec![item(0, "x".repeat(40_000))],
            next_offset: None,
            selection: None,
        };
        assert!(
            context_page(oversized, 0, 20)
                .unwrap_err()
                .to_string()
                .contains("not truncated")
        );
    }

    #[test]
    fn invalid_omitted_items_and_cursor_overflow_still_fail_closed() {
        let mut page = InspectionPage {
            items: (0..20).map(|i| item(i, "x".repeat(5000))).collect(),
            next_offset: None,
            selection: None,
        };
        assert!(context_page(page.clone(), u64::MAX, 20).is_err());
        page.items[19].fingerprint = "altered".into();
        assert!(
            context_page(page, 0, 20)
                .unwrap_err()
                .to_string()
                .contains("fingerprint")
        );
    }

    #[test]
    fn reference_examples_are_bounded_and_labeled_when_incomplete() {
        let scope = AgentAnalysisScope {
            run_id: uuid::Uuid::new_v4(),
            iteration: 1,
            launch_fingerprint: fingerprint(&"launch").unwrap(),
            dataset_version_id: uuid::Uuid::new_v4(),
            dataset_fingerprint: fingerprint(&"dataset").unwrap(),
            development_evidence_fingerprint: fingerprint(&"evidence").unwrap(),
            objective: String::new(),
            analysis_protocol: 1,
            maximum_turns: 8,
            maximum_row_changes: 144,
        };
        let ids = (0..21).map(|i| format!("item-{i:02}")).collect();
        let guidance = proposal_requirements(&scope, &ids, &BTreeSet::new());
        assert_eq!(guidance["trainingRowIds"].as_array().unwrap().len(), 20);
        assert_eq!(guidance["trainingRowIdsComplete"], false);
        assert_eq!(guidance["developmentEvidenceIds"], json!([]));
        assert_eq!(guidance["developmentEvidenceIdsComplete"], true);
        assert_eq!(guidance["maximumRowChanges"], 144);
    }
}
