//! Safe model-inventory projection of exactly comparable development reports.
use crate::{
    BenchmarkSource, BoundIdentity, Invalid, ModelCatalog, ProjectBenchmarkVersion,
    ScientificBinding, require,
};
use encoder_experiment_core::{
    benchmark::{BenchmarkDefinition, BenchmarkResult},
    domain::ExternalProjectSnapshot,
    journal::{ExperimentEvent, ExperimentEventKind, ExperimentRunState, replay_experiment},
    metrics::{CandidateAssessment, EvaluationReport},
    protocol::ExperimentProtocol,
};
use serde::Serialize;
use uuid::Uuid;

/// Loaded by the owning adapters. Never serialized: it can contain sealed data.
pub struct BenchmarkRunEvidence<'a> {
    pub binding: &'a ScientificBinding,
    pub project: &'a ExternalProjectSnapshot,
    pub protocol: &'a ExperimentProtocol,
    pub definition: &'a BenchmarkDefinition,
    pub events: &'a [ExperimentEvent],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationContext {
    pub source: BenchmarkSource,
    pub run_id: Uuid,
    pub baseline_revision_id: Uuid,
    pub baseline_model_id: Uuid,
    pub candidate_id: Option<Uuid>,
    /// Original development verdict, never recomputed against today's baseline.
    pub assessment: Option<CandidateAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBenchmarkReport {
    pub result: BenchmarkResult,
    /// A baseline report may be reused by multiple protocols/runs.
    pub contexts: Vec<EvaluationContext>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBenchmarkResults {
    pub model_id: Uuid,
    pub name: String,
    pub is_baseline: bool,
    /// Empty means not evaluated on this version, not zero or failed.
    pub reports: Vec<ModelBenchmarkReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBenchmarkResults {
    pub version: ProjectBenchmarkVersion,
    pub models: Vec<ModelBenchmarkResults>,
}

impl ProjectBenchmarkResults {
    /// The workspace adapter has already verified the version's complete chain.
    pub fn new(version: ProjectBenchmarkVersion, catalog: &ModelCatalog) -> Result<Self, Invalid> {
        catalog.validate()?;
        version.definition.validate_integrity().map_err(invalid)?;
        version.source.validate()?;
        require(
            version.project_id == catalog.project_id
                && !version.id.is_nil()
                && version.fingerprint == version.reproduce()?,
            "Benchmark belongs to another project or changed.",
        )?;
        let baseline = catalog.active_model().id;
        let mut models: Vec<_> = catalog
            .artifacts
            .iter()
            .map(|model| ModelBenchmarkResults {
                model_id: model.id,
                name: model.name.clone(),
                is_baseline: model.id == baseline,
                reports: vec![],
            })
            .collect();
        models.sort_by_key(|model| (!model.is_baseline, model.model_id));
        Ok(Self { version, models })
    }

    pub fn include_run(
        &mut self,
        catalog: &ModelCatalog,
        evidence: BenchmarkRunEvidence<'_>,
    ) -> Result<(), Invalid> {
        let BenchmarkRunEvidence {
            binding,
            project,
            protocol,
            definition,
            events,
        } = evidence;
        catalog.validate()?;
        binding.validate()?;
        definition.validate_integrity().map_err(invalid)?;
        require(
            definition == &self.version.definition
                && catalog.project_id == self.version.project_id
                && binding.project_id == catalog.project_id
                && binding.runtime.project_snapshot.id == project.id.to_string()
                && binding.runtime.project_snapshot.fingerprint == project.fingerprint
                && binding.adapter.key == project.backend.name
                && binding.adapter.protocol == project.backend.protocol_version
                && binding.adapter.configuration_fingerprint
                    == project.backend.configuration_fingerprint,
            "Result source does not match this project's benchmark.",
        )?;
        let baseline = catalog
            .baseline_revisions
            .iter()
            .find(|revision| revision.id == binding.baseline_revision_id)
            .ok_or_else(|| Invalid("The result's original baseline revision is missing.".into()))?;
        let view = replay_experiment(project, protocol, events).map_err(invalid)?;
        let context = EvaluationContext {
            source: BenchmarkSource {
                scientific_binding: BoundIdentity {
                    id: binding.id.to_string(),
                    fingerprint: binding.fingerprint.clone(),
                },
                project_snapshot: binding.runtime.project_snapshot.clone(),
                protocol: BoundIdentity {
                    id: protocol.id.to_string(),
                    fingerprint: protocol.fingerprint.clone(),
                },
            },
            run_id: view.run_id,
            baseline_revision_id: baseline.id,
            baseline_model_id: baseline.model_artifact_id,
            candidate_id: None,
            assessment: None,
        };
        // Validate the entire input before updating the projection.
        let mut additions = vec![];
        for report in protocol.baseline_development_reports() {
            additions.push((
                baseline.model_artifact_id,
                safe_report(definition, project, report)?,
                context.clone(),
            ));
        }
        for (candidate_id, candidate) in &view.candidates {
            let Some(output) = &candidate.train_output else {
                continue;
            };
            for model in &catalog.artifacts {
                let (Some(source), Some(run)) = (&model.source_model, &model.producing_run) else {
                    continue;
                };
                if source.id != output.model.id.to_string() || run.id != view.run_id.to_string() {
                    continue;
                }
                require(
                    source.fingerprint == output.model.fingerprint,
                    "Registered model differs from its recorded scientific output.",
                )?;
                let receipt_matches = events.iter().any(|event| event.fingerprint == run.fingerprint
                    && matches!(&event.event, ExperimentEventKind::CandidateTrainingCompleted { candidate_id: id, output: recorded }
                        if id == candidate_id && recorded == output));
                // Older accepted imports pinned the terminal experiment head.
                require(
                    receipt_matches
                        || (view.state == ExperimentRunState::Completed
                            && run.fingerprint == view.last_event_fingerprint),
                    "Registered model's producing-run receipt changed.",
                )?;
                for report in candidate.development_reports.values() {
                    let mut context = context.clone();
                    context.candidate_id = Some(*candidate_id);
                    context.assessment = candidate
                        .development_assessments
                        .get(&report.suite_key)
                        .cloned();
                    additions.push((model.id, safe_report(definition, project, report)?, context));
                }
            }
        }
        // Identity collisions must not leave a partially changed result on error.
        let mut next = self.clone();
        for (model_id, result, context) in additions {
            next.insert(model_id, result, context)?;
        }
        *self = next;
        Ok(())
    }

    fn insert(
        &mut self,
        model_id: Uuid,
        result: BenchmarkResult,
        context: EvaluationContext,
    ) -> Result<(), Invalid> {
        for model in &self.models {
            for report in &model.reports {
                if report.result.report_id == result.report_id {
                    require(
                        model.model_id == model_id && report.result == result,
                        "An evaluation report identity has conflicting contents or model ownership.",
                    )?;
                }
            }
        }
        let model = self
            .models
            .iter_mut()
            .find(|model| model.model_id == model_id)
            .ok_or_else(|| {
                Invalid("Evaluated model is absent from the project inventory.".into())
            })?;
        if let Some(report) = model
            .reports
            .iter_mut()
            .find(|report| report.result.report_id == result.report_id)
        {
            if !report.contexts.contains(&context) {
                report.contexts.push(context);
            }
            report.contexts.sort_by_key(|context| {
                (context.run_id, context.source.scientific_binding.id.clone())
            });
        } else {
            model.reports.push(ModelBenchmarkReport {
                result,
                contexts: vec![context],
            });
            model
                .reports
                .sort_by_key(|report| (report.result.created_at, report.result.report_id));
        }
        Ok(())
    }
}

fn safe_report(
    definition: &BenchmarkDefinition,
    project: &ExternalProjectSnapshot,
    report: &EvaluationReport,
) -> Result<BenchmarkResult, Invalid> {
    BenchmarkResult::from_development(
        definition,
        project,
        &definition.evaluation_configuration_fingerprint,
        report,
    )
    .map_err(invalid)
}
fn invalid(error: impl std::fmt::Display) -> Invalid {
    Invalid(error.to_string())
}
