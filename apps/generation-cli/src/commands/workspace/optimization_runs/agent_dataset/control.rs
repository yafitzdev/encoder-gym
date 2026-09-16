//! Durable Stop, explicit lease-based reconciliation and a scoped cancellation
//! probe for the existing coordinator. No second execution loop or worker.
use crate::commands::encoder_optimize::OptimizationExecutionLease;
use anyhow::Result;
use project_workspace_local::optimization_execution;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

pub(in crate::commands::workspace::optimization_runs) async fn stop(
    folder: &Path,
    run_id: Uuid,
    request_id: Uuid,
) -> Result<()> {
    optimization_execution::request_stop(folder, run_id, request_id).await?;
    reconcile(folder, run_id).await
}

pub(in crate::commands::workspace::optimization_runs) async fn reconcile(
    folder: &Path,
    run_id: Uuid,
) -> Result<()> {
    let database = super::super::super::sqlite_file_url(&folder.join("project.sqlite"));
    if let Some(_lease) = OptimizationExecutionLease::try_acquire(&database, run_id).await? {
        optimization_execution::reconcile(folder, run_id).await?;
    }
    super::super::super::print(
        &project_workspace_local::optimization_runs::show(folder, run_id).await?,
    )
}

pub(super) struct StopWatcher {
    pub signal: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl StopWatcher {
    pub fn start(folder: &Path, run: Uuid) -> Self {
        let signal = Arc::new(AtomicBool::new(false));
        let observed = signal.clone();
        let folder = folder.to_owned();
        let task = tokio::spawn(async move {
            loop {
                match optimization_execution::stopped(&folder, run).await {
                    Ok(false) => {}
                    Ok(true) => {
                        observed.store(true, Ordering::Release);
                        return Ok(());
                    }
                    Err(error) => {
                        observed.store(true, Ordering::Release);
                        return Err(error);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        Self { signal, task }
    }

    pub async fn finish(&mut self) -> Result<()> {
        self.task.abort();
        match (&mut self.task).await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for StopWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}
