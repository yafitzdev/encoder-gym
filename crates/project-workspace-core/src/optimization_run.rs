//! Project-owned root and preparation journal for one input-first optimization run.
//!
//! Slice-specific generation, training, evaluation, and advisor records remain
//! in their owning stores. This root binds them to the exact launch authority
//! without translating the user's inputs into a legacy repair recipe.

use crate::{
    BoundIdentity, Invalid, OptimizationLaunchAuthorization, OptimizationSetup, require,
    validate_name,
};
use chrono::{DateTime, Utc};
use dataset_core::versions::DatasetVersionRef;
use encoder_experiment_core::domain::{EvidenceRole, ExternalArtifactIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub launch: BoundIdentity,
    pub setup: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationRun {
    pub fn reserve(
        id: Uuid,
        launch: &OptimizationLaunchAuthorization,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        launch.validate_identity()?;
        let mut value = Self {
            id,
            project_id: launch.scope.project_id,
            launch: BoundIdentity {
                id: launch.id.to_string(),
                fingerprint: launch.fingerprint.clone(),
            },
            setup: launch.scope.setup.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate(launch)?;
        Ok(value)
    }

    pub fn identity(&self) -> BoundIdentity {
        BoundIdentity {
            id: self.id.to_string(),
            fingerprint: self.fingerprint.clone(),
        }
    }

    /// Stable slice-child identity that survives a process interruption before
    /// the child can be attached to this journal.
    pub fn child_id(&self, kind: &str, sequence: u64) -> Result<Uuid, Invalid> {
        validate_name(kind)?;
        require(
            sequence > 0,
            "Optimization child sequence must be positive.",
        )?;
        let mut hash = Sha256::new();
        hash.update(b"project-optimization-child-v1");
        hash.update(self.fingerprint.as_bytes());
        hash.update(kind.as_bytes());
        hash.update(sequence.to_le_bytes());
        let mut bytes: [u8; 16] = hash.finalize()[..16]
            .try_into()
            .expect("SHA-256 contains 16 UUID bytes");
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(bytes))
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "id": self.id,
            "projectId": self.project_id,
            "launch": self.launch,
            "setup": self.setup,
            "createdAt": self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate(&self, launch: &OptimizationLaunchAuthorization) -> Result<(), Invalid> {
        launch.validate_identity()?;
        self.launch.validate("Optimization launch")?;
        self.setup.validate("Optimization setup")?;
        require(
            !self.id.is_nil()
                && self.project_id == launch.scope.project_id
                && self.launch.id == launch.id.to_string()
                && self.launch.fingerprint == launch.fingerprint
                && self.setup == launch.scope.setup
                && self.reproduce()? == self.fingerprint,
            "Project optimization run changed or does not match its launch authority.",
        )
    }
}

/// Verified, row-free handoff from project custody to the compiled task adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationPreparation {
    pub run: BoundIdentity,
    pub setup: BoundIdentity,
    pub model: BoundIdentity,
    pub dataset: DatasetVersionRef,
    pub benchmark: BoundIdentity,
    pub provider_catalog: BoundIdentity,
    pub execution_binding: BoundIdentity,
    pub runtime_project: BoundIdentity,
    pub adapter: BoundIdentity,
    pub dataset_rows: u64,
    pub development_suites: Vec<String>,
    pub final_suite: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationPreparation {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
        execution_binding: BoundIdentity,
        runtime_project: BoundIdentity,
        adapter: BoundIdentity,
        dataset_rows: u64,
        mut development_suites: Vec<String>,
        final_suite: String,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        development_suites.sort();
        let mut value = Self {
            run: run.identity(),
            setup: run.setup.clone(),
            model: setup.inputs.model.clone(),
            dataset: setup.inputs.dataset.clone(),
            benchmark: setup.inputs.benchmark.clone(),
            provider_catalog: launch.scope.provider_catalog.clone(),
            execution_binding,
            runtime_project,
            adapter,
            dataset_rows,
            development_suites,
            final_suite,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_for(run, launch, setup)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "run":self.run,"setup":self.setup,"model":self.model,
            "dataset":self.dataset,"benchmark":self.benchmark,
            "providerCatalog":self.provider_catalog,
            "executionBinding":self.execution_binding,
            "runtimeProject":self.runtime_project,"adapter":self.adapter,
            "datasetRows":self.dataset_rows,
            "developmentSuites":self.development_suites,
            "finalSuite":self.final_suite,"createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_identity(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
    ) -> Result<(), Invalid> {
        run.validate(launch)?;
        for (label, value) in [
            ("Optimization run", &self.run),
            ("Optimization setup", &self.setup),
            ("Baseline model", &self.model),
            ("Benchmark", &self.benchmark),
            ("Provider catalog", &self.provider_catalog),
            ("Execution binding", &self.execution_binding),
            ("Runtime project", &self.runtime_project),
            ("Task adapter", &self.adapter),
        ] {
            value.validate(label)?;
        }
        self.dataset
            .validate()
            .map_err(|error| Invalid(error.to_string()))?;
        validate_name(&self.final_suite)?;
        let mut suites = BTreeSet::new();
        for suite in &self.development_suites {
            validate_name(suite)?;
            require(
                suites.insert(suite) && suite != &self.final_suite,
                "Evaluation suite identities must be unique.",
            )?;
        }
        let expected_suites = launch.scope.limits.maximum_development_evaluations
            / launch.scope.limits.maximum_models;
        require(
            self.run == run.identity()
                && self.setup == run.setup
                && self.dataset.project_id == run.project_id
                && self.provider_catalog == launch.scope.provider_catalog
                && self.dataset_rows > 0
                && u32::try_from(self.development_suites.len()).ok() == Some(expected_suites)
                && expected_suites > 0
                && self.reproduce()? == self.fingerprint,
            "Optimization preparation changed or does not match its run authority.",
        )
    }

    pub fn validate_for(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
    ) -> Result<(), Invalid> {
        self.validate_identity(run, launch)?;
        setup.validate_identity()?;
        require(
            self.setup.id == setup.id.to_string()
                && self.setup.fingerprint == setup.fingerprint
                && self.model == setup.inputs.model
                && self.dataset == setup.inputs.dataset
                && self.benchmark == setup.inputs.benchmark,
            "Optimization preparation does not match its selected inputs.",
        )
    }
}

/// Row-free handoff proving that the selected managed version became one
/// immutable task-native training artifact and scientific project snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationMaterialization {
    pub run: BoundIdentity,
    pub preparation_fingerprint: String,
    pub dataset: DatasetVersionRef,
    pub native_materialization: BoundIdentity,
    pub training_artifact: ExternalArtifactIdentity,
    pub scientific_project: BoundIdentity,
    pub adapter: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Row-free link to the first finite encoder experiment. Creating this link
/// reserves work but does not itself train or evaluate a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationExperiment {
    pub run: BoundIdentity,
    pub materialization_fingerprint: String,
    pub scientific_project: BoundIdentity,
    pub benchmark: BoundIdentity,
    pub source_protocol: BoundIdentity,
    pub candidate: BoundIdentity,
    pub protocol: BoundIdentity,
    pub experiment_run: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Row-free outcome of training the attached candidate and comparing it on
/// the shared development benchmark. Final holdout evidence remains a
/// separate, explicitly governed transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationOutcomeKind {
    CandidateReady,
    BaselineRetained,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationOutcome {
    pub run: BoundIdentity,
    pub experiment_fingerprint: String,
    pub experiment_run: BoundIdentity,
    pub kind: ProjectOptimizationOutcomeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<BoundIdentity>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationFinalResultKind {
    CandidateAccepted,
    CandidateRejected,
}

/// Row-free result of the single final evaluation authorized by the launch.
/// Metric values remain in the evaluation slice's immutable report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationFinalResult {
    pub run: BoundIdentity,
    pub outcome_fingerprint: String,
    pub experiment_run: BoundIdentity,
    pub kind: ProjectOptimizationFinalResultKind,
    pub model: BoundIdentity,
    pub final_report: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationFinalResult {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
        experiment: &ProjectOptimizationExperiment,
        outcome: &ProjectOptimizationOutcome,
        experiment_run: BoundIdentity,
        kind: ProjectOptimizationFinalResultKind,
        final_report: BoundIdentity,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        outcome.validate_for(run, launch, preparation, materialization, experiment)?;
        let model = outcome.selected_model.clone().ok_or_else(|| {
            Invalid("Final evaluation requires a development-selected model.".into())
        })?;
        let mut value = Self {
            run: run.identity(),
            outcome_fingerprint: outcome.fingerprint.clone(),
            experiment_run,
            kind,
            model,
            final_report,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_for(
            run,
            launch,
            preparation,
            materialization,
            experiment,
            outcome,
        )?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "run":self.run,"outcomeFingerprint":self.outcome_fingerprint,
            "experimentRun":self.experiment_run,"kind":self.kind,
            "model":self.model,"finalReport":self.final_report,
            "createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn validate_for(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
        experiment: &ProjectOptimizationExperiment,
        outcome: &ProjectOptimizationOutcome,
    ) -> Result<(), Invalid> {
        outcome.validate_for(run, launch, preparation, materialization, experiment)?;
        self.run.validate("Optimization run")?;
        self.experiment_run.validate("Experiment run")?;
        self.model.validate("Selected model")?;
        self.final_report.validate("Final evaluation report")?;
        require(
            self.run == run.identity()
                && outcome.kind == ProjectOptimizationOutcomeKind::CandidateReady
                && self.outcome_fingerprint == outcome.fingerprint
                && self.experiment_run.id == experiment.experiment_run.id
                && self.experiment_run.fingerprint != outcome.experiment_run.fingerprint
                && outcome.selected_model.as_ref() == Some(&self.model)
                && Uuid::parse_str(&self.final_report.id).is_ok_and(|id| !id.is_nil())
                && self.created_at >= outcome.created_at
                && self.reproduce()? == self.fingerprint,
            "Final optimization result changed or does not match its selected candidate.",
        )
    }
}

impl ProjectOptimizationOutcome {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
        experiment: &ProjectOptimizationExperiment,
        experiment_run: BoundIdentity,
        kind: ProjectOptimizationOutcomeKind,
        selected_model: Option<BoundIdentity>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        experiment.validate_for(run, launch, preparation, materialization)?;
        let mut value = Self {
            run: run.identity(),
            experiment_fingerprint: experiment.fingerprint.clone(),
            experiment_run,
            kind,
            selected_model,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_for(run, launch, preparation, materialization, experiment)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "run":self.run,"experimentFingerprint":self.experiment_fingerprint,
            "experimentRun":self.experiment_run,"kind":self.kind,
            "selectedModel":self.selected_model,"createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_for(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
        experiment: &ProjectOptimizationExperiment,
    ) -> Result<(), Invalid> {
        experiment.validate_for(run, launch, preparation, materialization)?;
        self.run.validate("Optimization run")?;
        self.experiment_run.validate("Experiment run")?;
        if let Some(model) = &self.selected_model {
            model.validate("Selected model")?;
        }
        let outcome_is_consistent = match self.kind {
            ProjectOptimizationOutcomeKind::CandidateReady => self.selected_model.is_some(),
            ProjectOptimizationOutcomeKind::BaselineRetained => self.selected_model.is_none(),
        };
        require(
            self.run == run.identity()
                && self.experiment_fingerprint == experiment.fingerprint
                && self.experiment_run.id == experiment.experiment_run.id
                && self.experiment_run.fingerprint != experiment.experiment_run.fingerprint
                && outcome_is_consistent
                && self.created_at >= experiment.created_at
                && self.reproduce()? == self.fingerprint,
            "Optimization outcome changed or does not match its attached experiment.",
        )
    }
}

impl ProjectOptimizationExperiment {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
        source_protocol: BoundIdentity,
        candidate: BoundIdentity,
        protocol: BoundIdentity,
        experiment_run: BoundIdentity,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        materialization.validate_for(run, launch, preparation)?;
        let mut value = Self {
            run: run.identity(),
            materialization_fingerprint: materialization.fingerprint.clone(),
            scientific_project: materialization.scientific_project.clone(),
            benchmark: preparation.benchmark.clone(),
            source_protocol,
            candidate,
            protocol,
            experiment_run,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_for(run, launch, preparation, materialization)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "run":self.run,"materializationFingerprint":self.materialization_fingerprint,
            "scientificProject":self.scientific_project,"benchmark":self.benchmark,
            "sourceProtocol":self.source_protocol,"candidate":self.candidate,
            "protocol":self.protocol,"experimentRun":self.experiment_run,
            "createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_for(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        materialization: &ProjectOptimizationMaterialization,
    ) -> Result<(), Invalid> {
        materialization.validate_for(run, launch, preparation)?;
        for (label, value) in [
            ("Optimization run", &self.run),
            ("Scientific project", &self.scientific_project),
            ("Benchmark", &self.benchmark),
            ("Source protocol", &self.source_protocol),
            ("Candidate", &self.candidate),
            ("Experiment protocol", &self.protocol),
            ("Experiment run", &self.experiment_run),
        ] {
            value.validate(label)?;
        }
        require(
            self.run == run.identity()
                && self.materialization_fingerprint == materialization.fingerprint
                && self.scientific_project == materialization.scientific_project
                && self.benchmark == preparation.benchmark
                && Uuid::parse_str(&self.source_protocol.id).is_ok_and(|id| !id.is_nil())
                && self.candidate.id == run.child_id("candidate", 1)?.to_string()
                && self.protocol.id == run.child_id("experiment-protocol", 1)?.to_string()
                && self.experiment_run.id == run.child_id("experiment-run", 1)?.to_string()
                && self.created_at >= materialization.created_at
                && self.reproduce()? == self.fingerprint,
            "Attached experiment changed or does not match its materialized run.",
        )
    }
}

impl ProjectOptimizationMaterialization {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
        native_materialization: BoundIdentity,
        training_artifact: ExternalArtifactIdentity,
        scientific_project: BoundIdentity,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            run: run.identity(),
            preparation_fingerprint: preparation.fingerprint.clone(),
            dataset: preparation.dataset.clone(),
            native_materialization,
            training_artifact,
            scientific_project,
            adapter: preparation.adapter.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_for(run, launch, preparation)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "run":self.run,"preparationFingerprint":self.preparation_fingerprint,
            "dataset":self.dataset,"nativeMaterialization":self.native_materialization,
            "trainingArtifact":self.training_artifact,
            "scientificProject":self.scientific_project,"adapter":self.adapter,
            "createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_for(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        preparation: &ProjectOptimizationPreparation,
    ) -> Result<(), Invalid> {
        preparation.validate_identity(run, launch)?;
        self.run.validate("Optimization run")?;
        self.native_materialization
            .validate("Native dataset materialization")?;
        self.scientific_project.validate("Scientific project")?;
        self.adapter.validate("Task adapter")?;
        self.dataset
            .validate()
            .map_err(|error| Invalid(error.to_string()))?;
        self.training_artifact
            .validate()
            .map_err(|error| Invalid(error.to_string()))?;
        require(
            self.run == run.identity()
                && self.preparation_fingerprint == preparation.fingerprint
                && self.dataset == preparation.dataset
                && self.training_artifact.role == EvidenceRole::Training
                && self.adapter == preparation.adapter
                && self.created_at >= preparation.created_at
                && self.reproduce()? == self.fingerprint,
            "Optimization materialization changed or does not match its preparation.",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationEventKind {
    Reserved,
    PreparationStarted,
    PreparationCompleted,
    PreparationFailed,
    MaterializationStarted,
    MaterializationCompleted,
    MaterializationFailed,
    ExperimentAttachmentStarted,
    ExperimentAttached,
    ExperimentAttachmentFailed,
    ExecutionStarted,
    ExecutionCompleted,
    ExecutionFailed,
    FinalEvaluationStarted,
    FinalEvaluationCompleted,
    FinalEvaluationFailed,
    Cancelled,
}

impl ProjectOptimizationEventKind {
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::PreparationStarted => "preparation_started",
            Self::PreparationCompleted => "preparation_completed",
            Self::PreparationFailed => "preparation_failed",
            Self::MaterializationStarted => "materialization_started",
            Self::MaterializationCompleted => "materialization_completed",
            Self::MaterializationFailed => "materialization_failed",
            Self::ExperimentAttachmentStarted => "experiment_attachment_started",
            Self::ExperimentAttached => "experiment_attached",
            Self::ExperimentAttachmentFailed => "experiment_attachment_failed",
            Self::ExecutionStarted => "execution_started",
            Self::ExecutionCompleted => "execution_completed",
            Self::ExecutionFailed => "execution_failed",
            Self::FinalEvaluationStarted => "final_evaluation_started",
            Self::FinalEvaluationCompleted => "final_evaluation_completed",
            Self::FinalEvaluationFailed => "final_evaluation_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationEvent {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u64,
    pub previous_event_fingerprint: Option<String>,
    pub kind: ProjectOptimizationEventKind,
    pub launch: BoundIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ProjectOptimizationPreparation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<ProjectOptimizationMaterialization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<ProjectOptimizationExperiment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProjectOptimizationOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_result: Option<ProjectOptimizationFinalResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationEvent {
    pub fn reserved(
        id: Uuid,
        run: &ProjectOptimizationRun,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            run_id: run.id,
            sequence: 1,
            previous_event_fingerprint: None,
            kind: ProjectOptimizationEventKind::Reserved,
            launch: run.launch.clone(),
            attempt: None,
            preparation: None,
            materialization: None,
            experiment: None,
            outcome: None,
            final_result: None,
            failure_code: None,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_first(run)?;
        Ok(value)
    }

    pub fn preparation_started(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::PreparationStarted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn preparation_completed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        preparation: ProjectOptimizationPreparation,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::PreparationCompleted,
            attempt,
            Some(preparation),
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn preparation_failed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::PreparationFailed,
            attempt,
            None,
            None,
            None,
            Some(failure_code.into()),
            None,
            None,
            created_at,
        )
    }

    pub fn materialization_started(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::MaterializationStarted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn materialization_completed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        materialization: ProjectOptimizationMaterialization,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::MaterializationCompleted,
            attempt,
            None,
            Some(materialization),
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn materialization_failed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::MaterializationFailed,
            attempt,
            None,
            None,
            None,
            Some(failure_code.into()),
            None,
            None,
            created_at,
        )
    }

    pub fn experiment_attachment_started(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExperimentAttachmentStarted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn experiment_attached(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        experiment: ProjectOptimizationExperiment,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExperimentAttached,
            attempt,
            None,
            None,
            Some(experiment),
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn experiment_attachment_failed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExperimentAttachmentFailed,
            attempt,
            None,
            None,
            None,
            Some(failure_code.into()),
            None,
            None,
            created_at,
        )
    }

    pub fn execution_started(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExecutionStarted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn execution_completed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        outcome: ProjectOptimizationOutcome,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExecutionCompleted,
            attempt,
            None,
            None,
            None,
            None,
            Some(outcome),
            None,
            created_at,
        )
    }

    pub fn execution_failed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::ExecutionFailed,
            attempt,
            None,
            None,
            None,
            Some(failure_code.into()),
            None,
            None,
            created_at,
        )
    }

    pub fn final_evaluation_started(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::FinalEvaluationStarted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
        )
    }

    pub fn final_evaluation_completed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        result: ProjectOptimizationFinalResult,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::FinalEvaluationCompleted,
            attempt,
            None,
            None,
            None,
            None,
            None,
            Some(result),
            created_at,
        )
    }

    pub fn final_evaluation_failed(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        attempt: u32,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        Self::next(
            id,
            run,
            previous,
            ProjectOptimizationEventKind::FinalEvaluationFailed,
            attempt,
            None,
            None,
            None,
            Some(failure_code.into()),
            None,
            None,
            created_at,
        )
    }

    pub fn cancelled(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        failure_code: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            run_id: run.id,
            sequence: previous
                .sequence
                .checked_add(1)
                .ok_or_else(|| Invalid("Optimization journal sequence overflowed.".into()))?,
            previous_event_fingerprint: Some(previous.fingerprint.clone()),
            kind: ProjectOptimizationEventKind::Cancelled,
            launch: run.launch.clone(),
            attempt: None,
            preparation: None,
            materialization: None,
            experiment: None,
            outcome: None,
            final_result: None,
            failure_code: Some(failure_code.into()),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_after(run, previous)?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        id: Uuid,
        run: &ProjectOptimizationRun,
        previous: &Self,
        kind: ProjectOptimizationEventKind,
        attempt: u32,
        preparation: Option<ProjectOptimizationPreparation>,
        materialization: Option<ProjectOptimizationMaterialization>,
        experiment: Option<ProjectOptimizationExperiment>,
        failure_code: Option<String>,
        outcome: Option<ProjectOptimizationOutcome>,
        final_result: Option<ProjectOptimizationFinalResult>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            run_id: run.id,
            sequence: previous
                .sequence
                .checked_add(1)
                .ok_or_else(|| Invalid("Optimization journal sequence overflowed.".into()))?,
            previous_event_fingerprint: Some(previous.fingerprint.clone()),
            kind,
            launch: run.launch.clone(),
            attempt: Some(attempt),
            preparation,
            materialization,
            experiment,
            outcome,
            final_result,
            failure_code,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_after(run, previous)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        let value = if self.kind == ProjectOptimizationEventKind::Reserved {
            // Preserve fingerprints from the already shipped reservation schema.
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"createdAt":self.created_at,
            })
        } else if matches!(
            self.kind,
            ProjectOptimizationEventKind::PreparationStarted
                | ProjectOptimizationEventKind::PreparationCompleted
                | ProjectOptimizationEventKind::PreparationFailed
        ) {
            // Preserve fingerprints from the shipped preparation schema.
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"attempt":self.attempt,
                "preparation":self.preparation,"failureCode":self.failure_code,
                "createdAt":self.created_at,
            })
        } else if matches!(
            self.kind,
            ProjectOptimizationEventKind::MaterializationStarted
                | ProjectOptimizationEventKind::MaterializationCompleted
                | ProjectOptimizationEventKind::MaterializationFailed
        ) {
            // Preserve fingerprints from the shipped materialization schema.
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"attempt":self.attempt,
                "preparation":self.preparation,"materialization":self.materialization,
                "failureCode":self.failure_code,"createdAt":self.created_at,
            })
        } else if matches!(
            self.kind,
            ProjectOptimizationEventKind::ExperimentAttachmentStarted
                | ProjectOptimizationEventKind::ExperimentAttached
                | ProjectOptimizationEventKind::ExperimentAttachmentFailed
        ) {
            // Preserve fingerprints from the shipped experiment-attachment schema.
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"attempt":self.attempt,
                "preparation":self.preparation,"materialization":self.materialization,
                "experiment":self.experiment,"failureCode":self.failure_code,
                "createdAt":self.created_at,
            })
        } else if matches!(
            self.kind,
            ProjectOptimizationEventKind::ExecutionStarted
                | ProjectOptimizationEventKind::ExecutionCompleted
                | ProjectOptimizationEventKind::ExecutionFailed
        ) {
            // Preserve fingerprints from the shipped candidate-execution schema.
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"attempt":self.attempt,
                "preparation":self.preparation,"materialization":self.materialization,
                "experiment":self.experiment,"outcome":self.outcome,
                "failureCode":self.failure_code,"createdAt":self.created_at,
            })
        } else {
            serde_json::json!({
                "id":self.id,"runId":self.run_id,"sequence":self.sequence,
                "previousEventFingerprint":self.previous_event_fingerprint,
                "kind":self.kind,"launch":self.launch,"attempt":self.attempt,
                "preparation":self.preparation,"materialization":self.materialization,
                "experiment":self.experiment,"outcome":self.outcome,
                "finalResult":self.final_result,"failureCode":self.failure_code,
                "createdAt":self.created_at,
            })
        };
        artifact_core::fingerprint(&value).map_err(|error| Invalid(error.to_string()))
    }

    fn validate_common(&self, run: &ProjectOptimizationRun) -> Result<(), Invalid> {
        self.launch.validate("Optimization launch")?;
        require(
            !self.id.is_nil()
                && self.run_id == run.id
                && self.launch == run.launch
                && self.reproduce()? == self.fingerprint,
            "Project optimization event changed or is invalid.",
        )
    }

    fn validate_first(&self, run: &ProjectOptimizationRun) -> Result<(), Invalid> {
        self.validate_common(run)?;
        require(
            self.sequence == 1
                && self.previous_event_fingerprint.is_none()
                && self.kind == ProjectOptimizationEventKind::Reserved
                && self.attempt.is_none()
                && self.preparation.is_none()
                && self.materialization.is_none()
                && self.experiment.is_none()
                && self.outcome.is_none()
                && self.final_result.is_none()
                && self.failure_code.is_none()
                && self.created_at == run.created_at,
            "Project optimization reservation event changed or is invalid.",
        )
    }

    fn validate_after(&self, run: &ProjectOptimizationRun, previous: &Self) -> Result<(), Invalid> {
        self.validate_common(run)?;
        let attempt = self.attempt.unwrap_or_default();
        require(
            self.sequence == previous.sequence.checked_add(1).unwrap_or(0)
                && self.previous_event_fingerprint.as_deref()
                    == Some(previous.fingerprint.as_str())
                && self.created_at >= previous.created_at
                && (self.kind == ProjectOptimizationEventKind::Cancelled || attempt > 0),
            "Optimization preparation event does not continue the journal.",
        )?;
        match self.kind {
            ProjectOptimizationEventKind::Reserved => Err(Invalid(
                "A reservation can only be the first optimization event.".into(),
            )),
            ProjectOptimizationEventKind::PreparationStarted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Preparation start cannot contain a result or failure.",
            ),
            ProjectOptimizationEventKind::PreparationCompleted => require(
                self.preparation.is_some()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Preparation completion requires exactly one verified receipt.",
            ),
            ProjectOptimizationEventKind::PreparationFailed => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Preparation failure requires a safe stable code.",
                )
            }
            ProjectOptimizationEventKind::MaterializationStarted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Materialization start cannot contain a result or failure.",
            ),
            ProjectOptimizationEventKind::MaterializationCompleted => require(
                self.preparation.is_none()
                    && self.materialization.is_some()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Materialization completion requires exactly one verified receipt.",
            ),
            ProjectOptimizationEventKind::MaterializationFailed => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Materialization failure requires a safe stable code.",
                )
            }
            ProjectOptimizationEventKind::ExperimentAttachmentStarted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Experiment attachment start cannot contain a result or failure.",
            ),
            ProjectOptimizationEventKind::ExperimentAttached => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_some()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Experiment attachment requires exactly one child receipt.",
            ),
            ProjectOptimizationEventKind::ExperimentAttachmentFailed => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Experiment attachment failure requires a safe stable code.",
                )
            }
            ProjectOptimizationEventKind::ExecutionStarted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Optimization execution start cannot contain a result or failure.",
            ),
            ProjectOptimizationEventKind::ExecutionCompleted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_some()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Optimization execution completion requires exactly one outcome.",
            ),
            ProjectOptimizationEventKind::ExecutionFailed => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Optimization execution failure requires a safe stable code.",
                )
            }
            ProjectOptimizationEventKind::FinalEvaluationStarted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_none()
                    && self.failure_code.is_none(),
                "Final evaluation start cannot contain a result or failure.",
            ),
            ProjectOptimizationEventKind::FinalEvaluationCompleted => require(
                self.preparation.is_none()
                    && self.materialization.is_none()
                    && self.experiment.is_none()
                    && self.outcome.is_none()
                    && self.final_result.is_some()
                    && self.failure_code.is_none(),
                "Final evaluation completion requires exactly one result.",
            ),
            ProjectOptimizationEventKind::FinalEvaluationFailed => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Final evaluation failure requires a safe stable code.",
                )
            }
            ProjectOptimizationEventKind::Cancelled => {
                let code = self.failure_code.as_deref().unwrap_or_default();
                require(
                    self.attempt.is_none()
                        && self.preparation.is_none()
                        && self.materialization.is_none()
                        && self.experiment.is_none()
                        && self.outcome.is_none()
                        && self.final_result.is_none()
                        && !code.is_empty()
                        && code.len() <= 80
                        && code.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-' | b'.')
                        }),
                    "Optimization cancellation requires a safe stable code.",
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationRunState {
    Queued,
    Preparing,
    Ready,
    PreparationFailed,
    Materializing,
    Materialized,
    MaterializationFailed,
    AttachingExperiment,
    ReadyToRun,
    ExperimentAttachmentFailed,
    Optimizing,
    ReadyForFinalEvaluation,
    BaselineRetained,
    ExecutionFailed,
    EvaluatingFinal,
    CandidateAccepted,
    CandidateRejected,
    FinalEvaluationFailed,
    Cancelled,
    AgentRunning,
    AgentStopping,
    AgentPaused,
    AgentInterrupted,
    AgentFailed,
    AgentBudgetExhausted,
    AgentCompleted,
}

