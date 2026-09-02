use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use encoder_experiment_core::{
    domain::{OptimizationBudget, ParameterValue, TrainingCandidate},
    journal::{ExperimentView, replay_experiment},
    metrics::{MetricContract, MetricDefinition, MetricGate},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde::Deserialize;

use crate::{
    cli::{
        ExperimentAuthorizeArgs, ExperimentCommand, ExperimentPrepareArgs, ExperimentProtocolArgs,
        ExperimentRunArgs, NomosWorkspaceArgs,
    },
    presentation,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolInput {
    budget: OptimizationBudget,
    maximum_evaluation_seconds: u64,
    #[serde(default)]
    development_suite_key: Option<String>,
    #[serde(default)]
    development_suite_keys: Vec<String>,
    sealed_suite_key: String,
    metric_definitions: Vec<MetricDefinition>,
    primary_metric: String,
    gates: Vec<MetricGate>,
    candidates: Vec<CandidateInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateInput {
    maximum_training_seconds: u64,
    parameters: BTreeMap<String, ParameterValue>,
}

pub async fn execute(command: ExperimentCommand, database_url: &str) -> anyhow::Result<()> {
    let backend_args = backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &backend_args.workspace)?;
    let store = SqliteExperimentStore::connect(database_url).await?;
    let backend = NomosBackend::open(&backend_args.workspace, backend_args.python.clone())?;
    let runner = ExperimentRunner::new(&store, &backend);

    match command {
        ExperimentCommand::NomosVerify(_) => {
            let project = backend.project_snapshot()?;
            let inspection = backend.inspect(project.clone()).await?;
            presentation::print(&serde_json::json!({
                "verified": true,
                "project_name": project.name,
                "source_revision": project.source_revision,
                "source_fingerprint": project.source_fingerprint,
                "baseline_model_fingerprint": project.baseline_model.fingerprint,
                "immutable_inputs": inspection.verified_artifact_keys,
                "adapter": project.backend,
            }))
        }
        ExperimentCommand::NomosRegister(_) => {
            let project = runner.register_project(backend.project_snapshot()?).await?;
            presentation::print(&serde_json::json!({
                "project_id": project.id,
                "project_name": project.name,
                "source_revision": project.source_revision,
                "source_fingerprint": project.source_fingerprint,
                "project_fingerprint": project.fingerprint,
                "adapter": project.backend,
            }))
        }
        ExperimentCommand::Prepare(args) => prepare(args, &runner, &store).await,
        ExperimentCommand::Start(ExperimentProtocolArgs { protocol_id, .. }) => {
            let view = runner.create_run(protocol_id).await?;
            presentation::print(&view)
        }
        ExperimentCommand::RunDevelopment(ExperimentRunArgs { run_id, .. }) => {
            let view = runner.run_development(run_id).await?;
            presentation::print(&view)
        }
        ExperimentCommand::Status(ExperimentRunArgs { run_id, .. }) => {
            let view = load_verified_view(run_id, &store, &backend).await?;
            presentation::print(&view)
        }
        ExperimentCommand::AuthorizeSealed(ExperimentAuthorizeArgs {
            run_id,
            authorized_by,
            ..
        }) => {
            let view = runner.authorize_sealed(run_id, authorized_by).await?;
            presentation::print(&view)
        }
        ExperimentCommand::RunSealed(ExperimentRunArgs { run_id, .. }) => {
            let view = runner.run_sealed(run_id).await?;
            presentation::print(&view)
        }
    }
}

async fn prepare(
    args: ExperimentPrepareArgs,
    runner: &ExperimentRunner<'_, SqliteExperimentStore, NomosBackend>,
    store: &SqliteExperimentStore,
) -> anyhow::Result<()> {
    let input = read_protocol_input(&args.file)?;
    let project = store
        .get_project(args.project_id)
        .await?
        .with_context(|| format!("experiment project {} does not exist", args.project_id))?;
    let contract =
        MetricContract::create(input.metric_definitions, input.primary_metric, input.gates)?;
    let candidates = input
        .candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| {
            Ok(TrainingCandidate::create(
                &project,
                u32::try_from(index + 1).context("candidate sequence overflowed")?,
                candidate.maximum_training_seconds,
                candidate.parameters,
            )?)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let protocol = match (input.development_suite_key, input.development_suite_keys) {
        (Some(suite_key), suite_keys) if suite_keys.is_empty() => {
            runner
                .prepare_protocol(
                    project.id,
                    contract,
                    input.budget,
                    input.maximum_evaluation_seconds,
                    suite_key,
                    input.sealed_suite_key,
                    candidates,
                )
                .await?
        }
        (None, suite_keys) if !suite_keys.is_empty() => {
            runner
                .prepare_multi_protocol(
                    project.id,
                    contract,
                    input.budget,
                    input.maximum_evaluation_seconds,
                    suite_keys,
                    input.sealed_suite_key,
                    candidates,
                )
                .await?
        }
        _ => anyhow::bail!(
            "protocol must define exactly one of development_suite_key or development_suite_keys"
        ),
    };
    let baseline_development_metrics = protocol
        .baseline_development_reports()
        .into_iter()
        .map(|report| (report.suite_key.clone(), report.metrics.clone()))
        .collect::<BTreeMap<_, _>>();
    presentation::print(&serde_json::json!({
        "protocol_id": protocol.id,
        "protocol_fingerprint": protocol.fingerprint,
        "project_id": protocol.project_snapshot_id,
        "candidate_count": protocol.candidates.len(),
        "budget": protocol.budget,
        "baseline_development_metrics": baseline_development_metrics,
        "baseline_sealed_reference": {
            "status": "pinned_hidden_until_final_acceptance",
            "report_id": protocol.baseline_sealed_report.id,
            "fingerprint": protocol.baseline_sealed_report.fingerprint,
        },
        "metric_contract_fingerprint": protocol.metric_contract.fingerprint,
    }))
}

async fn load_verified_view(
    run_id: uuid::Uuid,
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
) -> anyhow::Result<ExperimentView> {
    let events = store.load_events(run_id).await?;
    let first = events
        .first()
        .with_context(|| format!("experiment run {run_id} does not exist"))?;
    let protocol = store
        .get_protocol(first.protocol_id)
        .await?
        .context("experiment protocol does not exist")?;
    let project = store
        .get_project(protocol.project_snapshot_id)
        .await?
        .context("experiment project does not exist")?;
    backend.inspect(project.clone()).await?;
    Ok(replay_experiment(&project, &protocol, &events)?)
}

fn read_protocol_input(path: &Path) -> anyhow::Result<ProtocolInput> {
    let metadata = path
        .metadata()
        .with_context(|| format!("could not inspect protocol input {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        anyhow::bail!("experiment protocol input must be a file of at most 1 MiB");
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read protocol input {}", path.display()))?;
    serde_json::from_slice(&bytes).context("experiment protocol input is invalid strict JSON")
}

fn backend_args(command: &ExperimentCommand) -> &NomosWorkspaceArgs {
    match command {
        ExperimentCommand::NomosVerify(args) | ExperimentCommand::NomosRegister(args) => args,
        ExperimentCommand::Prepare(args) => &args.backend,
        ExperimentCommand::Start(args) => &args.backend,
        ExperimentCommand::RunDevelopment(args)
        | ExperimentCommand::Status(args)
        | ExperimentCommand::RunSealed(args) => &args.backend,
        ExperimentCommand::AuthorizeSealed(args) => &args.backend,
    }
}

fn ensure_database_belongs_to_workspace(
    database_url: &str,
    workspace: &Path,
) -> anyhow::Result<()> {
    let workspace = workspace
        .canonicalize()
        .context("could not resolve isolated Nomos workspace")?;
    let raw = database_url
        .strip_prefix("sqlite://")
        .context("experiment database must use an explicit sqlite:// file URL")?
        .split('?')
        .next()
        .context("experiment database URL has no path")?;
    if raw.is_empty() || raw.contains('%') {
        anyhow::bail!("experiment database URL must contain a plain local file path");
    }
    let raw = if raw.len() >= 4
        && raw.starts_with('/')
        && raw.as_bytes()[2] == b':'
        && raw.as_bytes()[1].is_ascii_alphabetic()
    {
        &raw[1..]
    } else {
        raw
    };
    let database = std::path::PathBuf::from(raw.replace('/', std::path::MAIN_SEPARATOR_STR));
    let database = if database.is_absolute() {
        database
    } else {
        std::env::current_dir()?.join(database)
    };
    let parent = database
        .parent()
        .context("experiment database path has no parent")?
        .canonicalize()
        .context("experiment database parent does not exist")?;
    let database = parent.join(
        database
            .file_name()
            .context("experiment database path has no filename")?,
    );
    if !database.starts_with(&workspace) {
        anyhow::bail!("experiment database must remain inside the isolated Nomos workspace");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_database_is_fenced_to_the_isolated_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let inside = workspace.path().join("encoder-gym.sqlite");
        let inside_url = format!("sqlite://{}", inside.to_string_lossy().replace('\\', "/"));
        assert!(ensure_database_belongs_to_workspace(&inside_url, workspace.path()).is_ok());
        assert!(
            ensure_database_belongs_to_workspace(
                "sqlite://C:/Users/source/nomos.sqlite",
                workspace.path()
            )
            .is_err()
        );
        assert!(ensure_database_belongs_to_workspace("sqlite::memory:", workspace.path()).is_err());
    }
}
