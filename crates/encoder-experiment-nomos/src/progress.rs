//! Optional operational telemetry. No native text, paths, or evaluation values cross this port.
use serde::{Deserialize, Serialize};
use std::{future::Future, path::Path, sync::Arc};

type FileObserver = Arc<dyn Fn(&str, u64, u64) + Send + Sync>;
tokio::task_local! { static FILE_OBSERVER: FileObserver; }

pub async fn with_file_progress<T>(observer: FileObserver, work: impl Future<Output = T>) -> T {
    FILE_OBSERVER.scope(observer, work).await
}

pub(crate) fn file_progress(path: &Path, completed: u64, total: u64) {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let _ = FILE_OBSERVER.try_with(|observer| observer(name, completed, total));
    }
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
