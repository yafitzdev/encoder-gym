//! Actual consent CLI over a completed production loop, with offline providers.
use super::*;
use project_workspace_core::optimization_final::{
    AgentFinalAuthorization, AgentFinalScope, SelectedFinalEvidence,
};
use project_workspace_local::{
    optimization_completions, optimization_final, optimization_iteration_execution,
    optimization_iterations,
};

fn command(root: &Path, run_id: Uuid, operation: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args([
            "--output",
            "json",
            "workspace",
            "optimization-run",
            "project",
            operation,
            &run_id.to_string(),
        ])
        .args(extra)
        .output()
        .unwrap()
}

pub(super) async fn assert_final_consent(root: &Path, folder: &Path, run_id: Uuid, mode: &str) {
    eprintln!("Checking final consent for {mode}");
    let database = folder.join("project.sqlite");
    let scientific_database = folder.join("runs/scientific.sqlite");
    let calls = fs::read(root.join("agent-calls.jsonl")).unwrap();
    let native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let before = fs::read(&database).unwrap();
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "final-agent-authorization",
                &run_id.to_string()
            ]
        ),
        Value::Null
    );
    let output = command(root, run_id, "preview-final-agent", &[]);
    assert_eq!(fs::read(&database).unwrap(), before);
    if mode != "eligible" {
        assert!(
            !output.status.success(),
            "Diagnostic/no-winner runs cannot use holdout"
        );
        assert_eq!(fs::read(root.join("agent-calls.jsonl")).unwrap(), calls);
        assert_eq!(
            fs::read(root.join("runtime/native-invocations.log")).unwrap(),
            native
        );
        return;
    }
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scope: AgentFinalScope = serde_json::from_slice(&output.stdout).unwrap();
    let view = optimization_runs::show(folder, run_id).await.unwrap();
    let launch = optimization_launch::list(folder)
        .await
        .unwrap()
        .into_iter()
        .find(|launch| launch.id.to_string() == view.run.launch.id)
        .unwrap();
    let completions = optimization_completions::list(folder, run_id)
        .await
        .unwrap();
    let completion = completions.last().unwrap();
    let iterations = optimization_iterations::list(folder, run_id).await.unwrap();
    assert_eq!(scope.iteration.id, iterations[0].id.to_string());
    assert_eq!(scope.completion.id, iterations[2].id.to_string());
    assert_ne!(scope.iteration.id, scope.completion.id);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("99999.125"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_DISCLOSE_HOLDOUT"));
    let store = SqliteExperimentStore::connect_read_only(&format!(
        "sqlite://{}",
        scientific_database.display()
    ))
    .await
    .unwrap();
    let mut scientific = Vec::new();
    let mut bindings = Vec::new();
    for iteration in &iterations {
        let Some(training) =
            optimization_iteration_execution::training(folder, run_id, iteration.id)
                .await
                .unwrap()
        else {
            continue;
        };
        let project = store
            .get_project(training.scientific_project.id.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        let protocol = store
            .get_protocol(training.protocol.id.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(protocol.budget.maximum_sealed_evaluations, 0);
        let events = store.load_events(training.experiment_run_id).await.unwrap();
        scientific.push(optimization_completions::IterationScientificEvidence {
            iteration_id: iteration.id,
            project,
            protocol,
            events,
        });
        bindings.push(training);
    }
    store.pool().close().await;
    let scientific_before = fs::read(&scientific_database).unwrap();
    assert_eq!(
        optimization_final::preview(folder, run_id, &scientific)
            .await
            .unwrap(),
        scope
    );
    assert!(
        optimization_final::preview(folder, run_id, &scientific[..1])
            .await
            .is_err(),
        "An unselected iteration must also be revalidated against its original journal"
    );
    let bind = |view: &project_workspace_core::ProjectOptimizationRunView, index: usize| {
        AgentFinalScope::bind(SelectedFinalEvidence {
            run: view,
            launch: &launch,
            completion,
            iteration: &iterations[index],
            training: &bindings[index],
            project: &scientific[index].project,
            protocol: &scientific[index].protocol,
            events: &scientific[index].events,
        })
    };
    assert_eq!(bind(&view, 0).unwrap(), scope);
    assert!(
        bind(&view, 1).is_err(),
        "A later eligible checkpoint cannot replace the deterministic winner"
    );
    use project_workspace_core::{
        ProjectOptimizationRunState as Run, optimization_execution::AgentExecutionState as Agent,
    };
    for (state, agent_state) in [
        (Run::AgentRunning, Agent::Running),
        (Run::AgentPaused, Agent::Paused),
        (Run::AgentInterrupted, Agent::Interrupted),
        (Run::AgentFailed, Agent::Failed),
    ] {
        let mut changed = view.clone();
        changed.state = state;
        changed.agent_execution.as_mut().unwrap().state = agent_state;
        assert!(bind(&changed, 0).is_err());
    }
    let mut exhausted = view.clone();
    exhausted.state = Run::AgentBudgetExhausted;
    exhausted.agent_execution.as_mut().unwrap().state = Agent::BudgetExhausted;
    exhausted.agent_execution.as_mut().unwrap().completion = None;
    assert!(
        bind(&exhausted, 0).is_ok(),
        "A budget stop retains its earlier completed winner"
    );
    assert!(
        AgentFinalAuthorization::create(
            Uuid::new_v4(),
            scope.clone(),
            "operator".into(),
            scope.adaptive_closed_at - chrono::Duration::seconds(1)
        )
        .is_err()
    );
    // Preview on a pre-consent schema must neither migrate nor create authority.
    let mut db = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    sqlx::query("DROP TABLE optimization_agent_final_authorizations")
        .execute(&mut db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=23")
        .execute(&mut db)
        .await
        .unwrap();
    db.close().await.unwrap();
    let old_schema = fs::read(&database).unwrap();
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "preview-final-agent",
                &run_id.to_string()
            ]
        ),
        serde_json::to_value(&scope).unwrap()
    );
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "final-agent-authorization",
                &run_id.to_string()
            ]
        ),
        Value::Null
    );
    assert_eq!(fs::read(&database).unwrap(), old_schema);
    let request = optimization_final::FinalAuthorizationRequest {
        id: Uuid::new_v4(),
        scope: scope.clone(),
    };
    let mut stale = request.clone();
    stale.scope.model.id = Uuid::new_v4().to_string();
    stale.scope.fingerprint = stale.scope.reproduce().unwrap();
    assert!(
        optimization_final::authorize(folder, run_id, stale, "operator", &scientific)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&database).unwrap(), old_schema);
    fs::write(
        root.join("final-consent.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let args = [
        "optimization-run",
        "project",
        "authorize-final-agent",
        &run_id.to_string(),
        "--file",
        "final-consent.json",
        "--authorized-by",
        "operator",
    ];
    let (saved, concurrent_retry) = std::thread::scope(|threads| {
        let first = threads.spawn(|| run(root, &args));
        let second = threads.spawn(|| run(root, &args));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(
        concurrent_retry, saved,
        "Concurrent first requests reuse one grant"
    );
    assert_eq!(
        run(root, &args),
        saved,
        "Lost-response retries reuse the same grant"
    );
    let grant: AgentFinalAuthorization = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(grant.scope, scope);
    let authorized = fs::read(&database).unwrap();
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "final-agent-authorization",
                &run_id.to_string()
            ]
        ),
        saved
    );
    assert_eq!(fs::read(&database).unwrap(), authorized);
    let other = optimization_final::FinalAuthorizationRequest {
        id: Uuid::new_v4(),
        scope: scope.clone(),
    };
    let (retry, competing) = tokio::join!(
        optimization_final::authorize(folder, run_id, request.clone(), "operator", &scientific),
        optimization_final::authorize(folder, run_id, other, "operator", &scientific),
    );
    assert_eq!(retry.unwrap(), grant);
    assert!(competing.is_err());
    assert!(
        optimization_final::authorize(
            folder,
            run_id,
            request.clone(),
            "another-operator",
            &scientific
        )
        .await
        .is_err()
    );
    let mut unknown = serde_json::to_value(&request).unwrap();
    unknown["execute"] = true.into();
    fs::write(
        root.join("bad-final-consent.json"),
        serde_json::to_vec(&unknown).unwrap(),
    )
    .unwrap();
    assert!(
        !command(
            root,
            run_id,
            "authorize-final-agent",
            &["--file", "bad-final-consent.json"]
        )
        .status
        .success()
    );
    assert_eq!(optimization_runs::show(folder, run_id).await.unwrap(), view);
    assert_eq!(fs::read(&scientific_database).unwrap(), scientific_before);
    assert_eq!(fs::read(root.join("agent-calls.jsonl")).unwrap(), calls);
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
    let mut db = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    for sql in [
        "UPDATE optimization_agent_final_authorizations SET scope_fingerprint='changed'",
        "DELETE FROM optimization_agent_final_authorizations",
    ] {
        assert!(sqlx::query(sql).execute(&mut db).await.is_err());
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .is_empty()
    );
    // Rehashing a changed scope and every normalized column does not make it consent.
    sqlx::query("DROP TRIGGER immutable_agent_final_authorization_update")
        .execute(&mut db)
        .await
        .unwrap();
    let mut changed_scope = scope;
    changed_scope.model.id = Uuid::new_v4().to_string();
    changed_scope.fingerprint = changed_scope.reproduce().unwrap();
    let changed = AgentFinalAuthorization::create(
        grant.id,
        changed_scope,
        grant.authorized_by,
        grant.created_at,
    )
    .unwrap();
    sqlx::query("UPDATE optimization_agent_final_authorizations SET scope_fingerprint=?,fingerprint=?,metadata_json=? WHERE run_id=?")
        .bind(&changed.scope.fingerprint).bind(&changed.fingerprint).bind(serde_json::to_string(&changed).unwrap()).bind(run_id.to_string())
        .execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    assert!(
        !command(root, run_id, "final-agent-authorization", &[])
            .status
            .success()
    );
}
