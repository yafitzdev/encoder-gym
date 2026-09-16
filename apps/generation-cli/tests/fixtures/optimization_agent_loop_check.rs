use super::*;
use project_workspace_local::{
    optimization_completions, optimization_iteration_execution, optimization_iterations,
};

pub(super) async fn assert_loop(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    mode: &str,
    first: &Value,
    execute: impl Fn() -> Value,
) {
    let view = optimization_runs::show(folder, run_id).await.unwrap();
    assert_eq!(
        view.state,
        project_workspace_core::ProjectOptimizationRunState::AgentCompleted
    );
    assert!(view.state.is_terminal());
    let execution = view.agent_execution.as_ref().unwrap();
    assert_eq!(
        execution.attempts,
        if mode == "no_change_first" { 3 } else { 2 }
    );
    let terminal: project_workspace_core::optimization_loop::IterationCompletion =
        serde_json::from_value(first["completion"].clone()).unwrap();
    assert_eq!(execution.completion, Some(terminal.identity()));
    assert!(optimization_runs::cancel(folder, run_id).await.is_err());
    let mut execution_db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM optimization_agent_execution_events WHERE run_id=? ORDER BY sequence",
    )
    .bind(run_id.to_string())
    .fetch_all(&mut execution_db)
    .await
    .unwrap();
    let expected = if mode == "no_change_first" {
        vec![
            "started",
            "failed",
            "started",
            "interrupted",
            "started",
            "completed",
        ]
    } else {
        vec![
            "started",
            if mode == "two_iterations" {
                "interrupted"
            } else {
                "failed"
            },
            "started",
            "completed",
        ]
    };
    assert_eq!(kinds, expected);
    assert!(
        sqlx::query("UPDATE optimization_agent_execution_events SET kind='started' WHERE run_id=?")
            .bind(run_id.to_string())
            .execute(&mut execution_db)
            .await
            .is_err()
    );
    execution_db.close().await.unwrap();
    assert_eq!(execute(), *first);
    assert_eq!(optimization_runs::show(folder, run_id).await.unwrap(), view);
    let expected_end = match mode {
        "no_change" | "eligible" | "no_change_first" => "no_change",
        "row_limit" => "row_change_limit",
        _ => "iteration_limit",
    };
    assert_eq!(first["completion"]["end"], expected_end);
    if mode == "no_change_first" {
        let completions = optimization_completions::list(folder, run_id)
            .await
            .unwrap();
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].total_row_changes, 0);
        assert!(completions[0].result.is_none() && completions[0].selected.is_none());
        let calls = fs::read(root.join("agent-calls.jsonl")).unwrap();
        let native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
        assert!(!String::from_utf8_lossy(&native).contains("tools.train_dense_triplet_router"));
        let inventory = project_workspace_local::open_workspace(folder, true)
            .await
            .unwrap();
        assert_eq!(inventory.model_catalog.as_ref().unwrap().artifacts.len(), 1);
        assert!(inventory.model_dataset_links.is_empty());
        let history = run(
            root,
            &[
                "optimization-run",
                "project",
                "history",
                &run_id.to_string(),
            ],
        );
        assert_eq!(history["iterations"].as_array().unwrap().len(), 1);
        let iteration = &history["iterations"][0];
        assert_eq!(iteration["noChange"], true);
        assert_eq!(iteration["completed"], true);
        assert!(iteration["modelId"].is_null() && iteration["developmentPassed"].is_null());
        assert!(iteration["checks"].as_array().unwrap().is_empty());
        assert_eq!(execute(), *first);
        assert_eq!(fs::read(root.join("agent-calls.jsonl")).unwrap(), calls);
        assert_eq!(
            fs::read(root.join("runtime/native-invocations.log")).unwrap(),
            native
        );
        return;
    }
    let full_cycles = if ["two_iterations", "eligible"].contains(&mode) {
        2
    } else {
        1
    };
    let count = if mode == "row_limit" {
        1
    } else if mode == "eligible" {
        3
    } else {
        2
    };
    let inputs = optimization_iterations::list(folder, run_id).await.unwrap();
    let completions = optimization_completions::list(folder, run_id)
        .await
        .unwrap();
    assert_eq!(inputs.len(), count);
    assert_eq!(completions.len(), count);
    assert_eq!(completions[0].row_changes, 2);
    assert_eq!(
        completions.last().unwrap().total_row_changes,
        if full_cycles == 2 { 3 } else { 2 }
    );
    if mode != "eligible" {
        assert!(
            completions.iter().all(|value| value.selected.is_none()),
            "Rejected candidates cannot become the next starting data"
        );
    }
    let first_training = optimization_iteration_execution::training(folder, run_id, inputs[0].id)
        .await
        .unwrap()
        .unwrap();
    if count >= 2 {
        let next = &inputs[1];
        assert_eq!(next.predecessor, Some(completions[0].identity()));
        assert_eq!(
            next.dataset,
            if mode == "eligible" {
                first_training.qualified_dataset.clone()
            } else {
                inputs[0].dataset.clone()
            }
        );
        assert_eq!(next.starting_model, inputs[0].starting_model);
        assert_eq!(
            next.comparison_baseline_revision,
            inputs[0].comparison_baseline_revision
        );
        assert_eq!(next.benchmark, inputs[0].benchmark);
        assert_eq!(next.scope.maximum_row_changes, 6);
        assert_ne!(next.development.model, inputs[0].development.model);
        assert_eq!(next.development.protocol, first_training.protocol);
        let mut db = SqliteConnection::connect(&format!(
            "sqlite://{}",
            folder.join("project.sqlite").display()
        ))
        .await
        .unwrap();
        let saved: String = sqlx::query_scalar(
            "SELECT metadata_json FROM optimization_iteration_results WHERE iteration_id=?",
        )
        .bind(inputs[0].id.to_string())
        .fetch_one(&mut db)
        .await
        .unwrap();
        let saved: project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult = serde_json::from_str(&saved).unwrap();
        for (suite, reference) in &next.development.reports {
            assert_eq!(reference.id, saved.reports[suite].id.to_string());
            assert_eq!(reference.fingerprint, saved.reports[suite].fingerprint);
        }
        // Persisted normalized predecessor IDs cannot be rewritten in place.
        assert!(
            sqlx::query("UPDATE optimization_iteration_completions SET iteration=9 WHERE run_id=?")
                .bind(run_id.to_string())
                .execute(&mut db)
                .await
                .is_err()
        );
        let foreign_keys = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap();
        assert!(foreign_keys.is_empty());
        db.close().await.unwrap();
        if mode == "no_change" {
            assert!(completions[1].result.is_none());
            assert_eq!(completions[1].row_changes, 0);
            assert!(
                optimization_iteration_execution::training(folder, run_id, next.id)
                    .await
                    .unwrap()
                    .is_none()
            );
        } else {
            let next_training = optimization_iteration_execution::training(folder, run_id, next.id)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(next_training.candidate, first_training.candidate);
            assert_ne!(
                next_training.qualified_dataset,
                first_training.qualified_dataset
            );
            let publication =
                project_workspace_local::optimization_dataset::publication(folder, run_id, 2)
                    .await
                    .unwrap();
            assert_eq!(publication.removed.len(), 1);
            assert_eq!(publication.generated.len(), 0);
            assert_eq!(publication.parent, next.dataset);
            if mode == "eligible" {
                assert_eq!(inputs[2].dataset, first_training.qualified_dataset);
                assert_eq!(inputs[2].development.protocol, next_training.protocol);
                assert_eq!(inputs[2].scope.maximum_row_changes, 5);
                assert!(
                    completions
                        .iter()
                        .all(|value| value.selected.as_ref().unwrap().candidate
                            == first_training.candidate)
                );
                assert_ne!(
                    completions[1].result,
                    completions[1]
                        .selected
                        .as_ref()
                        .map(|value| value.result.clone())
                );
                assert!(completions[2].result.is_none());
            }
        }
    }
    let calls_before = fs::read(root.join("agent-calls.jsonl")).unwrap();
    let native_before = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let native = String::from_utf8_lossy(&native_before);
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "tools.train_dense_triplet_router")
            .count(),
        full_cycles
    );
    assert_eq!(
        String::from_utf8_lossy(&calls_before).lines().count(),
        if mode == "eligible" {
            8
        } else if mode == "row_limit" {
            3
        } else if mode == "no_change" {
            5
        } else {
            6
        }
    );
    let inventory = project_workspace_local::open_workspace(folder, true)
        .await
        .unwrap();
    assert_eq!(
        inventory.model_catalog.as_ref().unwrap().artifacts.len(),
        full_cycles + 1
    );
    assert_eq!(inventory.model_dataset_links.len(), full_cycles);
    let benchmark = &inventory.benchmark_versions[0];
    let reports = run(
        root,
        &["benchmark", "project", "results", &benchmark.id.to_string()],
    );
    let candidates: Vec<_> = reports["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|model| model["isBaseline"] == false)
        .collect();
    assert_eq!(candidates.len(), full_cycles);
    assert!(
        candidates
            .iter()
            .all(|model| model["reports"].as_array().unwrap().len() == 2)
    );
    assert!(!reports.to_string().contains("99999.125"));
    let history = run(
        root,
        &[
            "optimization-run",
            "project",
            "history",
            &run_id.to_string(),
        ],
    );
    assert_eq!(history["runId"], run_id.to_string());
    assert_eq!(history["projectId"], inventory.manifest.id.to_string());
    let rows = history["iterations"].as_array().unwrap();
    assert_eq!(rows.len(), count);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["id"], inputs[index].id.to_string());
        assert_eq!(row["number"], index + 1);
        assert_eq!(
            row["inputDatasetVersionId"],
            inputs[index].dataset.id.to_string()
        );
        assert_eq!(row["startingModelId"], inputs[0].starting_model.id);
        assert_eq!(row["benchmarkVersionId"], benchmark.id.to_string());
        assert_eq!(row["completed"], true);
        assert_eq!(row["selected"], mode == "eligible" && index == 0);
        if index >= full_cycles {
            assert_eq!(row["noChange"], true);
            assert!(row["modelId"].is_null() && row["developmentPassed"].is_null());
            continue;
        }
        assert_eq!(row["noChange"], false);
        let training = optimization_iteration_execution::training(folder, run_id, inputs[index].id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row["trainingDatasetVersionId"],
            training.training_dataset.id.to_string()
        );
        assert_eq!(
            row["qualifiedDatasetVersionId"],
            training.qualified_dataset.id.to_string()
        );
        assert_eq!(
            row["experimentRunId"],
            training.experiment_run_id.to_string()
        );
        let model = candidates
            .iter()
            .find(|model| model["modelId"] == row["modelId"])
            .unwrap();
        let mut checks = 0;
        let mut passed = true;
        for report in model["reports"].as_array().unwrap() {
            for context in report["contexts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|context| context["runId"] == row["experimentRunId"])
            {
                passed &= context["assessment"]["verdict"] == "passed";
                for gate in context["assessment"]["gates"].as_array().unwrap() {
                    checks += 1;
                    let check = row["checks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|check| {
                            check["reportId"] == report["result"]["report_id"]
                                && check["metric"] == gate["key"]
                        })
                        .unwrap();
                    assert_eq!(check["baseline"], gate["baseline"]);
                    assert_eq!(check["candidate"], gate["candidate"]);
                    assert_eq!(check["passed"], gate["passed"]);
                }
            }
        }
        assert!(checks > 0);
        assert_eq!(row["developmentPassed"], passed);
        assert_eq!(row["checks"].as_array().unwrap().len(), checks);
    }
    assert!(!history.to_string().contains("99999.125"));
    assert!(!history.to_string().contains("NEVER_DISCLOSE_HOLDOUT"));
    assert_eq!(
        run(
            root,
            &[
                "optimization-run",
                "project",
                "history",
                &run_id.to_string()
            ]
        ),
        history
    );
    let versions = dataset_versions::list(folder).await.unwrap();
    let again = execute();
    assert_eq!(again, *first);
    assert_eq!(
        fs::read(root.join("agent-calls.jsonl")).unwrap(),
        calls_before
    );
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native_before
    );
    assert_eq!(
        serde_json::to_value(dataset_versions::list(folder).await.unwrap()).unwrap(),
        serde_json::to_value(versions).unwrap()
    );
    let after = project_workspace_local::open_workspace(folder, true)
        .await
        .unwrap();
    assert_eq!(after.model_catalog, inventory.model_catalog);
    assert_eq!(after.model_dataset_links, inventory.model_dataset_links);
    assert_eq!(after.datasets, inventory.datasets);
    let activity = run(root, &["activity", "project", "list"]).to_string();
    assert!(activity.contains("Replace the ambiguous search example"));
    if count >= 2 {
        assert!(activity.contains("candidate regression"));
    }
    assert!(!activity.contains("NEVER_DISCLOSE_HOLDOUT"));
    // A self-consistently re-fingerprinted completion is not authority. Reads
    // must reconstruct its proposal and selection from their owning journals.
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER immutable_iteration_completions_update")
        .execute(&mut db)
        .await
        .unwrap();
    let mut changed = completions.last().unwrap().clone();
    changed.proposal_fingerprint = format!("sha256:{}", "e".repeat(64));
    changed.fingerprint = changed.reproduce().unwrap();
    sqlx::query("UPDATE optimization_iteration_completions SET fingerprint=?,metadata_json=? WHERE iteration_id=?")
        .bind(&changed.fingerprint).bind(serde_json::to_string(&changed).unwrap()).bind(&changed.iteration.id)
        .execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    assert!(
        optimization_completions::list(folder, run_id)
            .await
            .is_err()
    );
    assert!(optimization_runs::show(folder, run_id).await.is_err());
    let rejected = Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args([
            "--output",
            "json",
            "workspace",
            "optimization-run",
            "project",
            "drive-agent",
            &run_id.to_string(),
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Completion differs"));
    assert_eq!(
        fs::read(root.join("agent-calls.jsonl")).unwrap(),
        calls_before
    );
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native_before
    );
}
