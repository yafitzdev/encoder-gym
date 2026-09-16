//! Real final CLI dispatch with deterministic native work and lost-receipt recovery.
use super::*;
use project_workspace_core::optimization_final_execution::AgentFinalReceipt;
use project_workspace_local::optimization_final_execution as custody;
use std::collections::BTreeSet;

fn files(root: &Path) -> BTreeSet<std::path::PathBuf> {
    let mut result = BTreeSet::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path));
        } else {
            result.insert(path);
        }
    }
    result
}

pub(super) async fn assert_execution(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    grant: &AgentFinalAuthorization,
    scientific: &[optimization_completions::IterationScientificEvidence],
) {
    eprintln!("Checking at-most-once final execution and native-only recovery");
    let database = folder.join("project.sqlite");
    let scientific_database = folder.join("runs/scientific.sqlite");
    let scientific_before = fs::read(&scientific_database).unwrap();
    let calls = fs::read(root.join("agent-calls.jsonl")).unwrap();
    let native_path = root.join("runtime/native-invocations.log");
    let native_before = fs::read(&native_path).unwrap();
    let history = run(
        root,
        &[
            "optimization-run",
            "project",
            "history",
            &run_id.to_string(),
        ],
    );
    let project_before = fs::read(&database).unwrap();
    let before = command(root, run_id, "final-agent-result", &[]);
    assert!(
        before.status.success(),
        "{}",
        String::from_utf8_lossy(&before.stderr)
    );
    let before: Value = serde_json::from_slice(&before.stdout).unwrap();
    assert_eq!(before["state"], "authorized");
    assert_eq!(fs::read(&database).unwrap(), project_before);
    let wrong = Uuid::new_v4().to_string();
    assert_injected(
        command(
            root,
            run_id,
            "finalize-agent",
            &["--authorization-id", &wrong],
        ),
        "differs",
    );
    assert_eq!(fs::read(&database).unwrap(), project_before);
    assert_eq!(fs::read(&native_path).unwrap(), native_before);
    let id = grant.id.to_string();
    let execute = || command(root, run_id, "finalize-agent", &["--authorization-id", &id]);
    // A failed reservation must never reach either native evaluation component.
    install_trigger(&database, "CREATE TRIGGER fail_final_dispatch BEFORE INSERT ON optimization_agent_final_dispatches BEGIN SELECT RAISE(ABORT,'injected final dispatch failure'); END").await;
    assert_injected(execute(), "injected final dispatch failure");
    assert!(
        custody::show(folder, run_id, scientific)
            .await
            .unwrap()
            .unwrap()
            .dispatch
            .is_none()
    );
    assert_eq!(fs::read(&native_path).unwrap(), native_before);
    drop_trigger(&database, "fail_final_dispatch").await;
    let cache = root.join("runtime/runs/encoder-gym-evaluations");
    let files_before = files(&cache);
    // Simulate loss between native completion and the durable project receipt.
    install_trigger(&database, "CREATE TRIGGER fail_final_receipt BEFORE INSERT ON optimization_agent_final_results BEGIN SELECT RAISE(ABORT,'injected final receipt failure'); END").await;
    assert_injected(execute(), "injected final receipt failure");
    let pending = custody::show(folder, run_id, scientific)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.state(), "outcome_unknown");
    let dispatch = pending.dispatch.unwrap();
    let (first_retry, second_retry) = tokio::join!(
        custody::reserve(folder, run_id, grant.id, scientific),
        custody::reserve(folder, run_id, grant.id, scientific),
    );
    assert_eq!(first_retry.unwrap(), (dispatch.clone(), false));
    assert_eq!(second_retry.unwrap(), (dispatch.clone(), false));
    let native_after = fs::read(&native_path).unwrap();
    let added_calls = String::from_utf8_lossy(&native_after[native_before.len()..]);
    assert_eq!(
        added_calls.lines().collect::<Vec<_>>(),
        [
            "tools.evaluate_dense_router",
            "tools.evaluate_real_agent_sessions"
        ]
    );
    let outputs: Vec<_> = files(&cache).difference(&files_before).cloned().collect();
    assert_eq!(
        outputs.len(),
        3,
        "Retrieval report, agent report and trace only"
    );
    for path in &outputs {
        // Any missing component makes recovery incomplete. It must not be filled.
        let saved = fs::read(path).unwrap();
        fs::remove_file(path).unwrap();
        assert_injected(execute(), "Final outcome is unknown");
        assert!(!path.exists());
        assert_eq!(fs::read(&native_path).unwrap(), native_after);
        fs::write(path, &saved).unwrap();
    }
    let retrieval = outputs
        .iter()
        .find(|path| {
            path.components()
                .any(|part| part.as_os_str() == "retrieval")
        })
        .unwrap();
    let original_retrieval = fs::read(retrieval).unwrap();
    fs::write(retrieval, b"{broken").unwrap();
    assert!(!execute().status.success());
    assert_eq!(fs::read(&native_path).unwrap(), native_after);
    fs::write(retrieval, &original_retrieval).unwrap();
    drop_trigger(&database, "fail_final_receipt").await;
    let recovered = execute();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let recovered: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(recovered["state"], "completed");
    let receipt: AgentFinalReceipt =
        serde_json::from_value(recovered["execution"]["result"].clone()).unwrap();
    let source = scientific
        .iter()
        .find(|source| source.iteration_id.to_string() == grant.scope.iteration.id)
        .unwrap();
    let training = optimization_iteration_execution::training(folder, run_id, source.iteration_id)
        .await
        .unwrap()
        .unwrap();
    let development = project_workspace_core::optimization_iteration_execution::IterationDevelopmentResult::from_journal(&training, &source.project, &source.protocol, &source.events).unwrap();
    assert_domain(grant, &dispatch, source, development.output.model);
    assert_eq!(receipt.report.id, dispatch.report_id.to_string());
    assert_eq!(receipt.assessment.id, dispatch.assessment_id.to_string());
    assert_eq!(
        recovered["execution"]["dispatch"],
        serde_json::to_value(&dispatch).unwrap()
    );
    for operation in ["finalize-agent", "final-agent-result"] {
        let output = if operation == "finalize-agent" {
            execute()
        } else {
            command(root, run_id, operation, &[])
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            recovered
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("99999.125"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_DISCLOSE_HOLDOUT"));
    }
    assert_eq!(fs::read(&native_path).unwrap(), native_after);
    let trace = outputs
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .unwrap();
    let saved_trace = fs::read(trace).unwrap();
    fs::remove_file(trace).unwrap();
    assert_injected(execute(), "Completed final native evidence is missing");
    assert!(
        !command(root, run_id, "final-agent-result", &[])
            .status
            .success()
    );
    assert!(!trace.exists());
    assert_eq!(fs::read(&native_path).unwrap(), native_after);
    fs::write(trace, saved_trace).unwrap();
    // Cached metrics cannot be substituted after their identities were pinned.
    let mut changed: Value = serde_json::from_slice(&original_retrieval).unwrap();
    for suite in changed["inputs"].as_object_mut().unwrap().values_mut() {
        suite["metrics"]["mrr"] = 0.1.into();
    }
    fs::write(retrieval, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_injected(execute(), "differs from its completed native evidence");
    assert!(
        !command(root, run_id, "final-agent-result", &[])
            .status
            .success()
    );
    assert_eq!(fs::read(&native_path).unwrap(), native_after);
    fs::write(retrieval, original_retrieval).unwrap();
    let mut db = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    for sql in [
        "UPDATE optimization_agent_final_dispatches SET fingerprint='changed'",
        "DELETE FROM optimization_agent_final_dispatches",
        "UPDATE optimization_agent_final_results SET fingerprint='changed'",
        "DELETE FROM optimization_agent_final_results",
    ] {
        assert!(sqlx::query(sql).execute(&mut db).await.is_err());
    }
    let saved: String =
        sqlx::query_scalar("SELECT metadata_json FROM optimization_agent_final_results")
            .fetch_one(&mut db)
            .await
            .unwrap();
    assert!(
        !saved.contains("metrics")
            && !saved.contains("99999.125")
            && !saved.contains("NEVER_DISCLOSE_HOLDOUT")
    );
    // Even a rehashed project verdict is checked against native scientific facts.
    sqlx::query("DROP TRIGGER immutable_agent_final_result_update")
        .execute(&mut db)
        .await
        .unwrap();
    let mut forged = serde_json::to_value(&receipt).unwrap();
    forged["accepted"] = (!receipt.accepted).into();
    forged.as_object_mut().unwrap().remove("fingerprint");
    forged["fingerprint"] = artifact_core::fingerprint(&forged).unwrap().into();
    sqlx::query("UPDATE optimization_agent_final_results SET fingerprint=?,metadata_json=?")
        .bind(forged["fingerprint"].as_str().unwrap())
        .bind(serde_json::to_string(&forged).unwrap())
        .execute(&mut db)
        .await
        .unwrap();
    db.close().await.unwrap();
    assert!(
        !command(root, run_id, "final-agent-result", &[])
            .status
            .success()
    );
    let mut db = SqliteConnection::connect(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE optimization_agent_final_results SET fingerprint=?,metadata_json=?")
        .bind(&receipt.fingerprint)
        .bind(saved)
        .execute(&mut db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER immutable_agent_final_result_update BEFORE UPDATE ON optimization_agent_final_results BEGIN SELECT RAISE(ABORT,'Final result is immutable'); END").execute(&mut db).await.unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .is_empty()
    );
    db.close().await.unwrap();
    assert_eq!(fs::read(&scientific_database).unwrap(), scientific_before);
    assert_eq!(fs::read(root.join("agent-calls.jsonl")).unwrap(), calls);
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
}

fn assert_domain(
    grant: &AgentFinalAuthorization,
    dispatch: &project_workspace_core::optimization_final_execution::AgentFinalDispatch,
    source: &optimization_completions::IterationScientificEvidence,
    model: encoder_experiment_core::domain::ModelArtifactIdentity,
) {
    use encoder_experiment_core::{
        domain::EvidenceRole,
        metrics::{CandidateVerdict, EvaluationReport},
    };
    use project_workspace_core::optimization_final_execution::AgentFinalResult;
    let now = chrono::Utc::now();
    let baseline = &source.protocol.baseline_sealed_report;
    let report = EvaluationReport::create(
        &source.project,
        model,
        EvidenceRole::SealedAcceptance,
        baseline.suite_key.clone(),
        baseline.suite_fingerprint.clone(),
        &source.protocol.metric_contract,
        baseline.metrics.clone(),
        baseline.support,
        now,
    )
    .unwrap();
    let create = |report| {
        AgentFinalResult::create(
            grant,
            dispatch,
            &source.project,
            &source.protocol,
            report,
            now,
        )
    };
    let result = create(report.clone()).unwrap();
    let receipt = AgentFinalReceipt::from_result(&result).unwrap();
    let mut normalized_again = report.clone();
    normalized_again.id = Uuid::new_v4();
    normalized_again.created_at = now + chrono::Duration::seconds(1);
    normalized_again.fingerprint = normalized_again.reproduce_fingerprint().unwrap();
    assert_eq!(
        receipt
            .recover(
                grant,
                dispatch,
                &source.project,
                &source.protocol,
                normalized_again
            )
            .unwrap(),
        result
    );
    for change in 0..4 {
        let mut changed = report.clone();
        match change {
            0 => changed.model.id = Uuid::new_v4(),
            1 => changed.created_at = dispatch.created_at - chrono::Duration::seconds(1),
            2 => changed.evidence_role = EvidenceRole::Development,
            _ => changed.suite_key = "development".into(),
        }
        changed.fingerprint = changed.reproduce_fingerprint().unwrap();
        assert!(create(changed).is_err());
    }
    let mut changed = result;
    changed.assessment.verdict = match changed.assessment.verdict {
        CandidateVerdict::Passed => CandidateVerdict::Failed,
        CandidateVerdict::Failed => CandidateVerdict::Passed,
    };
    changed.assessment.fingerprint = changed.assessment.reproduce_fingerprint().unwrap();
    assert!(
        changed
            .validate(grant, dispatch, &source.project, &source.protocol)
            .is_err()
    );
}
