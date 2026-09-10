//! Offline desktop acceptance seed, excluded from the production executable.
mod benchmark_support;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    domain::ModelArtifactIdentity, journal::first_event, ports::ExperimentStore,
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use std::{env, fs, path::PathBuf};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(
        env::args_os()
            .nth(1)
            .context("Provide a new fixture directory")?,
    );
    ensure!(!root.exists(), "Fixture directory must not already exist");
    fs::create_dir(&root)?;
    let (folder, project, first, changed) = benchmark_support::fixture(&root).await;
    let store = SqliteExperimentStore::connect(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await?;
    let (model, receipt, configuration) =
        benchmark_support::complete_rejected_candidate(&store, &project, first).await;
    let mut repeated = benchmark_support::protocol_for(&project, 0.0, 3);
    repeated
        .baseline_development_report
        .metrics
        .insert("mrr".into(), 0.72);
    repeated.baseline_development_report.fingerprint = repeated
        .baseline_development_report
        .reproduce_fingerprint()?;
    repeated.fingerprint = repeated.reproduce_fingerprint()?;
    store.create_protocol(repeated.clone()).await?;
    store
        .create_run(first_event(&repeated, Uuid::new_v4(), Utc::now())?)
        .await?;
    store.pool().close().await;
    benchmark_support::register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &model,
        first,
        &receipt,
        &configuration,
        "Candidate 01",
    )
    .await;
    let pending = ModelArtifactIdentity::new(
        "not-evaluated",
        model.format.clone(),
        model.bytes,
        format!("sha256:{}", "e".repeat(64)),
    )?;
    benchmark_support::register_fixture_model(
        &folder,
        &root.join("checkpoint"),
        &pending,
        Uuid::new_v4(),
        &receipt,
        &configuration,
        "Candidate 02",
    )
    .await;
    println!(
        "{}",
        serde_json::json!({"folder":folder,"firstRun":first,"changedRun":changed})
    );
    Ok(())
}
