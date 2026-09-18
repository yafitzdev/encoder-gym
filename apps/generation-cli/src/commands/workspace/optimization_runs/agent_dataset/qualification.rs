//! Handoff of the immutable Agent result into native full-population clearance.
//! Never rewrites the root setup or uses its single-candidate training receipt.
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_nomos::{NomosTrainingClearance, NomosTrainingDataset};
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityNarrative, ActivityNarrativeKind,
    ActivityNarrativeOrigin, ActivityReference, ActivitySource,
};
use project_workspace_local::{
    dataset_versions, optimization_dataset::OptimizationDatasetPublication,
    optimization_iterations, optimization_runs,
};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

const QUALIFICATION_REJECTED: &str = "Derived dataset rejected before training:";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QualifiedDataset {
    pub(super) native_dataset: NomosTrainingDataset,
    pub(super) clearance: NomosTrainingClearance,
}

fn rejection_message(clearance: &NomosTrainingClearance) -> String {
    format!(
        "{QUALIFICATION_REJECTED} {} invalid rows, {} duplicate model inputs, and {} benchmark overlaps; no training started",
        clearance.invalid_rows, clearance.duplicate_rows, clearance.overlap_rows
    )
}

pub(super) async fn prepare(
    folder: &Path,
    run_id: Uuid,
    publication: &OptimizationDatasetPublication,
) -> Result<QualifiedDataset> {
    let action_id = Uuid::new_v4();
    let event = |state, narrative, failure| project_workspace_local::AppendActivity {
        action_id,
        operation: "optimization.qualification".into(),
        source: ActivitySource::Cli,
        state,
        stage: (state == ActivityEventState::Progress).then(|| "checking_training_data".into()),
        completed: None,
        total: None,
        narrative,
        failure,
        references: vec![
            ActivityReference::new("run_stage", "preparing_data").expect("stage"),
            ActivityReference::new("run", run_id.to_string()).expect("UUID"),
            ActivityReference::new("iteration", publication.iteration.to_string())
                .expect("iteration"),
            ActivityReference::new("dataset_version", publication.version.id.to_string())
                .expect("UUID"),
        ],
        created_at: Utc::now(),
    };
    project_workspace_local::append_activity(
        folder,
        event(ActivityEventState::Started, None, None),
    )
    .await?;
    project_workspace_local::append_activity(folder, event(ActivityEventState::Progress, Some(ActivityNarrative::new(
        ActivityNarrativeOrigin::System, ActivityNarrativeKind::Intent,
        "Checking the complete candidate dataset: native schema, duplicate inputs and benchmark isolation.",
    )?), None)).await?;
    let result = prepare_native(folder, run_id, publication).await;
    let terminal = match &result {
        Ok(_) => event(ActivityEventState::Succeeded, None, None),
        Err(error) => {
            let message = error.to_string();
            let safe = if message.starts_with(QUALIFICATION_REJECTED) {
                message.as_str()
            } else {
                "Dataset qualification did not complete cleanly; no training started."
            };
            event(
                ActivityEventState::Failed,
                None,
                Some(ActivityFailure::new("dataset_qualification_failed", safe)?),
            )
        }
    };
    project_workspace_local::append_activity(folder, terminal).await?;
    result
}

async fn prepare_native(
    folder: &Path,
    run_id: Uuid,
    publication: &OptimizationDatasetPublication,
) -> Result<QualifiedDataset> {
    let run = optimization_runs::show(folder, run_id).await?;
    ensure!(
        !run.state.is_terminal(),
        "Run stopped before dataset qualification"
    );
    let iteration = optimization_iterations::list(folder, run_id)
        .await?
        .into_iter()
        .find(|value| value.scope.iteration == publication.iteration)
        .context("Dataset publication has no recorded iteration")?;
    ensure!(
        publication.run_id == run_id && publication.parent == iteration.dataset,
        "Dataset publication changed its iteration inputs"
    );
    let version = dataset_versions::inspect(folder, publication.version.id).await?;
    ensure!(
        version.reference() == publication.version,
        "Derived dataset changed"
    );
    let preparation = run
        .preparation
        .as_ref()
        .context("Run preparation is missing")?;
    let workspace = project_workspace_local::open_workspace(folder, false).await?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Native runtime is missing")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after preparation"
    );
    let (benchmark, _) =
        project_workspace_local::benchmarks::inspect(folder, iteration.benchmark.id.parse()?)
            .await?;
    iteration.validate_benchmark(&benchmark)?;
    let store = super::super::super::open_bound_store(&workspace.folder, binding).await?;
    let loaded = super::super::super::load_bound_project(&store, binding).await;
    store.pool().close().await;
    let project = loaded?;
    let backend = super::super::super::open_nomos_binding(binding, &project)?;
    let rows = dataset_versions::materialization_rows(folder, version.id).await?;
    ensure!(
        rows.len() == version.members.len(),
        "Derived dataset membership is incomplete"
    );
    // An iteration-owned native key, distinct from the fixed root's dataset.
    let mut writer = backend.materialize_training_dataset(
        iteration.id,
        version.id,
        &version.fingerprint,
        rows.len() as u64,
    )?;
    for row in rows {
        writer.append(&row.member.id, &row.member.content_fingerprint, &row.value)?;
    }
    let native_dataset = writer.finish()?;
    let clearance = backend
        .qualify_training_dataset(&project, &benchmark.definition, &native_dataset)
        .await?;
    ensure!(
        !optimization_runs::show(folder, run_id)
            .await?
            .state
            .is_terminal(),
        "Run stopped during dataset qualification"
    );
    if !clearance.training_allowed() {
        anyhow::bail!(rejection_message(&clearance));
    }
    Ok(QualifiedDataset {
        native_dataset,
        clearance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualification_rejection_reports_only_safe_aggregate_counts() {
        let clearance = NomosTrainingClearance {
            protocol: "nomos-training-clearance-v2".into(),
            request_fingerprint: format!("sha256:{}", "1".repeat(64)),
            training_rows: 6_864,
            benchmark_rows: 2_512,
            invalid_rows: 0,
            duplicate_rows: 549,
            overlap_rows: 0,
            missing_group_rows: 0,
            missing_lineage_rows: 0,
            fingerprint: String::new(),
        };
        assert_eq!(
            rejection_message(&clearance),
            "Derived dataset rejected before training: 0 invalid rows, 549 duplicate model inputs, and 0 benchmark overlaps; no training started"
        );
    }
}
