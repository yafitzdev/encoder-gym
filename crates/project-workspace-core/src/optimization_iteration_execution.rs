//! Immutable links from an Agent iteration to its exact trainer population and
//! development result. No native rows, sealed payloads or execution adapters.
use crate::optimization_iteration::ProjectOptimizationIteration;
use crate::{BoundIdentity, Invalid, OptimizationLaunchAuthorization, require};
use chrono::{DateTime, Utc};
use dataset_core::versions::DatasetVersionRef;
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalArtifactIdentity, ExternalProjectSnapshot},
    journal::{CandidateExecutionState, ExperimentEvent, replay_experiment},
    metrics::{CandidateAssessment, CandidateVerdict, EvaluationReport},
    ports::TrainOutput,
    protocol::ExperimentProtocol,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationTrainingBinding {
    pub iteration: BoundIdentity,
    /// Full, qualified Agent result, before any diagnostic sampling.
    pub qualified_dataset: DatasetVersionRef,
    pub clearance_fingerprint: String,
    /// Exact version actually passed to the trainer, also for Quick test.
    pub training_dataset: DatasetVersionRef,
    pub training_rows: u64,
    pub native_materialization_fingerprint: String,
    pub training_artifact: ExternalArtifactIdentity,
    pub scientific_project: BoundIdentity,
    pub protocol: BoundIdentity,
    pub candidate: BoundIdentity,
    pub experiment_run_id: Uuid,
    pub resolved_device: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl IterationTrainingBinding {
    pub fn reproduce(&self) -> Result<String, Invalid> {
        fingerprint(self)
    }

    pub fn validate_for(
        &self,
        iteration: &ProjectOptimizationIteration,
        launch: &OptimizationLaunchAuthorization,
    ) -> Result<(), Invalid> {
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .ok_or_else(|| Invalid("Missing Agent settings".into()))?;
        settings.validate()?;
        self.qualified_dataset.validate().map_err(invalid)?;
        self.training_dataset.validate().map_err(invalid)?;
        self.training_artifact.validate().map_err(invalid)?;
        for identity in [
            &self.iteration,
            &self.scientific_project,
            &self.protocol,
            &self.candidate,
        ] {
            identity.validate("Iteration training link")?;
            require(
                Uuid::parse_str(&identity.id).is_ok_and(|id| !id.is_nil()),
                "Invalid training link UUID",
            )?;
        }
        crate::validate_hash(&self.clearance_fingerprint)?;
        crate::validate_hash(&self.native_materialization_fingerprint)?;
        require(
            self.iteration == bound(iteration.id, &iteration.fingerprint)
                && iteration.scope.launch_fingerprint == launch.fingerprint
                && self.qualified_dataset.project_id == iteration.dataset.project_id
                && self.training_dataset.project_id == iteration.dataset.project_id
                && self.training_artifact.role == EvidenceRole::Training
                && self.training_rows > 0
                && !self.experiment_run_id.is_nil()
                && ["cpu", "cuda"].contains(&self.resolved_device.as_str())
                && match settings.training.device {
                    crate::OptimizationDevice::Auto => true,
                    crate::OptimizationDevice::Cpu => self.resolved_device == "cpu",
                    crate::OptimizationDevice::Cuda => self.resolved_device == "cuda",
                }
                && self.created_at >= iteration.created_at
                && self.reproduce()? == self.fingerprint,
            "Iteration training binding changed",
        )?;
        match settings.training.maximum_training_rows {
            Some(limit) => require(
                self.training_rows <= u64::from(limit),
                "Training sample exceeds its limit",
            ),
            None => require(
                self.training_dataset == self.qualified_dataset,
                "Unsampled training must use the complete qualified dataset",
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationDevelopmentResult {
    pub training_binding_fingerprint: String,
    pub experiment_run_id: Uuid,
    pub journal_head: String,
    pub output: TrainOutput,
    pub reports: BTreeMap<String, EvaluationReport>,
    pub assessments: BTreeMap<String, CandidateAssessment>,
    pub development_passed: bool,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl IterationDevelopmentResult {
    /// Replays normal scientific evidence instead of accepting caller verdicts.
    pub fn from_journal(
        binding: &IterationTrainingBinding,
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        events: &[ExperimentEvent],
    ) -> Result<Self, Invalid> {
        require(
            binding.scientific_project == bound(project.id, &project.fingerprint)
                && binding.protocol == bound(protocol.id, &protocol.fingerprint)
                && protocol.budget.maximum_sealed_evaluations == 0
                && protocol.candidates.len() == 1
                && binding.candidate
                    == bound(
                        protocol.candidates[0].id,
                        &protocol.candidates[0].fingerprint,
                    ),
            "Iteration scientific evidence belongs to another training binding",
        )?;
        let view = replay_experiment(project, protocol, events).map_err(invalid)?;
        require(
            view.run_id == binding.experiment_run_id
                && view.sealed_authorized_by.is_none()
                && view.sealed_report.is_none()
                && view.sealed_assessment.is_none(),
            "Iteration must contain development evidence only",
        )?;
        let candidate = view
            .candidates
            .get(&protocol.candidates[0].id)
            .ok_or_else(|| Invalid("Candidate is missing".into()))?;
        require(
            candidate.state == CandidateExecutionState::DevelopmentCompleted
                && candidate
                    .development_reports
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    == protocol.development_suite_keys(),
            "Iteration requires successful execution of every development suite",
        )?;
        let mut result = Self {
            training_binding_fingerprint: binding.fingerprint.clone(),
            experiment_run_id: view.run_id,
            journal_head: view.last_event_fingerprint,
            output: candidate
                .train_output
                .clone()
                .ok_or_else(|| Invalid("Trained output missing".into()))?,
            reports: candidate.development_reports.clone(),
            assessments: candidate.development_assessments.clone(),
            development_passed: candidate
                .development_assessments
                .values()
                .all(|assessment| assessment.verdict == CandidateVerdict::Passed),
            created_at: view.updated_at,
            fingerprint: String::new(),
        };
        result.fingerprint = result.reproduce()?;
        Ok(result)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        fingerprint(self)
    }
}

fn fingerprint(value: &impl Serialize) -> Result<String, Invalid> {
    let mut object = serde_json::to_value(value).map_err(invalid)?;
    object
        .as_object_mut()
        .expect("record is an object")
        .remove("fingerprint");
    artifact_core::fingerprint(&object).map_err(invalid)
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
