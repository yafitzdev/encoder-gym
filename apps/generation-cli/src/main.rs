mod cli;
mod commands;
mod document;
mod presentation;
mod training_examples;

use anyhow::Context;
use clap::Parser;
use cli::{Cli, Command};
use recovery_core::RecoveryStore;
use synthetic_data_sqlite::SqliteStore;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let cli = Cli::parse();
    presentation::set_output(cli.output)?;
    let database_url = cli.database_url();
    let command = match cli.command {
        Command::Experiment { command } => {
            // Keep the experiment handler's aggregate future off the small Windows
            // main-thread stack, just like the ordinary command dispatcher below.
            return Box::pin(commands::experiment::execute(command, &database_url)).await;
        }
        command => command,
    };
    let store = SqliteStore::connect(&database_url)
        .await
        .with_context(|| format!("could not open database at {database_url}"))?;
    let interrupted = store.detect_interrupted_workflows().await?;
    if !interrupted.is_empty() {
        eprintln!(
            "detected {} interrupted workflow(s); inspect them with `synth recovery list`",
            interrupted.len()
        );
    }
    commands::execute(command, store).await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::CommandFactory;

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
