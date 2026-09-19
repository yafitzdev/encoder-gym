//! Finite iteration completion and selection. No provider or persistence types.
use crate::{
    BoundIdentity, Invalid, OptimizationLaunchAuthorization,
    optimization_iteration::ProjectOptimizationIteration,
    optimization_iteration_execution::{IterationDevelopmentResult, IterationTrainingBinding},
    require,
};
use chrono::{DateTime, Utc};
use dataset_core::versions::DatasetVersionRef;
use encoder_experiment_core::metrics::{
    DevelopmentCandidateEvidence, select_multi_development_candidate,
};
use encoder_optimization_core::agent::DatasetEditProposal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopEnd {
    NoChange,
    CanaryRejected,
    IterationLimit,
    RowChangeLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationSelection {
    pub iteration: BoundIdentity,
    pub candidate: BoundIdentity,
    pub dataset: DatasetVersionRef,
    pub result: BoundIdentity,
}

/// The composition root must load these from their owning journals and verify
/// each result with `IterationDevelopmentResult::from_journal` before admission.
pub struct EvaluatedIteration<'a> {
    pub iteration: &'a ProjectOptimizationIteration,
    pub training: &'a IterationTrainingBinding,
    pub result: &'a IterationDevelopmentResult,
}

pub struct IterationDecision<'a> {
    pub proposal_call_id: Uuid,
    pub proposal: &'a DatasetEditProposal,
    pub not_executed: Option<&'a encoder_optimization_core::generation::RepairNotExecuted>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationCompletion {
    pub run: BoundIdentity,
    pub iteration: BoundIdentity,
    pub number: u32,
    pub proposal_call_id: Uuid,
    pub proposal_fingerprint: String,
    pub result: Option<BoundIdentity>,
    pub selected: Option<IterationSelection>,
    pub row_changes: u32,
    pub total_row_changes: u32,
    pub end: Option<AgentLoopEnd>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl IterationCompletion {
    pub fn create(
        iteration: &ProjectOptimizationIteration,
        launch: &OptimizationLaunchAuthorization,
        decision: IterationDecision<'_>,
        history: &[EvaluatedIteration<'_>],
        previous: Option<&Self>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let IterationDecision {
            proposal_call_id,
            proposal,
            not_executed,
        } = decision;
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .ok_or_else(|| Invalid("Agent settings missing".into()))?;
        settings.validate()?;
        iteration.scope.validate().map_err(invalid)?;
        require(
            !proposal_call_id.is_nil()
                && iteration.scope.launch_fingerprint == launch.fingerprint
                && iteration.run.id == iteration.scope.run_id.to_string()
                && iteration.fingerprint == iteration.reproduce()?,
            "Completion belongs to another launch or has no Agent call",
        )?;
        let number = iteration.scope.iteration;
        if let Some(stopped) = not_executed {
            stopped.validate().map_err(invalid)?;
            require(
                !proposal.stop
                    && stopped.run_id == iteration.scope.run_id
                    && stopped.iteration == number
                    && stopped.proposal_fingerprint
                        == encoder_optimization_core::fingerprint(proposal).map_err(invalid)?,
                "Repair execution stop belongs to another proposal",
            )?;
        }
        let previous_changes = match previous {
            Some(previous) => {
                previous.validate_identity()?;
                require(
                    previous.end.is_none()
                        && previous.run == iteration.run
                        && previous.number.checked_add(1) == Some(number)
                        && iteration.predecessor.as_ref() == Some(&previous.identity()),
                    "Iteration does not follow its completed predecessor",
                )?;
                previous.total_row_changes
            }
            None => {
                require(
                    number == 1 && iteration.predecessor.is_none(),
                    "First completion needs first-iteration inputs",
                )?;
                0
            }
        };
        let row_changes = proposal.additions.iter().try_fold(
            u32::try_from(proposal.removals.len()).map_err(invalid)?,
            |count, target| {
                count
                    .checked_add(target.count)
                    .ok_or_else(|| Invalid("Row accounting overflow".into()))
            },
        )?;
        let total_row_changes = previous_changes
            .checked_add(row_changes)
            .ok_or_else(|| Invalid("Row accounting overflow".into()))?;
        require(
            row_changes <= iteration.scope.maximum_row_changes
                && total_row_changes <= settings.maximum_row_changes
                && proposal.stop == (row_changes == 0)
                && number <= settings.maximum_iterations
                && history.len()
                    == (number - u32::from(proposal.stop || not_executed.is_some())) as usize,
            "Completion is missing an evaluation, exceeds its limits, or contradicts the proposal",
        )?;
        let mut candidates = Vec::new();
        for (index, entry) in history.iter().enumerate() {
            entry.training.validate_for(entry.iteration, launch)?;
            require(
                entry.iteration.scope.iteration as usize == index + 1
                    && entry.iteration.run == iteration.run
                    && entry.iteration.comparison_baseline_revision
                        == iteration.comparison_baseline_revision
                    && entry.iteration.benchmark == iteration.benchmark
                    && entry.result.fingerprint == entry.result.reproduce()?
                    && entry.result.training_binding_fingerprint == entry.training.fingerprint
                    && entry.result.experiment_run_id == entry.training.experiment_run_id,
                "Selection contains a foreign or incomplete iteration result",
            )?;
            candidates.push(DevelopmentCandidateEvidence {
                candidate_id: entry.training.candidate.id.parse().map_err(invalid)?,
                suite_assessments: entry.result.assessments.clone(),
            });
        }
        if !proposal.stop && not_executed.is_none() {
            require(
                history
                    .last()
                    .is_some_and(|entry| entry.iteration == iteration),
                "Completion is missing this iteration's result",
            )?;
        }
        let suites: Vec<_> = iteration.development.reports.keys().cloned().collect();
        let selection =
            select_multi_development_candidate(&suites, &candidates).map_err(invalid)?;
        let selected = selection.map(|selection| {
            let entry = history
                .iter()
                .find(|entry| entry.training.candidate.id == selection.candidate_id.to_string())
                .expect("selection came from checked history");
            IterationSelection {
                iteration: bound(entry.iteration.id, &entry.iteration.fingerprint),
                candidate: entry.training.candidate.clone(),
                dataset: entry.training.qualified_dataset.clone(),
                result: bound(entry.result.experiment_run_id, &entry.result.fingerprint),
            }
        });
        let result = (!proposal.stop && not_executed.is_none()).then(|| {
            let result = history.last().expect("evaluation count checked").result;
            bound(result.experiment_run_id, &result.fingerprint)
        });
        let end = if proposal.stop {
            Some(AgentLoopEnd::NoChange)
        } else if not_executed.is_some() {
            Some(AgentLoopEnd::CanaryRejected)
        } else if number == settings.maximum_iterations {
            Some(AgentLoopEnd::IterationLimit)
        } else if total_row_changes == settings.maximum_row_changes {
            Some(AgentLoopEnd::RowChangeLimit)
        } else {
            None
        };
        let mut value = Self {
            run: iteration.run.clone(),
            iteration: bound(iteration.id, &iteration.fingerprint),
            number,
            proposal_call_id,
            proposal_fingerprint: encoder_optimization_core::fingerprint(proposal)
                .map_err(invalid)?,
            result,
            selected,
            row_changes,
            total_row_changes,
            end,
            created_at,
            fingerprint: String::new(),
        };
        require(
            created_at >= iteration.created_at
                && previous.is_none_or(|p| created_at >= p.created_at),
            "Completion predates its inputs",
        )?;
        value.fingerprint = value.reproduce()?;
        value.validate_identity()?;
        Ok(value)
    }

    pub fn identity(&self) -> BoundIdentity {
        BoundIdentity {
            id: self.iteration.id.clone(),
            fingerprint: self.fingerprint.clone(),
        }
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        let mut value = serde_json::to_value(self).map_err(invalid)?;
        value.as_object_mut().expect("record").remove("fingerprint");
        artifact_core::fingerprint(&value).map_err(invalid)
    }

    pub fn validate_identity(&self) -> Result<(), Invalid> {
        for value in [&self.run, &self.iteration]
            .into_iter()
            .chain(self.result.iter())
        {
            value.validate("Iteration completion")?;
            require(
                Uuid::parse_str(&value.id).is_ok_and(|id| !id.is_nil()),
                "Completion needs exact UUIDs",
            )?;
        }
        crate::validate_hash(&self.proposal_fingerprint)?;
        if let Some(selection) = &self.selected {
            for value in [
                &selection.iteration,
                &selection.candidate,
                &selection.result,
            ] {
                value.validate("Selected iteration")?;
                require(
                    Uuid::parse_str(&value.id).is_ok_and(|id| !id.is_nil()),
                    "Selection needs exact UUIDs",
                )?;
            }
            selection.dataset.validate().map_err(invalid)?;
        }
        require(
            (1..=10).contains(&self.number)
                && !self.proposal_call_id.is_nil()
                && self.row_changes <= self.total_row_changes
                && self.total_row_changes <= 5000
                && match (&self.result, &self.end, self.row_changes) {
                    (None, Some(AgentLoopEnd::NoChange), 0) => true,
                    (None, Some(AgentLoopEnd::CanaryRejected), changes) => changes > 0,
                    (Some(_), end, changes) => {
                        changes > 0
                            && *end != Some(AgentLoopEnd::NoChange)
                            && *end != Some(AgentLoopEnd::CanaryRejected)
                    }
                    _ => false,
                }
                && self.fingerprint == self.reproduce()?,
            "Iteration completion changed",
        )
    }
}

fn bound(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}
fn invalid(error: impl std::fmt::Display) -> Invalid {
    Invalid(error.to_string())
}
