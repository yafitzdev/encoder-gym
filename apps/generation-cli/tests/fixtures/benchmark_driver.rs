//! Offline desktop acceptance seed, excluded from the production executable.
mod benchmark_support;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::{
    domain::ModelArtifactIdentity, journal::first_event, ports::ExperimentStore,
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use std::{env, fs, io::Write, path::PathBuf};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|value| value == "-m") {
        return native_evaluation(&arguments);
    }
    let root = PathBuf::from(
        arguments
            .first()
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

fn native_evaluation(arguments: &[String]) -> Result<()> {
    let module = arguments
        .get(1)
        .context("Native fixture module is missing")?;
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("native-invocations.log")?,
        "{module}"
    )?;
    match module.as_str() {
        "tools.evaluate_dense_router" => {
            let input = argument(arguments, "--input")?;
            let output = PathBuf::from(argument(arguments, "--output")?);
            let mrr = if input.contains("holdout") {
                99_999.125
            } else if input.contains("regression") {
                0.72
            } else {
                0.75
            };
            write_json(
                &output,
                &serde_json::json!({
                    "inputs":{input:{"metrics":{
                        "states":11,
                        "recall_at_1":0.5,
                        "recall_at_2":0.7,
                        "recall_at_3":0.8,
                        "mrr":mrr,
                        "mean_positive_margin":0.2
                    }}}
                }),
            )?;
        }
        "tools.evaluate_real_agent_sessions" => {
            let output = PathBuf::from(argument(arguments, "--output")?);
            let trace = PathBuf::from(argument(arguments, "--trace-output")?);
            let sessions: u64 = argument(arguments, "--sessions")?.parse()?;
            let attempts = sessions;
            let prompt_tokens = attempts * 10;
            let condition = argument(arguments, "--condition")?;
            write_json(
                &output,
                &serde_json::json!({
                    "backend":argument(arguments, "--backend")?,
                    "suite":argument(arguments, "--suite")?,
                    "suite_version":"offline-fixture-v1",
                    "sessions_per_condition":sessions,
                    "pairing":argument(arguments, "--pairing")?,
                    "max_attempts":argument(arguments, "--max-attempts")?.parse::<u64>()?,
                    "nomos_top_k":argument(arguments, "--nomos-top-k")?.parse::<u64>()?,
                    "summaries":{condition:{
                        "sessions":sessions,
                        "success_rate":0.8,
                        "mean_completed_stage_rate":0.9,
                        "prompt_tokens":prompt_tokens,
                        "completion_tokens":attempts * 5,
                        "tool_call_attempts":attempts,
                        "prompt_tokens_per_attempt":10.0,
                        "successful_execution_rate":0.9,
                        "tool_selection_accuracy":0.9,
                        "schema_valid_call_rate":1.0,
                        "wrong_tool_executions":0,
                        "invalid_calls":0,
                        "visible_oracle_hit_rate":0.8,
                        "tool_description_reduction":0.2
                    }}
                }),
            )?;
            if let Some(parent) = trace.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(trace, "{}\n")?;
        }
        _ => anyhow::bail!("Unsupported native fixture module: {module}"),
    }
    Ok(())
}

fn argument<'a>(arguments: &'a [String], name: &str) -> Result<&'a str> {
    let index = arguments
        .iter()
        .position(|value| value == name)
        .with_context(|| format!("Native fixture argument is missing: {name}"))?;
    arguments
        .get(index + 1)
        .map(String::as_str)
        .with_context(|| format!("Native fixture argument has no value: {name}"))
}

fn write_json(path: &PathBuf, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
