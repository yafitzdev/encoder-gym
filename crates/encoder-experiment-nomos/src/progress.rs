//! Optional operational telemetry. No native text, paths, or evaluation values cross this port.
use serde::{Deserialize, Serialize};
use std::{future::Future, path::Path, sync::Arc};

type FileObserver = Arc<dyn Fn(&str, u64, u64) + Send + Sync>;
tokio::task_local! { static FILE_OBSERVER: FileObserver; }
type StopProbe = Arc<dyn Fn() -> bool + Send + Sync>;
tokio::task_local! { static STOP_PROBE: StopProbe; }

pub async fn with_stop_probe<T>(probe: StopProbe, work: impl Future<Output = T>) -> T {
    STOP_PROBE.scope(probe, work).await
}

pub(crate) fn stopped() -> bool {
    STOP_PROBE.try_with(|probe| probe()).unwrap_or(false)
}

pub(crate) fn check_stop() -> Result<(), crate::EncoderTaskAdapterError> {
    if stopped() {
        return Err(crate::adapter_error("Optimization stopped"));
    }
    Ok(())
}

pub(crate) async fn wait_for_stop() {
    loop {
        if stopped() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Do not return a stoppable native operation while its owned child could still
/// be executing. A failed OS kill/wait is not a stopped acknowledgement: retain
/// the live coordinator/lease and retry cleanup, without dispatching more work.
pub(crate) async fn terminate_child(child: &mut tokio::process::Child) {
    loop {
        if child.kill().await.is_ok() || child.wait().await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

pub async fn with_file_progress<T>(observer: FileObserver, work: impl Future<Output = T>) -> T {
    FILE_OBSERVER.scope(observer, work).await
}

pub(crate) fn file_progress(
    path: &Path,
    completed: u64,
    total: u64,
) -> Result<(), crate::EncoderTaskAdapterError> {
    check_stop()?;
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let _ = FILE_OBSERVER.try_with(|observer| observer(name, completed, total));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePhase {
    CheckingFiles,
    CheckingTrainingData,
    LoadingModel,
    PreparingBatches,
    Training,
    SavingCheckpoint,
    EvaluatingRetrieval,
    EvaluatingAgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProgress {
    pub phase: NativePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

pub trait ProgressObserver: std::fmt::Debug + Send + Sync {
    fn observe(&self, progress: NativeProgress);
}

impl NativeProgress {
    pub fn phase(phase: NativePhase) -> Self {
        Self {
            phase,
            subject: None,
            completed: None,
            total: None,
        }
    }
}

// Only numerical counters from the pinned trainer's tqdm bars. Never forward raw output.
pub(crate) fn training_counter(line: &str) -> Option<NativeProgress> {
    let line = line.trim();
    let (phase, bar) = if let Some(bar) = line.strip_prefix("Batches:") {
        (NativePhase::PreparingBatches, bar.trim())
    } else {
        (NativePhase::Training, line)
    };
    let (percent, rest) = bar.split_once('%')?;
    if percent.trim().parse::<u8>().ok()? > 100 {
        return None;
    }
    let count = rest.rsplit('|').next()?.split_whitespace().next()?;
    let (completed, total) = count.split_once('/')?;
    let completed = completed.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    if total == 0 || completed > total || total > 1_000_000_000 {
        return None;
    }
    Some(NativeProgress {
        phase,
        subject: None,
        completed: Some(completed),
        total: Some(total),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "child-process helper; invoked only by the bounded Stop test"]
    fn native_stop_child() {
        std::fs::write("child-started", "ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
        panic!("native child was not terminated");
    }

    #[tokio::test]
    async fn native_stop_terminates_the_child_before_acknowledging_interruption() {
        use crate::*;
        use std::sync::atomic::{AtomicBool, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let identity =
            BackendIdentity::new("fixture", "v1", format!("sha256:{}", "a".repeat(64))).unwrap();
        let backend = NomosBackend {
            root: temp.path().to_owned(),
            python: std::env::current_exe().unwrap(),
            manifest: crate::tests::manifest_with_named_suites(),
            baseline_override: None,
            training_override: None,
            identity: identity.clone(),
            observer_identity: identity.clone(),
            repair_delta_identity: identity,
            progress: None,
            training_accounting: None,
        };
        let signal = Arc::new(AtomicBool::new(false));
        let observed = signal.clone();
        let ready = temp.path().join("child-started");
        let stop = async {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "native child did not start"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            signal.store(true, Ordering::Release);
        };
        let arguments = [
            "--exact",
            "progress::tests::native_stop_child",
            "--ignored",
            "--nocapture",
        ]
        .map(String::from);
        let work = with_stop_probe(
            Arc::new(move || observed.load(Ordering::Acquire)),
            backend.run_bounded(&arguments, 10),
        );
        let (result, ()) = tokio::join!(work, stop);
        assert_eq!(
            result.unwrap_err(),
            EncoderTaskAdapterError::Failure("Optimization stopped".into())
        );
    }
    #[tokio::test]
    async fn stop_interrupts_native_hashing_and_is_task_scoped() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("model.bin");
        std::fs::write(&file, vec![3; 256 * 1024]).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let result = with_stop_probe(
            Arc::new(move || counted.fetch_add(1, Ordering::SeqCst) >= 2),
            async { crate::sha256_file(&file) },
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("stopped"));
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        assert!(!stopped());
        assert!(crate::sha256_file(&file).is_ok());
    }
    #[test]
    fn only_normalized_training_counters_leave_the_adapter() {
        assert_eq!(
            training_counter("Batches:  25%|██ | 2/8 [00:01<00:03]"),
            Some(NativeProgress {
                phase: NativePhase::PreparingBatches,
                subject: None,
                completed: Some(2),
                total: Some(8)
            })
        );
        assert_eq!(
            training_counter(" 50%|#### | 50/100 [00:10<00:10]"),
            Some(NativeProgress {
                phase: NativePhase::Training,
                subject: None,
                completed: Some(50),
                total: Some(100)
            })
        );
        for line in [
            "Bearer sk-private-secret",
            "score 50%| | 2/4",
            "120%| | 2/4",
            "50%| | 5/4",
            "0%| | 0/0",
            "{\"sealed_score\":0.95}",
        ] {
            assert!(training_counter(line).is_none());
        }
    }
}
