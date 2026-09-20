//! Exact, development-only Agent input adapter. Load selected sources once per
//! execution, then serve bounded projections without rehashing on every tool.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, ensure};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_experiment_nomos::{
    NomosBackend, NomosDatasetInvestigation, NomosDatasetLandscape, NomosDevelopmentEvidence,
    NomosGenerationTemplates, NomosRepairMetricPoint, project_repair_metric_points,
};
use encoder_optimization_core::{
    OptimizationError,
    agent::{
        AgentAnalysisScope, DatasetEditProposal, InspectionItem, InspectionPage,
        InspectionSelection,
    },
    ports::{BoxFuture, OptimizationInspection},
    repair_outcome::RepairOutcomeSummary,
    repair_strategy::RepairPlan,
};
use project_workspace_core::optimization_iteration::ProjectOptimizationIteration;
use project_workspace_local::dataset_versions;
use serde_json::Value;

pub(super) struct IterationInspection {
    scope: AgentAnalysisScope,
    failures: Vec<InspectionItem>,
    rows: BTreeMap<String, Value>,
    landscape: Option<NomosDatasetLandscape>,
    investigation: Option<NomosDatasetInvestigation>,
    current_evidence: Vec<NomosDevelopmentEvidence>,
    baseline_evidence: Vec<NomosDevelopmentEvidence>,
    repair_memory: Vec<RepairOutcomeSummary>,
    prior_interventions: std::collections::BTreeSet<String>,
}

