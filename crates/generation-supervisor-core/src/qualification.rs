//! Immutable admission evidence from a completed supervisor run.
//!
//! The handoff selects exact, directly assessed source rows. It does not
//! approve snapshot membership: Dataset Qualification retains that authority.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use generation_core::domain::GenerationPlan;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    SupervisorError,
    contract::{ArtifactBinding, GenerationQualityContract},
    fingerprint,
    lifecycle::{SupervisorRun, SupervisorRunEvent, SupervisorRunState},
    observation::{
        ContractRowVerdict, RowCriterionFailure, RowQualityObservation, StructuralOutcome,
    },
    revision::{PromptGuidanceVersion, PromptRevisionActivation},
    strategy::StrategyAssignmentSet,
};

pub const SUPERVISOR_QUALIFICATION_HANDOFF_SCHEMA_VERSION: u32 = 1;
pub const SUPERVISOR_QUALIFICATION_APPLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationExclusionReason {
    StructurallyRejected,
    Unassessed,
    Borderline,
    Quarantined,
    InactivePromptRevision,
    DuplicateQualifiedAssignment,
    SurplusQualifiedRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", content = "reason", rename_all = "snake_case")]
pub enum QualificationDisposition {
    Selected,
    Excluded(QualificationExclusionReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorQualificationEntry {
    pub observation_id: Uuid,
    pub observation_fingerprint: String,
    pub generated_row_id: Uuid,
    pub generated_row_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_row_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_row_fingerprint: Option<String>,
    pub cell_key: String,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_fingerprint: Option<String>,
    pub contract_verdict: ContractRowVerdict,
    #[serde(default)]
    pub criterion_failures: BTreeSet<RowCriterionFailure>,
    pub disposition: QualificationDisposition,
    pub fingerprint: String,
}

impl SupervisorQualificationEntry {
    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorQualificationCoverage {
    pub target: u32,
    pub attempted: u32,
    pub structurally_accepted: u32,
    pub assessed: u32,
    pub qualified_candidates: u32,
    pub selected: u32,
    pub excluded: u32,
    pub remaining: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorQualificationSelection {
    pub entries: Vec<SupervisorQualificationEntry>,
    pub coverage_by_cell: BTreeMap<String, SupervisorQualificationCoverage>,
    pub complete_evidence_fingerprint: String,
    pub selected_member_fingerprint: String,
}

impl SupervisorQualificationSelection {
    #[allow(clippy::too_many_arguments)]
    pub fn compile(
        contract: &GenerationQualityContract,
        run: &SupervisorRun,
        plan: &GenerationPlan,
        events: &[SupervisorRunEvent],
        observations: &[RowQualityObservation],
        assignments: Option<&StrategyAssignmentSet>,
        prompt_versions: &[PromptGuidanceVersion],
        activations: &[PromptRevisionActivation],
    ) -> Result<Self, SupervisorError> {
        verify_completed_run(contract, run, plan, events)?;
        let prompt_index = verify_prompt_authority(run, prompt_versions, activations)?;
        let assignment_index = verify_assignments(contract, plan, assignments)?;
        let targets = plan
            .cells
            .iter()
            .map(|planned| (planned.cell.key(), planned.target_count))
            .collect::<BTreeMap<_, _>>();

        let mut entries = Vec::with_capacity(observations.len());
        for observation in observations {
            observation.validate(contract)?;
            if observation.supervisor_run_id != run.id
                || !targets.contains_key(&observation.cell_key)
            {
                return Err(SupervisorError::Integrity(
                    "qualification observation is outside the completed run or plan".into(),
                ));
            }
            let prompt = prompt_index
                .get(&observation.prompt_version_id)
                .ok_or_else(|| {
                    SupervisorError::Integrity(
                        "qualification observation references an unknown prompt version".into(),
                    )
                })?;
            if prompt.fingerprint != observation.prompt_version_fingerprint {
                return Err(SupervisorError::Integrity(
                    "qualification observation prompt fingerprint is stale".into(),
                ));
            }
            let prompt_is_active = prompt.sequence == 0
                || activations.iter().any(|activation| {
                    activation.prompt_version_id == prompt.id
                        && activation.prompt_version_fingerprint == prompt.fingerprint
                });
            let verdict = observation.contract_verdict(contract);
            let disposition = QualificationDisposition::Excluded(match verdict {
                ContractRowVerdict::Invalid => QualificationExclusionReason::StructurallyRejected,
                ContractRowVerdict::Unassessed => QualificationExclusionReason::Unassessed,
                ContractRowVerdict::Borderline => QualificationExclusionReason::Borderline,
                ContractRowVerdict::Quarantined => QualificationExclusionReason::Quarantined,
                ContractRowVerdict::Qualified if !prompt_is_active => {
                    QualificationExclusionReason::InactivePromptRevision
                }
                ContractRowVerdict::Qualified => QualificationExclusionReason::SurplusQualifiedRow,
            });
            if observation.structural_outcome == StructuralOutcome::Accepted
                && (observation.source_row_id.is_none()
                    || observation
                        .source_row_fingerprint
                        .as_deref()
                        .is_none_or(str::is_empty))
            {
                return Err(SupervisorError::Integrity(
                    "accepted supervisor observation has no immutable source-row binding".into(),
                ));
            }
            if let Some(fingerprint) = observation.strategy_assignment_fingerprint.as_deref()
                && !assignment_index.contains_key(fingerprint)
            {
                return Err(SupervisorError::Integrity(
                    "qualification observation references an unknown strategy assignment".into(),
                ));
            }
            let mut entry = SupervisorQualificationEntry {
                observation_id: observation.id,
                observation_fingerprint: observation.fingerprint.clone(),
                generated_row_id: observation.generated_row_id,
                generated_row_fingerprint: observation.generated_row_fingerprint.clone(),
                source_row_id: observation.source_row_id,
                source_row_fingerprint: observation.source_row_fingerprint.clone(),
                cell_key: observation.cell_key.clone(),
                prompt_version_id: observation.prompt_version_id,
                prompt_version_fingerprint: observation.prompt_version_fingerprint.clone(),
                strategy_assignment_fingerprint: observation
                    .strategy_assignment_fingerprint
                    .clone(),
                assessment_id: observation
                    .assessment
                    .as_ref()
                    .map(|assessment| assessment.assessment_id),
                assessment_fingerprint: observation
                    .assessment
                    .as_ref()
                    .map(|assessment| assessment.assessment_fingerprint.clone()),
                contract_verdict: verdict,
                criterion_failures: observation.criterion_failures(contract),
                disposition,
                fingerprint: String::new(),
            };
            entry.fingerprint = entry.reproduce_fingerprint()?;
            entries.push(entry);
        }

        select_qualified_entries(&mut entries, plan, assignments, &assignment_index)?;
        entries.sort_by_key(|entry| entry.observation_id);
        let mut selected_rows = entries
            .iter()
            .filter(|entry| entry.disposition == QualificationDisposition::Selected)
            .map(|entry| {
                Ok((
                    entry.source_row_id.ok_or_else(|| {
                        SupervisorError::Integrity("selected entry has no source row".into())
                    })?,
                    entry.source_row_fingerprint.clone().ok_or_else(|| {
                        SupervisorError::Integrity(
                            "selected entry has no source-row fingerprint".into(),
                        )
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, SupervisorError>>()?;
        selected_rows.sort_by_key(|(id, _)| *id);
        if selected_rows
            .iter()
            .map(|(id, _)| id)
            .collect::<BTreeSet<_>>()
            .len()
            != selected_rows.len()
        {
            return Err(SupervisorError::Integrity(
                "qualification selection repeats a source row".into(),
            ));
        }
        let coverage_by_cell = qualification_coverage(plan, &entries)?;
        if coverage_by_cell
            .values()
            .any(|coverage| coverage.remaining != 0)
        {
            return Err(SupervisorError::InvalidTransition(
                "completed supervisor run has not reached every directly qualified cell target"
                    .into(),
            ));
        }
        Ok(Self {
            complete_evidence_fingerprint: fingerprint(&entries)?,
            selected_member_fingerprint: fingerprint(&selected_rows)?,
            entries,
            coverage_by_cell,
        })
    }

    pub fn selected_source_row_ids(&self) -> Vec<Uuid> {
        let mut ids = self
            .entries
            .iter()
            .filter(|entry| entry.disposition == QualificationDisposition::Selected)
            .filter_map(|entry| entry.source_row_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorQualificationHandoff {
    pub id: Uuid,
    pub schema_version: u32,
    pub supervisor_run: ArtifactBinding,
    pub quality_contract: ArtifactBinding,
    pub dataset: ArtifactBinding,
    pub generation_plan: ArtifactBinding,
    pub completion_event: ArtifactBinding,
    pub replay_audit_plan: ArtifactBinding,
    pub replay_audit_run: ArtifactBinding,
    pub entries: Vec<SupervisorQualificationEntry>,
    pub coverage_by_cell: BTreeMap<String, SupervisorQualificationCoverage>,
    pub complete_evidence_fingerprint: String,
    pub selected_member_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SupervisorQualificationHandoff {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        contract: &GenerationQualityContract,
        run: &SupervisorRun,
        completion_event: &SupervisorRunEvent,
        selection: SupervisorQualificationSelection,
        replay_audit_plan: ArtifactBinding,
        replay_audit_run: ArtifactBinding,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        if id.is_nil()
            || completion_event.run_id != run.id
            || completion_event.to_state != SupervisorRunState::Completed
            || completion_event.reproduce_fingerprint()? != completion_event.fingerprint
        {
            return Err(SupervisorError::InvalidTransition(
                "qualification handoff requires the exact terminal completion event".into(),
            ));
        }
        let mut value = Self {
            id,
            schema_version: SUPERVISOR_QUALIFICATION_HANDOFF_SCHEMA_VERSION,
            supervisor_run: ArtifactBinding::new(run.id, run.fingerprint.clone())?,
            quality_contract: ArtifactBinding::new(contract.id, contract.fingerprint.clone())?,
            dataset: contract.dataset.clone(),
            generation_plan: contract.plan.clone(),
            completion_event: ArtifactBinding::new(
                completion_event.id,
                completion_event.fingerprint.clone(),
            )?,
            replay_audit_plan,
            replay_audit_run,
            entries: selection.entries,
            coverage_by_cell: selection.coverage_by_cell,
            complete_evidence_fingerprint: selection.complete_evidence_fingerprint,
            selected_member_fingerprint: selection.selected_member_fingerprint,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_integrity()?;
        Ok(value)
    }

    pub fn selected_source_row_ids(&self) -> Vec<Uuid> {
        let mut ids = self
            .entries
            .iter()
            .filter(|entry| entry.disposition == QualificationDisposition::Selected)
            .filter_map(|entry| entry.source_row_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub fn verify_integrity(&self) -> Result<(), SupervisorError> {
        let mut selected = self
            .entries
            .iter()
            .filter(|entry| entry.disposition == QualificationDisposition::Selected)
            .map(|entry| {
                Ok((
                    entry.source_row_id.ok_or_else(|| {
                        SupervisorError::Integrity(
                            "selected handoff entry has no source row".into(),
                        )
                    })?,
                    entry.source_row_fingerprint.clone().ok_or_else(|| {
                        SupervisorError::Integrity(
                            "selected handoff entry has no source fingerprint".into(),
                        )
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, SupervisorError>>()?;
        selected.sort_by_key(|(id, _)| *id);
        if self.id.is_nil()
            || self.schema_version != SUPERVISOR_QUALIFICATION_HANDOFF_SCHEMA_VERSION
            || self.entries.is_empty()
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].observation_id < pair[1].observation_id)
            || self.entries.iter().any(|entry| {
                entry.fingerprint.is_empty()
                    || entry.reproduce_fingerprint().ok().as_deref()
                        != Some(entry.fingerprint.as_str())
            })
            || self
                .coverage_by_cell
                .values()
                .any(|coverage| coverage.remaining != 0)
            || fingerprint(&self.entries)? != self.complete_evidence_fingerprint
            || fingerprint(&selected)? != self.selected_member_fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "supervisor qualification handoff does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorQualificationApplication {
    pub id: Uuid,
    pub schema_version: u32,
    pub handoff: ArtifactBinding,
    pub replay_audit_plan: ArtifactBinding,
    pub replay_audit_run: ArtifactBinding,
    pub quality_report: ArtifactBinding,
    pub curation_proposal: ArtifactBinding,
    pub selected_member_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SupervisorQualificationApplication {
    pub fn create(
        id: Uuid,
        handoff: &SupervisorQualificationHandoff,
        quality_report: ArtifactBinding,
        curation_proposal: ArtifactBinding,
        created_at: DateTime<Utc>,
    ) -> Result<Self, SupervisorError> {
        handoff.verify_integrity()?;
        if id.is_nil() {
            return Err(SupervisorError::Validation(
                "qualification application ID must not be nil".into(),
            ));
        }
        let mut value = Self {
            id,
            schema_version: SUPERVISOR_QUALIFICATION_APPLICATION_SCHEMA_VERSION,
            handoff: ArtifactBinding::new(handoff.id, handoff.fingerprint.clone())?,
            replay_audit_plan: handoff.replay_audit_plan.clone(),
            replay_audit_run: handoff.replay_audit_run.clone(),
            quality_report,
            curation_proposal,
            selected_member_fingerprint: handoff.selected_member_fingerprint.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.verify_integrity(handoff)?;
        Ok(value)
    }

    pub fn verify_integrity(
        &self,
        handoff: &SupervisorQualificationHandoff,
    ) -> Result<(), SupervisorError> {
        if self.id.is_nil()
            || self.schema_version != SUPERVISOR_QUALIFICATION_APPLICATION_SCHEMA_VERSION
            || self.handoff.id != handoff.id
            || self.handoff.fingerprint != handoff.fingerprint
            || self.replay_audit_plan != handoff.replay_audit_plan
            || self.replay_audit_run != handoff.replay_audit_run
            || self.selected_member_fingerprint != handoff.selected_member_fingerprint
            || self.fingerprint.is_empty()
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "supervisor qualification application does not reproduce".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SupervisorError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }
}

fn verify_completed_run(
    contract: &GenerationQualityContract,
    run: &SupervisorRun,
    plan: &GenerationPlan,
    events: &[SupervisorRunEvent],
) -> Result<(), SupervisorError> {
    contract.validate()?;
    if run.contract_id != contract.id
        || run.contract_fingerprint != contract.fingerprint
        || run.reproduce_fingerprint()? != run.fingerprint
        || plan.id != contract.plan.id
        || fingerprint(plan)? != contract.plan.fingerprint
        || SupervisorRunEvent::verify_chain(run.id, events)? != SupervisorRunState::Completed
    {
        return Err(SupervisorError::InvalidTransition(
            "qualification selection requires an intact completed supervisor run".into(),
        ));
    }
    Ok(())
}

fn verify_prompt_authority<'a>(
    run: &SupervisorRun,
    prompts: &'a [PromptGuidanceVersion],
    activations: &[PromptRevisionActivation],
) -> Result<BTreeMap<Uuid, &'a PromptGuidanceVersion>, SupervisorError> {
    let mut index = BTreeMap::new();
    for prompt in prompts {
        prompt.validate()?;
        if prompt.supervisor_run_id != run.id || index.insert(prompt.id, prompt).is_some() {
            return Err(SupervisorError::Integrity(
                "prompt authority contains a duplicate or foreign version".into(),
            ));
        }
    }
    let initial = index.get(&run.initial_prompt_version_id).ok_or_else(|| {
        SupervisorError::Integrity("qualification prompt authority has no initial version".into())
    })?;
    if initial.fingerprint != run.initial_prompt_version_fingerprint || initial.sequence != 0 {
        return Err(SupervisorError::Integrity(
            "qualification initial prompt binding is stale".into(),
        ));
    }
    for activation in activations {
        let prompt = index.get(&activation.prompt_version_id).ok_or_else(|| {
            SupervisorError::Integrity("activation references an unknown prompt version".into())
        })?;
        if activation.supervisor_run_id != run.id
            || activation.prompt_version_fingerprint != prompt.fingerprint
            || activation.reproduce_fingerprint()? != activation.fingerprint
        {
            return Err(SupervisorError::Integrity(
                "qualification activation does not reproduce".into(),
            ));
        }
    }
    Ok(index)
}

fn verify_assignments<'a>(
    contract: &GenerationQualityContract,
    plan: &GenerationPlan,
    assignments: Option<&'a StrategyAssignmentSet>,
) -> Result<BTreeMap<&'a str, usize>, SupervisorError> {
    match (&contract.strategy_context, assignments) {
        (None, None) => Ok(BTreeMap::new()),
        (Some(_), Some(assignments)) => {
            assignments.validate(plan)?;
            if assignments.plan_id != contract.plan.id
                || assignments.plan_fingerprint != contract.plan.fingerprint
            {
                return Err(SupervisorError::Integrity(
                    "qualification strategy assignments are outside the contract".into(),
                ));
            }
            Ok(assignments
                .assignments
                .iter()
                .enumerate()
                .map(|(index, assignment)| (assignment.fingerprint.as_str(), index))
                .collect())
        }
        _ => Err(SupervisorError::Integrity(
            "qualification strategy authority does not match the contract".into(),
        )),
    }
}

fn select_qualified_entries(
    entries: &mut [SupervisorQualificationEntry],
    plan: &GenerationPlan,
    assignments: Option<&StrategyAssignmentSet>,
    assignment_index: &BTreeMap<&str, usize>,
) -> Result<(), SupervisorError> {
    if let Some(assignments) = assignments {
        let mut candidates = BTreeMap::<String, Vec<usize>>::new();
        for (index, entry) in entries.iter().enumerate() {
            if entry.contract_verdict == ContractRowVerdict::Qualified
                && !matches!(
                    entry.disposition,
                    QualificationDisposition::Excluded(
                        QualificationExclusionReason::InactivePromptRevision
                    )
                )
            {
                let assignment = entry
                    .strategy_assignment_fingerprint
                    .as_deref()
                    .ok_or_else(|| {
                        SupervisorError::Integrity(
                            "qualified strategy row has no assignment binding".into(),
                        )
                    })?;
                candidates
                    .entry(assignment.to_owned())
                    .or_default()
                    .push(index);
            }
        }
        for assignment in &assignments.assignments {
            let values = candidates
                .remove(assignment.fingerprint.as_str())
                .ok_or_else(|| {
                    SupervisorError::InvalidTransition(format!(
                        "completed run has no qualified row for strategy assignment {} / {}",
                        assignment.cell_key, assignment.row_sequence
                    ))
                })?;
            let selected = values
                .iter()
                .copied()
                .min_by_key(|index| entries[*index].observation_id)
                .ok_or_else(|| {
                    SupervisorError::Integrity(
                        "qualification assignment candidate set is unexpectedly empty".into(),
                    )
                })?;
            set_disposition(&mut entries[selected], QualificationDisposition::Selected)?;
            for index in values.into_iter().filter(|index| *index != selected) {
                set_disposition(
                    &mut entries[index],
                    QualificationDisposition::Excluded(
                        QualificationExclusionReason::DuplicateQualifiedAssignment,
                    ),
                )?;
            }
        }
        if !candidates.is_empty() || assignment_index.len() != assignments.assignments.len() {
            return Err(SupervisorError::Integrity(
                "qualification candidates contain duplicate or foreign assignments".into(),
            ));
        }
    } else {
        for planned in &plan.cells {
            let cell_key = planned.cell.key();
            let mut candidates = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.cell_key == cell_key
                        && entry.contract_verdict == ContractRowVerdict::Qualified
                        && !matches!(
                            entry.disposition,
                            QualificationDisposition::Excluded(
                                QualificationExclusionReason::InactivePromptRevision
                            )
                        )
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            candidates.sort_by_key(|index| entries[*index].observation_id);
            if candidates.len() < planned.target_count as usize {
                return Err(SupervisorError::InvalidTransition(format!(
                    "completed run has only {} directly qualified rows for {cell_key}, target {}",
                    candidates.len(),
                    planned.target_count
                )));
            }
            for (position, index) in candidates.into_iter().enumerate() {
                let disposition = if position < planned.target_count as usize {
                    QualificationDisposition::Selected
                } else {
                    QualificationDisposition::Excluded(
                        QualificationExclusionReason::SurplusQualifiedRow,
                    )
                };
                set_disposition(&mut entries[index], disposition)?;
            }
        }
    }
    Ok(())
}

fn set_disposition(
    entry: &mut SupervisorQualificationEntry,
    disposition: QualificationDisposition,
) -> Result<(), SupervisorError> {
    entry.disposition = disposition;
    entry.fingerprint = entry.reproduce_fingerprint()?;
    Ok(())
}

fn qualification_coverage(
    plan: &GenerationPlan,
    entries: &[SupervisorQualificationEntry],
) -> Result<BTreeMap<String, SupervisorQualificationCoverage>, SupervisorError> {
    let mut coverage = plan
        .cells
        .iter()
        .map(|planned| {
            (
                planned.cell.key(),
                SupervisorQualificationCoverage {
                    target: planned.target_count,
                    ..SupervisorQualificationCoverage::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for entry in entries {
        let cell = coverage.get_mut(&entry.cell_key).ok_or_else(|| {
            SupervisorError::Integrity("qualification entry cell is outside the plan".into())
        })?;
        cell.attempted = increment(cell.attempted)?;
        if entry.source_row_id.is_some() {
            cell.structurally_accepted = increment(cell.structurally_accepted)?;
        }
        if entry.assessment_id.is_some() {
            cell.assessed = increment(cell.assessed)?;
        }
        if entry.contract_verdict == ContractRowVerdict::Qualified {
            cell.qualified_candidates = increment(cell.qualified_candidates)?;
        }
        match entry.disposition {
            QualificationDisposition::Selected => cell.selected = increment(cell.selected)?,
            QualificationDisposition::Excluded(_) => cell.excluded = increment(cell.excluded)?,
        }
    }
    for value in coverage.values_mut() {
        value.remaining = value.target.saturating_sub(value.selected);
    }
    Ok(coverage)
}

fn increment(value: u32) -> Result<u32, SupervisorError> {
    value.checked_add(1).ok_or_else(|| {
        SupervisorError::Validation("qualification coverage counter overflowed".into())
    })
}
