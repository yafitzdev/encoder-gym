//! Immutable inputs for an Agent iteration. These records bind existing slice
//! evidence; they neither grant training admission nor perform a provider call.

use crate::{
    BoundIdentity, Invalid, OptimizationLaunchAuthorization, OptimizationSetup,
    ProjectBenchmarkVersion, ProjectOptimizationPreparation, ProjectOptimizationRun, require,
};
use chrono::{DateTime, Utc};
use dataset_core::versions::DatasetVersionRef;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition,
    domain::{EvidenceRole, ExternalProjectSnapshot},
    protocol::ExperimentProtocol,
};
use encoder_optimization_core::agent::AgentAnalysisScope;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Row-free references to the complete declared development population. Never
/// serialize the source protocol here: it also contains sealed evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationDevelopmentEvidence {
    pub project: BoundIdentity,
    pub protocol: BoundIdentity,
    pub model: BoundIdentity,
    pub benchmark_definition_fingerprint: String,
    pub reports: BTreeMap<String, BoundIdentity>,
    pub fingerprint: String,
}

impl IterationDevelopmentEvidence {
    /// Project custody has verified and persisted the scientific result. The
    /// inspection adapter must still verify its source journal before dispatch.
    pub fn completed(
        training: &crate::optimization_iteration_execution::IterationTrainingBinding,
        result: &crate::optimization_iteration_execution::IterationDevelopmentResult,
        benchmark_fingerprint: &str,
    ) -> Result<Self, Invalid> {
        require(
            result.fingerprint == result.reproduce()?
                && result.training_binding_fingerprint == training.fingerprint
                && result.experiment_run_id == training.experiment_run_id,
            "Development result differs from its training receipt",
        )?;
        let mut reports = BTreeMap::new();
        for (suite, report) in &result.reports {
            require(
                report.evidence_role == EvidenceRole::Development
                    && report.model == result.output.model
                    && report.suite_key == *suite,
                "Next iteration requires candidate development reports only",
            )?;
            reports.insert(suite.clone(), bound(report.id, &report.fingerprint));
        }
        let mut value = Self {
            project: training.scientific_project.clone(),
            protocol: training.protocol.clone(),
            model: bound(result.output.model.id, &result.output.model.fingerprint),
            benchmark_definition_fingerprint: benchmark_fingerprint.into(),
            reports,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_identity()?;
        Ok(value)
    }
    /// The composition root loads this exact protocol from its owning store and
    /// obtains `definition` from the task adapter, not mutable GUI defaults.
    pub fn baseline(
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        definition: &BenchmarkDefinition,
    ) -> Result<Self, Invalid> {
        protocol.validate_integrity(project).map_err(invalid)?;
        definition.validate_integrity().map_err(invalid)?;
        let mut reports = BTreeMap::new();
        for report in protocol.baseline_development_reports() {
            definition
                .validate_development_report(
                    project,
                    &definition.evaluation_configuration_fingerprint,
                    report,
                )
                .map_err(invalid)?;
            require(
                report.model == project.baseline_model
                    && reports
                        .insert(
                            report.suite_key.clone(),
                            bound(report.id, &report.fingerprint),
                        )
                        .is_none(),
                "Iteration evidence contains a foreign model or duplicate development suite.",
            )?;
        }
        let expected: Vec<_> = definition
            .suites
            .iter()
            .filter(|suite| suite.role == EvidenceRole::Development)
            .map(|suite| &suite.key)
            .collect();
        require(
            reports.keys().collect::<Vec<_>>() == expected,
            "Iteration evidence must cover every development suite of the selected benchmark.",
        )?;
        let mut value = Self {
            project: bound(project.id, &project.fingerprint),
            protocol: bound(protocol.id, &protocol.fingerprint),
            model: bound(
                project.baseline_model.id,
                &project.baseline_model.fingerprint,
            ),
            benchmark_definition_fingerprint: definition.fingerprint.clone(),
            reports,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_identity()?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "project": self.project, "protocol": self.protocol, "model": self.model,
            "benchmarkDefinitionFingerprint": self.benchmark_definition_fingerprint,
            "reports": self.reports,
        }))
        .map_err(invalid)
    }

    pub fn validate_identity(&self) -> Result<(), Invalid> {
        for value in [&self.project, &self.protocol, &self.model]
            .into_iter()
            .chain(self.reports.values())
        {
            value.validate("Iteration development source")?;
            require(
                Uuid::parse_str(&value.id).is_ok_and(|id| !id.is_nil()),
                "Iteration evidence requires exact source UUIDs.",
            )?;
        }
        crate::validate_hash(&self.benchmark_definition_fingerprint)?;
        for suite in self.reports.keys() {
            crate::validate_name(suite)?;
        }
        require(
            !self.reports.is_empty() && self.reproduce()? == self.fingerprint,
            "Iteration development evidence changed or is empty.",
        )
    }
}

