use super::*;
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
    let id = run_id.to_string();
    let canonical_folder = folder.canonicalize().unwrap();
    let queued_stop = Uuid::new_v4().to_string();
    let queued_args = [
        "optimization-run",
        canonical_folder.to_str().unwrap(),
        "stop-agent",
        &id,
        "--request-id",
        &queued_stop,
    ];
    let queued = run(root, &queued_args);
    assert_eq!(queued["state"], "agent_paused");
    assert_eq!(queued["agentExecution"]["attempts"], 0);
    assert_eq!(run(root, &queued_args), queued);
    // A Resume prepared from this view must not clear a later Stop, even
    // though the run was already paused when that new command arrived.
    let newer = run(root, &["optimization-run", "project", "stop-agent", &id]);
    assert_ne!(
        newer["agentExecution"]["headFingerprint"],
        queued["agentExecution"]["headFingerprint"]
    );
    let stale = command()
        .args([
            "--resume",
            queued["agentExecution"]["headFingerprint"]
                .as_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("Resume state changed"));
    assert_eq!(
        run(root, &queued_args),
        newer,
        "retrying an older Stop must not append a new intent"
    );
    let rejected = command().output().unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--resume"));
    assert!(!calls.exists());

    let ready = root.join("agent-held");
    let output_path = root.join("held-output.json");
    let errors_path = root.join("held-errors.txt");
    let mut child = command()
        .arg("--resume")
        .arg(newer["agentExecution"]["headFingerprint"].as_str().unwrap())
        .env("AGENT_FIXTURE_HOLD", &ready)
        .stdout(Stdio::from(fs::File::create(&output_path).unwrap()))
        .stderr(Stdio::from(fs::File::create(&errors_path).unwrap()))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "worker exited {status}: {}",
                fs::read_to_string(&errors_path).unwrap()
            );
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Agent never reached held second call");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let live = run(
        root,
        &["optimization-run", "project", "reconcile-agent", &id],
    );
    assert_eq!(live["state"], "agent_running");
    assert_eq!(
        run(root, &queued_args)["state"],
        "agent_running",
        "an old Stop retry must not stop the resumed attempt"
    );
    assert!(!command().output().unwrap().status.success());
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 2);
    let stopped = run(root, &["optimization-run", "project", "stop-agent", &id]);
    assert!(matches!(
        stopped["state"].as_str(),
        Some("agent_stopping" | "agent_paused")
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Stop did not interrupt the active Agent call");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(&errors_path).unwrap()
    );
    let output: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(output["state"], "paused");
    let paused = run(root, &["optimization-run", "project", "show", &id]);
    assert_eq!(paused["state"], "agent_paused");
    assert_eq!(paused["agentExecution"]["attempts"], 1);
    assert!(!command().output().unwrap().status.success());
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 2);

    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    let before: Vec<String> = sqlx::query_scalar("SELECT o.metadata_json FROM optimization_agent_outcomes o JOIN optimization_agent_calls c ON c.id=o.call_id ORDER BY c.sequence")
        .fetch_all(&mut db).await.unwrap();
    assert_eq!(before.len(), 2);
    let interrupted: Value = serde_json::from_str(&before[1]).unwrap();
    assert_eq!(interrupted["interrupted"], true);
    assert!(interrupted["usage"]["inputTokens"].is_null());
    assert!(interrupted["call"]["inputTokenCeiling"].as_u64().unwrap() > 0);
    db.close().await.unwrap();

    let stale = command()
        .args([
            "--resume",
            newer["agentExecution"]["headFingerprint"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("Resume state changed"));
    let resumed = command()
        .args([
            "--resume",
            paused["agentExecution"]["headFingerprint"]
                .as_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let completed = run(root, &["optimization-run", "project", "show", &id]);
    assert_eq!(completed["state"], "agent_completed");
    assert_eq!(completed["run"]["id"], run_id.to_string());
    assert_eq!(completed["agentExecution"]["attempts"], 2);
    assert_eq!(
        fs::read_to_string(calls).unwrap().lines().count(),
        4,
        "completed inspection was repeated"
    );
    let replay = command().output().unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(resumed.stdout, replay.stdout);
    assert_eq!(
        run(root, &["optimization-run", "project", "show", &id]),
        completed
    );
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 4);
    let mut db = SqliteConnection::connect(&format!(
        "sqlite://{}",
        folder.join("project.sqlite").display()
    ))
    .await
    .unwrap();
    let after: Vec<String> = sqlx::query_scalar("SELECT o.metadata_json FROM optimization_agent_outcomes o JOIN optimization_agent_calls c ON c.id=o.call_id ORDER BY c.sequence")
        .fetch_all(&mut db).await.unwrap();
    assert_eq!(&after[..2], &before);
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut db)
        .await
        .unwrap();
    assert!(violations.is_empty());
    db.close().await.unwrap();
}
