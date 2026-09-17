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

pub(super) fn proposal_requirements(
    scope: &AgentAnalysisScope,
    rows: &BTreeSet<String>,
    evidence: &BTreeSet<String>,
) -> Value {
    json!({
        "maximumRowChanges": scope.maximum_row_changes,
        "maximumSummaryCharacters": 400,
        "maximumEvidenceIdsPerEdit": 20,
        "rowChangeRule": "removals.length + sum(additions[*].count) must not exceed maximumRowChanges",
        "referenceRule": "Every evidenceIds entry must be an exact outer item.id from inspect_development_failures. Never use reportId, source row IDs, fingerprints or training-row IDs found inside content. rowId and templateRowId instead use inspect_training_rows item.id values.",
        "trainingRowIds": rows.iter().take(20).collect::<Vec<_>>(),
        "trainingRowIdsComplete": rows.len() <= 20,
        "developmentEvidenceIds": evidence.iter().take(20).collect::<Vec<_>>(),
        "developmentEvidenceIdsComplete": evidence.len() <= 20,
    })
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
            let page = context_page(page, input.offset, input.limit)?;
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
            let page = context_page(page, input.offset, input.limit)?;
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
        };
        assert!(serde_json::to_vec(&original).unwrap().len() > 90_000);
        let mut offset = 0;
        let mut seen = Vec::new();
        loop {
            let remaining = InspectionPage {
                items: items[offset as usize..].to_vec(),
                next_offset: None,
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
        };
        assert_eq!(context_page(page.clone(), 90, 20).unwrap(), page);
        let empty = InspectionPage {
            items: vec![],
            next_offset: None,
        };
        assert_eq!(context_page(empty.clone(), 0, 20).unwrap(), empty);
        let oversized = InspectionPage {
            items: vec![item(0, "x".repeat(40_000))],
            next_offset: None,
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
