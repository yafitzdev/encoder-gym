use anyhow::Context;
use chrono::Utc;
use generation_core::{
    domain::{BackendConfiguration, GenerationParameters},
    ports::BackendConfigurationStore,
};
use synthetic_data_sqlite::SqliteStore;

use crate::cli::BackendCommand;

const BACKEND_NAME: &str = "openai-compatible";

pub async fn execute(command: BackendCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        BackendCommand::Configure {
            base_url,
            model,
            temperature,
            max_tokens,
            seed,
        } => {
            let configuration = BackendConfiguration {
                name: BACKEND_NAME.into(),
                base_url: Some(base_url),
                model,
                parameters: GenerationParameters {
                    temperature,
                    max_tokens,
                    seed,
                    ..GenerationParameters::default()
                },
                updated_at: Utc::now(),
            };
            store.save_backend_configuration(&configuration).await?;
            crate::presentation::print(&configuration)?;
            Ok(())
        }
        BackendCommand::Show => {
            let configuration = store
                .get_backend_configuration(BACKEND_NAME)
                .await?
                .context("OpenAI-compatible backend has not been configured")?;
            crate::presentation::print(&configuration)?;
            Ok(())
        }
    }
}
