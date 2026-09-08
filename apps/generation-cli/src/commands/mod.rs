mod advisor;
mod allocation;
mod analysis;
mod architect;
mod authenticity;
mod backend;
mod benchmark;
mod benchmark_architect;
mod campaign;
mod config;
mod contamination;
mod dataset;
mod doctor;
mod encoder;
pub(crate) mod encoder_optimize;
mod evaluation;
pub(crate) mod experiment;
mod export;
mod generation;
mod governance;
mod ingestion;
mod inspect;
mod optimization;
mod plan;
pub(crate) mod production_campaign;
pub(crate) mod production_repair;
mod project_bootstrap;
mod project_preparation;
mod provenance;
mod quality;
mod recovery;
mod research;
mod semantic;
mod snapshot;
mod supervisor;
mod training;
mod workflow;
pub(crate) mod workspace;

use crate::cli::Command;
use std::{future::Future, pin::Pin};
use synthetic_data_sqlite::SqliteStore;

pub fn execute_without_store(command: &Command) -> Option<anyhow::Result<()>> {
    use crate::cli::{BenchmarkArchitectCommand, ProjectCommand, QualityCommand};
    match command {
        Command::Config { command } => config::execute_without_store(command),
        Command::Project {
            command: ProjectCommand::BootstrapPreview { manifest },
        } => Some(project_bootstrap::preview(manifest)),
        Command::Quality {
            command: QualityCommand::PolicyPreview(args),
        } => Some(
            quality::compile_policy(args).and_then(|policy| crate::presentation::print(&policy)),
        ),
        Command::BenchmarkArchitect {
            command: BenchmarkArchitectCommand::BriefValidate { file },
        } => Some(
            benchmark_architect::resolve_brief(file)
                .and_then(|brief| crate::presentation::print(&brief)),
        ),
        _ => None,
    }
}

/// Keep the aggregate command-dispatch future off the comparatively small
/// Windows main-thread stack. Individual feature handlers remain independent,
/// while adding a large handler cannot inflate every unrelated CLI command's
/// stack frame.
pub fn execute(
    command: Command,
    store: SqliteStore,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
    Box::pin(async move {
        match command {
            Command::Workspace { .. } => {
                unreachable!("workspace commands use only their own project database")
            }
            Command::Database { .. } => {
                unreachable!("database maintenance is dispatched before startup")
            }
            Command::Doctor(args) => doctor::execute(args, &store).await,
            Command::Config { command } => config::execute(command, &store).await,
            Command::Project { command } => project_preparation::execute(command, &store).await,
            Command::Analysis { command } => analysis::execute(command, &store).await,
            Command::Advisor { command } => advisor::execute(command, &store).await,
            Command::Optimize { command } => optimization::execute(command, &store).await,
            Command::Campaign { command } => campaign::execute(command, &store).await,
            Command::Allocation { command } => allocation::execute(command, &store).await,
            Command::Cohort { command } => governance::cohort(command, &store).await,
            Command::Exposure { command } => governance::exposure(command, &store).await,
            Command::Contamination { command } => contamination::execute(command, &store).await,
            Command::Benchmark { command } => benchmark::execute(command, &store).await,
            Command::Workflow { command } => workflow::execute(command, &store).await,
            Command::Dataset { command } => dataset::execute(command, &store).await,
            Command::Semantic { command } => semantic::execute(command, &store).await,
            Command::Research { command } => research::execute(command, &store).await,
            Command::Architect { command } => architect::execute(command, &store).await,
            Command::BenchmarkArchitect { command } => {
                benchmark_architect::execute(command, &store).await
            }
            Command::Quality { command } => quality::execute(command, &store).await,
            Command::Supervisor { command } => supervisor::execute(command, &store).await,
            Command::Snapshot { command } => snapshot::execute(command, &store).await,
            Command::Encoder { command } => encoder::execute(command, &store).await,
            Command::Training { command } => training::execute(command, store).await,
            Command::Evaluation { command } => evaluation::execute(command, store).await,
            Command::Experiment { .. } => {
                unreachable!("experiment commands are dispatched before the synthetic-data store")
            }
            Command::BenchmarkGeneration { .. }
            | Command::ProductionCampaign { .. }
            | Command::ProductionRepair { .. } => {
                unreachable!(
                    "production campaign commands are dispatched before the synthetic-data store"
                )
            }
            Command::Plan { command } => plan::execute(command, &store).await,
            Command::Backend { command } => backend::execute(command, &store).await,
            Command::Recovery { command } => recovery::execute(command, store).await,
            Command::Provenance { kind, id } => provenance::execute(kind, id, &store).await,
            Command::Generate(args) => generation::execute(args, store).await,
            Command::Job { command } => inspect::job(command, &store).await,
            Command::Coverage { plan_id } => inspect::coverage(plan_id, &store).await,
            Command::Rows(args) => inspect::rows(args, &store).await,
            Command::Export(args) => export::execute(args, &store).await,
        }
    })
}
