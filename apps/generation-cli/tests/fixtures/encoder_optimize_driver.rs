//! Test-only compiled composition. The production CLI has no fake adapter override.
#![allow(dead_code)]
#[path = "../../src/cli.rs"]
mod cli;
#[path = "../../src/commands/mod.rs"]
mod commands;
#[path = "../../src/database_access.rs"]
mod database_access;
#[path = "../../src/document.rs"]
mod document;
#[path = "../../src/presentation.rs"]
mod presentation;
#[path = "../../src/training_examples.rs"]
mod training_examples;

use clap::Parser;
use encoder_experiment_core::{
    domain::{
        BackendIdentity, EvidenceRole, ExternalProjectSnapshot, ModelArtifactIdentity,
        TrainingCandidate,
    },
    metrics::{EvaluationReport, MetricContract},
    ports::{
        AdapterInspection, BoxFuture, EncoderTaskAdapterError, EncoderTaskBackend, TrainOutput,
    },
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
use workflow_core::benchmark_generation::BenchmarkGeneration;

fn main() -> anyhow::Result<()> {
    match std::thread::Builder::new()
        .stack_size(16 * 1_048_576)
        .spawn(run)?
        .join()
    {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    presentation::set_output(args.output)?;
    let database_url = args.database_url();
    let cli::Command::Encoder {
        command: cli::EncoderCommand::Optimize { command },
    } = args.command
    else {
        anyhow::bail!("the test composition supports only encoder optimize");
    };
    let workspace = commands::encoder_optimize::backend_args(&command)
        .workspace
        .clone();
    commands::experiment::ensure_database_belongs_to_workspace(&database_url, &workspace)?;
    let store = command.database_access().production(&database_url).await?;
    commands::encoder_optimize::execute_lifecycle(*command, &store, || {
        FakeBackend::open(&workspace)
    })
    .await
}

#[derive(Deserialize)]
struct FixtureConfiguration {
    project: ExternalProjectSnapshot,
    generation: BenchmarkGeneration,
    outcome: String,
}

struct FakeBackend {
    configuration: FixtureConfiguration,
    calls: PathBuf,
}

impl FakeBackend {
    fn open(workspace: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            configuration: serde_json::from_slice(&std::fs::read(
                workspace.join("fake-backend.json"),
            )?)?,
            calls: workspace.join("backend-calls.jsonl"),
        })
    }

    fn record(&self, value: serde_json::Value) -> Result<(), EncoderTaskAdapterError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.calls)
            .map_err(adapter_error)?;
        writeln!(file, "{value}").map_err(adapter_error)
    }
}

fn adapter_error(error: impl std::fmt::Display) -> EncoderTaskAdapterError {
    EncoderTaskAdapterError(error.to_string())
}

impl EncoderTaskBackend for FakeBackend {
    fn identity(&self) -> BackendIdentity {
        self.configuration.project.backend.clone()
    }

    fn inspect(
        &self,
        project: ExternalProjectSnapshot,
    ) -> BoxFuture<'_, Result<AdapterInspection, EncoderTaskAdapterError>> {
        Box::pin(async move {
            project.validate_integrity().map_err(adapter_error)?;
            if project != self.configuration.project {
                return Err(adapter_error("fixture project identity changed"));
            }
            Ok(AdapterInspection {
                source_fingerprint: project.source_fingerprint,
                verified_artifact_keys: project
                    .inputs
                    .iter()
                    .map(|input| input.key.clone())
                    .collect(),
                metadata: json!({"fake": true}),
            })
        })
    }

    fn train(
        &self,
        project: ExternalProjectSnapshot,
        candidate: TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainOutput, EncoderTaskAdapterError>> {
        Box::pin(async move {
            candidate
                .validate_integrity(&project)
                .map_err(adapter_error)?;
            self.record(json!({"operation": "train", "candidate_id": candidate.id}))?;
            if self.configuration.outcome == "training_failure" {
                return Err(adapter_error("deterministic training failure"));
            }
            Ok(TrainOutput {
                model: ModelArtifactIdentity::new(
                    format!("candidate-{}", candidate.id),
                    "fake-ranking",
                    13,
                    artifact_core::fingerprint(&(candidate.id, "model")).map_err(adapter_error)?,
                )
                .map_err(adapter_error)?,
                duration_seconds: 1,
                metadata: json!({"fake": true, "candidate_id": candidate.id}),
            })
        })
    }

    fn evaluate(
        &self,
        project: ExternalProjectSnapshot,
        model: ModelArtifactIdentity,
        contract: MetricContract,
        suite_key: String,
        _maximum_seconds: u64,
    ) -> BoxFuture<'_, Result<EvaluationReport, EncoderTaskAdapterError>> {
        Box::pin(async move {
            let baseline = model == project.baseline_model;
            let (role, fingerprint) = if suite_key == "sealed" {
                (
                    EvidenceRole::SealedAcceptance,
                    self.configuration
                        .generation
                        .sealed_suite_fingerprint
                        .clone(),
                )
            } else {
                let authority = self
                    .configuration
                    .generation
                    .development_suites
                    .iter()
                    .find(|value| value.suite_key == suite_key)
                    .ok_or_else(|| adapter_error("unknown fixture suite"))?;
                (
                    EvidenceRole::Development,
                    authority.bundle.development_suite_fingerprint.clone(),
                )
            };
            self.record(json!({"operation": "evaluate", "baseline": baseline, "role": role, "suite": suite_key}))?;
            let score = if baseline {
                0.8
            } else if (self.configuration.outcome == "development_rejection"
                && suite_key == "retired")
                || (self.configuration.outcome == "sealed_rejection"
                    && role == EvidenceRole::SealedAcceptance)
            {
                0.7
            } else {
                0.9
            };
            EvaluationReport::create(
                &project,
                model,
                role,
                suite_key,
                fingerprint,
                &contract,
                BTreeMap::from([("mrr".into(), score), ("recall_at_2".into(), score)]),
                3,
                chrono::Utc::now(),
            )
            .map_err(adapter_error)
        })
    }
}
