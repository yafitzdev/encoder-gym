//! First Agent input handoff. Reads only exact persisted scientific sources;
//! no native evaluation, training, generation or provider dispatch happens here.

use super::*;

pub(super) async fn bind(folder: &Path, run_id: Uuid) -> Result<()> {
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| AppendActivity {
        action_id,
        operation: "optimization.bind_iteration".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references: vec![
            ActivityReference::new("run", run_id.to_string()).expect("UUID is a valid reference"),
            ActivityReference::new("iteration", "1").expect("Iteration is a valid reference"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result = bind_inputs(folder, run_id).await;
    let terminal = match &result {
        Ok(_) => event(ActivityEventState::Succeeded, None),
        Err(_) => event(
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "iteration_inputs_unavailable",
                "The run's prepared inputs and saved development evidence could not be bound. No provider was called.",
            )?),
        ),
    };
    append_activity(folder, terminal).await?;
    super::super::print(&serde_json::json!({"actionId":action_id,"iteration":result?}))
}

pub(super) async fn bind_inputs(
    folder: &Path,
    run_id: Uuid,
) -> Result<project_workspace_core::optimization_iteration::ProjectOptimizationIteration> {
    let run = optimization_runs::show(folder, run_id).await?;
    let preparation = run
        .preparation
        .as_ref()
        .context("Verify this run's inputs before starting Agent analysis")?;
    // Read the benchmark's pinned historical connection, not today's default.
    let (benchmark, binding) =
        project_workspace_local::benchmarks::inspect(folder, preparation.benchmark.id.parse()?)
            .await?;
    let store = super::super::open_bound_store(&folder.to_string_lossy(), &binding).await?;
    let result = async {
        let project = super::super::load_bound_project(&store, &binding).await?;
        ensure!(
            preparation.runtime_project.id == project.id.to_string()
                && preparation.runtime_project.fingerprint == project.fingerprint,
            "Saved baseline development evidence belongs to another prepared runtime; matching evidence is required"
        );
        let protocol = store.get_protocol(benchmark.source.protocol.id.parse()?).await?
            .context("The benchmark's saved baseline protocol is missing")?;
        ensure!(protocol.fingerprint == benchmark.source.protocol.fingerprint,
            "The saved development protocol changed");
        let definition = NomosBackend::recorded_benchmark(&project, &protocol)?;
        project_workspace_local::optimization_iterations::begin_first(
            folder, run_id, &project, &protocol, &definition,
        ).await
    }.await;
    store.pool().close().await;
    result
}
