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
    pub development: IterationDevelopmentEvidence,
    pub scope: AgentAnalysisScope,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationIteration {
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
        artifact_core::fingerprint(&serde_json::json!({
            "id":self.id, "run":self.run,
            "preparationFingerprint":self.preparation_fingerprint,
            "comparisonBaselineRevision":self.comparison_baseline_revision,
            "startingModel":self.starting_model, "dataset":self.dataset,
            "benchmark":self.benchmark, "providerCatalog":self.provider_catalog,
            "development":self.development, "scope":self.scope,
            "createdAt":self.created_at,
        }))
        .map_err(invalid)
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
