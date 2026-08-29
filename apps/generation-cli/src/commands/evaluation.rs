use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::Arc,
};

use anyhow::Context;
use dataset_core::{domain::SnapshotSplit, ports::SnapshotStore};
use evaluation_core::{
    comparison::{leaderboard, paired_comparison, select_model},
    domain::{
        EvaluationProtocol, EvaluationRun, EvaluationRunState, EvaluationSourceIdentity,
        LeaderboardMetric,
    },
    ports::{EvaluationExampleSource, EvaluationRunQuery, EvaluationStore, PredictionQuery},
    run_input_fingerprint,
    runner::EvaluationRunner,
};
use generation_core::ports::DatasetStore;
use recovery_core::{RecoveryStore, WorkflowKind};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::{CheckpointSink, Predictor, PredictorLoader, TrainingStore};
use training_linear::{HashingLinearPredictorLoader, LocalCheckpointStore, verify_checksum};
use training_transformer::BertPredictorLoader;

use crate::cli::{
    ComparisonChangeArg, EvaluationCommand, EvaluationMetricArg, EvaluationRunStateArg,
    ExportFormat, SnapshotSplitArg,
};

use super::config;

pub async fn execute(command: EvaluationCommand, store: SqliteStore) -> anyhow::Result<()> {
    match command {
        EvaluationCommand::Run {
            checkpoint_id,
            snapshot_id,
            split,
            config: config_path,
            batch_size,
            top_k,
            calibration_bins,
            minimum_slice_support,
            bootstrap_samples,
            statistical_seed,
            confidence_level,
            dimension_intersections,
        } => {
            let configured = config_path
                .as_deref()
                .map(config::resolve_path)
                .transpose()?;
            let split = split
                .map(snapshot_split)
                .or_else(|| configured.as_ref().map(|config| config.evaluation.split));
            let evaluation = configured.as_ref().map(|config| &config.evaluation);
            run(
                checkpoint_id,
                snapshot_id,
                split.unwrap_or(SnapshotSplit::Test),
                ProtocolOverrides {
                    batch_size: batch_size.or_else(|| evaluation.map(|value| value.batch_size)),
                    top_k: top_k.or_else(|| evaluation.map(|value| value.top_k.clone())),
                    calibration_bins: calibration_bins
                        .or_else(|| evaluation.map(|value| value.calibration_bins)),
                    minimum_slice_support: minimum_slice_support
                        .or_else(|| evaluation.map(|value| value.minimum_slice_support)),
                    bootstrap_samples: bootstrap_samples
                        .or_else(|| evaluation.map(|value| value.bootstrap_samples)),
                    statistical_seed: statistical_seed
                        .or_else(|| evaluation.map(|value| value.statistical_seed)),
                    confidence_level: confidence_level
                        .or_else(|| evaluation.map(|value| value.confidence_level)),
                    dimension_intersections: if dimension_intersections.is_empty() {
                        evaluation
                            .map(|value| value.dimension_intersections.clone())
                            .unwrap_or_default()
                    } else {
                        parse_intersections(dimension_intersections)?
                    },
                },
                store,
            )
            .await
        }
        EvaluationCommand::List {
            checkpoint_id,
            snapshot_id,
            state,
            page,
        } => {
            let runs = store
                .query_evaluation_runs(EvaluationRunQuery {
                    checkpoint_id,
                    snapshot_id,
                    state: state.map(evaluation_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&runs, runs.len(), page)
        }
        EvaluationCommand::Status { id } => print_json(
            &store
                .get_evaluation_run(id)
                .await?
                .with_context(|| format!("evaluation run not found: {id}"))?,
        ),
        EvaluationCommand::Cancel { id } => print_json(
            &serde_json::json!({"cancel_requested": store.request_evaluation_cancellation(id).await?}),
        ),
        EvaluationCommand::Metrics { id } => {
            let run = store
                .get_evaluation_run(id)
                .await?
                .with_context(|| format!("evaluation run not found: {id}"))?;
            anyhow::ensure!(
                run.state == EvaluationRunState::Completed,
                "evaluation run is incomplete"
            );
            print_json(&run.metrics)
        }
        EvaluationCommand::Predictions {
            run_id,
            incorrect,
            correct,
            expected_label,
            predicted_label,
            minimum_confidence,
            maximum_confidence,
            dimensions,
            snapshot_member_id,
            source_row_id,
            page,
        } => {
            validate_confidence(minimum_confidence, maximum_confidence)?;
            let query = PredictionQuery {
                run_id,
                correct: if incorrect {
                    Some(false)
                } else if correct {
                    Some(true)
                } else {
                    None
                },
                expected_label,
                predicted_label,
                minimum_confidence,
                maximum_confidence,
                dimensions: parse_dimensions(dimensions)?,
                snapshot_member_id,
                source_row_id,
                limit: page.limit,
                offset: page.offset,
            };
            print_json(&store.query_predictions(query).await?)
        }
        EvaluationCommand::Export {
            run_id,
            format,
            output,
        } => export_predictions(&store, run_id, format, &output).await,
        EvaluationCommand::Compare {
            left_run_id,
            right_run_id,
        } => {
            let left = store
                .get_evaluation_run(left_run_id)
                .await?
                .with_context(|| format!("evaluation run not found: {left_run_id}"))?;
            let right = store
                .get_evaluation_run(right_run_id)
                .await?
                .with_context(|| format!("evaluation run not found: {right_run_id}"))?;
            let left_predictions = store.list_predictions(left.id).await?;
            let right_predictions = store.list_predictions(right.id).await?;
            let report = paired_comparison(&left, &right, &left_predictions, &right_predictions)?;
            store.create_comparison(&report).await?;
            print_json(&report)
        }
        EvaluationCommand::Comparison { id } => print_json(
            &store
                .get_comparison(id)
                .await?
                .with_context(|| format!("comparison not found: {id}"))?,
        ),
        EvaluationCommand::ComparisonExamples { id, change, page } => {
            let report = store
                .get_comparison(id)
                .await?
                .with_context(|| format!("comparison not found: {id}"))?;
            let ids = match change {
                ComparisonChangeArg::Fixed => &report.fixed_snapshot_member_ids,
                ComparisonChangeArg::Regressed => &report.regressed_snapshot_member_ids,
            };
            let start = (page.offset as usize).min(ids.len());
            let end = start.saturating_add(page.limit as usize).min(ids.len());
            let run_id = match change {
                ComparisonChangeArg::Fixed => report.right_run_id,
                ComparisonChangeArg::Regressed => report.left_run_id,
            };
            let mut predictions = Vec::with_capacity(end - start);
            for member_id in &ids[start..end] {
                let mut rows = store
                    .query_predictions(PredictionQuery {
                        snapshot_member_id: Some(*member_id),
                        ..PredictionQuery::page(run_id, 1, 0)
                    })
                    .await?;
                if let Some(row) = rows.pop() {
                    predictions.push(row);
                }
            }
            print_json(&predictions)
        }
        EvaluationCommand::ComparisonExport { id, output } => {
            let report = store
                .get_comparison(id)
                .await?
                .with_context(|| format!("comparison not found: {id}"))?;
            std::fs::write(&output, serde_json::to_vec_pretty(&report)?)
                .with_context(|| format!("could not write {}", output.display()))?;
            print_json(&serde_json::json!({"comparison_id": id, "file": output}))
        }
        EvaluationCommand::Comparisons { page } => {
            print_json(&store.list_comparisons(page.limit, page.offset).await?)
        }
        EvaluationCommand::Leaderboard {
            snapshot_id,
            split,
            metric,
        } => {
            let runs = compatible_runs(&store, snapshot_id, snapshot_split(split)).await?;
            let ranking = leaderboard(&runs, evaluation_metric(metric))?;
            let mut enriched = Vec::with_capacity(ranking.len());
            for entry in ranking {
                let run = runs
                    .iter()
                    .find(|run| run.id == entry.evaluation_run_id)
                    .expect("ranked run");
                let checkpoint = store
                    .get_checkpoint(entry.checkpoint_id)
                    .await?
                    .with_context(|| format!("checkpoint not found: {}", entry.checkpoint_id))?;
                enriched.push(serde_json::json!({
                    "rank": entry.rank, "evaluation_run_id": entry.evaluation_run_id,
                    "checkpoint_id": entry.checkpoint_id, "training_run_id": checkpoint.run_id,
                    "model_format": run.source_identity.checkpoint_model_format,
                    "base_model_fingerprint": run.source_identity.base_model_fingerprint,
                    "tokenizer_fingerprint": run.source_identity.tokenizer_fingerprint,
                    "metric_value": entry.metric_value, "snapshot_id": run.snapshot_id,
                    "split": run.split, "protocol_fingerprint": run.protocol_fingerprint,
                    "created_at": entry.created_at,
                }));
            }
            print_json(&enriched)
        }
        EvaluationCommand::Select {
            snapshot_id,
            split,
            metric,
            minimum_improvement,
            required_confidence,
        } => {
            let runs = compatible_runs(&store, snapshot_id, snapshot_split(split)).await?;
            let report = select_model(
                &runs,
                evaluation_metric(metric),
                minimum_improvement,
                required_confidence,
            )?;
            store.create_selection(&report).await?;
            print_json(&report)
        }
        EvaluationCommand::Selection { id } => print_json(
            &store
                .get_selection(id)
                .await?
                .with_context(|| format!("selection not found: {id}"))?,
        ),
        EvaluationCommand::Selections { page } => {
            print_json(&store.list_selections(page.limit, page.offset).await?)
        }
    }
}

async fn run(
    checkpoint_id: uuid::Uuid,
    snapshot_id: Option<uuid::Uuid>,
    split: SnapshotSplit,
    overrides: ProtocolOverrides,
    store: SqliteStore,
) -> anyhow::Result<()> {
    let checkpoint = store
        .get_checkpoint(checkpoint_id)
        .await?
        .with_context(|| format!("checkpoint not found: {checkpoint_id}"))?;
    let training_run = store
        .get_training_run(checkpoint.run_id)
        .await?
        .with_context(|| format!("training run not found: {}", checkpoint.run_id))?;
    let snapshot_id = snapshot_id.unwrap_or(training_run.snapshot_id);
    let snapshot = store
        .get_snapshot(snapshot_id)
        .await?
        .with_context(|| format!("snapshot not found: {snapshot_id}"))?;
    let dataset = store
        .get_dataset(snapshot.source_dataset_id)
        .await?
        .with_context(|| format!("source dataset not found: {}", snapshot.source_dataset_id))?;
    let total_examples = store.count_examples(snapshot.id, split).await?;
    anyhow::ensure!(
        total_examples > 0,
        "snapshot {snapshot_id} has no examples in the {split:?} split"
    );

    let files = LocalCheckpointStore::new(".");
    let artifact = files.read(&checkpoint.artifact_path)?;
    anyhow::ensure!(
        checkpoint.artifact_size_bytes == artifact.len() as u64,
        "checkpoint size mismatch: expected {}, read {}",
        checkpoint.artifact_size_bytes,
        artifact.len()
    );
    verify_checksum(&artifact, &checkpoint.artifact_checksum)?;
    let (predictor, base_model_fingerprint, tokenizer_fingerprint): (
        Box<dyn Predictor>,
        Option<String>,
        Option<String>,
    ) = match checkpoint.model_format.as_str() {
        "hashing-linear-v1" => (HashingLinearPredictorLoader.load(&artifact)?, None, None),
        "bert-classifier-v1" => {
            let metadata = BertPredictorLoader::inspect(&artifact)?;
            (
                BertPredictorLoader.load(&artifact)?,
                Some(metadata.base_model_fingerprint),
                Some(metadata.tokenizer_fingerprint),
            )
        }
        other => anyhow::bail!("unsupported checkpoint model format: {other}"),
    };
    let expected_labels = dataset.labels.iter().collect::<BTreeSet<_>>();
    let model_labels = predictor.labels().iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        expected_labels == model_labels,
        "checkpoint and evaluation dataset use different label sets"
    );

    anyhow::ensure!(
        predictor.labels() == dataset.labels.as_slice(),
        "checkpoint label order differs from the evaluation dataset label order"
    );
    let defaults = EvaluationProtocol::default();
    let protocol = EvaluationProtocol {
        split,
        batch_size: overrides.batch_size.unwrap_or(defaults.batch_size),
        top_k: overrides.top_k.unwrap_or(defaults.top_k),
        calibration_bins: overrides
            .calibration_bins
            .unwrap_or(defaults.calibration_bins),
        minimum_slice_support: overrides
            .minimum_slice_support
            .unwrap_or(defaults.minimum_slice_support),
        bootstrap_samples: overrides
            .bootstrap_samples
            .unwrap_or(defaults.bootstrap_samples),
        statistical_seed: overrides
            .statistical_seed
            .unwrap_or(defaults.statistical_seed),
        confidence_level: overrides
            .confidence_level
            .unwrap_or(defaults.confidence_level),
        dimension_intersections: overrides.dimension_intersections,
    };
    let snapshot_fingerprint = snapshot.fingerprint.clone();
    let cohort_fingerprint = artifact_core::fingerprint(&(snapshot_fingerprint.as_str(), split))?;
    let source_identity = EvaluationSourceIdentity {
        checkpoint_checksum: checkpoint.artifact_checksum.clone(),
        checkpoint_model_format: checkpoint.model_format.clone(),
        base_model_fingerprint,
        tokenizer_fingerprint,
        snapshot_fingerprint,
        cohort_fingerprint,
        labels: dataset.labels.clone(),
    };
    let input_fingerprint = run_input_fingerprint(&protocol, &source_identity)?;
    let evaluation_run = EvaluationRun::queued_with_protocol(
        checkpoint.id,
        snapshot.id,
        protocol,
        source_identity,
        total_examples,
        input_fingerprint,
    )?;
    store.create_evaluation_run(&evaluation_run).await?;
    store
        .acquire_execution_lease(WorkflowKind::Evaluation, evaluation_run.id)
        .await?;
    let runner = EvaluationRunner::new(
        Arc::new(store.clone()),
        Arc::new(store.clone()),
        Arc::from(predictor),
    );
    let result = runner.run(evaluation_run.id, dataset.labels).await;
    let release = store
        .release_execution_lease(WorkflowKind::Evaluation, evaluation_run.id)
        .await;
    let completed = result?;
    release?;
    let prediction_preview = store
        .query_predictions(PredictionQuery::page(evaluation_run.id, 100, 0))
        .await?;
    print_json(&serde_json::json!({
        "run": completed,
        "predictions": prediction_preview,
        "predictions_truncated": evaluation_run.total_examples > 100,
    }))
}

const fn snapshot_split(split: SnapshotSplitArg) -> SnapshotSplit {
    match split {
        SnapshotSplitArg::Train => SnapshotSplit::Train,
        SnapshotSplitArg::Validation => SnapshotSplit::Validation,
        SnapshotSplitArg::Test => SnapshotSplit::Test,
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}

const fn evaluation_state(state: EvaluationRunStateArg) -> EvaluationRunState {
    match state {
        EvaluationRunStateArg::Queued => EvaluationRunState::Queued,
        EvaluationRunStateArg::Running => EvaluationRunState::Running,
        EvaluationRunStateArg::Completed => EvaluationRunState::Completed,
        EvaluationRunStateArg::Failed => EvaluationRunState::Failed,
        EvaluationRunStateArg::Cancelled => EvaluationRunState::Cancelled,
    }
}

struct ProtocolOverrides {
    batch_size: Option<usize>,
    top_k: Option<Vec<usize>>,
    calibration_bins: Option<usize>,
    minimum_slice_support: Option<u64>,
    bootstrap_samples: Option<u32>,
    statistical_seed: Option<u64>,
    confidence_level: Option<f64>,
    dimension_intersections: Vec<Vec<String>>,
}

fn parse_intersections(values: Vec<String>) -> anyhow::Result<Vec<Vec<String>>> {
    let mut intersections = Vec::new();
    for value in values {
        let mut names = value
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !names.is_empty() && names.iter().all(|name| !name.is_empty()),
            "dimension intersections require comma-separated non-empty names"
        );
        names.sort();
        names.dedup();
        intersections.push(names);
    }
    intersections.sort();
    intersections.dedup();
    Ok(intersections)
}

fn parse_dimensions(values: Vec<String>) -> anyhow::Result<BTreeMap<String, String>> {
    values
        .into_iter()
        .map(|value| {
            let (name, value) = value
                .split_once('=')
                .context("dimensions must use NAME=VALUE")?;
            anyhow::ensure!(
                !name.is_empty() && !value.is_empty(),
                "dimensions must use non-empty NAME=VALUE"
            );
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn validate_confidence(minimum: Option<f64>, maximum: Option<f64>) -> anyhow::Result<()> {
    for value in [minimum, maximum].into_iter().flatten() {
        anyhow::ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "confidence filters must be finite and in [0, 1]"
        );
    }
    if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
        anyhow::ensure!(
            minimum <= maximum,
            "minimum confidence exceeds maximum confidence"
        );
    }
    Ok(())
}

async fn compatible_runs(
    store: &SqliteStore,
    snapshot_id: uuid::Uuid,
    split: SnapshotSplit,
) -> anyhow::Result<Vec<EvaluationRun>> {
    let runs = store
        .query_evaluation_runs(EvaluationRunQuery {
            checkpoint_id: None,
            snapshot_id: Some(snapshot_id),
            state: Some(EvaluationRunState::Completed),
            limit: u32::MAX,
            offset: 0,
        })
        .await?;
    let mut runs = runs
        .into_iter()
        .filter(|run| run.split == split)
        .collect::<Vec<_>>();
    let first = runs
        .first()
        .context("no completed evaluations match the requested snapshot and split")?;
    let cohort = first.source_identity.cohort_fingerprint.clone();
    let protocol = first.protocol_fingerprint.clone();
    runs.retain(|run| {
        run.source_identity.cohort_fingerprint == cohort && run.protocol_fingerprint == protocol
    });
    Ok(runs)
}

const fn evaluation_metric(metric: EvaluationMetricArg) -> LeaderboardMetric {
    match metric {
        EvaluationMetricArg::Accuracy => LeaderboardMetric::Accuracy,
        EvaluationMetricArg::MacroF1 => LeaderboardMetric::MacroF1,
        EvaluationMetricArg::WeightedF1 => LeaderboardMetric::WeightedF1,
        EvaluationMetricArg::LogLoss => LeaderboardMetric::LogLoss,
        EvaluationMetricArg::BrierScore => LeaderboardMetric::BrierScore,
        EvaluationMetricArg::ExpectedCalibrationError => {
            LeaderboardMetric::ExpectedCalibrationError
        }
    }
}

async fn export_predictions(
    store: &SqliteStore,
    run_id: uuid::Uuid,
    format: ExportFormat,
    path: &Path,
) -> anyhow::Result<()> {
    let run = store
        .get_evaluation_run(run_id)
        .await?
        .with_context(|| format!("evaluation run not found: {run_id}"))?;
    anyhow::ensure!(
        run.state == EvaluationRunState::Completed,
        "only completed evaluations can be exported"
    );
    let file =
        File::create(path).with_context(|| format!("could not create {}", path.display()))?;
    match format {
        ExportFormat::Jsonl => {
            let mut writer = BufWriter::new(file);
            let mut offset = 0_u32;
            loop {
                let page = store
                    .query_predictions(PredictionQuery::page(run_id, 1_000, offset))
                    .await?;
                for prediction in &page {
                    serde_json::to_writer(&mut writer, &export_record(&run, prediction))?;
                    writer.write_all(b"\n")?;
                }
                if page.len() < 1_000 {
                    break;
                }
                offset = offset
                    .checked_add(1_000)
                    .context("prediction export offset overflow")?;
            }
            writer.flush()?;
        }
        ExportFormat::Csv => {
            let mut writer = csv::Writer::from_writer(BufWriter::new(file));
            writer.write_record([
                "evaluation_run_id",
                "checkpoint_id",
                "snapshot_member_id",
                "source_row_id",
                "text",
                "expected_label",
                "predicted_label",
                "confidence",
                "probabilities_json",
                "dimensions_json",
                "correct",
            ])?;
            let mut offset = 0_u32;
            loop {
                let page = store
                    .query_predictions(PredictionQuery::page(run_id, 1_000, offset))
                    .await?;
                for prediction in &page {
                    writer.write_record([
                        run_id.to_string(),
                        run.checkpoint_id.to_string(),
                        prediction.snapshot_member_id.to_string(),
                        prediction.source_row_id.to_string(),
                        prediction.text.clone(),
                        prediction.expected_label.clone(),
                        prediction.predicted_label.clone(),
                        prediction.confidence.to_string(),
                        serde_json::to_string(&prediction.probabilities)?,
                        serde_json::to_string(&prediction.dimensions)?,
                        (prediction.expected_label == prediction.predicted_label).to_string(),
                    ])?;
                }
                if page.len() < 1_000 {
                    break;
                }
                offset = offset
                    .checked_add(1_000)
                    .context("prediction export offset overflow")?;
            }
            writer.flush()?;
        }
    }
    print_json(&serde_json::json!({"run_id": run_id, "file": path, "count": run.example_count}))
}

fn export_record(
    run: &EvaluationRun,
    prediction: &evaluation_core::domain::EvaluationPrediction,
) -> serde_json::Value {
    serde_json::json!({"evaluation_run_id": run.id, "checkpoint_id": run.checkpoint_id,
        "snapshot_member_id": prediction.snapshot_member_id, "source_row_id": prediction.source_row_id,
        "text": prediction.text, "expected_label": prediction.expected_label,
        "predicted_label": prediction.predicted_label, "confidence": prediction.confidence,
        "probabilities": prediction.probabilities, "dimensions": prediction.dimensions,
        "correct": prediction.expected_label == prediction.predicted_label})
}
