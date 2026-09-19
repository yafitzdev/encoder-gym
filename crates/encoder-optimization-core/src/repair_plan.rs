//! Read-only explanations of accepted decisions and recorded generation slots.
//! These observations grant no execution, training admission or acceptance.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use uuid::Uuid;

use crate::{
    OptimizationError,
    agent::{
        AgentAnalysisScope, AgentCallReservation, AgentTurnRecord, DatasetEditProposal,
        InspectionItem, InspectionPage, restore_inspections,
    },
    fingerprint,
    generation::{
        GenerationOutcome, GenerationPhase, GenerationReservation, GenerationStrategy,
        GenerationTask,
    },
    require,
};

pub type GenerationRecord = (
    GenerationTask,
    GenerationReservation,
    Option<GenerationOutcome>,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairTargetProgress {
    pub target_index: u32,
    pub requested: u32,
    /// Accepted by the task adapter, not necessarily published or qualified.
    pub admitted: u32,
    pub rejected: u32,
    /// Includes undispatched and interrupted slots; does not imply live work.
    pub unresolved: u32,
    pub attempts: u32,
    pub rejection_reasons: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetRepairPlan {
    pub decision_call_id: Uuid,
    pub proposal_fingerprint: String,
    pub maximum_row_changes: u32,
    pub proposal: DatasetEditProposal,
    pub generation: Vec<RepairTargetProgress>,
}

/// Opaque native evidence must be projected by its task adapter before display.
/// Intentionally not serializable as a transport/presentation payload.
pub struct RecordedRepairPlan {
    pub plan: DatasetRepairPlan,
    pub evidence: Vec<InspectionItem>,
}

pub fn recorded_plan(
    scope: &AgentAnalysisScope,
    calls: &[(AgentCallReservation, Option<AgentTurnRecord>)],
    generation: &[GenerationRecord],
) -> Result<Option<RecordedRepairPlan>, OptimizationError> {
    let scope_fingerprint = scope.fingerprint()?;
    let mut rows = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();
    let mut evidence = BTreeMap::<String, InspectionItem>::new();
    let mut decision = None;
    for (index, (call, record)) in calls.iter().enumerate() {
        require(
            call.scope_fingerprint == scope_fingerprint
                && call.sequence as usize == index + 1
                && call.sequence <= scope.maximum_turns,
            "Repair decision belongs to another scope or call sequence",
        )?;
        require(
            decision.is_none(),
            "Agent history continues after its accepted decision",
        )?;
        let Some(record) = record else {
            require(
                index + 1 == calls.len(),
                "An unfinished Agent call precedes later work",
            )?;
            continue;
        };
        record.validate()?;
        require(record.call == *call, "Repair decision reservation changed")?;
        restore_inspections(
            record,
            scope.analysis_protocol,
            &mut rows,
            &mut evidence_ids,
        )?;
        for tool in record.tools.iter().filter(|tool| {
            !tool.failed
                && matches!(
                    tool.name.as_str(),
                    "inspect_development_failures" | "inspect_dataset_landscape"
                )
        }) {
            let page: InspectionPage = serde_json::from_value(tool.result.clone())?;
            for item in page.items {
                if let Some(previous) = evidence.insert(item.id.clone(), item.clone()) {
                    require(
                        previous == item,
                        "Recorded inspection identity has conflicting evidence",
                    )?;
                }
            }
        }
        if let Some(proposal) = &record.proposal {
            proposal.validate(scope, &rows, &evidence_ids)?;
            decision = Some((call.id, proposal.clone()));
        }
    }
    let Some((decision_call_id, proposal)) = decision else {
        require(
            generation.is_empty(),
            "Generation has no accepted repair decision",
        )?;
        return Ok(None);
    };
    let proposal_fingerprint = fingerprint(&proposal)?;
    let progress = generation_progress(scope, &proposal, &proposal_fingerprint, generation)?;
    let references = proposal
        .removals
        .iter()
        .flat_map(|row| &row.evidence_ids)
        .chain(
            proposal
                .additions
                .iter()
                .flat_map(|target| &target.evidence_ids),
        )
        .collect::<BTreeSet<_>>();
    let evidence = references
        .into_iter()
        .map(|id| evidence[id].clone())
        .collect();
    Ok(Some(RecordedRepairPlan {
        plan: DatasetRepairPlan {
            decision_call_id,
            proposal_fingerprint,
            maximum_row_changes: scope.maximum_row_changes,
            proposal,
            generation: progress,
        },
        evidence,
    }))
}

fn generation_progress(
    scope: &AgentAnalysisScope,
    proposal: &DatasetEditProposal,
    proposal_fingerprint: &str,
    records: &[GenerationRecord],
) -> Result<Vec<RepairTargetProgress>, OptimizationError> {
    let mut result: Vec<_> = proposal
        .additions
        .iter()
        .enumerate()
        .map(|(index, target)| RepairTargetProgress {
            target_index: index as u32,
            requested: target.count,
            admitted: 0,
            rejected: 0,
            unresolved: target.count,
            attempts: 0,
            rejection_reasons: BTreeMap::new(),
        })
        .collect();
    let mut slots = BTreeMap::<(u32, u32), Vec<&GenerationRecord>>::new();
    for record in records {
        let (task, call, outcome) = record;
        task.validate()?;
        let target = proposal
            .additions
            .get(task.target_index as usize)
            .ok_or_else(|| {
                OptimizationError::Validation(
                    "Generation target is absent from the repair plan".into(),
                )
            })?;
        let slot_matches = if scope.analysis_protocol == 3 {
            task.execution_v3.as_ref().is_some_and(|execution| {
                task.first_row < target.count
                    && task.first_row + task.requested_rows <= target.count
                    && match execution.phase {
                        GenerationPhase::Canary => {
                            task.first_row == 0
                                && task.requested_rows
                                    == match execution.strategy {
                                        GenerationStrategy::LabelPreservingVariant => {
                                            target.count.min(2)
                                        }
                                        GenerationStrategy::ExistingAnchorContrast => 1,
                                    }
                        }
                        GenerationPhase::Bulk => task.first_row > 0,
                    }
            })
        } else {
            task.execution_v3.is_none()
                && task.first_row < target.count
                && task.first_row % 8 == 0
                && task.requested_rows == (target.count - task.first_row).min(8)
        };
        require(
            task.run_id == scope.run_id
                && task.iteration == scope.iteration
                && task.proposal_fingerprint == proposal_fingerprint
                && task.template_row_id == target.template_row_id
                && slot_matches,
            "Generation slot differs from the accepted repair plan",
        )?;
        require(
            !call.id.is_nil()
                && call.task_id == task.id
                && call.task_fingerprint == task.fingerprint()?
                && call.attempt > 0,
            "Generation reservation differs from its task",
        )?;
        if let Some(outcome) = outcome {
            outcome.validate(task)?;
            require(
                outcome.reservation == *call,
                "Generation outcome differs from its reservation",
            )?;
        }
        slots
            .entry((task.target_index, task.first_row))
            .or_default()
            .push(record);
    }
    let mut call_ids = BTreeSet::new();
    for ((target_index, _), mut attempts) in slots {
        attempts.sort_by_key(|(_, call, _)| call.attempt);
        let task = &attempts[0].0;
        let target = &mut result[target_index as usize];
        let mut finished = false;
        for (index, (current, call, outcome)) in attempts.iter().enumerate() {
            require(
                current == task
                    && call.attempt as usize == index + 1
                    && call_ids.insert(call.id)
                    && !finished,
                "Generation attempts conflict or continue after admission",
            )?;
            target.attempts += 1;
            if let Some(admission) = outcome
                .as_ref()
                .and_then(|outcome| outcome.admission.as_ref())
            {
                target.admitted += admission.accepted.len() as u32;
                target.rejected += admission.rejected.len() as u32;
                target.unresolved -= task.requested_rows;
                for row in &admission.rejected {
                    *target
                        .rejection_reasons
                        .entry(row.reason.clone())
                        .or_default() += 1;
                }
                finished = true;
            } else if outcome.is_none() {
                require(
                    index + 1 == attempts.len(),
                    "An unfinished generation call precedes later work",
                )?;
            }
        }
    }
    Ok(result)
}