impl ProjectOptimizationRunState {
    pub const fn has_preparation(self) -> bool {
        matches!(
            self,
            Self::Ready
                | Self::Materializing
                | Self::Materialized
                | Self::MaterializationFailed
                | Self::AttachingExperiment
                | Self::ReadyToRun
                | Self::ExperimentAttachmentFailed
                | Self::Optimizing
                | Self::ReadyForFinalEvaluation
                | Self::BaselineRetained
                | Self::ExecutionFailed
                | Self::EvaluatingFinal
                | Self::CandidateAccepted
                | Self::CandidateRejected
                | Self::FinalEvaluationFailed
        )
    }

    pub const fn has_materialization(self) -> bool {
        matches!(
            self,
            Self::Materialized
                | Self::AttachingExperiment
                | Self::ReadyToRun
                | Self::ExperimentAttachmentFailed
                | Self::Optimizing
                | Self::ReadyForFinalEvaluation
                | Self::BaselineRetained
                | Self::ExecutionFailed
                | Self::EvaluatingFinal
                | Self::CandidateAccepted
                | Self::CandidateRejected
                | Self::FinalEvaluationFailed
        )
    }

    pub const fn has_experiment(self) -> bool {
        matches!(
            self,
            Self::ReadyToRun
                | Self::Optimizing
                | Self::ReadyForFinalEvaluation
                | Self::BaselineRetained
                | Self::ExecutionFailed
                | Self::EvaluatingFinal
                | Self::CandidateAccepted
                | Self::CandidateRejected
                | Self::FinalEvaluationFailed
        )
    }