impl IterationInspection {
    pub async fn load(folder: &Path, iteration: &ProjectOptimizationIteration) -> Result<Self> {
        let (benchmark, mut binding) =
            project_workspace_local::benchmarks::inspect(folder, iteration.benchmark.id.parse()?)
                .await?;
        iteration.validate_benchmark(&benchmark)?;
        let prior = if iteration.scope.iteration > 1 {
            let run =
                project_workspace_local::optimization_runs::show(folder, iteration.scope.run_id)
                    .await?;
            let preparation = run.preparation.as_ref().context("Preparation missing")?;
            binding = project_workspace_local::scientific_binding_history(folder)
                .await?
                .into_iter()
                .find(|value| {
                    value.id.to_string() == preparation.execution_binding.id
                        && value.fingerprint == preparation.execution_binding.fingerprint
                })
                .context("Pinned runtime missing")?;
            let history = super::super::iteration_inputs::scientific_history(
                folder,
                iteration.scope.run_id,
                iteration.scope.iteration - 1,
            )
            .await?;
            project_workspace_local::optimization_completions::finish(
                folder,
                iteration.scope.run_id,
                iteration.scope.iteration - 1,
                &history,
            )
            .await?;
            let source = history
                .into_iter()
                .last()
                .context("Previous development evidence missing")?;
            let training = project_workspace_local::optimization_iteration_execution::training(
                folder,
                iteration.scope.run_id,
                source.iteration_id,
            )
            .await?
            .context("Previous training missing")?;
            let result = project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult::from_journal(&training, &source.project, &source.protocol, &source.events)?;
            let evidence = project_workspace_core::optimization_iteration::IterationDevelopmentEvidence::completed(&training, &result, &benchmark.definition.fingerprint)?;
            ensure!(
                evidence == iteration.development,
                "Agent evidence differs from the previous result"
            );
            Some(result)
        } else {
            None
        };
        let store =
            super::super::super::open_bound_store(&folder.to_string_lossy(), &binding).await?;
        let result: Result<_> = async {
            let runtime_project = super::super::super::load_bound_project(&store, &binding).await?;
            let project = store.get_project(iteration.development.project.id.parse()?).await?.context("Development project missing")?;
            ensure!(
                iteration.development.project.id == project.id.to_string()
                    && iteration.development.project.fingerprint == project.fingerprint,
                "Iteration development source changed"
            );
            let protocol = store
                .get_protocol(iteration.development.protocol.id.parse()?)
                .await?
                .context("Iteration development protocol is missing")?;
            ensure!(
                protocol.fingerprint == iteration.development.protocol.fingerprint,
                "Iteration development protocol changed"
            );
            let backend = super::super::super::open_nomos_binding(folder, &binding, &runtime_project)?;
            let mut failures = Vec::new();
            let mut current_evidence = Vec::<NomosDevelopmentEvidence>::new();
            let reports: Vec<_> = if let Some(prior) = &prior { prior.reports.values().collect() }
                else { protocol.baseline_development_reports() };
            for (suite, identity) in &iteration.development.reports {
                let report = reports.iter().copied()
                    .find(|report| {
                        report.suite_key == *suite
                            && report.id.to_string() == identity.id
                            && report.fingerprint == identity.fingerprint
                    })
                    .context("Pinned development report is missing")?;
                let evidence = backend.read_development_evidence(
                    &project,
                    &protocol.metric_contract,
                    report,
                )?;
                failures.extend(evidence.failures.iter().cloned());
                current_evidence.push(evidence);
            }
            let baseline_evidence = if iteration.scope.analysis_protocol >= 2 {
                protocol
                    .baseline_development_reports()
                    .into_iter()
                    .map(|report| {
                        backend.read_development_evidence(
                            &project,
                            &protocol.metric_contract,
                            report,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            // Metrics and verdicts are explicit evidence, even when the native
            // diagnostic sample is empty. They supplement, not replace, failures.
            let content = serde_json::json!({
                "kind":"development_summary", "iteration":iteration.scope.iteration.saturating_sub(1),
                "model":iteration.development.model, "reports":reports,
                "assessments":prior.as_ref().map(|value| &value.assessments),
            });
            // Keep the comparison on the first inspection page even when the
            // native sample spans many pages of concrete failures.
            failures.insert(0, InspectionItem {id:format!("development-summary-{}", iteration.development.fingerprint),
                fingerprint:encoder_optimization_core::fingerprint(&content)?, content});
            Ok((failures, current_evidence, baseline_evidence, backend))
        }
        .await;
        store.pool().close().await;
        let (failures, current_evidence, baseline_evidence, backend) = result?;
        let dataset = dataset_versions::inspect(folder, iteration.dataset.id).await?;
        ensure!(
            dataset.reference() == iteration.dataset,
            "Iteration dataset changed"
        );
        let source_rows = dataset_versions::materialization_rows(folder, dataset.id).await?;
        ensure!(
            source_rows.len() == dataset.members.len(),
            "Iteration dataset membership is incomplete"
        );
        let rows = source_rows
            .into_iter()
            .map(|row| (row.member.id, row.value))
            .collect::<BTreeMap<_, _>>();
        let landscape = (iteration.scope.analysis_protocol == 2)
            .then(|| NomosDatasetLandscape::build(&rows, &current_evidence, &baseline_evidence))
            .transpose()?;
        let investigation = if iteration.scope.analysis_protocol == 3 {
            let inventory = backend
                .inspect_training_inventory(
                    iteration.scope.run_id,
                    &iteration.scope.dataset_fingerprint,
                    &rows,
                )
                .await?;
            Some(NomosDatasetInvestigation::build(
                &rows,
                &current_evidence,
                &baseline_evidence,
                &inventory,
            )?)
        } else {
            None
        };
        let outcomes = if iteration.scope.analysis_protocol == 3 {
            project_workspace_local::optimization_repair_outcomes::list(
                folder,
                iteration.scope.run_id,
            )
            .await?
            .into_iter()
            .filter(|outcome| outcome.iteration < iteration.scope.iteration)
            .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let prior_interventions = outcomes
            .iter()
            .filter(|outcome| {
                outcome.output_development_evidence_fingerprint == iteration.development.fingerprint
            })
            .map(|outcome| outcome.intervention_fingerprint.clone())
            .collect();
        let mut repair_memory = Vec::new();
        for outcome in outcomes.iter().rev().take(32) {
            let summary = RepairOutcomeSummary::from(outcome);
            let mut candidate = repair_memory.clone();
            candidate.push(summary.clone());
            if serde_json::to_vec(&candidate)?.len() > 131_072 {
                break;
            }
            repair_memory.push(summary);
        }
        repair_memory.reverse();
        Ok(Self {
            scope: iteration.scope.clone(),
            failures,
            rows,
            landscape,
            investigation,
            current_evidence,
            baseline_evidence,
            repair_memory,
            prior_interventions,
        })
    }

    pub fn repair_metric_points(&self, plan: &RepairPlan) -> Result<Vec<NomosRepairMetricPoint>> {
        Ok(project_repair_metric_points(
            plan,
            &self.current_evidence,
            &self.baseline_evidence,
        )?)
    }

    pub fn templates(&self, proposal: &DatasetEditProposal) -> Result<NomosGenerationTemplates> {
        let ids = proposal
            .additions
            .iter()
            .map(|row| &row.template_row_id)
            .chain(proposal.removals.iter().map(|row| &row.row_id));
        let mut rows = BTreeMap::new();
        for id in ids {
            rows.insert(
                id.clone(),
                self.rows
                    .get(id)
                    .context("Proposal row is not in the pinned dataset")?
                    .clone(),
            );
        }
        Ok(NomosGenerationTemplates::new(rows)?)
    }

    fn validate(&self, scope: &AgentAnalysisScope) -> Result<(), OptimizationError> {
        if scope != &self.scope {
            return Err(OptimizationError::Validation(
                "Inspection scope differs from its recorded iteration".into(),
            ));
        }
        Ok(())
    }
}

pub(super) async fn result_repair_metric_points(
    folder: &Path,
    iteration: &ProjectOptimizationIteration,
    result: &project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult,
    plan: &RepairPlan,
) -> Result<(Vec<NomosRepairMetricPoint>, String)> {
    let run =
        project_workspace_local::optimization_runs::show(folder, iteration.scope.run_id).await?;
    let preparation = run.preparation.as_ref().context("Preparation missing")?;
    let binding = project_workspace_local::scientific_binding_history(folder)
        .await?
        .into_iter()
        .find(|value| {
            value.id.to_string() == preparation.execution_binding.id
                && value.fingerprint == preparation.execution_binding.fingerprint
        })
        .context("Pinned runtime missing")?;
    let training = project_workspace_local::optimization_iteration_execution::training(
        folder,
        iteration.scope.run_id,
        iteration.id,
    )
    .await?
    .context("Iteration training binding missing")?;
    ensure!(
        result.training_binding_fingerprint == training.fingerprint
            && result.fingerprint == result.reproduce()?,
        "Repair outcome development result changed"
    );
    let output_evidence =
        project_workspace_core::optimization_iteration::IterationDevelopmentEvidence::completed(
            &training,
            result,
            &iteration.development.benchmark_definition_fingerprint,
        )?;
    let store = super::super::super::open_bound_store(&folder.to_string_lossy(), &binding).await?;
    let projected: Result<_> = async {
        let runtime_project = super::super::super::load_bound_project(&store, &binding).await?;
        let project = store
            .get_project(training.scientific_project.id.parse()?)
            .await?
            .context("Iteration scientific project missing")?;
        let protocol = store
            .get_protocol(training.protocol.id.parse()?)
            .await?
            .context("Iteration scientific protocol missing")?;
        let backend = super::super::super::open_nomos_binding(folder, &binding, &runtime_project)?;
        let candidate = result
            .reports
            .values()
            .map(|report| {
                backend.read_development_evidence(&project, &protocol.metric_contract, report)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let baseline = protocol
            .baseline_development_reports()
            .into_iter()
            .map(|report| {
                backend.read_development_evidence(&project, &protocol.metric_contract, report)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(project_repair_metric_points(plan, &candidate, &baseline)?)
    }
    .await;
    store.pool().close().await;
    Ok((projected?, output_evidence.fingerprint))
}

impl OptimizationInspection for IterationInspection {
    fn repair_memory(&self, scope: AgentAnalysisScope) -> BoxFuture<'_, Vec<RepairOutcomeSummary>> {
        Box::pin(async move {
            self.validate(&scope)?;
            Ok(self.repair_memory.clone())
        })
    }

    fn development_failures(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            self.validate(&scope)?;
            page(self.failures.iter().cloned().map(Ok), offset, limit)
        })
    }
    fn training_rows(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
        query: Option<String>,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            self.validate(&scope)?;
            if query.as_ref().is_some_and(|q| q.chars().count() > 200) {
                return Err(OptimizationError::Validation(
                    "Training query exceeds 200 characters".into(),
                ));
            }
            let query = query.unwrap_or_default().trim().to_lowercase();
            let rows = self
                .rows
                .iter()
                .filter(|(id, row)| {
                    query.is_empty()
                        || id.to_lowercase().contains(&query)
                        || row["question"]
                            .as_str()
                            .is_some_and(|text| text.to_lowercase().contains(&query))
                })
                .map(|(id, row)| {
                    NomosBackend::training_row_for_agent(id, row)
                        .map_err(|error| OptimizationError::Validation(error.to_string()))
                });
            page(rows, offset, limit)
        })
    }

    fn dataset_landscape(
        &self,
        scope: AgentAnalysisScope,
        offset: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            self.validate(&scope)?;
            let summaries = if let Some(investigation) = &self.investigation {
                investigation.summaries()
            } else {
                self.landscape
                    .as_ref()
                    .ok_or_else(|| {
                        OptimizationError::Validation(
                            "Dataset landscape is unavailable for analysis protocol V1".into(),
                        )
                    })?
                    .summaries()
            };
            page(summaries.iter().cloned().map(Ok), offset, limit)
        })
    }

    fn dataset_cluster_rows(
        &self,
        scope: AgentAnalysisScope,
        cluster_ids: Vec<String>,
        examples_per_cluster: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            self.validate(&scope)?;
            let landscape = self.landscape.as_ref().ok_or_else(|| {
                OptimizationError::Validation(
                    "Dataset cluster inspection is unavailable for analysis protocol V1".into(),
                )
            })?;
            let rows = landscape
                .sample_rows(&self.rows, &cluster_ids, examples_per_cluster)
                .map_err(|error| OptimizationError::Validation(error.to_string()))?;
            page(rows.into_iter().map(Ok), 0, 20)
        })
    }

