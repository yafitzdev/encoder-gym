mod advisor;
mod allocation;
mod analysis;
mod backend;
mod benchmark;
mod campaign;
mod config;
mod contamination;
mod dataset;
mod doctor;
mod encoder;
mod evaluation;
mod export;
mod generation;
mod governance;
mod ingestion;
mod inspect;
mod optimization;
mod plan;
mod project_preparation;
mod provenance;
mod recovery;
mod snapshot;
mod training;
mod workflow;

use crate::cli::Command;
use synthetic_data_sqlite::SqliteStore;

pub async fn execute(command: Command, store: SqliteStore) -> anyhow::Result<()> {
    match command {
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
        Command::Snapshot { command } => snapshot::execute(command, &store).await,
        Command::Encoder { command } => encoder::execute(command, &store).await,
        Command::Training { command } => training::execute(command, store).await,
        Command::Evaluation { command } => evaluation::execute(command, store).await,
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
}
