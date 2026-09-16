//! Process-lifetime containment, outside the domain and scientific journals.
//!
//! On Windows the coordinator joins an anonymous, non-inheritable,
//! kill-on-close job BEFORE it can spawn. Membership (not the handle) is
//! inherited atomically by children and their descendants. Closing the
//! coordinator's sole handle on abrupt exit therefore stops even children that
//! the durable lease observer never saw, including reparented descendants.
//!
//! The job must live until OS process teardown: dropping it earlier would kill
//! the coordinator itself. A static owns exactly one handle; initialization
//! failures stop the CLI before work starts. No breakaway limits are enabled.
//! Other platforms retain the existing recorded-child recovery; they do not
//! yet have this spawn-time crash guarantee.

#[cfg(windows)]
static JOB: std::sync::OnceLock<Result<win32job::Job, String>> = std::sync::OnceLock::new();

pub(crate) fn initialize() -> anyhow::Result<()> {
    #[cfg(windows)]
    job()?;
    Ok(())
}

#[cfg(windows)]
fn job() -> anyhow::Result<&'static win32job::Job> {
    JOB.get_or_init(|| {
        let mut limits = win32job::ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let job = win32job::Job::create_with_limit_info(&limits).map_err(|e| e.to_string())?;
        job.assign_current_process().map_err(|e| e.to_string())?;
        Ok(job)
    })
    .as_ref()
    .map_err(|error| anyhow::anyhow!("Could not establish CLI process ownership: {error}"))
}

/// Prevent a lease destructor from advertising idle while contained children
/// are still unwinding. An uninitialized/test or non-Windows process falls back
/// to the lease's recorded-child check; an ownership/query error fails closed.
pub(crate) fn has_contained_descendants() -> anyhow::Result<bool> {
    #[cfg(windows)]
    if let Some(job) = JOB.get() {
        let job = job
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        return Ok(job
            .query_process_id_list()?
            .into_iter()
            .any(|pid| pid != std::process::id() as usize));
    }
    Ok(false)
}

/// Called only after the finite coordinator has returned and can no longer
/// dispatch. Do not publish a paused/completed attempt while a sidecar or a
/// reparented native descendant remains. Failure leaves the attempt unclosed;
/// process exit still closes the lifetime job. The job is NOT terminated here,
/// because it also owns this coordinator.
pub(crate) async fn quiesce() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::time::{Duration, Instant};
        use winsafe::{HPROCESS, co, prelude::*};
        let job = JOB
            .get()
            .ok_or_else(|| anyhow::anyhow!("CLI process ownership was not initialized"))?
            .as_ref()
            .map_err(|error| anyhow::anyhow!("CLI process ownership failed: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_error = None;
        loop {
            let children = job
                .query_process_id_list()?
                .into_iter()
                .filter(|pid| *pid != std::process::id() as usize)
                .collect::<Vec<_>>();
            if children.is_empty() {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Owned subprocesses have not stopped; the execution attempt remains unclosed: {}",
                last_error
                    .as_deref()
                    .unwrap_or("termination is still pending")
            );
            for pid in children {
                let process = HPROCESS::OpenProcess(
                    co::PROCESS::TERMINATE | co::PROCESS::SYNCHRONIZE,
                    false,
                    pid.try_into()?,
                );
                // Holding the handle prevents PID reuse. Recheck membership
                // after opening it, then terminate that exact process object.
                if !job.query_process_id_list()?.contains(&pid) {
                    continue;
                }
                let process = match process {
                    Ok(process) => process,
                    Err(error) => {
                        // Windows may deny opening an already terminating
                        // process before removing it from the job. This is not
                        // evidence of either liveness or successful cleanup:
                        // retry the authoritative membership query, bounded by
                        // the same deadline as a genuinely unkillable child.
                        last_error = Some(error.to_string());
                        continue;
                    }
                };
                if let Err(error) = process.TerminateProcess(1) {
                    if process.WaitForSingleObject(Some(0))? != co::WAIT::OBJECT_0 {
                        last_error = Some(error.to_string());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    #[cfg(not(windows))]
    Ok(())
}

/// Compatibility cleanup for an older lease. Verify creation time using the
/// opened process object, then terminate through that same non-inherited
/// handle. Missing/reused PIDs are not targets; access failures stay errors.
#[cfg(windows)]
pub(crate) fn terminate_recorded(process_id: u32, started_at: u64) -> anyhow::Result<()> {
    use winsafe::{HPROCESS, co, prelude::*};
    let process = match HPROCESS::OpenProcess(
        co::PROCESS::TERMINATE | co::PROCESS::SYNCHRONIZE | co::PROCESS::QUERY_LIMITED_INFORMATION,
        false,
        process_id,
    ) {
        Ok(process) => process,
        Err(co::ERROR::INVALID_PARAMETER) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let (created, _, _, _) = process.GetProcessTimes()?;
    // FILETIME uses 100 ns ticks since 1601; lease/sysinfo time is Unix seconds.
    if (u64::from(created) / 10_000_000).checked_sub(11_644_473_600) != Some(started_at) {
        return Ok(());
    }
    if let Err(error) = process.TerminateProcess(1) {
        if process.WaitForSingleObject(Some(0))? != co::WAIT::OBJECT_0 {
            return Err(error.into());
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "process_ownership/tests.rs"]
mod tests;

/// Lease observers own a coordinator's entire process tree. Tests modeling
/// independent coordinators must not share the unit-test runner's PID and
/// accidentally observe or terminate another concurrent test's children.
#[cfg(test)]
pub(crate) fn isolate_lease_test(name: &str) -> bool {
    const KEY: &str = "ENCODER_EXECUTION_LEASE_TEST";
    if std::env::var(KEY).as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(KEY, name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