    fn dataset_investigation_rows(
        &self,
        scope: AgentAnalysisScope,
        cluster_ids: Vec<String>,
        cursor: u64,
        limit: u32,
    ) -> BoxFuture<'_, InspectionPage> {
        Box::pin(async move {
            self.validate(&scope)?;
            let investigation = self.investigation.as_ref().ok_or_else(|| {
                OptimizationError::Validation(
                    "Dataset investigation is unavailable for this analysis protocol".into(),
                )
            })?;
            let selected = investigation
                .sample_rows(&self.rows, &cluster_ids, cursor, limit)
                .map_err(|error| OptimizationError::Validation(error.to_string()))?;
            let returned = selected.items.len() as u64;
            let next_offset =
                (cursor + returned < selected.total_inspectable).then_some(cursor + returned);
            let page = InspectionPage {
                selection: Some(InspectionSelection {
                    method: selected.selection_method.into(),
                    cursor,
                    selected_count: selected.items.len() as u32,
                    total_eligible_count: selected.total_eligible,
                    total_inspectable_count: selected.total_inspectable,
                }),
                items: selected.items,
                next_offset,
            };
            page.validate(limit)?;
            Ok(page)
        })
    }

    fn repair_planning_context(
        &self,
        scope: AgentAnalysisScope,
        inspected_cluster_ids: Vec<String>,
        inspected_row_ids: Vec<String>,
    ) -> BoxFuture<'_, encoder_optimization_core::repair_strategy::RepairPlanningContext> {
        Box::pin(async move {
            self.validate(&scope)?;
            let mut context = self
                .investigation
                .as_ref()
                .ok_or_else(|| {
                    OptimizationError::Validation(
                        "Repair planning is unavailable for this analysis protocol".into(),
                    )
                })?
                .planning_context(
                    &self.rows,
                    &inspected_cluster_ids,
                    &inspected_row_ids,
                    scope.maximum_row_changes,
                )
                .map_err(|error| OptimizationError::Validation(error.to_string()))?;
            context
                .prior_interventions
                .clone_from(&self.prior_interventions);
            context.validate()?;
            Ok(context)
        })
    }
}

