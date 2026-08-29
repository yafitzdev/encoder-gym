use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use dataset_core::ports::SnapshotStore;
use generation_core::ports::DatasetStore;
use recovery_core::{RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{
        EncoderTrainingMode, TrainingConfiguration, TrainingExample, TrainingRequest, TrainingRun,
        TrainingRunState, TransformerTrainingConfiguration,
    },
    ports::{
        CheckpointSink, EncoderRegistry, PredictorLoader, TrainingBackend, TrainingRunQuery,
        TrainingStore,
    },
    runner::TrainingRunner,
};
use training_linear::{
    HashingLinearBackend, HashingLinearPredictorLoader, LocalCheckpointStore, verify_checksum,
};
use training_transformer::{BertPredictorLoader, BertTrainingBackend};

use crate::cli::{
    EncoderTrainingModeArg, TrainingBackendKind, TrainingCommand, TrainingContinueArgs,
    TrainingRunArgs, TrainingRunStateArg,
};
use crate::training_examples::TrainingExampleSpoolBuilder;

use super::config;

pub async fn execute(command: TrainingCommand, store: SqliteStore) -> anyhow::Result<()> {
    match command {
        TrainingCommand::Run(args) => run(args, store).await,
        TrainingCommand::Continue(args) => continue_run(args, store).await,
        TrainingCommand::List {
            snapshot_id,
            state,
            page,
        } => {
            let runs = store
                .query_training_runs(TrainingRunQuery {
                    snapshot_id,
                    state: state.map(training_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&runs, runs.len(), page)
        }
        TrainingCommand::Status { id } => print_json(
            &store
                .get_training_run(id)
                .await?
                .with_context(|| format!("training run not found: {id}"))?,
        ),
        TrainingCommand::Cancel { id } => print_json(&serde_json::json!({
            "cancel_requested": store.request_training_cancellation(id).await?,
        })),
        TrainingCommand::Checkpoints { run_id } => {
            print_json(&store.list_checkpoints(run_id).await?)
        }
        TrainingCommand::Checkpoint { id } => print_json(
            &store
                .get_checkpoint(id)
                .await?
                .with_context(|| format!("checkpoint not found: {id}"))?,
        ),
        TrainingCommand::Predict {
            checkpoint_id,
            text,
        } => predict(checkpoint_id, &text, &store).await,
    }
}

async fn run(args: TrainingRunArgs, store: SqliteStore) -> anyhow::Result<()> {
    let configured = args
        .config
        .as_deref()
        .map(config::resolve_path)
        .transpose()?;
    let defaults = TrainingConfiguration::default();
    let configured_training = configured.as_ref().map(|config| &config.training);
    let configuration = TrainingConfiguration {
        feature_dimension: args.feature_dimension.unwrap_or_else(|| {
            configured_training.map_or(defaults.feature_dimension, |value| value.feature_dimension)
        }),
        epochs: args
            .epochs
            .unwrap_or_else(|| configured_training.map_or(defaults.epochs, |value| value.epochs)),
        learning_rate: args.learning_rate.unwrap_or_else(|| {
            configured_training.map_or(defaults.learning_rate, |value| value.learning_rate)
        }),
        l2: args
            .l2
            .unwrap_or_else(|| configured_training.map_or(defaults.l2, |value| value.l2)),
        checkpoint_every: args.checkpoint_every.unwrap_or_else(|| {
            configured_training.map_or(defaults.checkpoint_every, |value| value.checkpoint_every)
        }),
        seed: args
            .seed
            .unwrap_or_else(|| configured_training.map_or(defaults.seed, |value| value.seed)),
    };
    configuration.validate()?;
    let backend_kind = args.backend.unwrap_or_else(|| {
        configured_training.map_or(
            TrainingBackendKind::HashingLinear,
            |training| match training.backend {
                project_config::TrainingBackendKind::HashingLinear => {
                    TrainingBackendKind::HashingLinear
                }
                project_config::TrainingBackendKind::BertCpu => TrainingBackendKind::BertCpu,
            },
        )
    });
    let artifact_root = args.artifact_root.unwrap_or_else(|| {
        configured_training.map_or_else(
            || PathBuf::from("artifacts/training"),
            |value| value.artifact_root.clone(),
        )
    });
    let (backend, base_model_id, transformer_configuration, backend_configuration_fingerprint) =
        match backend_kind {
            TrainingBackendKind::HashingLinear => (
                Arc::new(HashingLinearBackend) as Arc<dyn TrainingBackend>,
                None,
                None,
                None,
            ),
            TrainingBackendKind::BertCpu => {
                let encoder_id = args
                    .encoder_id
                    .or_else(|| configured_training.and_then(|training| training.base_model_id))
                    .context("--encoder-id or training.base_model_id is required for bert-cpu")?;
                let encoder = store
                    .get_encoder(encoder_id)
                    .await?
                    .with_context(|| format!("registered encoder not found: {encoder_id}"))?;
                let transformer_defaults = configured_training
                    .map_or_else(TransformerTrainingConfiguration::default, |training| {
                        training.transformer.clone()
                    });
                let transformer = TransformerTrainingConfiguration {
                    maximum_sequence_length: args
                        .maximum_sequence_length
                        .unwrap_or(transformer_defaults.maximum_sequence_length),
                    batch_size: args.batch_size.unwrap_or(transformer_defaults.batch_size),
                    weight_decay: args
                        .weight_decay
                        .unwrap_or(transformer_defaults.weight_decay),
                    warmup_ratio: args
                        .warmup_ratio
                        .unwrap_or(transformer_defaults.warmup_ratio),
                    gradient_clip_norm: args
                        .gradient_clip_norm
                        .unwrap_or(transformer_defaults.gradient_clip_norm),
                    mode: args
                        .encoder_mode
                        .map(encoder_mode)
                        .unwrap_or(transformer_defaults.mode),
                };
                transformer.validate()?;
                let fingerprint = artifact_core::fingerprint(&transformer)?;
                (
                    Arc::new(BertTrainingBackend::new(encoder, transformer.clone())?)
                        as Arc<dyn TrainingBackend>,
                    Some(encoder_id),
                    Some(transformer),
                    Some(fingerprint),
                )
            }
        };
    let training_run = TrainingRun::queued_with_context(
        args.snapshot_id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
        base_model_id,
        None,
        transformer_configuration,
        backend_configuration_fingerprint,
    )?;
    let request =
        training_request(&store, training_run.id, args.snapshot_id, configuration).await?;
    let completed = execute_training(training_run, request, backend, artifact_root, store).await?;
    print_json(&completed)
}

async fn continue_run(args: TrainingContinueArgs, store: SqliteStore) -> anyhow::Result<()> {
    let checkpoint = store
        .get_checkpoint(args.checkpoint_id)
        .await?
        .with_context(|| format!("checkpoint not found: {}", args.checkpoint_id))?;
    anyhow::ensure!(
        checkpoint.model_format == "bert-classifier-v1",
        "only bert-classifier-v1 checkpoints support explicit continuation"
    );
    let parent_run = store
        .get_training_run(checkpoint.run_id)
        .await?
        .with_context(|| format!("parent training run not found: {}", checkpoint.run_id))?;
    let files = LocalCheckpointStore::new(".");
    let artifact = files.read(&checkpoint.artifact_path)?;
    verify_artifact(&checkpoint, &artifact)?;
    let metadata = BertPredictorLoader::inspect(&artifact)?;
    let encoder = store
        .get_encoder(metadata.base_model_id)
        .await?
        .with_context(|| format!("registered encoder not found: {}", metadata.base_model_id))?;
    anyhow::ensure!(
        encoder.fingerprint == metadata.base_model_fingerprint,
        "checkpoint base-model fingerprint does not match its registered encoder"
    );
    let configured = args
        .config
        .as_deref()
        .map(config::resolve_path)
        .transpose()?;
    let mut configuration = metadata.training_configuration;
    if let Some(epochs) = args.epochs {
        configuration.epochs = epochs;
    }
    if let Some(learning_rate) = args.learning_rate {
        configuration.learning_rate = learning_rate;
    }
    if let Some(checkpoint_every) = args.checkpoint_every {
        configuration.checkpoint_every = checkpoint_every;
    }
    configuration.validate()?;
    let transformer_fingerprint = artifact_core::fingerprint(&metadata.transformer_configuration)?;
    let backend = Arc::new(BertTrainingBackend::continuing_from(
        encoder,
        metadata.transformer_configuration.clone(),
        artifact,
    )?) as Arc<dyn TrainingBackend>;
    let snapshot_id = args.snapshot_id.unwrap_or(parent_run.snapshot_id);
    let run = TrainingRun::queued_with_context(
        snapshot_id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
        Some(metadata.base_model_id),
        Some(checkpoint.id),
        Some(metadata.transformer_configuration.clone()),
        Some(transformer_fingerprint),
    )?;
    let request = training_request(&store, run.id, snapshot_id, configuration).await?;
    anyhow::ensure!(
        request.labels == metadata.labels,
        "continuation snapshot label mapping differs from the parent checkpoint"
    );
    let artifact_root = args.artifact_root.unwrap_or_else(|| {
        configured.map_or_else(
            || PathBuf::from("artifacts/training"),
            |config| config.training.artifact_root,
        )
    });
    let completed = execute_training(run, request, backend, artifact_root, store).await?;
    print_json(&completed)
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct CompletedTraining {
    pub run: TrainingRun,
    pub checkpoints: Vec<training_core::domain::TrainingCheckpoint>,
}

pub(crate) async fn run_workflow(
    snapshot_id: uuid::Uuid,
    configured: &project_config::ResolvedProjectConfig,
    store: SqliteStore,
) -> anyhow::Result<CompletedTraining> {
    let configuration = configured.training_configuration()?;
    let (backend, base_model_id, transformer_configuration, backend_fingerprint) =
        match configured.training.backend {
            project_config::TrainingBackendKind::HashingLinear => (
                Arc::new(HashingLinearBackend) as Arc<dyn TrainingBackend>,
                None,
                None,
                None,
            ),
            project_config::TrainingBackendKind::BertCpu => {
                let encoder_id = configured
                    .training
                    .base_model_id
                    .context("persisted workflow configuration has no base model ID")?;
                let encoder = store
                    .get_encoder(encoder_id)
                    .await?
                    .with_context(|| format!("registered encoder not found: {encoder_id}"))?;
                configured.training.transformer.validate()?;
                (
                    Arc::new(BertTrainingBackend::new(
                        encoder,
                        configured.training.transformer.clone(),
                    )?) as Arc<dyn TrainingBackend>,
                    Some(encoder_id),
                    Some(configured.training.transformer.clone()),
                    Some(configured.transformer_configuration_fingerprint()?),
                )
            }
        };
    let run = TrainingRun::queued_with_context(
        snapshot_id,
        backend.name(),
        backend.model_format(),
        configuration.clone(),
        base_model_id,
        None,
        transformer_configuration,
        backend_fingerprint,
    )?;
    let request = training_request(&store, run.id, snapshot_id, configuration).await?;
    execute_training(
        run,
        request,
        backend,
        configured.training.artifact_root.clone(),
        store,
    )
    .await
}

async fn training_request(
    store: &SqliteStore,
    run_id: uuid::Uuid,
    snapshot_id: uuid::Uuid,
    configuration: TrainingConfiguration,
) -> anyhow::Result<TrainingRequest> {
    let snapshot = store
        .get_snapshot(snapshot_id)
        .await?
        .with_context(|| format!("snapshot not found: {snapshot_id}"))?;
    let dataset = store
        .get_dataset(snapshot.source_dataset_id)
        .await?
        .with_context(|| format!("source dataset not found: {}", snapshot.source_dataset_id))?;
    const PAGE_SIZE: u32 = 512;
    let mut builder = TrainingExampleSpoolBuilder::new().map_err(anyhow::Error::msg)?;
    let mut offset = 0_u32;
    loop {
        let members = store
            .query_snapshot_members(snapshot.id, PAGE_SIZE, offset)
            .await?;
        let page_length = u32::try_from(members.len()).context("snapshot page is too large")?;
        for member in members {
            builder
                .push(
                    member.split,
                    &TrainingExample {
                        snapshot_member_id: member.id,
                        text: member.text,
                        label: member.label,
                    },
                )
                .map_err(anyhow::Error::msg)?;
        }
        if page_length < PAGE_SIZE {
            break;
        }
        offset = offset
            .checked_add(page_length)
            .context("snapshot has too many members to page with u32 offsets")?;
    }
    let request = TrainingRequest {
        run_id,
        snapshot_id,
        labels: dataset.labels,
        examples: Arc::new(builder.finish().map_err(anyhow::Error::msg)?),
        configuration,
    };
    request.validate()?;
    Ok(request)
}

async fn execute_training(
    training_run: TrainingRun,
    request: TrainingRequest,
    backend: Arc<dyn TrainingBackend>,
    artifact_root: PathBuf,
    store: SqliteStore,
) -> anyhow::Result<CompletedTraining> {
    store.create_training_run(&training_run).await?;
    store
        .acquire_execution_lease(WorkflowKind::Training, training_run.id)
        .await?;
    eprintln!(
        "training run {} queued for {} epochs",
        training_run.id, training_run.configuration.epochs
    );
    let runner = TrainingRunner::new(
        Arc::new(store.clone()),
        backend,
        Arc::new(LocalCheckpointStore::new(artifact_root)),
    );
    let run_id = training_run.id;
    let mut handle = tokio::spawn(async move { runner.run(run_id, request).await });
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut cancellation_sent = false;
    let result: anyhow::Result<_> = loop {
        tokio::select! {
            result = &mut handle => break result.context("training worker panicked")?.map_err(Into::into),
            _ = interval.tick() => {
                if let Some(current) = store.get_training_run(run_id).await? {
                    eprint!(
                        "\r{:?}: epoch {}/{} batch {}/{} examples {} loss {:?} lr {:?}   ",
                        current.state,
                        current.current_epoch,
                        current.configuration.epochs,
                        current.completed_batches,
                        current.batches_in_epoch,
                        current.processed_examples,
                        current.latest_training_loss,
                        current.latest_learning_rate,
                    );
                }
            }
            result = tokio::signal::ctrl_c(), if !cancellation_sent => {
                result.context("could not listen for Ctrl+C")?;
                cancellation_sent = store.request_training_cancellation(run_id).await?;
                eprintln!("\ncancellation requested; finishing the active batch");
            }
        }
    };
    eprintln!();
    let release = store
        .release_execution_lease(WorkflowKind::Training, run_id)
        .await;
    let completed = result?;
    release?;
    Ok(CompletedTraining {
        run: completed,
        checkpoints: store.list_checkpoints(run_id).await?,
    })
}

async fn predict(checkpoint_id: uuid::Uuid, text: &str, store: &SqliteStore) -> anyhow::Result<()> {
    let checkpoint = store
        .get_checkpoint(checkpoint_id)
        .await?
        .with_context(|| format!("checkpoint not found: {checkpoint_id}"))?;
    let files = LocalCheckpointStore::new(".");
    let artifact = files.read(&checkpoint.artifact_path)?;
    verify_artifact(&checkpoint, &artifact)?;
    let predictor = match checkpoint.model_format.as_str() {
        "hashing-linear-v1" => HashingLinearPredictorLoader.load(&artifact)?,
        "bert-classifier-v1" => BertPredictorLoader.load(&artifact)?,
        other => anyhow::bail!("unsupported checkpoint model format: {other}"),
    };
    print_json(&predictor.predict(text)?)
}

fn verify_artifact(
    checkpoint: &training_core::domain::TrainingCheckpoint,
    artifact: &[u8],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        checkpoint.artifact_size_bytes == artifact.len() as u64,
        "checkpoint size mismatch: expected {}, read {}",
        checkpoint.artifact_size_bytes,
        artifact.len()
    );
    verify_checksum(artifact, &checkpoint.artifact_checksum)?;
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}

const fn training_state(state: TrainingRunStateArg) -> TrainingRunState {
    match state {
        TrainingRunStateArg::Queued => TrainingRunState::Queued,
        TrainingRunStateArg::Running => TrainingRunState::Running,
        TrainingRunStateArg::Completed => TrainingRunState::Completed,
        TrainingRunStateArg::Failed => TrainingRunState::Failed,
        TrainingRunStateArg::Cancelled => TrainingRunState::Cancelled,
    }
}

const fn encoder_mode(mode: EncoderTrainingModeArg) -> EncoderTrainingMode {
    match mode {
        EncoderTrainingModeArg::FineTune => EncoderTrainingMode::FineTune,
        EncoderTrainingModeArg::Frozen => EncoderTrainingMode::Frozen,
    }
}
