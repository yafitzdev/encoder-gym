//! Offline desktop acceptance seed, excluded from the production executable.
mod benchmark_support;
mod desktop_agent_seed;
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
    if arguments
        .first()
        .is_some_and(|value| value == "--agent-project")
    {
        return desktop_agent_seed::create(&arguments[1..]).await;
    }
    if arguments.first().is_some_and(|value| value == "-B") {
        return if arguments.len() == 4 {
            native_inventory(&arguments)
        } else {
            native_capabilities(&arguments)
        };
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--held-native-descendant")
    {
        let ready = PathBuf::from(arguments.get(1).context("Missing descendant handshake")?);
        fs::write(ready, std::process::id().to_string())?;
        std::thread::sleep(std::time::Duration::from_secs(30));
        return Ok(());
    }
    if arguments.first().is_some_and(|value| value == "-c") {
        return native_clearance(&arguments);
    }
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

// Deterministic native adapter boundary, not an alternative production path.
// Python tests separately exercise the embedded complete-population algorithm.
fn native_capabilities(arguments: &[String]) -> Result<()> {
    const SCRIPT: &str = r#"import importlib.util,json,sys
names=['torch','sentence_transformers','transformers','datasets','accelerate','numpy','sklearn','psutil','onnxruntime_genai']
print(json.dumps({'version':'.'.join(map(str,sys.version_info[:3])),'major':sys.version_info[0],'minor':sys.version_info[1],'modules':{name:importlib.util.find_spec(name) is not None for name in names}}))"#;
    ensure!(
        arguments == ["-B", "-c", SCRIPT],
        "Unexpected native capability program: {arguments:?}"
    );
    println!(
        "{}",
        serde_json::json!({
            "version":"3.11.11", "major":3, "minor":11,
            "modules":{
                "torch":true, "sentence_transformers":true, "transformers":true,
                "datasets":true, "accelerate":true, "numpy":true, "sklearn":true,
                "psutil":true, "onnxruntime_genai":true,
            },
        })
    );
    Ok(())
}

fn native_inventory(arguments: &[String]) -> Result<()> {
    const PROGRAM: &str = include_str!(
        "../../../../crates/encoder-experiment-nomos/src/inspect_training_inventory.py"
    );
    ensure!(
        arguments.get(1).map(String::as_str) == Some("-c")
            && arguments.get(2).map(String::as_str) == Some(PROGRAM),
        "Unexpected native inventory program"
    );
    let root = env::current_dir()?;
    let request_path = root.join(arguments.get(3).context("Missing inventory request")?);
    let request: serde_json::Value = serde_json::from_slice(&fs::read(&request_path)?)?;
    let members_path = root.join(
        request["membersKey"]
            .as_str()
            .context("Missing inventory members")?,
    );
    let members = fs::read_to_string(members_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        request["rows"].as_u64() == Some(members.len() as u64),
        "Inventory member count changed"
    );
    let projected = members
        .iter()
        .map(|member| {
            let id = member["memberId"]
                .as_str()
                .context("Missing inventory member identity")?;
            Ok(serde_json::json!({
                "memberId": id,
                "nativeContextFingerprint": artifact_core::fingerprint(&serde_json::json!(["fixture-context", id]))?,
                "nativeModelInputFingerprint": artifact_core::fingerprint(&serde_json::json!(["fixture-model-input", id, member["row"]["question"]]))?,
                "labelFingerprint": artifact_core::fingerprint(&member["row"]["label"])?
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let output = serde_json::json!({
        "protocol": "nomos-training-inventory-v1",
        "requestFingerprint": request["fingerprint"],
        "datasetFingerprint": request["datasetFingerprint"],
        "rows": projected.len(),
        "members": projected,
    });
    fs::write(
        request_path.with_file_name("result.json"),
        serde_json::to_vec(&output)?,
    )?;
    Ok(())
}

fn native_clearance(arguments: &[String]) -> Result<()> {
    ensure!(
        arguments.get(1).map(String::as_str)
            == Some(include_str!(
                "../../../../crates/encoder-experiment-nomos/src/qualify_training.py"
            )),
        "Unexpected native audit program"
    );
    let path = PathBuf::from(arguments.get(2).context("Missing audit request")?);
    let request: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        request["protocol"] == "nomos-training-clearance-v2",
        "Unknown audit protocol"
    );
    let training = fs::read_to_string(
        request["training"]["key"]
            .as_str()
            .context("Missing training artifact")?,
    )?;
    ensure!(
        training.lines().count() as u64 == request["rows"].as_u64().unwrap(),
        "Wrong population"
    );
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("native-invocations.log")?,
        "encoder_gym.qualify_training"
    )?;
    let duplicate_rows = fs::read_to_string("runs/fixture-clearance-duplicates")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_json(
        &path.with_file_name("result.json"),
        &serde_json::json!({
            "protocol":request["protocol"], "requestFingerprint":request["fingerprint"],
            "trainingRows":request["rows"], "benchmarkRows":3,
            "invalidRows":0, "duplicateRows":duplicate_rows, "overlapRows":0,
            "missingGroupRows":request["rows"].as_u64().unwrap() + 3,
            "missingLineageRows":request["rows"].as_u64().unwrap() + 3,
        }),
    )
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
        "tools.train_dense_triplet_router" => {
            if let Some(ready) = env::var_os("ENCODER_FIXTURE_TRAINING_HOLD") {
                let ready = PathBuf::from(ready);
                let descendant_ready = ready.with_extension("descendant");
                let mut descendant = std::process::Command::new(env::current_exe()?)
                    .arg("--held-native-descendant")
                    .arg(&descendant_ready)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !descendant_ready.exists() {
                    if std::time::Instant::now() >= deadline {
                        let _ = descendant.kill();
                        let _ = descendant.wait();
                        anyhow::bail!("Native descendant did not start");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                fs::write(
                    ready,
                    serde_json::to_vec(&[std::process::id(), descendant.id()])?,
                )?;
                std::thread::sleep(std::time::Duration::from_secs(30));
                anyhow::bail!("The held fixture trainer was not stopped");
            }
            let input = argument(arguments, "--input")?;
            let count = fs::read_to_string(input)?.lines().count();
            eprintln!("50%|#### | 1/2 [00:01<00:01, 1.0it/s]");
            eprintln!("100%|######## | 2/2 [00:02<00:00, 1.0it/s]");
            let output = PathBuf::from(argument(arguments, "--output")?);
            training_transformer::fixture::write_tiny_bert_bundle(&output)
                .map_err(anyhow::Error::msg)?;
            write_json(
                &output.join("modules.json"),
                &serde_json::json!([
                    {"idx":0,"name":"0","path":"","type":"sentence_transformers.models.Transformer"}
                ]),
            )?;
            write_json(
                &output.join("nomos_training_manifest.json"),
                &serde_json::json!({
                    "output":output.to_string_lossy(), "base_model":argument(arguments,"--base-model")?,
                    "inputs":[input], "input_row_counts":{input:count}, "trainable_row_counts":{input:count},
                    "unique_trainable_rows":count, "training_triplets":count,
                    "epochs":argument(arguments,"--epochs")?.parse::<f64>()?,
                    "batch_size":argument(arguments,"--batch-size")?.parse::<u64>()?,
                    "learning_rate":argument(arguments,"--learning-rate")?.parse::<f64>()?,
                    "device":argument(arguments,"--device")?, "seed":argument(arguments,"--seed")?.parse::<u64>()?,
                    "margin":argument(arguments,"--margin")?.parse::<f64>()?,
                    "query_strategy":argument(arguments,"--query-strategy")?,
                    "positive_strategy":argument(arguments,"--positive-strategy")?,
                    "training_script":"tools.train_dense_triplet_router.v2",
                    "training_loss":0.25,"training_duration_seconds":2.0
                }),
            )?;
        }
        "tools.evaluate_dense_router" => {
            let input = argument(arguments, "--input")?;
            let output = PathBuf::from(argument(arguments, "--output")?);
            let mrr = if input.contains("holdout") {
                if PathBuf::from("runs/fixture-eligible-candidates").exists()
                    && !PathBuf::from("runs/fixture-final-rejected").exists()
                    && PathBuf::from(argument(arguments, "--model")?)
                        .join("nomos_training_manifest.json")
                        .is_file()
                {
                    99_999.225
                } else {
                    99_999.125
                }
            } else if PathBuf::from("runs/fixture-eligible-candidates").exists() {
                let manifest: serde_json::Value = serde_json::from_slice(&fs::read(
                    PathBuf::from(argument(arguments, "--model")?)
                        .join("nomos_training_manifest.json"),
                )?)?;
                if manifest["unique_trainable_rows"] == 2 {
                    0.95
                } else if PathBuf::from("runs/fixture-reject-smaller-candidate").exists() {
                    0.70
                } else {
                    0.76
                }
            } else if input.contains("regression") {
                0.72
            } else {
                0.75
            };
            write_json(
                &output,
                &serde_json::json!({
                    "model":argument(arguments,"--model")?,
                    "inputs":{input:{"metrics":{
                        "states":11,
                        "recall_at_1":0.5,
                        "recall_at_2":0.7,
                        "recall_at_3":0.8,
                        "mrr":mrr,
                        "mean_positive_margin":0.2
                    },
                    "by_task_kind":{"route":{"states":11,"recall_at_1":0.5,"recall_at_2":0.7,"recall_at_3":0.8,"mrr":mrr,"mean_positive_margin":0.2}},
                    "by_pool_size":{"2":{"states":11,"recall_at_1":0.5,"recall_at_2":0.7,"recall_at_3":0.8,"mrr":mrr,"mean_positive_margin":0.2}},
                    "by_scenario_family":{"unspecified":{"states":11,"recall_at_1":0.5,"recall_at_2":0.7,"recall_at_3":0.8,"mrr":mrr,"mean_positive_margin":0.2}},
                    "by_expected_capability":{"search":{"states":11,"recall_at_1":0.5,"recall_at_2":0.7,"recall_at_3":0.8,"mrr":mrr,"mean_positive_margin":0.2}},
                    "disagreements":[{"decision_state_id":"candidate-failure", "question":"Retain useful coverage after candidate regression", "task_kind":"route", "expected_capabilities":["search"], "predicted_capabilities":["write"], "expected_rank":3}]}}
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
