//! Optional file-verification telemetry, scoped to one caller's async task.
//! Only a basename and byte counts leave this adapter; never file contents.
use std::{future::Future, path::Path, sync::Arc};

type Observer = Arc<dyn Fn(&str, u64, u64) + Send + Sync>;
tokio::task_local! { static OBSERVER: Observer; }
tokio::task_local! { static ROW_OBSERVER: Observer; }
type StopProbe = Arc<dyn Fn() -> bool + Send + Sync>;
tokio::task_local! { static STOP_PROBE: StopProbe; }

pub async fn with_stop_probe<T>(probe: StopProbe, work: impl Future<Output = T>) -> T {
    STOP_PROBE.scope(probe, work).await
}

pub(crate) fn check_stop() -> anyhow::Result<()> {
    anyhow::ensure!(
        !STOP_PROBE.try_with(|probe| probe()).unwrap_or(false),
        "Optimization stopped"
    );
    Ok(())
}

pub async fn with_file_progress<T>(observer: Observer, work: impl Future<Output = T>) -> T {
    OBSERVER.scope(observer, work).await
}

pub async fn with_row_progress<T>(observer: Observer, work: impl Future<Output = T>) -> T {
    ROW_OBSERVER.scope(observer, work).await
}

pub(crate) fn row_progress(name: &str, completed: u64, total: u64) -> anyhow::Result<()> {
    check_stop()?;
    let _ = ROW_OBSERVER.try_with(|observer| observer(name, completed, total));
    Ok(())
}

pub(crate) fn file_progress(path: &Path, completed: u64, total: u64) -> anyhow::Result<()> {
    check_stop()?;
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let _ = OBSERVER.try_with(|observer| observer(name, completed, total));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn stop_interrupts_a_hash_between_reads_without_changing_the_source() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("weights.bin");
        std::fs::write(&file, vec![7; 256 * 1024]).unwrap();
        let checks = Arc::new(AtomicUsize::new(0));
        let counted = checks.clone();
        let result = with_stop_probe(
            Arc::new(move || counted.fetch_add(1, Ordering::SeqCst) >= 2),
            async { crate::files::hash(&file, "weights.bin") },
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("stopped"));
        assert_eq!(checks.load(Ordering::SeqCst), 3);
        assert_eq!(std::fs::read(&file).unwrap(), vec![7; 256 * 1024]);
        assert!(crate::files::hash(&file, "weights.bin").is_ok());
    }

    #[tokio::test]
    async fn file_checks_report_actual_bytes_without_parent_paths_and_remain_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("weights.bin");
        std::fs::write(&file, [7; 1024]).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        with_file_progress(
            Arc::new(move |name, done, total| {
                captured
                    .lock()
                    .unwrap()
                    .push((name.to_owned(), done, total));
            }),
            async {
                crate::files::hash(&file, "weights.bin").unwrap();
            },
        )
        .await;
        crate::files::hash(&file, "weights.bin").unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ("weights.bin".into(), 0, 1024),
                ("weights.bin".into(), 1024, 1024)
            ]
        );
    }
}
