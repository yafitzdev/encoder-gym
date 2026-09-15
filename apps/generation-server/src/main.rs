use anyhow::Context;
use synthetic_data_server::app;
use synthetic_data_sqlite::SqliteStore;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = std::env::var("SYNTH_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://encoder-gym.db?mode=rwc".into());
    let bind_address =
        std::env::var("SYNTH_BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let store = SqliteStore::connect(&database_url)
        .await
        .with_context(|| format!("could not open database at {database_url}"))?;
    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("could not bind {bind_address}"))?;
    tracing::info!(address = %bind_address, "synthetic-data control surface is ready");
    axum::serve(listener, app(store)).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
