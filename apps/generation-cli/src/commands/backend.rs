use anyhow::Context;
use chrono::Utc;
use generation_core::{
    domain::{BackendConfiguration, GenerationParameters},
    ports::BackendConfigurationStore,
};
use generation_openai_compatible::OpenAICompatibleBackend;
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
        BackendCommand::Check {
            base_url,
            model,
            api_key_env,
        } => {
            let persisted = store.get_backend_configuration(BACKEND_NAME).await?;
            let base_url = base_url
                .or_else(|| {
                    persisted
                        .as_ref()
                        .and_then(|configuration| configuration.base_url.clone())
                })
                .context("provide --base-url or configure the OpenAI-compatible backend first")?;
            let model = model
                .or_else(|| {
                    persisted
                        .as_ref()
                        .map(|configuration| configuration.model.clone())
                })
                .context("provide --model or configure the OpenAI-compatible backend first")?;
            let api_key = std::env::var(&api_key_env).ok();
            let probe = OpenAICompatibleBackend::probe(&base_url, api_key, &model).await?;
            crate::presentation::print(&serde_json::json!({
                "ready": true,
                "probe": probe,
                "api_key_env": api_key_env,
            }))?;
            Ok(())
        }
    }
}
