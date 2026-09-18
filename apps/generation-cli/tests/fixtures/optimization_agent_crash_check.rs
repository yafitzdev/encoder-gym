//! Abrupt coordinator death at durable artifact boundaries, not returned errors.
//! A test-only SQLite trigger holds the target write transaction until its exact
//! owned process is killed. Production has no fault-injection switches.
use super::*;
use sqlx::sqlite::SqliteConnectOptions;
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

pub(super) async fn exercise(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    calls: &Path,
    command: impl Fn() -> Command,
) {
    let project = folder.join("project.sqlite");
    let scientific = folder.join("runs/scientific.sqlite");
    let boundaries = [
        (
            &project,
            "optimization_dataset_publications",
            "1",
            "dataset publication",
            "SELECT COUNT(*) > 0 FROM optimization_generation_outcomes",
            1,
            0,
            3,
        ),
        (
            &project,
            "project_activity_events",
            "NEW.operation='optimization.qualification' AND NEW.state='succeeded'",
            "qualification",
            "SELECT COUNT(*) > 0 FROM optimization_dataset_publications",
            2,
            0,
            3,
        ),
        (
            &scientific,
            "encoder_experiment_events",
            "json_extract(NEW.artifact_json,'$.event.kind')='candidate_training_completed'",
            "training completion",
            "SELECT 1",
            2,
            1,
            3,
        ),
        (
            &scientific,
            "encoder_experiment_events",
            "json_extract(NEW.artifact_json,'$.event.kind')='candidate_development_suite_completed' AND (SELECT COUNT(*) FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed')=0",
            "first development report",
            "SELECT COUNT(*) > 0 FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_training_completed'",
            2,
            1,
            4,
        ),
        (
            &scientific,
            "encoder_experiment_events",
            "json_extract(NEW.artifact_json,'$.event.kind')='candidate_development_suite_completed' AND (SELECT COUNT(*) FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed')=1",
            "second development report",
            "SELECT COUNT(*) = 1 FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed'",
            2,
            1,
            5,
        ),
        (
            &project,
            "model_artifacts",
            "NEW.origin='trained'",
            "model registration",
            "SELECT 1",
            2,
            1,
            5,
        ),
        (
            &project,
            "optimization_iteration_results",
            "1",
            "iteration result",
            "SELECT COUNT(*) > 0 FROM model_artifacts WHERE origin='trained'",
            2,
            1,
            5,
        ),
        (
            &project,
            "optimization_iteration_completions",
            "1",
            "iteration completion",
            "SELECT COUNT(*) > 0 FROM optimization_iteration_results",
            2,
            1,
            5,
        ),
        (
            &project,
            "optimization_agent_execution_events",
            "NEW.kind='completed'",
            "root completion",
            "SELECT COUNT(*) > 0 FROM optimization_iteration_completions",
            2,
            1,
            5,
        ),
    ];
    let mut scientific_probe = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&scientific)
            .read_only(true)
            .busy_timeout(Duration::from_millis(100)),
    )
    .await
    .unwrap();
    let mut provider_bytes = None;
    for (database, table, condition, label, prerequisite, qualifications, trainings, evaluations) in
        boundaries
    {
        eprintln!("Abrupt-death boundary: {label}");
        install_trigger(database, &format!("CREATE TRIGGER crash_boundary BEFORE INSERT ON {table} WHEN {condition} BEGIN SELECT (WITH RECURSIVE hold(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM hold WHERE n<1000000000) SELECT sum(n) FROM hold); END")).await;
        let options = SqliteConnectOptions::new()
            .filename(database)
            .busy_timeout(Duration::from_millis(100));
        let mut probe = SqliteConnection::connect_with(&options).await.unwrap();
        let mut child = command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut locked_since = None;
        let reached = loop {
            if child.try_wait().unwrap().is_some() || Instant::now() > deadline {
                break false;
            }
            match sqlx::query("BEGIN IMMEDIATE").execute(&mut probe).await {
                Ok(_) => {
                    sqlx::query("ROLLBACK").execute(&mut probe).await.unwrap();
                    locked_since = None;
                }
                Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("5") => {
                    // Migrations can also hold a write lock under parallel test
                    // load. Require the target stage's durable predecessors and
                    // native invocations before identifying the injected stall.
                    let native = fs::read_to_string(root.join("runtime/native-invocations.log"))
                        .unwrap_or_default();
                    let count = |name| native.lines().filter(|line| *line == name).count();
                    let reports_complete = table != "model_artifacts"
                        || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM encoder_experiment_events WHERE json_extract(artifact_json,'$.event.kind')='candidate_development_suite_completed'")
                            .fetch_one(&mut scientific_probe).await.unwrap_or_default() == 2;
                    let ready = reports_complete
                        && sqlx::query_scalar::<_, bool>(prerequisite)
                            .fetch_one(&mut probe)
                            .await
                            .unwrap_or(false)
                        && count("encoder_gym.qualify_training") >= qualifications
                        && count("tools.train_dense_triplet_router") >= trainings
                        && count("tools.evaluate_dense_router") >= evaluations
                        && count("tools.evaluate_real_agent_sessions") >= evaluations;
                    if !ready {
                        locked_since = None;
                        continue;
                    }
                    if locked_since.get_or_insert_with(Instant::now).elapsed()
                        > Duration::from_secs(2)
                    {
                        break true;
                    }
                }
                Err(error) => panic!("Could not inspect {label} transaction: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        probe.close().await.unwrap();
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        drop_trigger(database, "crash_boundary").await;
        assert!(
            reached,
            "Did not reach {label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "The coordinator must actually be killed"
        );
        let saved_calls = fs::read(calls).unwrap();
        if let Some(previous) = &provider_bytes {
            assert_eq!(&saved_calls, previous, "Provider replay at {label}");
        } else {
            provider_bytes = Some(saved_calls);
        }
    }
    let output = command().output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let completed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(completed["completion"]["end"], "iteration_limit");
    assert_eq!(fs::read(calls).unwrap(), provider_bytes.unwrap());
    let native = fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap();
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "tools.train_dense_triplet_router")
            .count(),
        1
    );
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "encoder_gym.qualify_training")
            .count(),
        2
    );
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "tools.evaluate_dense_router")
            .count(),
        5
    );
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "tools.evaluate_real_agent_sessions")
            .count(),
        5
    );
    let inventory = project_workspace_local::open_workspace(folder, true)
        .await
        .unwrap();
    assert_eq!(inventory.model_catalog.unwrap().artifacts.len(), 2);
    assert_eq!(inventory.model_dataset_links.len(), 1);
    assert_eq!(
        optimization_runs::show(folder, run_id).await.unwrap().state,
        project_workspace_core::ProjectOptimizationRunState::AgentCompleted
    );
    assert_eq!(command().output().unwrap().stdout, output.stdout);
    assert_eq!(
        fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
}
