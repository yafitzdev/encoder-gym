//! Exact, development-only Agent input adapter. Load selected sources once per
//! execution, then serve bounded projections without rehashing on every tool.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, ensure};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_experiment_nomos::{NomosBackend, NomosGenerationTemplates};
use encoder_optimization_core::{
    OptimizationError,
    agent::{AgentAnalysisScope, DatasetEditProposal, InspectionItem, InspectionPage},
    ports::{BoxFuture, OptimizationInspection},
};
use project_workspace_core::optimization_iteration::ProjectOptimizationIteration;
use project_workspace_local::dataset_versions;
use serde_json::Value;

pub(super) struct IterationInspection {
    scope: AgentAnalysisScope,
    failures: Vec<InspectionItem>,
    rows: BTreeMap<String, Value>,
}

impl IterationInspection {
    pub async fn load(folder: &Path, iteration: &ProjectOptimizationIteration) -> Result<Self> {
        let (benchmark, binding) =
            project_workspace_local::benchmarks::inspect(folder, iteration.benchmark.id.parse()?)
                .await?;
        iteration.validate_benchmark(&benchmark)?;
        let store =
            super::super::super::open_bound_store(&folder.to_string_lossy(), &binding).await?;
        let result: Result<_> = async {
            let project = super::super::super::load_bound_project(&store, &binding).await?;
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
            let backend = super::super::super::open_nomos_binding(&binding, &project)?;
            let mut failures = Vec::new();
            for (suite, identity) in &iteration.development.reports {
                let report = protocol
                    .baseline_development_reports()
                    .into_iter()
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
                failures.extend(evidence.failures);
            }
            Ok(failures)
        }
        .await;
        store.pool().close().await;
        let failures = result?;
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
            .collect();
        Ok(Self {
            scope: iteration.scope.clone(),
            failures,
            rows,
        })
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

impl OptimizationInspection for IterationInspection {
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
