use std::{sync::Arc, time::Duration};

use anyhow::Context;
use generation_core::{
    domain::GenerationParameters,
    jobs::{GenerationJob, JobRunner, JobRunnerPolicy},
    planning::calculate_generation_needs,
    ports::{BackendConfigurationStore, GenerationBackend, JobStore, PlanStore, RowStore},
    validation::ValidationPipeline,
};
use generation_fake::FakeGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
use project_config::GenerationBackendKind;
use recovery_core::{RecoveryState, RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::{BackendKind, GenerateArgs, ResumeGenerationArgs};

use super::config;

pub async fn execute(args: GenerateArgs, store: SqliteStore) -> anyhow::Result<()> {
    let options = ExecutionOptions::from_generate(args);
    let configured = options
        .config
        .as_deref()
        .map(config::resolve_path)
        .transpose()?;
    let plan = store
        .get_plan(options.plan_id.context("generation plan is required")?)
        .await?
        .context("generation plan not found")?;
    let accepted = store
        .dataset_cell_counts(plan.dataset_id)
        .await?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let requested_rows = calculate_generation_needs(&plan, &accepted)
        .iter()
        .map(|need| u64::from(need.remaining_count))
        .sum();
    let backend_kind = options
        .backend
        .unwrap_or_else(|| configured_backend(&configured));
    let (backend, parameters) =
        build_backend(backend_kind, &options, configured.as_ref(), &store).await?;

    let job = GenerationJob::queued(
        plan.dataset_id,
        plan.id,
        backend.name(),
        backend.model(),
        requested_rows,
    );
    store.create_job(&job).await?;
    eprintln!(
        "generation job {} queued for {} rows",
        job.id, job.requested_rows
    );

    let policy = runner_policy(&options, configured.as_ref());
    run_job(job.id, backend, parameters, policy, store, false).await
}

pub async fn resume(args: ResumeGenerationArgs, store: SqliteStore) -> anyhow::Result<()> {
    let mut options = ExecutionOptions::from_resume(args);
    let job_id = options.job_id.context("generation job is required")?;
    let job = store
        .get_job(job_id)
        .await?
        .with_context(|| format!("generation job not found: {job_id}"))?;
    options.plan_id = Some(job.plan_id);
    let configured = options
        .config
        .as_deref()
        .map(config::resolve_path)
        .transpose()?;
    let backend_kind = match job.backend_name.as_str() {
        "fake" => BackendKind::Fake,
        "openai-compatible" => BackendKind::OpenaiCompatible,
        name => anyhow::bail!("generation backend {name:?} is not available for local resume"),
    };
    let (backend, parameters) =
        build_backend(backend_kind, &options, configured.as_ref(), &store).await?;
    anyhow::ensure!(
        backend.name() == job.backend_name && backend.model() == job.backend_model,
        "resume backend identity does not match the interrupted job: expected {}/{}, got {}/{}",
        job.backend_name,
        job.backend_model,
        backend.name(),
        backend.model()
    );
    let policy = runner_policy(&options, configured.as_ref());
    store
        .acquire_execution_lease(WorkflowKind::Generation, job_id)
        .await?;
    if let Err(error) = store.prepare_generation_resume(job_id).await {
        store
            .release_execution_lease(WorkflowKind::Generation, job_id)
            .await?;
        return Err(error.into());
    }
    run_job(job_id, backend, parameters, policy, store, true).await
}

/// Executes an ordinary generation job for workflow orchestration without
/// writing presentation output. The returned job is the persisted terminal fact.
pub(crate) async fn run_workflow(
    plan_id: uuid::Uuid,
    configured: &project_config::ResolvedProjectConfig,
    store: SqliteStore,
) -> anyhow::Result<GenerationJob> {
    let plan = store
        .get_plan(plan_id)
        .await?
        .with_context(|| format!("generation plan not found: {plan_id}"))?;
    let accepted = store
        .dataset_cell_counts(plan.dataset_id)
        .await?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect();
    let requested_rows = calculate_generation_needs(&plan, &accepted)
        .iter()
        .map(|need| u64::from(need.remaining_count))
        .sum();
    let options = ExecutionOptions {
        plan_id: Some(plan_id),
        job_id: None,
        backend: None,
        batch_size: None,
        max_retries: None,
        max_attempt_multiplier: None,
        api_key_env: None,
        config: None,
    };
    let backend_kind = match configured.generation.backend {
        GenerationBackendKind::Fake => BackendKind::Fake,
        GenerationBackendKind::OpenaiCompatible => BackendKind::OpenaiCompatible,
    };
    let (backend, parameters) =
        build_backend(backend_kind, &options, Some(configured), &store).await?;
    let job = GenerationJob::queued(
        plan.dataset_id,
        plan.id,
        backend.name(),
        backend.model(),
        requested_rows,
    );
    store.create_job(&job).await?;
    store
        .acquire_execution_lease(WorkflowKind::Generation, job.id)
        .await?;
    let runner = JobRunner::new(
        Arc::new(store.clone()),
        backend,
        runner_policy(&options, Some(configured)),
        ValidationPipeline::standard(None),
    );
    let result = runner.run(job.id, parameters).await;
    let release = store
        .release_execution_lease(WorkflowKind::Generation, job.id)
        .await;
    let completed = result?;
    release?;
    Ok(completed)
}

async fn run_job(
    job_id: uuid::Uuid,
    backend: Arc<dyn GenerationBackend>,
    parameters: GenerationParameters,
    policy: JobRunnerPolicy,
    store: SqliteStore,
    recovery_resume: bool,
) -> anyhow::Result<()> {
    if !recovery_resume {
        store
            .acquire_execution_lease(WorkflowKind::Generation, job_id)
            .await?;
    }

    let runner = JobRunner::new(
        Arc::new(store.clone()),
        backend,
        policy,
        ValidationPipeline::standard(None),
    );
    let mut handle = tokio::spawn(async move { runner.run(job_id, parameters).await });
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut cancellation_sent = false;
    let result: anyhow::Result<_> = loop {
        tokio::select! {
            result = &mut handle => break result.context("generation worker panicked")?.map_err(Into::into),
            _ = interval.tick() => {
                if let Some(current) = store.get_job(job_id).await? {
                    eprint!(
                        "\r{:?}: accepted {} rejected {} failed requests {} remaining {}   ",
                        current.state,
                        current.accepted_rows,
                        current.rejected_rows,
                        current.failed_requests,
                        current.remaining_rows(),
                    );
                }
            }
            result = tokio::signal::ctrl_c(), if !cancellation_sent => {
                result.context("could not listen for Ctrl+C")?;
                cancellation_sent = store.request_job_cancellation(job_id).await?;
                eprintln!("\ncancellation requested; finishing the active batch");
            }
        }
    };
    eprintln!();
    let release = store
        .release_execution_lease(WorkflowKind::Generation, job_id)
        .await;
    let completed = result?;
    release?;
    if recovery_resume {
        store
            .resolve_recovery(
                WorkflowKind::Generation,
                job_id,
                RecoveryState::Resumed,
                None,
            )
            .await?;
    }
    crate::presentation::print(&completed)
}

#[derive(Debug)]
struct ExecutionOptions {
    plan_id: Option<uuid::Uuid>,
    job_id: Option<uuid::Uuid>,
    backend: Option<BackendKind>,
    batch_size: Option<u32>,
    max_retries: Option<u32>,
    max_attempt_multiplier: Option<u32>,
    api_key_env: Option<String>,
    config: Option<std::path::PathBuf>,
}

impl ExecutionOptions {
    fn from_generate(args: GenerateArgs) -> Self {
        Self {
            plan_id: Some(args.plan_id),
            job_id: None,
            backend: args.backend,
            batch_size: args.batch_size,
            max_retries: args.max_retries,
            max_attempt_multiplier: args.max_attempt_multiplier,
            api_key_env: args.api_key_env,
            config: args.config,
        }
    }

    fn from_resume(args: ResumeGenerationArgs) -> Self {
        Self {
            plan_id: None,
            job_id: Some(args.job_id),
            backend: None,
            batch_size: args.batch_size,
            max_retries: args.max_retries,
            max_attempt_multiplier: args.max_attempt_multiplier,
            api_key_env: args.api_key_env,
            config: args.config,
        }
    }
}

fn configured_backend(config: &Option<project_config::ResolvedProjectConfig>) -> BackendKind {
    config.as_ref().map_or(BackendKind::Fake, |config| {
        match config.generation.backend {
            GenerationBackendKind::Fake => BackendKind::Fake,
            GenerationBackendKind::OpenaiCompatible => BackendKind::OpenaiCompatible,
        }
    })
}

async fn build_backend(
    backend_kind: BackendKind,
    options: &ExecutionOptions,
    configured: Option<&project_config::ResolvedProjectConfig>,
    store: &SqliteStore,
) -> anyhow::Result<(Arc<dyn GenerationBackend>, GenerationParameters)> {
    match backend_kind {
        BackendKind::Fake => Ok((
            Arc::new(FakeGenerationBackend::with_namespace(
                options
                    .plan_id
                    .map_or_else(|| "unscoped".into(), |id| id.to_string()),
            )),
            configured.map_or_else(GenerationParameters::default, |config| {
                config.generation_parameters()
            }),
        )),
        BackendKind::OpenaiCompatible => {
            let configuration = match configured.and_then(|config| config.backend_configuration()) {
                Some(configuration) => configuration,
                None => store
                    .get_backend_configuration("openai-compatible")
                    .await?
                    .context("configure the OpenAI-compatible backend first or provide --config")?,
            };
            let base_url = configuration
                .base_url
                .as_deref()
                .context("backend base URL is missing")?;
            let api_key_env = options.api_key_env.as_deref().unwrap_or_else(|| {
                configured.map_or("SYNTH_OPENAI_API_KEY", |config| {
                    config.generation.api_key_env.as_str()
                })
            });
            let api_key = std::env::var(api_key_env).ok();
            Ok((
                Arc::new(OpenAICompatibleBackend::new(
                    base_url,
                    api_key,
                    configuration.model.clone(),
                )?),
                configuration.parameters,
            ))
        }
    }
}

fn runner_policy(
    options: &ExecutionOptions,
    configured: Option<&project_config::ResolvedProjectConfig>,
) -> JobRunnerPolicy {
    let generation = configured.map(|config| &config.generation);
    JobRunnerPolicy {
        batch_size: options
            .batch_size
            .unwrap_or_else(|| generation.map_or(20, |value| value.batch_size)),
        max_request_retries: options
            .max_retries
            .unwrap_or_else(|| generation.map_or(3, |value| value.max_retries)),
        max_attempt_multiplier: options
            .max_attempt_multiplier
            .unwrap_or_else(|| generation.map_or(3, |value| value.max_attempt_multiplier)),
        retry_delay: Duration::from_millis(500),
    }
}
