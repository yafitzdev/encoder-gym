//! Row-free desktop history through the existing custody and benchmark readers.
//! Does not execute/reconcile work or expose protocol/holdout payloads.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use encoder_experiment_core::metrics::{CandidateVerdict, MetricDirection};
use project_workspace_local::{
    open_workspace, optimization_completions, optimization_iteration_execution,
    optimization_iterations, optimization_runs,
};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct History {
    project_id: Uuid,
    run_id: Uuid,
    iterations: Vec<Iteration>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Iteration {
    id: Uuid,
    number: u32,
    created_at: DateTime<Utc>,
    starting_model_id: String,
    input_dataset_version_id: Uuid,
    benchmark_version_id: String,
    experiment_run_id: Option<Uuid>,
    qualified_dataset_version_id: Option<Uuid>,
    training_dataset_version_id: Option<Uuid>,
    model_id: Option<Uuid>,
    completed: bool,
    no_change: bool,
    selected: bool,
    development_passed: Option<bool>,
    checks: Vec<Check>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Check {
    report_id: Uuid,
    suite: String,
    metric: String,
    direction: MetricDirection,
    baseline: f64,
    candidate: f64,
    passed: bool,
}

pub(super) async fn read(folder: &Path, run_id: Uuid) -> Result<History> {
    let run = optimization_runs::show(folder, run_id).await?;
    let inputs = optimization_iterations::list(folder, run_id).await?;
    let completed = optimization_completions::list(folder, run_id).await?;
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model inventory missing")?;
    // The shared reader verifies the original baseline, unchanged benchmark,
    // scientific journals and complete iteration ancestry. Never rescore in UI.
    let results = match inputs.first() {
        Some(first) => {
            Some(super::super::benchmarks::results(folder, first.benchmark.id.parse()?).await?)
        }
        None => None,
    };
    let mut iterations = Vec::new();
    for input in inputs {
        let training = optimization_iteration_execution::training(folder, run_id, input.id).await?;
        let completion = completed
            .iter()
            .find(|item| item.iteration.id == input.id.to_string());
        let model = training.as_ref().and_then(|training| {
            catalog.artifacts.iter().find(|model| {
                model
                    .producing_run
                    .as_ref()
                    .is_some_and(|source| source.id == training.experiment_run_id.to_string())
            })
        });
        let mut checks = Vec::new();
        let mut verdicts = std::collections::BTreeMap::new();
        if let (Some(training), Some(model), Some(results)) = (&training, model, &results) {
            let reports = &results
                .models
                .iter()
                .find(|value| value.model_id == model.id)
                .context("Iteration model missing from benchmark inventory")?
                .reports;
            for report in reports {
                for context in report.contexts.iter().filter(|context| {
                    context.run_id == training.experiment_run_id && context.candidate_id.is_some()
                }) {
                    let assessment = context
                        .assessment
                        .as_ref()
                        .context("Candidate assessment missing")?;
                    ensure!(
                        context.candidate_id.map(|id| id.to_string()).as_ref()
                            == Some(&training.candidate.id),
                        "Candidate custody changed"
                    );
                    ensure!(
                        verdicts
                            .insert(report.result.suite_key.clone(), assessment.verdict)
                            .is_none(),
                        "Duplicate iteration assessment"
                    );
                    for gate in &assessment.gates {
                        let definition = results
                            .version
                            .definition
                            .metric_contract
                            .definitions
                            .iter()
                            .find(|item| item.key == gate.key)
                            .context("Metric direction missing")?;
                        checks.push(Check {
                            report_id: report.result.report_id,
                            suite: report.result.suite_key.clone(),
                            metric: gate.key.clone(),
                            direction: definition.direction,
                            baseline: gate.baseline,
                            candidate: gate.candidate,
                            passed: gate.passed,
                        });
                    }
                }
            }
        }
        let development_passed = if completion.is_some_and(|item| item.result.is_some()) {
            ensure!(
                verdicts.keys().eq(input.development.reports.keys()),
                "Completed iteration is missing development evidence"
            );
            Some(
                verdicts
                    .values()
                    .all(|verdict| *verdict == CandidateVerdict::Passed),
            )
        } else {
            None
        };
        iterations.push(Iteration {
            id: input.id,
            number: input.scope.iteration,
            created_at: input.created_at,
            starting_model_id: input.starting_model.id,
            input_dataset_version_id: input.dataset.id,
            benchmark_version_id: input.benchmark.id,
            experiment_run_id: training.as_ref().map(|value| value.experiment_run_id),
            qualified_dataset_version_id: training.as_ref().map(|value| value.qualified_dataset.id),
            training_dataset_version_id: training.as_ref().map(|value| value.training_dataset.id),
            model_id: model.map(|value| value.id),
            completed: completion.is_some(),
            no_change: completion.is_some_and(|value| value.result.is_none()),
            selected: completed
                .last()
                .and_then(|value| value.selected.as_ref())
                .is_some_and(|value| value.iteration.id == input.id.to_string()),
            development_passed,
            checks,
        });
    }
    Ok(History {
        project_id: run.run.project_id,
        run_id,
        iterations,
    })
}