    pub const fn has_outcome(self) -> bool {
        matches!(
            self,
            Self::ReadyForFinalEvaluation
                | Self::BaselineRetained
                | Self::EvaluatingFinal
                | Self::CandidateAccepted
                | Self::CandidateRejected
                | Self::FinalEvaluationFailed
        )
    }

    pub const fn has_final_result(self) -> bool {
        matches!(self, Self::CandidateAccepted | Self::CandidateRejected)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::BaselineRetained
                | Self::CandidateAccepted
                | Self::CandidateRejected
                | Self::Cancelled
                | Self::AgentBudgetExhausted
                | Self::AgentCompleted
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationRunView {
    pub run: ProjectOptimizationRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_execution: Option<crate::optimization_execution::AgentExecutionView>,
    pub state: ProjectOptimizationRunState,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ProjectOptimizationPreparation>,
    pub materialization_attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<ProjectOptimizationMaterialization>,
    pub experiment_attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<ProjectOptimizationExperiment>,
    pub execution_attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProjectOptimizationOutcome>,
    pub final_attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_result: Option<ProjectOptimizationFinalResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub last_sequence: u64,
    pub head_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

pub fn replay_project_optimization(
    run: &ProjectOptimizationRun,
    launch: &OptimizationLaunchAuthorization,
    events: &[ProjectOptimizationEvent],
) -> Result<ProjectOptimizationRunView, Invalid> {
    run.validate(launch)?;
    let first = events
        .first()
        .ok_or_else(|| Invalid("Project optimization journal is missing.".into()))?;
    first.validate_first(run)?;
    let mut state = ProjectOptimizationRunState::Queued;
    let mut attempt = 0;
    let mut preparation = None;
    let mut materialization_attempt = 0;
    let mut materialization = None;
    let mut experiment_attempt = 0;
    let mut experiment = None;
    let mut execution_attempt = 0;
    let mut outcome = None;
    let mut final_attempt = 0;
    let mut final_result = None;
    let mut failure_code = None;
    let mut previous = first;
    for event in &events[1..] {
        event.validate_after(run, previous)?;
        let event_attempt = event.attempt.unwrap_or_default();
        match event.kind {
            ProjectOptimizationEventKind::Reserved => {
                return Err(Invalid(
                    "Optimization journal repeats its reservation.".into(),
                ));
            }
            ProjectOptimizationEventKind::PreparationStarted => {
                require(
                    matches!(
                        state,
                        ProjectOptimizationRunState::Queued
                            | ProjectOptimizationRunState::PreparationFailed
                    ) && event_attempt == attempt + 1,
                    "Optimization preparation cannot start from this state.",
                )?;
                state = ProjectOptimizationRunState::Preparing;
                attempt = event_attempt;
                failure_code = None;
            }
            ProjectOptimizationEventKind::PreparationCompleted => {
                require(
                    state == ProjectOptimizationRunState::Preparing && event_attempt == attempt,
                    "Optimization preparation completion has no matching active attempt.",
                )?;
                let receipt = event.preparation.clone().expect("validated preparation");
                receipt.validate_identity(run, launch)?;
                state = ProjectOptimizationRunState::Ready;
                preparation = Some(receipt);
            }
            ProjectOptimizationEventKind::PreparationFailed => {
                require(
                    state == ProjectOptimizationRunState::Preparing && event_attempt == attempt,
                    "Optimization preparation failure has no matching active attempt.",
                )?;
                state = ProjectOptimizationRunState::PreparationFailed;
                failure_code.clone_from(&event.failure_code);
            }
            ProjectOptimizationEventKind::MaterializationStarted => {
                require(
                    matches!(
                        state,
                        ProjectOptimizationRunState::Ready
                            | ProjectOptimizationRunState::MaterializationFailed
                    ) && event_attempt == materialization_attempt + 1,
                    "Optimization materialization cannot start from this state.",
                )?;
                state = ProjectOptimizationRunState::Materializing;
                materialization_attempt = event_attempt;
                failure_code = None;
            }
            ProjectOptimizationEventKind::MaterializationCompleted => {
                require(
                    state == ProjectOptimizationRunState::Materializing
                        && event_attempt == materialization_attempt,
                    "Optimization materialization completion has no matching active attempt.",
                )?;
                let receipt = event
                    .materialization
                    .clone()
                    .expect("validated materialization");
                receipt.validate_for(
                    run,
                    launch,
                    preparation.as_ref().expect("ready state has preparation"),
                )?;
                state = ProjectOptimizationRunState::Materialized;
                materialization = Some(receipt);
            }
            ProjectOptimizationEventKind::MaterializationFailed => {
                require(
                    state == ProjectOptimizationRunState::Materializing
                        && event_attempt == materialization_attempt,
                    "Optimization materialization failure has no matching active attempt.",
                )?;
                state = ProjectOptimizationRunState::MaterializationFailed;
                failure_code.clone_from(&event.failure_code);
            }
            ProjectOptimizationEventKind::ExperimentAttachmentStarted => {
                require(
                    matches!(
                        state,
                        ProjectOptimizationRunState::Materialized
                            | ProjectOptimizationRunState::ExperimentAttachmentFailed
                    ) && event_attempt == experiment_attempt + 1,
                    "Experiment attachment cannot start from this state.",
                )?;
                state = ProjectOptimizationRunState::AttachingExperiment;
                experiment_attempt = event_attempt;
                failure_code = None;
            }
            ProjectOptimizationEventKind::ExperimentAttached => {
                require(
                    state == ProjectOptimizationRunState::AttachingExperiment
                        && event_attempt == experiment_attempt,
                    "Experiment attachment has no matching active attempt.",
                )?;
                let receipt = event.experiment.clone().expect("validated experiment");
                receipt.validate_for(
                    run,
                    launch,
                    preparation.as_ref().expect("prepared run has preparation"),
                    materialization
                        .as_ref()
                        .expect("materialized run has materialization"),
                )?;
                state = ProjectOptimizationRunState::ReadyToRun;
                experiment = Some(receipt);
            }
            ProjectOptimizationEventKind::ExperimentAttachmentFailed => {
                require(
                    state == ProjectOptimizationRunState::AttachingExperiment
                        && event_attempt == experiment_attempt,
                    "Experiment attachment failure has no matching active attempt.",
                )?;
                state = ProjectOptimizationRunState::ExperimentAttachmentFailed;
                failure_code.clone_from(&event.failure_code);
            }
            ProjectOptimizationEventKind::ExecutionStarted => {
                require(
                    matches!(
                        state,
                        ProjectOptimizationRunState::ReadyToRun
                            | ProjectOptimizationRunState::ExecutionFailed
                    ) && event_attempt == execution_attempt + 1,
                    "Optimization execution cannot start from this state.",
                )?;
                state = ProjectOptimizationRunState::Optimizing;
                execution_attempt = event_attempt;
                failure_code = None;
            }
            ProjectOptimizationEventKind::ExecutionCompleted => {
                require(
                    state == ProjectOptimizationRunState::Optimizing
                        && event_attempt == execution_attempt,
                    "Optimization execution completion has no matching active attempt.",
                )?;
                let receipt = event.outcome.clone().expect("validated outcome");
                receipt.validate_for(
                    run,
                    launch,
                    preparation.as_ref().expect("prepared run has preparation"),
                    materialization
                        .as_ref()
                        .expect("materialized run has materialization"),
                    experiment.as_ref().expect("attached run has experiment"),
                )?;
                state = match receipt.kind {
                    ProjectOptimizationOutcomeKind::CandidateReady => {
                        ProjectOptimizationRunState::ReadyForFinalEvaluation
                    }
                    ProjectOptimizationOutcomeKind::BaselineRetained => {
                        ProjectOptimizationRunState::BaselineRetained
                    }
                };
                outcome = Some(receipt);
            }
            ProjectOptimizationEventKind::ExecutionFailed => {
                require(
                    state == ProjectOptimizationRunState::Optimizing
                        && event_attempt == execution_attempt,
                    "Optimization execution failure has no matching active attempt.",
                )?;
                state = ProjectOptimizationRunState::ExecutionFailed;
                failure_code.clone_from(&event.failure_code);
            }
            ProjectOptimizationEventKind::FinalEvaluationStarted => {
                require(
                    matches!(
                        state,
                        ProjectOptimizationRunState::ReadyForFinalEvaluation
                            | ProjectOptimizationRunState::FinalEvaluationFailed
                    ) && event_attempt == final_attempt + 1,
                    "Final evaluation cannot start from this state.",
                )?;
                state = ProjectOptimizationRunState::EvaluatingFinal;
                final_attempt = event_attempt;
                failure_code = None;
            }
            ProjectOptimizationEventKind::FinalEvaluationCompleted => {
                require(
                    state == ProjectOptimizationRunState::EvaluatingFinal
                        && event_attempt == final_attempt,
                    "Final evaluation completion has no matching active attempt.",
                )?;
                let receipt = event.final_result.clone().expect("validated final result");
                receipt.validate_for(
                    run,
                    launch,
                    preparation.as_ref().expect("prepared run has preparation"),
                    materialization
                        .as_ref()
                        .expect("materialized run has materialization"),
                    experiment.as_ref().expect("attached run has experiment"),
                    outcome.as_ref().expect("executed run has outcome"),
                )?;
                state = match receipt.kind {
                    ProjectOptimizationFinalResultKind::CandidateAccepted => {
                        ProjectOptimizationRunState::CandidateAccepted
                    }
                    ProjectOptimizationFinalResultKind::CandidateRejected => {
                        ProjectOptimizationRunState::CandidateRejected
                    }
                };
                final_result = Some(receipt);
            }
            ProjectOptimizationEventKind::FinalEvaluationFailed => {
                require(
                    state == ProjectOptimizationRunState::EvaluatingFinal
                        && event_attempt == final_attempt,
                    "Final evaluation failure has no matching active attempt.",
                )?;
                state = ProjectOptimizationRunState::FinalEvaluationFailed;
                failure_code.clone_from(&event.failure_code);
            }
            ProjectOptimizationEventKind::Cancelled => {
                require(
                    !state.is_terminal(),
                    "A completed optimization run cannot be cancelled.",
                )?;
                state = ProjectOptimizationRunState::Cancelled;
                failure_code.clone_from(&event.failure_code);
            }
        }
        previous = event;
    }
    Ok(ProjectOptimizationRunView {
        run: run.clone(),
        agent_execution: None,
        state,
        attempt,
        preparation,
        materialization_attempt,
        materialization,
        experiment_attempt,
        experiment,
        execution_attempt,
        outcome,
        final_attempt,
        final_result,
        failure_code,
        last_sequence: previous.sequence,
        head_fingerprint: previous.fingerprint.clone(),
        updated_at: previous.created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FinalEvaluationAuthorization, OptimizationExecutionLimits, OptimizationInputs,
        OptimizationLaunchScope, ProviderLimits,
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }
    fn reference(character: char) -> BoundIdentity {
        BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: digest(character),
        }
    }
    fn launch() -> OptimizationLaunchAuthorization {
        let provider = ProviderLimits {
            maximum_requests: 2,
            maximum_input_tokens: 1_000,
            maximum_output_tokens: 500,
            maximum_cost_microusd: 0,
        };
        let mut scope = OptimizationLaunchScope {
            project_id: Uuid::new_v4(),
            setup: reference('a'),
            provider_catalog: reference('b'),
            limits: OptimizationExecutionLimits {
                maximum_iterations: 3,
                maximum_models: 3,
                maximum_dataset_row_changes: 5_000,
                maximum_training_seconds: 21_600,
                maximum_development_evaluations: 6,
                maximum_final_evaluations: 1,
            },
            generation: provider.clone(),
            advisor: provider,
            final_evaluation: FinalEvaluationAuthorization::SelectedCandidateOnce,
            agentic: None,
            fingerprint: String::new(),
        };
        scope.fingerprint = scope.reproduce().unwrap();
        OptimizationLaunchAuthorization::create(Uuid::new_v4(), scope, "operator", Utc::now())
            .unwrap()
    }
    fn fixture() -> (OptimizationSetup, OptimizationLaunchAuthorization) {
        let mut launch = launch();
        let inputs = OptimizationInputs {
            project_id: launch.scope.project_id,
            baseline_revision: reference('c'),
            model: reference('d'),
            dataset: DatasetVersionRef {
                id: Uuid::new_v4(),
                dataset_id: Uuid::new_v4(),
                project_id: launch.scope.project_id,
                number: 1,
                fingerprint: digest('e'),
            },
            benchmark: reference('f'),
        };
        let setup = OptimizationSetup::create(Uuid::new_v4(), None, inputs, Utc::now()).unwrap();
        launch.scope.setup = BoundIdentity {
            id: setup.id.to_string(),
            fingerprint: setup.fingerprint.clone(),
        };
        launch.scope.fingerprint = launch.scope.reproduce().unwrap();
        launch.fingerprint = launch.reproduce().unwrap();
        (setup, launch)
    }
    fn preparation(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
    ) -> ProjectOptimizationPreparation {
        ProjectOptimizationPreparation::create(
            run,
            launch,
            setup,
            reference('1'),
            reference('2'),
            BoundIdentity {
                id: "nomos:nomos-ranking-v3".into(),
                fingerprint: digest('3'),
            },
            10,
            vec!["development-b".into(), "development-a".into()],
            "final".into(),
            Utc::now(),
        )
        .unwrap()
    }

    #[test]
    fn preparation_retries_are_ordered_and_finish_with_one_exact_receipt() {
        let (setup, launch) = fixture();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let reserved = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        let started = ProjectOptimizationEvent::preparation_started(
            Uuid::new_v4(),
            &run,
            &reserved,
            1,
            Utc::now(),
        )
        .unwrap();
        let failed = ProjectOptimizationEvent::preparation_failed(
            Uuid::new_v4(),
            &run,
            &started,
            1,
            "runtime_unavailable",
            Utc::now(),
        )
        .unwrap();
        let retried = ProjectOptimizationEvent::preparation_started(
            Uuid::new_v4(),
            &run,
            &failed,
            2,
            Utc::now(),
        )
        .unwrap();
        let receipt = preparation(&run, &launch, &setup);
        let completed = ProjectOptimizationEvent::preparation_completed(
            Uuid::new_v4(),
            &run,
            &retried,
            2,
            receipt.clone(),
            Utc::now(),
        )
        .unwrap();
        let events = [reserved, started, failed, retried, completed];
        let view = replay_project_optimization(&run, &launch, &events).unwrap();
        view.preparation
            .as_ref()
            .unwrap()
            .validate_for(&run, &launch, &setup)
            .unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::Ready);
        assert_eq!(view.attempt, 2);
        assert_eq!(view.preparation, Some(receipt));
        assert_eq!(view.last_sequence, 5);
    }

    #[test]
    fn prepared_data_materializes_once_and_retries_only_its_own_stage() {
        let (setup, launch) = fixture();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let reserved = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        let preparing =
            ProjectOptimizationEvent::preparation_started(Uuid::new_v4(), &run, &reserved, 1, now)
                .unwrap();
        let preparation = preparation(&run, &launch, &setup);
        let ready = ProjectOptimizationEvent::preparation_completed(
            Uuid::new_v4(),
            &run,
            &preparing,
            1,
            preparation.clone(),
            preparation.created_at,
        )
        .unwrap();
        let started = ProjectOptimizationEvent::materialization_started(
            Uuid::new_v4(),
            &run,
            &ready,
            1,
            Utc::now(),
        )
        .unwrap();
        let failed = ProjectOptimizationEvent::materialization_failed(
            Uuid::new_v4(),
            &run,
            &started,
            1,
            "native_row_invalid",
            Utc::now(),
        )
        .unwrap();
        let retried = ProjectOptimizationEvent::materialization_started(
            Uuid::new_v4(),
            &run,
            &failed,
            2,
            Utc::now(),
        )
        .unwrap();
        let artifact = ExternalArtifactIdentity::new(
            "runs/project/training.jsonl",
            EvidenceRole::Training,
            42,
            digest('7'),
        )
        .unwrap();
        let materialization = ProjectOptimizationMaterialization::create(
            &run,
            &launch,
            &preparation,
            reference('8'),
            artifact,
            reference('9'),
            Utc::now(),
        )
        .unwrap();
        let completed = ProjectOptimizationEvent::materialization_completed(
            Uuid::new_v4(),
            &run,
            &retried,
            2,
            materialization.clone(),
            materialization.created_at,
        )
        .unwrap();
        let view = replay_project_optimization(
            &run,
            &launch,
            &[
                reserved, preparing, ready, started, failed, retried, completed,
            ],
        )
        .unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::Materialized);
        assert_eq!(view.attempt, 1);
        assert_eq!(view.materialization_attempt, 2);
        assert_eq!(view.preparation, Some(preparation));
        assert_eq!(view.materialization, Some(materialization));
        assert!(view.failure_code.is_none());
    }

    #[test]
    fn materialized_run_attaches_and_executes_one_stable_finite_experiment() {
        let (setup, launch) = fixture();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        assert_eq!(
            run.child_id("candidate", 1).unwrap(),
            run.child_id("candidate", 1).unwrap()
        );
        assert_ne!(
            run.child_id("candidate", 1).unwrap(),
            run.child_id("experiment-run", 1).unwrap()
        );
        let reserved = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        let preparing =
            ProjectOptimizationEvent::preparation_started(Uuid::new_v4(), &run, &reserved, 1, now)
                .unwrap();
        let preparation = preparation(&run, &launch, &setup);
        let ready = ProjectOptimizationEvent::preparation_completed(
            Uuid::new_v4(),
            &run,
            &preparing,
            1,
            preparation.clone(),
            preparation.created_at,
        )
        .unwrap();
        let materializing = ProjectOptimizationEvent::materialization_started(
            Uuid::new_v4(),
            &run,
            &ready,
            1,
            Utc::now(),
        )
        .unwrap();
        let materialization = ProjectOptimizationMaterialization::create(
            &run,
            &launch,
            &preparation,
            reference('8'),
            ExternalArtifactIdentity::new(
                "runs/project/training.jsonl",
                EvidenceRole::Training,
                42,
                digest('7'),
            )
            .unwrap(),
            reference('9'),
            Utc::now(),
        )
        .unwrap();
        let materialized = ProjectOptimizationEvent::materialization_completed(
            Uuid::new_v4(),
            &run,
            &materializing,
            1,
            materialization.clone(),
            materialization.created_at,
        )
        .unwrap();
        let attaching = ProjectOptimizationEvent::experiment_attachment_started(
            Uuid::new_v4(),
            &run,
            &materialized,
            1,
            Utc::now(),
        )
        .unwrap();
        let experiment = ProjectOptimizationExperiment::create(
            &run,
            &launch,
            &preparation,
            &materialization,
            reference('a'),
            BoundIdentity {
                id: run.child_id("candidate", 1).unwrap().to_string(),
                fingerprint: digest('b'),
            },
            BoundIdentity {
                id: run.child_id("experiment-protocol", 1).unwrap().to_string(),
                fingerprint: digest('c'),
            },
            BoundIdentity {
                id: run.child_id("experiment-run", 1).unwrap().to_string(),
                fingerprint: digest('d'),
            },
            Utc::now(),
        )
        .unwrap();
        let attached = ProjectOptimizationEvent::experiment_attached(
            Uuid::new_v4(),
            &run,
            &attaching,
            1,
            experiment.clone(),
            experiment.created_at,
        )
        .unwrap();
        let view = replay_project_optimization(
            &run,
            &launch,
            &[
                reserved.clone(),
                preparing.clone(),
                ready.clone(),
                materializing.clone(),
                materialized.clone(),
                attaching.clone(),
                attached.clone(),
            ],
        )
        .unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::ReadyToRun);
        assert_eq!(view.experiment_attempt, 1);
        assert_eq!(view.experiment, Some(experiment.clone()));

        let optimizing = ProjectOptimizationEvent::execution_started(
            Uuid::new_v4(),
            &run,
            &attached,
            1,
            Utc::now(),
        )
        .unwrap();
        let failed = ProjectOptimizationEvent::execution_failed(
            Uuid::new_v4(),
            &run,
            &optimizing,
            1,
            "trainer_interrupted",
            Utc::now(),
        )
        .unwrap();
        let retried = ProjectOptimizationEvent::execution_started(
            Uuid::new_v4(),
            &run,
            &failed,
            2,
            Utc::now(),
        )
        .unwrap();
        let outcome = ProjectOptimizationOutcome::create(
            &run,
            &launch,
            &preparation,
            &materialization,
            &experiment,
            BoundIdentity {
                id: experiment.experiment_run.id.clone(),
                fingerprint: digest('e'),
            },
            ProjectOptimizationOutcomeKind::CandidateReady,
            Some(reference('f')),
            Utc::now(),
        )
        .unwrap();
        let completed = ProjectOptimizationEvent::execution_completed(
            Uuid::new_v4(),
            &run,
            &retried,
            2,
            outcome.clone(),
            Utc::now(),
        )
        .unwrap();
        let view = replay_project_optimization(
            &run,
            &launch,
            &[
                reserved.clone(),
                preparing.clone(),
                ready.clone(),
                materializing.clone(),
                materialized.clone(),
                attaching.clone(),
                attached.clone(),
                optimizing.clone(),
                failed.clone(),
                retried.clone(),
                completed.clone(),
            ],
        )
        .unwrap();
        assert_eq!(
            view.state,
            ProjectOptimizationRunState::ReadyForFinalEvaluation
        );
        assert_eq!(view.execution_attempt, 2);
        assert_eq!(view.outcome, Some(outcome.clone()));

        let evaluating = ProjectOptimizationEvent::final_evaluation_started(
            Uuid::new_v4(),
            &run,
            &completed,
            1,
            Utc::now(),
        )
        .unwrap();
        let final_result = ProjectOptimizationFinalResult::create(
            &run,
            &launch,
            &preparation,
            &materialization,
            &experiment,
            &outcome,
            BoundIdentity {
                id: experiment.experiment_run.id.clone(),
                fingerprint: digest('7'),
            },
            ProjectOptimizationFinalResultKind::CandidateAccepted,
            BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('8'),
            },
            Utc::now(),
        )
        .unwrap();
        let finalized = ProjectOptimizationEvent::final_evaluation_completed(
            Uuid::new_v4(),
            &run,
            &evaluating,
            1,
            final_result.clone(),
            Utc::now(),
        )
        .unwrap();
        let view = replay_project_optimization(
            &run,
            &launch,
            &[
                reserved,
                preparing,
                ready,
                materializing,
                materialized,
                attaching,
                attached,
                optimizing,
                failed,
                retried,
                completed,
                evaluating,
                finalized,
            ],
        )
        .unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::CandidateAccepted);
        assert_eq!(view.final_attempt, 1);
        assert_eq!(view.final_result, Some(final_result));
    }

    #[test]
    fn journal_rejects_skips_duplicate_reservations_and_unsafe_failures() {
        let (setup, launch) = fixture();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let reserved = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        let started = ProjectOptimizationEvent::preparation_started(
            Uuid::new_v4(),
            &run,
            &reserved,
            1,
            Utc::now(),
        )
        .unwrap();
        assert!(
            ProjectOptimizationEvent::preparation_failed(
                Uuid::new_v4(),
                &run,
                &started,
                1,
                "unsafe failure message!",
                Utc::now()
            )
            .is_err()
        );
        let mut skipped = ProjectOptimizationEvent::preparation_completed(
            Uuid::new_v4(),
            &run,
            &started,
            1,
            preparation(&run, &launch, &setup),
            Utc::now(),
        )
        .unwrap();
        skipped.sequence += 1;
        skipped.fingerprint = skipped.reproduce().unwrap();
        assert!(
            replay_project_optimization(&run, &launch, &[reserved.clone(), started, skipped])
                .is_err()
        );
        assert!(replay_project_optimization(&run, &launch, &[]).is_err());
        assert!(replay_project_optimization(&run, &launch, &[reserved.clone(), reserved]).is_err());
    }

    #[test]
    fn cancellation_is_terminal_and_keeps_the_exact_journal_head() {
        let (_, launch) = fixture();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let reserved = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        assert!(
            ProjectOptimizationEvent::cancelled(
                Uuid::new_v4(),
                &run,
                &reserved,
                "unsafe cancellation message!",
                Utc::now(),
            )
            .is_err()
        );
        let cancelled = ProjectOptimizationEvent::cancelled(
            Uuid::new_v4(),
            &run,
            &reserved,
            "user_requested",
            Utc::now(),
        )
        .unwrap();
        let view =
            replay_project_optimization(&run, &launch, &[reserved.clone(), cancelled.clone()])
                .unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::Cancelled);
        assert_eq!(view.failure_code.as_deref(), Some("user_requested"));
        assert_eq!(view.last_sequence, 2);
        assert_eq!(view.head_fingerprint, cancelled.fingerprint);

        let after_cancel = ProjectOptimizationEvent::cancelled(
            Uuid::new_v4(),
            &run,
            &cancelled,
            "user_requested",
            Utc::now(),
        )
        .unwrap();
        assert!(
            replay_project_optimization(&run, &launch, &[reserved, cancelled, after_cancel])
                .is_err()
        );
    }
}
