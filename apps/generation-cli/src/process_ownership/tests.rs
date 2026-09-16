//! Actual process-death tests. No lease observer or persisted child record is
//! involved, so passing cannot depend on winning the old 50 ms polling race.
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use winsafe::{HPROCESS, co, guard::CloseHandleGuard, prelude::*};

fn system() -> System {
    System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()))
}

fn helper(folder: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process_ownership::tests::ownership_fixture",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(folder)
        .env("ENCODER_OWNERSHIP_FIXTURE_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            fs::File::create(folder.join(format!("{mode}.stderr"))).unwrap(),
        ));
    command
}

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < deadline, "process fixture timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

struct Tree {
    coordinator: Child,
    observed: System,
    pids: Vec<Pid>,
    // Termination always uses opened process objects, never reused numeric PIDs.
    handles: Vec<CloseHandleGuard<HPROCESS>>,
}

impl Tree {
    fn start(folder: &Path, mode: &str) -> Self {
        let mut tree = Self {
            coordinator: helper(folder, mode).spawn().unwrap(),
            observed: system(),
            pids: Vec::new(),
            handles: Vec::new(),
        };
        let mut ready = None;
        wait_for(|| {
            assert!(tree.coordinator.try_wait().unwrap().is_none());
            ready = fs::read(folder.join("ready.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Vec<u32>>(&bytes).ok());
            ready.is_some()
        });
        tree.pids = ready.unwrap().into_iter().map(Pid::from_u32).collect();
        tree.observed = system();
        for pid in &tree.pids {
            assert!(tree.observed.process(*pid).is_some());
            tree.handles.push(
                HPROCESS::OpenProcess(
                    co::PROCESS::TERMINATE | co::PROCESS::SYNCHRONIZE,
                    false,
                    pid.as_u32(),
                )
                .unwrap(),
            );
        }
        tree
    }

    fn assert_descendants_stopped(&self) {
        wait_for(|| {
            self.handles
                .iter()
                .all(|process| process.WaitForSingleObject(Some(0)).unwrap() == co::WAIT::OBJECT_0)
        });
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = self.coordinator.kill();
        let _ = self.coordinator.wait();
        for process in &self.handles {
            let _ = process.TerminateProcess(1);
        }
    }
}

#[test]
fn abrupt_death_stops_a_child_before_its_first_instruction_without_observation() {
    let folder = tempfile::tempdir().unwrap();
    let mut tree = Tree::start(folder.path(), "spawn-boundary");
    assert!(!folder.path().join("leaf-started").exists());
    tree.coordinator.kill().unwrap();
    tree.coordinator.wait().unwrap();
    tree.assert_descendants_stopped();
    assert!(!folder.path().join("leaf-started").exists());
}

#[test]
fn abrupt_death_stops_an_unobserved_reparented_grandchild() {
    let folder = tempfile::tempdir().unwrap();
    let mut tree = Tree::start(folder.path(), "reparented");
    tree.coordinator.kill().unwrap();
    tree.coordinator.wait().unwrap();
    tree.assert_descendants_stopped();
}

#[test]
fn stop_drains_the_job_while_the_coordinator_remains_alive() {
    let folder = tempfile::tempdir().unwrap();
    let mut tree = Tree::start(folder.path(), "quiesce");
    fs::write(folder.path().join("stop"), "stop").unwrap();
    wait_for(|| {
        assert!(
            tree.coordinator.try_wait().unwrap().is_none(),
            "{}",
            fs::read_to_string(folder.path().join("quiesce.stderr")).unwrap()
        );
        folder.path().join("drained").exists()
    });
    assert!(tree.coordinator.try_wait().unwrap().is_none());
    tree.assert_descendants_stopped();
}

#[test]
fn legacy_recovery_preserves_a_pid_with_different_creation_time() {
    let folder = tempfile::tempdir().unwrap();
    let mut child = helper(folder.path(), "leaf").spawn().unwrap();
    let observed = system();
    let started = observed
        .process(Pid::from_u32(child.id()))
        .unwrap()
        .start_time();
    super::terminate_recorded(child.id(), started.saturating_sub(1)).unwrap();
    assert!(child.try_wait().unwrap().is_none());
    super::terminate_recorded(child.id(), started).unwrap();
    assert!(!child.wait().unwrap().success());
}

#[test]
#[ignore = "bounded child helper, invoked by process ownership tests"]
#[allow(
    clippy::zombie_processes,
    reason = "this crash fixture intentionally abandons children; the parent test verifies OS job cleanup and owns failure cleanup"
)]
fn ownership_fixture() {
    let mode = std::env::var("ENCODER_OWNERSHIP_FIXTURE_MODE").unwrap();
    let folder = std::env::current_dir().unwrap();
    if mode == "leaf" {
        fs::write("leaf-started", "ready").unwrap();
    } else if mode == "worker" || mode == "orphan-worker" {
        let _leaf = helper(&folder, "leaf").spawn().unwrap();
        fs::write("leaf-pid", _leaf.id().to_string()).unwrap();
        if mode == "orphan-worker" {
            return;
        }
    } else {
        super::initialize().unwrap();
        // Nested acquisitions must reuse the same lifetime handle, not drop a
        // duplicate job and accidentally kill this process.
        super::initialize().unwrap();
        let pids = if mode == "spawn-boundary" {
            use std::os::windows::process::CommandExt;
            // Stop the process before any child-side handshake/registration is
            // possible. It inherits ownership at CreateProcess itself.
            let child = helper(&folder, "leaf")
                .creation_flags(0x0000_0004) // CREATE_SUSPENDED
                .spawn()
                .unwrap();
            vec![child.id()]
        } else {
            let mut worker = helper(
                &folder,
                if mode == "reparented" {
                    "orphan-worker"
                } else {
                    "worker"
                },
            )
            .spawn()
            .unwrap();
            wait_for(|| folder.join("leaf-started").exists());
            let leaf = fs::read_to_string("leaf-pid")
                .unwrap()
                .parse::<u32>()
                .unwrap();
            if mode == "reparented" {
                assert!(worker.wait().unwrap().success());
                vec![leaf]
            } else {
                vec![worker.id(), leaf]
            }
        };
        fs::write("ready.json", serde_json::to_vec(&pids).unwrap()).unwrap();
        if mode == "quiesce" {
            wait_for(|| folder.join("stop").exists());
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(super::quiesce())
                .unwrap();
            fs::write("drained", "all stopped").unwrap();
        }
    }
    thread::sleep(Duration::from_secs(30));
}
