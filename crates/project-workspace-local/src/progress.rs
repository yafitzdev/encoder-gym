//! Optional file-verification telemetry, scoped to one caller's async task.
//! Only a basename and byte counts leave this adapter; never file contents.
use std::{future::Future, path::Path, sync::Arc};

type Observer = Arc<dyn Fn(&str, u64, u64) + Send + Sync>;
tokio::task_local! { static OBSERVER: Observer; }
tokio::task_local! { static ROW_OBSERVER: Observer; }

pub async fn with_file_progress<T>(observer: Observer, work: impl Future<Output = T>) -> T {
    OBSERVER.scope(observer, work).await
}

pub async fn with_row_progress<T>(observer: Observer, work: impl Future<Output = T>) -> T {
    ROW_OBSERVER.scope(observer, work).await
}

pub(crate) fn row_progress(name: &str, completed: u64, total: u64) {
    let _ = ROW_OBSERVER.try_with(|observer| observer(name, completed, total));
}

pub(crate) fn file_progress(path: &Path, completed: u64, total: u64) {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let _ = OBSERVER.try_with(|observer| observer(name, completed, total));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