fn page(
    items: impl Iterator<Item = Result<InspectionItem, OptimizationError>>,
    offset: u64,
    limit: u32,
) -> Result<InspectionPage, OptimizationError> {
    if !(1..=20).contains(&limit) {
        return Err(OptimizationError::Validation(
            "Inspection page size must be 1–20".into(),
        ));
    }
    let skip = usize::try_from(offset).map_err(|_| {
        OptimizationError::Validation("Inspection offset exceeds this platform".into())
    })?;
    let mut items = items.skip(skip).peekable();
    let mut output = InspectionPage {
        items: Vec::new(),
        next_offset: None,
        selection: None,
    };
    // Leave room for JSON framing and the continuation offset. Large rows may
    // shorten a page; the cursor always advances by the rows actually returned.
    let mut bytes = 128;
    while output.items.len() < limit as usize {
        let Some(item) = items.next() else {
            break;
        };
        let item = item?;
        let size = serde_json::to_vec(&item)?.len() + 1;
        if bytes + size > 262144 {
            if output.items.is_empty() {
                return Err(OptimizationError::Validation(
                    "An inspection item exceeds the page limit".into(),
                ));
            }
            output.next_offset = Some(offset + output.items.len() as u64);
            break;
        }
        bytes += size;
        output.items.push(item);
    }
    if output.next_offset.is_none() && items.peek().is_some() {
        output.next_offset = Some(offset + output.items.len() as u64);
    }
    output.validate(limit)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_shorten_at_payload_boundary_without_losing_a_row() {
        let values: Vec<_> = (0..20)
            .map(|i| {
                let content = serde_json::json!({"text":"x".repeat(30000)});
                InspectionItem {
                    id: i.to_string(),
                    fingerprint: encoder_optimization_core::fingerprint(&content).unwrap(),
                    content,
                }
            })
            .collect();
        let mut offset = 0;
        let mut seen = Vec::new();
        loop {
            let result = page(values.iter().cloned().map(Ok), offset, 20).unwrap();
            seen.extend(result.items.into_iter().map(|i| i.id));
            match result.next_offset {
                Some(next) => {
                    assert!(next > offset);
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(seen, (0..20).map(|i| i.to_string()).collect::<Vec<_>>());
        assert!(page(std::iter::empty(), 0, 0).is_err());
        assert!(
            page(values.into_iter().map(Ok), 1000, 20)
                .unwrap()
                .items
                .is_empty()
        );
    }
}
