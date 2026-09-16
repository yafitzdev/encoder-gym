//! Separate, row-free final-holdout authority after adaptive work has closed.
//! This never changes the zero-sealed-allowance iteration protocol.
use crate::{
    BoundIdentity, FinalEvaluationAuthorization, Invalid, OptimizationLaunchAuthorization,
    ProjectOptimizationRunState, ProjectOptimizationRunView,
    optimization_execution::AgentExecutionState,
    optimization_iteration::ProjectOptimizationIteration,
    optimization_iteration_execution::{IterationDevelopmentResult, IterationTrainingBinding},
    optimization_loop::IterationCompletion,
    require,
};
use chrono::{DateTime, Utc};
use dataset_core::versions::DatasetVersionRef;
use encoder_experiment_core::{
    domain::ExternalProjectSnapshot, journal::ExperimentEvent, protocol::ExperimentProtocol,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct SelectedFinalEvidence<'a> {
    pub run: &'a ProjectOptimizationRunView,
    pub launch: &'a OptimizationLaunchAuthorization,
    /// The latest completion, deeply revalidated from all original journals.
    pub completion: &'a IterationCompletion,
    pub iteration: &'a ProjectOptimizationIteration,
    pub training: &'a IterationTrainingBinding,
    pub project: &'a ExternalProjectSnapshot,
    pub protocol: &'a ExperimentProtocol,
    pub events: &'a [ExperimentEvent],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinalScope {
    pub run: BoundIdentity,
    pub launch: BoundIdentity,
    pub execution_head: String,
    pub adaptive_closed_at: DateTime<Utc>,
    pub completion: BoundIdentity,
    pub iteration: BoundIdentity,
    pub training_binding_fingerprint: String,
    pub development_result: BoundIdentity,
    pub comparison_baseline_revision: BoundIdentity,
    pub benchmark: BoundIdentity,
    pub scientific_project: BoundIdentity,
    pub protocol: BoundIdentity,
    pub candidate: BoundIdentity,
    pub model: BoundIdentity,
    pub dataset: DatasetVersionRef,
    pub final_suite: String,
    /// Identity only. Sealed baseline scores never enter the handoff.
    pub baseline_final_report: BoundIdentity,
    pub metric_contract_fingerprint: String,
    pub maximum_evaluation_seconds: u64,
    pub fingerprint: String,
}

