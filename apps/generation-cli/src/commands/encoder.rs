use anyhow::Context;
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::EncoderRegistry;
use training_transformer::BertBundle;

use crate::cli::EncoderCommand;

pub async fn execute(command: EncoderCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        EncoderCommand::Register { name, path } => {
            let bundle = BertBundle::inspect(name, path)?;
            let encoder = bundle.into_registration();
            store.register_encoder(&encoder).await?;
            crate::presentation::print(&encoder)
        }
        EncoderCommand::List => crate::presentation::print(&store.list_encoders().await?),
        EncoderCommand::Show { id } => {
            let encoder = store
                .get_encoder(id)
                .await?
                .with_context(|| format!("registered encoder not found: {id}"))?;
            crate::presentation::print(&encoder)
        }
        EncoderCommand::Verify { id } => {
            let encoder = store
                .get_encoder(id)
                .await?
                .with_context(|| format!("registered encoder not found: {id}"))?;
            BertBundle::verify(&encoder)?;
            crate::presentation::print(&serde_json::json!({
                "id": encoder.id,
                "fingerprint": encoder.fingerprint,
                "verified": true,
            }))
        }
    }
}
