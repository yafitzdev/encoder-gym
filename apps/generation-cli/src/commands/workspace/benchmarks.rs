use crate::cli::WorkspaceBenchmarkCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition, journal::replay_experiment, ports::ExperimentStore,
};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    ActivityEventState, ActivityFailure, ActivityReference, ActivitySource, BenchmarkSource,
    BoundIdentity, ProjectBenchmarkVersion,
};
use project_workspace_local::{
    AppendActivity, append_activity, benchmarks, initialize_activity, open_workspace,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkPreview {
    version_id: Uuid,
    expected_parent: Option<Uuid>,
    existing_version: Option<Uuid>,
    definition: BenchmarkDefinition,
    source: BenchmarkSource,
}

pub(super) async fn execute(folder: &Path, command: WorkspaceBenchmarkCommand) -> Result<()> {
    use WorkspaceBenchmarkCommand::*;
    match command {
        List => super::print(&benchmarks::list(folder).await?),
        PreviewRun { run_id } => super::print(&preview(folder, run_id).await?),
        AdoptRun {
            run_id,
            expected_parent,
        } => adopt(folder, run_id, expected_parent).await,
        Inspect { version_id } => super::print(&inspect(folder, version_id).await?),
        Results { version_id } => super::print(&results(folder, version_id).await?),
    }
}

async fn results(
    folder: &Path,
    version_id: Uuid,
) -> Result<project_workspace_core::benchmark_results::ProjectBenchmarkResults> {
    use project_workspace_core::benchmark_results::{
        BenchmarkRunEvidence, ProjectBenchmarkResults,
    };
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model inventory is not initialized.")?;
    let version = inspect(folder, version_id).await?;
    let mut results = ProjectBenchmarkResults::new(version, catalog)?;
    for binding in project_workspace_local::scientific_binding_history(folder).await? {
        let backend = &results.version.definition.backend;
        if binding.adapter.key != backend.name
            || binding.adapter.protocol != backend.protocol_version
            || binding.adapter.configuration_fingerprint != backend.configuration_fingerprint
        {
            continue;
        }
        let store = super::open_bound_store(&workspace.folder, &binding).await?;
        let project = super::load_bound_project(&store, &binding).await?;
        for run_id in store.experiment_run_ids_for_project(project.id).await? {
            let events = store.load_events(run_id).await?;
            let first = events
                .first()
                .context("Recorded experiment journal is missing.")?;
            ensure!(
                first.run_id == run_id,
                "Experiment lookup and journal disagree."
            );
            let protocol = store
                .get_protocol(first.protocol_id)
                .await?
                .context("Experiment protocol is missing.")?;
            let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
            if definition != results.version.definition {
                continue;
            }
            results.include_run(
                catalog,
                BenchmarkRunEvidence {
                    binding: &binding,
                    project: &project,
                    protocol: &protocol,
                    definition: &definition,
                    events: &events,
                },
            )?;
        }
        store.pool().close().await;
    }
    Ok(results)
}

async fn preview(folder: &Path, run_id: Uuid) -> Result<BenchmarkPreview> {
    let workspace = open_workspace(folder, false).await?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect an evaluation runtime first.")?;
    let store = super::open_bound_store(&workspace.folder, binding).await?;
    let project = super::load_bound_project(&store, binding).await?;
    let events = store.load_events(run_id).await?;
    let first = events.first().context("Experiment run is missing.")?;
    let protocol = store
        .get_protocol(first.protocol_id)
        .await?
        .context("Experiment protocol is missing.")?;
    replay_experiment(&project, &protocol, &events)?;
    let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
    store.pool().close().await;
    let source = BenchmarkSource {
        scientific_binding: BoundIdentity {
            id: binding.id.to_string(),
            fingerprint: binding.fingerprint.clone(),
        },
        project_snapshot: binding.runtime.project_snapshot.clone(),
        protocol: BoundIdentity {
            id: protocol.id.to_string(),
            fingerprint: protocol.fingerprint.clone(),
        },
    };
    let existing_version = workspace
        .benchmark_versions
        .iter()
        .find(|v| v.definition.fingerprint == definition.fingerprint)
        .map(|v| v.id);
    let mut hash = Sha256::new();
    hash.update(b"project-benchmark-version-v1");
    hash.update(workspace.manifest.id.as_bytes());
    hash.update(definition.fingerprint.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().expect("16 digest bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(BenchmarkPreview {
        version_id: existing_version.unwrap_or_else(|| Uuid::from_bytes(bytes)),
        expected_parent: workspace.benchmark_versions.last().map(|v| v.id),
        existing_version,
        definition,
        source,
    })
}

async fn inspect(folder: &Path, version_id: Uuid) -> Result<ProjectBenchmarkVersion> {
    let workspace = open_workspace(folder, false).await?;
    let (version, binding) = benchmarks::inspect(folder, version_id).await?;
    let store = super::open_bound_store(&workspace.folder, &binding).await?;
    let project = super::load_bound_project(&store, &binding).await?;
    let protocol = store
        .get_protocol(version.source.protocol.id.parse()?)
        .await?
        .context("The benchmark's original protocol is missing.")?;
    ensure!(
        protocol.fingerprint == version.source.protocol.fingerprint,
        "Benchmark protocol identity changed."
    );
    let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
    ensure!(
        definition == version.definition,
        "Benchmark differs from its original scientific definition."
    );
    store.pool().close().await;
    Ok(version)
}

async fn adopt(folder: &Path, run_id: Uuid, expected_parent: Option<Uuid>) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let refs = vec![ActivityReference::new("run", run_id.to_string())?];
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "benchmark.adopt_run".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        references,
        failure,
        created_at: Utc::now(),
    };
    append_activity(
        folder,
        event(ActivityEventState::Started, refs.clone(), None),
    )
    .await?;
    let result: Result<ProjectBenchmarkVersion> = async {
        let preview = preview(folder, run_id).await?;
        benchmarks::record(
            folder,
            preview.version_id,
            expected_parent,
            preview.definition,
            preview.source,
        )
        .await
    }
    .await;
    let mut references = refs;
    let (state, failure) = match &result {
        Ok(version) => {
            references.push(ActivityReference::new(
                "benchmark_version",
                version.id.to_string(),
            )?);
            references.push(ActivityReference::new(
                "protocol",
                version.source.protocol.id.clone(),
            )?);
            (ActivityEventState::Succeeded, None)
        }
        Err(_) => (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "benchmark_adoption_failed",
                "Benchmark was not adopted. Existing versions and scientific records are retained; retry reuses the same version.",
            )?),
        ),
    };
    append_activity(folder, event(state, references, failure)).await?;
    super::print(&serde_json::json!({"actionId":action_id,"version":result?}))
}
