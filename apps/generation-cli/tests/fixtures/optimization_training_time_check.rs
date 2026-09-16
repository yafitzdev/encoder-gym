use super::*;
use encoder_experiment_core::training_budget::{TrainingAttempt, TrainingAttemptOutcome};
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

#[cfg(windows)]
pub(super) async fn crash(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    calls: &Path,
    command: impl Fn() -> Command,
) {
    use winsafe::{HPROCESS, co, prelude::*};
    let ready = root.join("crash-held-trainer");
    let errors = root.join("crash-errors.txt");
    let mut coordinator = command()
        .env("ENCODER_FIXTURE_TRAINING_HOLD", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&errors).unwrap()))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let pids = loop {
        if let Ok(bytes) = fs::read(&ready) {
            if let Ok(pids) = serde_json::from_slice::<Vec<u32>>(&bytes) {
                break pids;
            }
        }
        if Instant::now() >= deadline || coordinator.try_wait().unwrap().is_some() {
            let _ = coordinator.kill();
            let _ = coordinator.wait();
            panic!(
                "Trainer did not reach crash boundary: {}",
                fs::read_to_string(&errors).unwrap()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(pids.len(), 2);
    let descendants = pids
        .iter()
        .map(|pid| {
            HPROCESS::OpenProcess(
                co::PROCESS::TERMINATE | co::PROCESS::SYNCHRONIZE,
                false,
                *pid,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        descendants
            .iter()
            .all(|process| process.WaitForSingleObject(Some(0)).unwrap() == co::WAIT::TIMEOUT)
    );
    coordinator.kill().unwrap();
    coordinator.wait().unwrap();
    // No successor/observer/reconciliation command is allowed to help cleanup.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if descendants
            .iter()
            .all(|process| process.WaitForSingleObject(Some(0)).unwrap() == co::WAIT::OBJECT_0)
        {
            break;
        }
        if Instant::now() >= deadline {
            for process in &descendants {
                let _ = process.TerminateProcess(1);
            }
            panic!("Native descendants survived coordinator death");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let history = project_workspace_local::optimization_training_time::history(folder, run_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].completion.is_none());
    assert_eq!(history[0].charge_millis(), 120_000);
    let native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let agent = fs::read(calls).unwrap();
    let id = run_id.to_string();
    let reconciled = run(
        root,
        &["optimization-run", "project", "reconcile-agent", &id],
    );
    assert_eq!(reconciled["state"], "agent_interrupted");
    let resumed = command().output().unwrap();
    assert!(!resumed.status.success());
    let exhausted = run(root, &["optimization-run", "project", "show", &id]);
    assert_eq!(exhausted["state"], "agent_budget_exhausted");
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
    assert_eq!(fs::read(calls).unwrap(), agent);
    assert_eq!(
        project_workspace_local::optimization_training_time::history(folder, run_id)
            .await
            .unwrap(),
        history
    );
}

pub(super) async fn timeout(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    calls: &Path,
    command: impl Fn() -> Command,
) {
    let failed = command()
        .env(
            "ENCODER_FIXTURE_TRAINING_HOLD",
            root.join("timed-out-trainer"),
        )
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("finite time limit"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    let before = project_workspace_local::optimization_training_time::history(folder, run_id)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(
        before[0].completion.as_ref().unwrap().outcome,
        TrainingAttemptOutcome::TimedOut
    );
    assert!(before[0].charge_millis() >= 1_000);
    let stopped = run(
        root,
        &["optimization-run", "project", "show", &run_id.to_string()],
    );
    assert_eq!(stopped["state"], "agent_budget_exhausted");
    assert_eq!(stopped["agentExecution"]["state"], "budget_exhausted");
    assert_eq!(stopped["agentExecution"]["attempts"], 1);
    let native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let exhausted = command().output().unwrap();
    assert!(!exhausted.status.success());
    assert!(
        String::from_utf8_lossy(&exhausted.stderr).contains("Training time budget exhausted"),
        "{}",
        String::from_utf8_lossy(&exhausted.stderr)
    );
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 3);
    assert_eq!(
        project_workspace_local::optimization_training_time::history(folder, run_id)
            .await
            .unwrap(),
        before
    );
}

pub(super) async fn exercise(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    calls: &Path,
    command: impl Fn() -> Command,
    stop: bool,
) {
    let id = run_id.to_string();
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    let mut resume = None;
    if stop {
        let ready = root.join("training-held");
        let errors = root.join("training-errors.txt");
        let mut child = command()
            .env("ENCODER_FIXTURE_TRAINING_HOLD", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::from(fs::File::create(&errors).unwrap()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(45);
        while !ready.exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "Trainer exited {status}: {}",
                    fs::read_to_string(&errors).unwrap()
                );
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Trainer did not start");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pending: Vec<TrainingAttempt> = serde_json::from_value(run(
            root,
            &["optimization-run", "project", "training-time", &id],
        ))
        .unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].completion.is_none());
        assert_eq!(pending[0].charge_millis(), 120_000);
        run(root, &["optimization-run", "project", "stop-agent", &id]);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "{}", fs::read_to_string(&errors).unwrap());
                break;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Trainer did not stop");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let paused = run(root, &["optimization-run", "project", "show", &id]);
        assert_eq!(paused["state"], "agent_paused");
        resume = Some(
            paused["agentExecution"]["headFingerprint"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    } else {
        sqlx::query("CREATE TRIGGER lose_training_settlement BEFORE INSERT ON optimization_training_completions BEGIN SELECT RAISE(ABORT, 'injected training settlement failure'); END").execute(&mut db).await.unwrap();
        let output = command().output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("injected training settlement failure"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        sqlx::query("DROP TRIGGER lose_training_settlement")
            .execute(&mut db)
            .await
            .unwrap();
    }
    let before = project_workspace_local::optimization_training_time::history(folder, run_id)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 3);
    if stop {
        assert_eq!(
            before[0].completion.as_ref().unwrap().outcome,
            TrainingAttemptOutcome::Interrupted
        );
        assert!((1..120_000).contains(&before[0].charge_millis()));
    } else {
        assert!(before[0].completion.is_none());
    }
    let mut restart = command();
    if let Some(head) = resume {
        restart.args(["--resume", &head]);
    }
    let output = restart.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = project_workspace_local::optimization_training_time::history(folder, run_id)
        .await
        .unwrap();
    assert_eq!(after[0], before[0], "Historical charges must not change");
    assert_eq!(after.len(), if stop { 2 } else { 1 });
    if stop {
        assert_eq!(
            after[1].permit.maximum_millis,
            120_000 - before[0].charge_millis()
        );
        assert_eq!(
            after[1].completion.as_ref().unwrap().outcome,
            TrainingAttemptOutcome::Completed
        );
    } else {
        assert_eq!(
            after[0].charge_millis(),
            120_000,
            "Lost settlement retains the full unknown charge even after output reuse"
        );
    }
    let native = fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap();
    assert_eq!(
        native
            .lines()
            .filter(|line| *line == "tools.train_dense_triplet_router")
            .count(),
        if stop { 2 } else { 1 }
    );
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 3);
    let completed = run(root, &["optimization-run", "project", "show", &id]);
    assert_eq!(completed["state"], "agent_completed");
    assert!(command().output().unwrap().status.success());
    assert_eq!(
        project_workspace_local::optimization_training_time::history(folder, run_id)
            .await
            .unwrap(),
        after
    );
    assert_eq!(
        fs::read_to_string(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
    for sql in [
        "UPDATE optimization_training_attempts SET fingerprint='changed'",
        "DELETE FROM optimization_training_attempts",
    ] {
        assert!(sqlx::query(sql).execute(&mut db).await.is_err());
    }
    if stop {
        for sql in [
            "UPDATE optimization_training_completions SET fingerprint='changed'",
            "DELETE FROM optimization_training_completions",
        ] {
            assert!(sqlx::query(sql).execute(&mut db).await.is_err());
        }
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .is_empty()
    );
    db.close().await.unwrap();
}
