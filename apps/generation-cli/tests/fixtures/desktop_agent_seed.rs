//! Test-only project construction; desktop execution uses the unmodified CLI.
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use encoder_experiment_core::ports::ExperimentStore;
use encoder_experiment_sqlite::SqliteExperimentStore;
use project_workspace_core::{
    DatasetPurpose, ProviderAuthentication, ProviderCatalog, ProviderConfiguration, ProviderKind,
    ProviderLimits, ProviderRole,
};
use serde_json::{Value, json};
use std::{env, fs, path::PathBuf, process::Command};
use uuid::Uuid;

pub async fn create(arguments: &[String]) -> Result<()> {
    let root = PathBuf::from(arguments.first().context("Missing new project root")?);
    let endpoint = arguments
        .get(1)
        .context("Missing local provider endpoint")?;
    ensure!(
        endpoint.starts_with("http://127.0.0.1:"),
        "Fixture provider must be loopback"
    );
    ensure!(!root.exists(), "Fixture root must be new");
    fs::create_dir(&root)?;
    let executable = env::current_exe()?;
    let (folder, project) =
        super::benchmark_support::initial_benchmark_fixture(&root, &executable).await;
    let output =
        Command::new(executable.with_file_name(if cfg!(windows) { "synth.exe" } else { "synth" }))
            .args(["--output", "json", "workspace", "benchmark"])
            .arg(&folder)
            .arg("initialize")
            .output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let initialized: Value = serde_json::from_slice(&output.stdout)?;
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        folder.join("runs/scientific.sqlite").display()
    ))
    .await?;
    let protocol = store
        .get_protocol(
            initialized["version"]["source"]["protocol"]["id"]
                .as_str()
                .context("Protocol")?
                .parse()?,
        )
        .await?
        .context("Missing protocol")?;
    store.pool().close().await;
    for report in protocol.baseline_development_reports() {
        let suite = &project.task_configuration["suites"][&report.suite_key];
        let path = root
            .join("runtime/runs/encoder-gym-evaluations/by-content")
            .join(&report.model.fingerprint[7..])
            .join("retrieval")
            .join(
                &suite["retrieval_fingerprint"]
                    .as_str()
                    .context("Suite fingerprint")?[7..],
            )
            .join(format!("{}.json", report.suite_key));
        let mut saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
        saved["model"] = report.model.key.clone().into();
        saved["inputs"][suite["path"].as_str().context("Suite path")?]["disagreements"] = json!((0..50).map(|index| json!({
            "decision_state_id":format!("dev-failure-{index}"), "question":"Search for an exact reference", "task_kind":"route", "expected_rank":2,
            "expected_capabilities":["search"], "predicted_capabilities":["write"]})).collect::<Vec<_>>());
        fs::write(path, serde_json::to_vec(&saved)?)?;
    }
    fs::write(
        root.join("runtime/runs/fixture-eligible-candidates"),
        "two candidates",
    )?;
    fs::write(
        root.join("runtime/runs/fixture-reject-smaller-candidate"),
        "keep the first candidate and retain rejected history",
    )?;
    let tool = |id| json!({"tool_id":id,"tool_family":"search","description":"Find evidence","capabilities":["search"],"input_modalities":["text"],"output_modalities":["text"],"evidence_roles":["primary"],"side_effect_class":"none","argument_schema":{}});
    let row = |id, question| json!({"schema_version":"decision-state.v2","decision_state_id":id,"question":question,"evaluation_partition":"train","accepted":true,"task_kind":"route","previous_candidate_ids":[],"legal_candidate_ids":["a","b"],"label":{"acceptable_tools":["a"],"hard_negative_tools":["b"]},"tool_registry":{"registry_id":"r","registry_fingerprint":"sha256:registry","tools":[tool("a"),tool("b")]}});
    let source = root.join("selected.jsonl");
    fs::write(
        &source,
        format!(
            "{}\n{}\n",
            row("old", "Search something"),
            row("keep", "Retain this useful example")
        ),
    )?;
    let preview = project_workspace_local::inspect_dataset(&source, DatasetPurpose::Training)?;
    let workspace = project_workspace_local::import_dataset(
        &folder,
        &source,
        "Selected data",
        DatasetPurpose::Training,
        &preview.artifact.fingerprint,
    )
    .await?;
    project_workspace_local::dataset_versions::create_base(
        &folder,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "Selected data",
        &[workspace.datasets[0].id],
    )
    .await?;
    let provider = |role, model: &str| ProviderConfiguration {
        role,
        kind: ProviderKind::OpenaiCompatible,
        endpoint: Some(endpoint.clone()),
        model: model.into(),
        authentication: ProviderAuthentication::None,
        secret: None,
        limits: ProviderLimits {
            maximum_requests: 20,
            maximum_input_tokens: 2_000_000,
            maximum_output_tokens: 200_000,
            maximum_cost_microusd: 0,
        },
    };
    let providers = ProviderCatalog::create(
        Uuid::new_v4(),
        workspace.manifest.id,
        1,
        None,
        vec![
            provider(ProviderRole::Advisor, "pinned-agent"),
            provider(ProviderRole::Generation, "pinned-generator"),
        ],
        "fixture",
        "offline desktop acceptance",
        Utc::now(),
    )?;
    project_workspace_local::record_provider_catalog(&folder, providers, None).await?;
    println!(
        "{}",
        json!({"folder":folder,"projectId":workspace.manifest.id})
    );
    Ok(())
}
