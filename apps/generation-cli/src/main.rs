mod cli;
mod commands;
mod database_access;
mod document;
mod presentation;
mod process_ownership;
mod training_examples;

use anyhow::Context;
use clap::Parser;
use cli::{Cli, Command, DatabaseCommand, DatabaseKind, EncoderCommand, RecoveryCommand};
use database_access::DatabaseAccess;
use encoder_experiment_sqlite::SqliteExperimentStore;
use recovery_core::RecoveryStore;
use synthetic_data_sqlite::SqliteStore;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    process_ownership::initialize()?;
    // Windows gives the process main thread a comparatively small stack. Clap's
    // feature-oriented command tree and deeply verified provenance traversal
    // are both finite but intentionally broad, so run the application on one
    // explicitly sized local thread. Async worker behavior is unchanged.
    match std::thread::Builder::new()
        .name("synth-main".into())
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
    dotenvy::dotenv().ok();
    init_tracing();
    let cli = Cli::parse();
    presentation::set_output(cli.output)?;
    let database_url = cli.database_url();
    if let Some(result) = commands::execute_without_store(&cli.command) {
        return result;
    }
    let command = match cli.command {
        Command::Workspace { command } => return commands::workspace::execute(command).await,
        Command::Database {
            command: DatabaseCommand::Migrate { kind },
        } => {
            match kind {
                DatabaseKind::Classification => {
                    SqliteStore::connect(&database_url)
                        .await?
                        .pool()
                        .close()
                        .await
                }
                DatabaseKind::Production => {
                    SqliteExperimentStore::connect(&database_url)
                        .await?
                        .pool()
                        .close()
                        .await
                }
            }
            return presentation::print(&serde_json::json!({"migrated": true}));
        }
        Command::Experiment { command } => {
            // Keep the experiment handler's aggregate future off the small Windows
            // main-thread stack, just like the ordinary command dispatcher below.
            return Box::pin(commands::experiment::execute(command, &database_url)).await;
        }
        Command::BenchmarkGeneration { command } => {
            return Box::pin(commands::production_campaign::execute_generation(
                *command,
                &database_url,
            ))
            .await;
        }
        Command::ProductionCampaign { command } => {
            return Box::pin(commands::production_campaign::execute_campaign(
                *command,
                &database_url,
            ))
            .await;
        }
        Command::ProductionRepair { command } => {
            return Box::pin(commands::production_repair::execute(
                *command,
                &database_url,
            ))
            .await;
        }
        Command::Encoder {
            command: EncoderCommand::Optimize { command },
        } => {
            return Box::pin(commands::encoder_optimize::execute(*command, &database_url)).await;
        }
        command => command,
    };
    let access = command.database_access();
    let store = access
        .classification(&database_url)
        .await
        .with_context(|| format!("could not open database at {database_url}"))?;
    if access == DatabaseAccess::ReadWrite
        && !matches!(
            command,
            Command::Recovery {
                command: RecoveryCommand::Scan
            }
        )
    {
        let interrupted = store.detect_interrupted_workflows().await?;
        if !interrupted.is_empty() {
            eprintln!(
                "detected {} interrupted workflow(s); inspect them with `synth recovery list`",
                interrupted.len()
            );
        }
    }
    commands::execute(command, store).await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
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
