use crate::cli::WorkspaceBenchmarkCommand;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    benchmark::BenchmarkDefinition, domain::OptimizationBudget, journal::replay_experiment,
    ports::ExperimentStore,
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
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
        Initialize { expected_parent } => initialize(folder, expected_parent).await,
        PreviewRun { run_id } => super::print(&preview(folder, run_id).await?),
        AdoptRun {
            run_id,
            expected_parent,
            expected_definition,
        } => adopt(folder, run_id, expected_parent, expected_definition).await,
        Inspect { version_id } => super::print(&inspect(folder, version_id).await?),
        Results { version_id } => super::print(&results(folder, version_id).await?),
    }
}

async fn initialize(folder: &Path, expected_parent: Option<Uuid>) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "benchmark.initialize".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references,
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, vec![], None)).await?;
    let result: Result<(ProjectBenchmarkVersion, Uuid)> = async {
        let workspace = open_workspace(folder, false).await?;
        let binding = workspace
            .scientific_binding
            .as_ref()
            .context("Connect an evaluation runtime first.")?;
        let store = super::open_bound_store_mutable(&workspace.folder, binding).await?;
        let project = super::load_bound_project(&store, binding).await?;
        let backend = super::open_nomos_binding(&workspace.folder, binding, &project)?
            .with_progress_observer(std::sync::Arc::new(
                crate::commands::encoder_optimize::activity::ProgressOutput,
            ));
        let plan = NomosBackend::initial_benchmark_plan(&project)?;
        let candidate_id = derived_uuid(
            "initial-benchmark-candidate-v1",
            &serde_json::json!({
                "project":project.id,
                "projectFingerprint":project.fingerprint,
                "metricContract":plan.metric_contract.fingerprint,
                "developmentSuites":plan.development_suite_keys,
                "sealedSuite":plan.sealed_suite_key,
            }),
        )?;
        let candidate =
            NomosBackend::initial_training_candidate_definition(candidate_id, &project, 1)?;
        let protocol_id = derived_uuid(
            "initial-benchmark-protocol-v1",
            &serde_json::json!({
                "project":project.id,
                "projectFingerprint":project.fingerprint,
                "metricContract":plan.metric_contract.fingerprint,
                "developmentSuites":plan.development_suite_keys,
                "sealedSuite":plan.sealed_suite_key,
                "maximumEvaluationSeconds":plan.maximum_evaluation_seconds,
                "candidate":candidate.fingerprint,
            }),
        )?;
        let development_evaluations = u32::try_from(plan.development_suite_keys.len())?;
        let runner = ExperimentRunner::new(&store, &backend);
        let protocol = runner
            .prepare_multi_protocol_identified(
                protocol_id,
                project.id,
                plan.metric_contract,
                OptimizationBudget {
                    maximum_candidates: 1,
                    maximum_training_seconds: 1,
                    maximum_development_evaluations: development_evaluations,
                    maximum_sealed_evaluations: 1,
                },
                plan.maximum_evaluation_seconds,
                plan.development_suite_keys,
                plan.sealed_suite_key,
                vec![candidate],
            )
            .await?;
        let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
        NomosBackend::verify_benchmark_definition(&project, &definition)?;
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
        store.pool().close().await;
        let version = benchmarks::record(
            folder,
            benchmark_version_id(workspace.manifest.id, &definition)?,
            expected_parent,
            definition,
            source,
        )
        .await?;
        Ok((version, protocol_id))
    }
    .await;
    let (state, references, failure) = match &result {
        Ok((version, protocol_id)) => (
            ActivityEventState::Succeeded,
            vec![
                ActivityReference::new("benchmark_version", version.id.to_string())?,
                ActivityReference::new("protocol", protocol_id.to_string())?,
            ],
            None,
        ),
        Err(_) => (
            ActivityEventState::Failed,
            vec![],
            Some(ActivityFailure::new(
                "benchmark_initialization_failed",
                "The benchmark was not created. Existing evaluation evidence is retained and retryable.",
            )?),
        ),
    };
    append_activity(folder, event(state, references, failure)).await?;
    let (version, _) = result?;
    super::print(&serde_json::json!({"actionId":action_id,"version":version}))
}

mod iteration_results;
mod optimization_results;

/// A run report consumes only that root's scientific children. Reuse the full
/// benchmark/iteration validators without projecting unrelated experiment runs.
pub(super) async fn iteration_run_results(
    folder: &Path,
    version_id: Uuid,
    catalog: &project_workspace_core::ModelCatalog,
    run: &project_workspace_core::ProjectOptimizationRunView,
) -> Result<project_workspace_core::benchmark_results::ProjectBenchmarkResults> {
    let version = inspect(folder, version_id).await?;
    let mut results =
        project_workspace_core::benchmark_results::ProjectBenchmarkResults::new(version, catalog)?;
    iteration_results::include_runs(folder, catalog, &mut results, std::slice::from_ref(run))
        .await?;
    Ok(results)
}

pub(super) async fn results(
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
    optimization_results::include(folder, catalog, &mut results).await?;
    iteration_results::include(folder, catalog, &mut results).await?;
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
    Ok(BenchmarkPreview {
        version_id: existing_version.map_or_else(
            || benchmark_version_id(workspace.manifest.id, &definition),
            Ok,
        )?,
        expected_parent: workspace.benchmark_versions.last().map(|v| v.id),
        existing_version,
        definition,
        source,
    })
}

fn benchmark_version_id(project_id: Uuid, definition: &BenchmarkDefinition) -> Result<Uuid> {
    let mut hash = Sha256::new();
    hash.update(b"project-benchmark-version-v1");
    hash.update(project_id.as_bytes());
    hash.update(definition.fingerprint.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().expect("16 digest bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

fn derived_uuid(schema: &str, value: &impl Serialize) -> Result<Uuid> {
    let mut hash = Sha256::new();
    hash.update(schema.as_bytes());
    hash.update(serde_json::to_vec(value)?);
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().expect("16 digest bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

pub(super) async fn inspect(folder: &Path, version_id: Uuid) -> Result<ProjectBenchmarkVersion> {
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

async fn adopt(
    folder: &Path,
    run_id: Uuid,
    expected_parent: Option<Uuid>,
    expected_definition: Option<String>,
) -> Result<()> {
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
        narrative: None,
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
        ensure!(
            expected_definition
                .as_ref()
                .is_none_or(|expected| *expected == preview.definition.fingerprint),
            "Benchmark definition changed since preview. Review it before saving."
        );
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
