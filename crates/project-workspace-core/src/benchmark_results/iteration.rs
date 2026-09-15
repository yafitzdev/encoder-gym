//! Agent iterations own their scientific children independently of the legacy
//! root materialization. Only their recorded chain admits reports to a project.
use super::{BenchmarkRunEvidence, ProjectBenchmarkResults, invalid};
use crate::{
    BoundIdentity, Invalid, ModelCatalog, OptimizationLaunchAuthorization, OptimizationSetup,
    ProjectOptimizationPreparation, ProjectOptimizationRun,
    optimization_iteration::{IterationContinuation, ProjectOptimizationIteration},
    optimization_iteration_execution::IterationTrainingBinding,
    require,
};
use encoder_experiment_core::domain::ExternalProjectSnapshot;
use uuid::Uuid;

/// Read-only receipts loaded from their owning stores. Never sent to the GUI.
pub struct IterationResultLineage<'a> {
    pub run: &'a ProjectOptimizationRun,
    pub launch: &'a OptimizationLaunchAuthorization,
    pub setup: &'a OptimizationSetup,
    pub preparation: &'a ProjectOptimizationPreparation,
    pub iteration: &'a ProjectOptimizationIteration,
    pub training: &'a IterationTrainingBinding,
    pub runtime_project: &'a ExternalProjectSnapshot,
    pub predecessors: &'a [IterationContinuation<'a>],
}

impl ProjectBenchmarkResults {
    pub fn include_agent_iteration(
        &mut self,
        catalog: &ModelCatalog,
        evidence: BenchmarkRunEvidence<'_>,
        lineage: IterationResultLineage<'_>,
    ) -> Result<(), Invalid> {
        let IterationResultLineage {
            run,
            launch,
            setup,
            preparation,
            iteration,
            training,
            runtime_project,
            predecessors,
        } = lineage;
        let BenchmarkRunEvidence {
            binding,
            project,
            protocol,
            definition,
            events,
        } = &evidence;
        catalog.validate()?;
        binding.validate()?;
        runtime_project.validate_integrity().map_err(invalid)?;
        definition.validate_integrity().map_err(invalid)?;
        require(
            predecessors.len() + 1 == iteration.scope.iteration as usize,
            "Iteration report is missing its predecessor chain",
        )?;
        for (index, prior) in predecessors.iter().enumerate() {
            if index == 0 {
                prior
                    .previous
                    .validate_first(run, launch, setup, preparation)?;
            } else {
                prior.previous.validate_next(
                    run,
                    launch,
                    setup,
                    preparation,
                    predecessors[index - 1],
                )?;
            }
            prior.previous.validate_benchmark(&self.version)?;
        }
        if let Some(prior) = predecessors.last() {
            iteration.validate_next(run, launch, setup, preparation, *prior)?;
        } else {
            iteration.validate_first(run, launch, setup, preparation)?;
        }
        iteration.validate_benchmark(&self.version)?;
        training.validate_for(iteration, launch)?;
        let identity = |id: Uuid, fingerprint: &str| BoundIdentity {
            id: id.to_string(),
            fingerprint: fingerprint.into(),
        };
        let baseline = catalog
            .baseline_revisions
            .iter()
            .find(|value| value.id.to_string() == iteration.comparison_baseline_revision.id)
            .ok_or_else(|| Invalid("The iteration's comparison baseline is missing.".into()))?;
        let starting_model = catalog
            .artifacts
            .iter()
            .find(|value| value.id.to_string() == iteration.starting_model.id)
            .ok_or_else(|| Invalid("The iteration's starting model is missing.".into()))?;
        let first = events
            .first()
            .ok_or_else(|| Invalid("Iteration experiment journal is missing.".into()))?;
        require(
            run.project_id == catalog.project_id
                && catalog.project_id == self.version.project_id
                && binding.project_id == catalog.project_id
                && preparation.execution_binding == identity(binding.id, &binding.fingerprint)
                && preparation.runtime_project == binding.runtime.project_snapshot
                && binding.runtime.project_snapshot
                    == identity(runtime_project.id, &runtime_project.fingerprint)
                && binding.baseline_revision_id == baseline.id
                && baseline.fingerprint == iteration.comparison_baseline_revision.fingerprint
                && starting_model.id == baseline.model_artifact_id
                && starting_model.fingerprint == iteration.starting_model.fingerprint
                && predecessors
                    .first()
                    .map_or(iteration, |prior| prior.previous)
                    .development
                    .protocol
                    == self.version.source.protocol
                && training.scientific_project == identity(project.id, &project.fingerprint)
                && project.inputs.contains(&training.training_artifact)
                && project
                    .baseline_model
                    .has_same_content(&runtime_project.baseline_model)
                && binding.adapter.key == project.backend.name
                && binding.adapter.protocol == project.backend.protocol_version
                && binding.adapter.configuration_fingerprint
                    == project.backend.configuration_fingerprint
                && *definition == &self.version.definition
                && training.protocol == identity(protocol.id, &protocol.fingerprint)
                && training.experiment_run_id == first.run_id
                && protocol.budget.maximum_sealed_evaluations == 0
                && protocol.candidates.len() == 1
                && protocol.candidates.first().is_some_and(|candidate| {
                    training.candidate == identity(candidate.id, &candidate.fingerprint)
                }),
            "Iteration result does not match its recorded project, benchmark or training child.",
        )?;
        // Normal journal replay validates every event and excludes sealed reports.
        // Already-completed suites remain visible even if a later suite failed.
        self.include_verified_run(catalog, evidence)
    }
}