impl AgentFinalScope {
    pub fn bind(evidence: SelectedFinalEvidence<'_>) -> Result<Self, Invalid> {
        let SelectedFinalEvidence {
            run,
            launch,
            completion,
            iteration,
            training,
            project,
            protocol,
            events,
        } = evidence;
        run.run.validate(launch)?;
        completion.validate_identity()?;
        training.validate_for(iteration, launch)?;
        let execution = run
            .agent_execution
            .as_ref()
            .ok_or_else(|| Invalid("Agent execution is missing".into()))?;
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .ok_or_else(|| Invalid("Agent settings are missing".into()))?;
        require(
            settings.permits_final_evaluation()
                && launch.scope.final_evaluation
                    == FinalEvaluationAuthorization::SelectedCandidateOnce
                && launch.scope.limits.maximum_final_evaluations == 1,
            "Diagnostic runs cannot authorize final holdout",
        )?;
        require(
            match (run.state, execution.state) {
                (ProjectOptimizationRunState::AgentCompleted, AgentExecutionState::Completed) => {
                    completion.end.is_some()
                        && execution.completion.as_ref() == Some(&completion.identity())
                }
                (
                    ProjectOptimizationRunState::AgentBudgetExhausted,
                    AgentExecutionState::BudgetExhausted,
                ) => execution.completion.is_none(),
                _ => false,
            },
            "Finish adaptive work before authorizing final holdout",
        )?;
        let selected = completion
            .selected
            .as_ref()
            .ok_or_else(|| Invalid("No development-eligible candidate was selected".into()))?;
        let result = IterationDevelopmentResult::from_journal(training, project, protocol, events)?;
        let preparation = run
            .preparation
            .as_ref()
            .ok_or_else(|| Invalid("Prepared inputs are missing".into()))?;
        require(
            completion.run == run.run.identity()
                && iteration.run == completion.run
                && selected.iteration == bound(iteration.id, &iteration.fingerprint)
                && selected.candidate == training.candidate
                && selected.dataset == training.qualified_dataset
                && selected.result == bound(result.experiment_run_id, &result.fingerprint)
                && training.training_dataset == training.qualified_dataset
                && result.development_passed
                && completion.created_at <= execution.updated_at
                && iteration.benchmark == preparation.benchmark
                && protocol.sealed_suite_key == preparation.final_suite,
            "Final holdout must use the exact development-selected full-data candidate",
        )?;
        let mut value = Self {
            run: run.run.identity(),
            launch: bound(launch.id, &launch.fingerprint),
            execution_head: execution.head_fingerprint.clone(),
            completion: completion.identity(),
            adaptive_closed_at: execution.updated_at,
            iteration: selected.iteration.clone(),
            training_binding_fingerprint: training.fingerprint.clone(),
            development_result: selected.result.clone(),
            comparison_baseline_revision: iteration.comparison_baseline_revision.clone(),
            benchmark: iteration.benchmark.clone(),
            scientific_project: training.scientific_project.clone(),
            protocol: training.protocol.clone(),
            candidate: training.candidate.clone(),
            model: bound(result.output.model.id, &result.output.model.fingerprint),
            dataset: selected.dataset.clone(),
            final_suite: protocol.sealed_suite_key.clone(),
            baseline_final_report: bound(
                protocol.baseline_sealed_report.id,
                &protocol.baseline_sealed_report.fingerprint,
            ),
            metric_contract_fingerprint: protocol.metric_contract.fingerprint.clone(),
            maximum_evaluation_seconds: protocol.maximum_evaluation_seconds,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_identity()?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        reproduce(self)
    }
    pub fn validate_identity(&self) -> Result<(), Invalid> {
        for identity in [
            &self.run,
            &self.launch,
            &self.completion,
            &self.iteration,
            &self.development_result,
            &self.comparison_baseline_revision,
            &self.benchmark,
            &self.scientific_project,
            &self.protocol,
            &self.candidate,
            &self.model,
            &self.baseline_final_report,
        ] {
            identity.validate("Final holdout identity")?;
            require(
                Uuid::parse_str(&identity.id).is_ok_and(|id| !id.is_nil()),
                "Final holdout requires exact UUIDs",
            )?;
        }
        for hash in [
            &self.execution_head,
            &self.training_binding_fingerprint,
            &self.metric_contract_fingerprint,
        ] {
            crate::validate_hash(hash)?;
        }
        self.dataset.validate().map_err(invalid)?;
        crate::validate_name(&self.final_suite)?;
        require(
            self.maximum_evaluation_seconds > 0 && self.fingerprint == self.reproduce()?,
            "Final holdout scope changed",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinalAuthorization {
    pub id: Uuid,
    pub scope: AgentFinalScope,
    pub authorized_by: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AgentFinalAuthorization {
    pub fn create(
        id: Uuid,
        scope: AgentFinalScope,
        authorized_by: String,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            scope,
            authorized_by,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = reproduce(&value)?;
        value.validate_identity()?;
        Ok(value)
    }
    pub fn validate_identity(&self) -> Result<(), Invalid> {
        self.scope.validate_identity()?;
        crate::validate_name(&self.authorized_by)?;
        require(
            !self.id.is_nil()
                && self.created_at >= self.scope.adaptive_closed_at
                && self.fingerprint == reproduce(self)?,
            "Final authorization changed or predates adaptive completion",
        )
    }
}

fn reproduce(value: &impl Serialize) -> Result<String, Invalid> {
    let mut value = serde_json::to_value(value).map_err(invalid)?;
    value.as_object_mut().expect("record").remove("fingerprint");
    artifact_core::fingerprint(&value).map_err(invalid)
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