/// The first iteration is admitted from the root's verified preparation.
/// Further iterations require a completion/selection lineage contract; merely
/// changing `scope.iteration` or substituting a dataset is deliberately illegal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationIteration {
    pub id: Uuid,
    pub run: BoundIdentity,
    pub preparation_fingerprint: String,
    pub comparison_baseline_revision: BoundIdentity,
    pub starting_model: BoundIdentity,
    pub dataset: DatasetVersionRef,
    pub benchmark: BoundIdentity,
    pub provider_catalog: BoundIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<BoundIdentity>,
    pub development: IterationDevelopmentEvidence,
    pub scope: AgentAnalysisScope,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Clone, Copy)]
pub struct IterationContinuation<'a> {
    pub previous: &'a ProjectOptimizationIteration,
    pub completion: &'a crate::optimization_loop::IterationCompletion,
    pub training: &'a crate::optimization_iteration_execution::IterationTrainingBinding,
    pub result: &'a crate::optimization_iteration_execution::IterationDevelopmentResult,
}

impl ProjectOptimizationIteration {
    pub fn next(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
        preparation: &ProjectOptimizationPreparation,
        continuation: IterationContinuation<'_>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let IterationContinuation {
            previous,
            completion,
            training,
            result,
        } = continuation;
        preparation.validate_for(run, launch, setup)?;
        completion.validate_identity()?;
        training.validate_for(previous, launch)?;
        let settings = launch
            .scope
            .agentic
            .as_ref()
            .ok_or_else(|| Invalid("Agent settings missing".into()))?;
        require(
            previous.fingerprint == previous.reproduce()?
                && previous.run == run.identity()
                && previous.benchmark == setup.inputs.benchmark
                && completion.run == run.identity()
                && completion.iteration == bound(previous.id, &previous.fingerprint)
                && completion.number == previous.scope.iteration
                && completion.result.as_ref()
                    == Some(&bound(result.experiment_run_id, &result.fingerprint))
                && completion.end.is_none()
                && completion.number < settings.maximum_iterations
                && completion.total_row_changes < settings.maximum_row_changes
                && created_at >= completion.created_at,
            "Next iteration requires an eligible completed predecessor and remaining budget",
        )?;
        let dataset = completion.selected.as_ref().map_or_else(
            || setup.inputs.dataset.clone(),
            |selected| selected.dataset.clone(),
        );
        require(
            dataset.project_id == run.project_id,
            "Selected dataset belongs to another project",
        )?;
        let development = IterationDevelopmentEvidence::completed(
            training,
            result,
            &previous.development.benchmark_definition_fingerprint,
        )?;
        let number = completion.number + 1;
        let mut value = Self {
            id: run.child_id("agent_iteration", u64::from(number))?,
            run: run.identity(),
            preparation_fingerprint: preparation.fingerprint.clone(),
            comparison_baseline_revision: setup.inputs.baseline_revision.clone(),
            // Every iteration fine-tunes the pinned starting checkpoint. Dataset
            // selection evolves; it never silently changes model warm-start policy.
            starting_model: setup.inputs.model.clone(),
            dataset: dataset.clone(),
            benchmark: setup.inputs.benchmark.clone(),
            provider_catalog: launch.scope.provider_catalog.clone(),
            predecessor: Some(completion.identity()),
            scope: AgentAnalysisScope {
                run_id: run.id,
                iteration: number,
                launch_fingerprint: launch.fingerprint.clone(),
                dataset_version_id: dataset.id,
                dataset_fingerprint: dataset.fingerprint,
                development_evidence_fingerprint: development.fingerprint.clone(),
                objective: settings.objective.clone(),
                analysis_protocol: settings.analysis_protocol,
                maximum_turns: settings.maximum_agent_turns_per_iteration,
                maximum_row_changes: settings.maximum_row_changes - completion.total_row_changes,
            },
            development,
            created_at,
            fingerprint: String::new(),
        };
        value.scope.validate().map_err(invalid)?;
        value.fingerprint = value.reproduce()?;
        Ok(value)
    }

    pub fn validate_next(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
        preparation: &ProjectOptimizationPreparation,
        continuation: IterationContinuation<'_>,
    ) -> Result<(), Invalid> {
        require(
            *self
                == Self::next(
                    run,
                    launch,
                    setup,
                    preparation,
                    continuation,
                    self.created_at,
                )?,
            "Next iteration differs from its persisted predecessor, selection or remaining budget",
        )
    }

    pub fn first(
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
        preparation: &ProjectOptimizationPreparation,
        development: IterationDevelopmentEvidence,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        preparation.validate_for(run, launch, setup)?;
        let settings = launch.scope.agentic.as_ref().ok_or_else(|| {
            Invalid("This fixed-recipe run has no Agent iteration authority.".into())
        })?;
        let scope = AgentAnalysisScope {
            run_id: run.id,
            iteration: 1,
            launch_fingerprint: launch.fingerprint.clone(),
            dataset_version_id: setup.inputs.dataset.id,
            dataset_fingerprint: setup.inputs.dataset.fingerprint.clone(),
            development_evidence_fingerprint: development.fingerprint.clone(),
            objective: settings.objective.clone(),
            analysis_protocol: settings.analysis_protocol,
            maximum_turns: settings.maximum_agent_turns_per_iteration,
            maximum_row_changes: settings.maximum_row_changes,
        };
        let mut value = Self {
            id: run.child_id("agent_iteration", 1)?,
            run: run.identity(),
            preparation_fingerprint: preparation.fingerprint.clone(),
            comparison_baseline_revision: setup.inputs.baseline_revision.clone(),
            starting_model: setup.inputs.model.clone(),
            dataset: setup.inputs.dataset.clone(),
            benchmark: setup.inputs.benchmark.clone(),
            provider_catalog: launch.scope.provider_catalog.clone(),
            predecessor: None,
            development,
            scope,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_first(run, launch, setup, preparation)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        let mut value = serde_json::json!({
            "id":self.id, "run":self.run,
            "preparationFingerprint":self.preparation_fingerprint,
            "comparisonBaselineRevision":self.comparison_baseline_revision,
            "startingModel":self.starting_model, "dataset":self.dataset,
            "benchmark":self.benchmark, "providerCatalog":self.provider_catalog,
            "development":self.development, "scope":self.scope,
            "createdAt":self.created_at,
        });
        if let Some(predecessor) = &self.predecessor {
            value["predecessor"] = serde_json::to_value(predecessor).map_err(invalid)?;
        }
        artifact_core::fingerprint(&value).map_err(invalid)
    }

    pub fn validate_first(
        &self,
        run: &ProjectOptimizationRun,
        launch: &OptimizationLaunchAuthorization,
        setup: &OptimizationSetup,
        preparation: &ProjectOptimizationPreparation,
    ) -> Result<(), Invalid> {
        preparation.validate_for(run, launch, setup)?;
        self.development.validate_identity()?;
        self.scope.validate().map_err(invalid)?;
        let settings = launch.scope.agentic.as_ref().ok_or_else(|| {
            Invalid("This fixed-recipe run has no Agent iteration authority.".into())
        })?;
        require(
            self.id == run.child_id("agent_iteration", 1)?
                && self.predecessor.is_none()
                && self.run == run.identity()
                && self.preparation_fingerprint == preparation.fingerprint
                && self.comparison_baseline_revision == setup.inputs.baseline_revision
                && self.starting_model == setup.inputs.model
                && self.dataset == setup.inputs.dataset
                && self.benchmark == setup.inputs.benchmark
                && self.provider_catalog == launch.scope.provider_catalog
                && self.development.project == preparation.runtime_project
                && self.development.reports.keys().cloned().collect::<Vec<_>>()
                    == preparation.development_suites
                && self.scope.run_id == run.id
                && self.scope.iteration == 1
                && self.scope.launch_fingerprint == launch.fingerprint
                && self.scope.dataset_version_id == self.dataset.id
                && self.scope.dataset_fingerprint == self.dataset.fingerprint
                && self.scope.development_evidence_fingerprint == self.development.fingerprint
                && self.scope.objective == settings.objective
                && self.scope.maximum_turns == settings.maximum_agent_turns_per_iteration
                && self.scope.maximum_row_changes == settings.maximum_row_changes
                && self.created_at >= preparation.created_at
                && self.reproduce()? == self.fingerprint,
            "Agent iteration differs from its exact prepared inputs, evidence or settings.",
        )
    }

    pub fn validate_benchmark(&self, benchmark: &ProjectBenchmarkVersion) -> Result<(), Invalid> {
        benchmark.definition.validate_integrity().map_err(invalid)?;
        require(
            benchmark.reproduce()? == benchmark.fingerprint
                && self.benchmark == bound(benchmark.id, &benchmark.fingerprint)
                && self.dataset.project_id == benchmark.project_id
                && self.development.benchmark_definition_fingerprint
                    == benchmark.definition.fingerprint
                && self.development.reports.keys().cloned().collect::<Vec<_>>()
                    == benchmark
                        .definition
                        .suites
                        .iter()
                        .filter(|suite| suite.role == EvidenceRole::Development)
                        .map(|suite| suite.key.clone())
                        .collect::<Vec<_>>(),
            "Agent iteration uses another benchmark or incomplete development evidence.",
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
