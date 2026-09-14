use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use encoder_optimization_core::{
    OptimizationError, fingerprint,
    generation::{
        AdmittedGenerationRow, GenerationAdmission, GenerationOutcome, GenerationReservation,
        GenerationTask,
    },
    ports::{BoxFuture, OptimizationGenerationAdmission, OptimizationGenerationStore},
};
use encoder_optimization_runner::generation::OptimizationGenerator;
use generation_core::{
    domain::UsageMetadata,
    ports::{BoxFuture as ProviderFuture, GenerationBackendError},
    structured::{
        StructuredGenerationBackend, StructuredGenerationRequest, StructuredGenerationResult,
    },
};
use serde_json::json;
use uuid::Uuid;

#[derive(Default)]
struct Store {
    pending: Mutex<BTreeMap<Uuid, GenerationReservation>>,
    history: Mutex<Vec<GenerationOutcome>>,
    stopped: AtomicBool,
    reservations: AtomicUsize,
    budget: usize,
}

impl OptimizationGenerationStore for Store {
    fn history(&self, task: GenerationTask) -> BoxFuture<'_, Vec<GenerationOutcome>> {
        Box::pin(async move {
            Ok(self
                .history
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v.reservation.task_id == task.id)
                .cloned()
                .collect())
        })
    }
    fn reserve(&self, task: GenerationTask, call: GenerationReservation) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if self.reservations.fetch_add(1, Ordering::SeqCst) >= self.budget {
                return Err(OptimizationError::Budget("test ceiling".into()));
            }
            assert_eq!(call.task_id, task.id);
            assert!(self.pending.lock().unwrap().insert(task.id, call).is_none());
            Ok(())
        })
    }
    fn finish(&self, task: GenerationTask, outcome: GenerationOutcome) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            outcome.validate(&task)?;
            assert_eq!(
                self.pending.lock().unwrap().remove(&task.id),
                Some(outcome.reservation.clone())
            );
            self.history.lock().unwrap().push(outcome);
            Ok(())
        })
    }
    fn stopped(&self, _: Uuid) -> BoxFuture<'_, bool> {
        Box::pin(async move { Ok(self.stopped.load(Ordering::SeqCst)) })
    }
}

struct Backend {
    store: Arc<Store>,
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
    failing: AtomicBool,
    slow: bool,
}

impl StructuredGenerationBackend for Backend {
    fn model(&self) -> &str {
        "selected-generator"
    }
    fn generate_structured(
        &self,
        request: StructuredGenerationRequest,
    ) -> ProviderFuture<'_, Result<StructuredGenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            assert!(
                !self.store.pending.lock().unwrap().is_empty(),
                "reservation precedes dispatch"
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _active = Active(&self.active);
            tokio::time::sleep(Duration::from_millis(if self.slow { 10000 } else { 20 })).await;
            if self.failing.swap(false, Ordering::SeqCst) {
                return Err(GenerationBackendError::Request(
                    "raw secret must not escape".into(),
                ));
            }
            Ok(StructuredGenerationResult {
                content: request.user_prompt,
                usage: Some(UsageMetadata {
                    input_tokens: Some(12),
                    output_tokens: Some(8),
                    total_tokens: Some(20),
                }),
            })
        })
    }
}

struct Admission;
impl OptimizationGenerationAdmission for Admission {
    fn admit(&self, task: GenerationTask, content: String) -> BoxFuture<'_, GenerationAdmission> {
        Box::pin(async move {
            assert_eq!(task.request.user_prompt, content);
            let row = json!({"question":content});
            Ok(GenerationAdmission {
                accepted: vec![AdmittedGenerationRow {
                    index: 0,
                    fingerprint: fingerprint(&row)?,
                    deduplication_fingerprint: fingerprint(&row)?,
                    content: row,
                }],
                rejected: vec![],
            })
        })
    }
}

fn fixture(
    concurrency: u32,
    budget: usize,
    slow: bool,
) -> (
    OptimizationGenerator,
    Arc<Store>,
    Arc<Backend>,
    Vec<GenerationTask>,
) {
    let store = Arc::new(Store {
        budget,
        ..Default::default()
    });
    let backend = Arc::new(Backend {
        store: store.clone(),
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        failing: AtomicBool::new(false),
        slow,
    });
    let generator = OptimizationGenerator {
        backend: backend.clone(),
        admission: Arc::new(Admission),
        store: store.clone(),
        concurrency,
        maximum_cost_microusd_per_call: 10,
    };
    let run = Uuid::new_v4();
    let tasks = (0..4)
        .map(|index| GenerationTask {
            id: Uuid::new_v4(),
            run_id: run,
            iteration: 1,
            proposal_fingerprint: fingerprint(&"proposal").unwrap(),
            template_row_id: "member".into(),
            template_fingerprint: fingerprint(&"template").unwrap(),
            target_index: index,
            first_row: 0,
            requested_rows: 1,
            request: StructuredGenerationRequest {
                system_prompt: "system".into(),
                user_prompt: format!("question {index}"),
                maximum_output_tokens: 64,
            },
        })
        .collect();
    (generator, store, backend, tasks)
}

#[tokio::test]
async fn concurrency_is_bounded_and_completed_rows_replay_in_plan_order() {
    for concurrency in [1, 3] {
        let (generator, store, backend, tasks) = fixture(concurrency, 20, false);
        let results = generator.generate(tasks.clone()).await.unwrap();
        assert_eq!(backend.peak.load(Ordering::SeqCst), concurrency as usize);
        assert_eq!(
            results
                .iter()
                .map(|v| v.reservation.task_id)
                .collect::<Vec<_>>(),
            tasks.iter().map(|t| t.id).collect::<Vec<_>>()
        );
        assert_eq!(results, generator.generate(tasks).await.unwrap());
        assert_eq!(backend.calls.load(Ordering::SeqCst), 4);
        assert!(store.pending.lock().unwrap().is_empty());
        assert!(results.iter().all(|v| v.usage.cost_microusd.is_none()));
    }
}

#[tokio::test]
async fn stop_interrupts_live_calls_and_resume_uses_new_attempts_not_completed_slots() {
    let (mut generator, store, backend, tasks) = fixture(2, 20, true);
    let worker = generator.clone();
    let pending = tasks.clone();
    let running = tokio::spawn(async move { worker.generate(pending).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    store.stopped.store(true, Ordering::SeqCst);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap(),
        Err(OptimizationError::Stopped)
    ));
    assert_eq!(backend.active.load(Ordering::SeqCst), 0);
    assert!(
        store
            .history
            .lock()
            .unwrap()
            .iter()
            .all(|v| v.interrupted && v.usage.input_tokens.is_none())
    );
    store.stopped.store(false, Ordering::SeqCst);
    let next = Arc::new(Backend {
        store: store.clone(),
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        failing: AtomicBool::new(false),
        slow: false,
    });
    generator.backend = next.clone();
    let results = generator.generate(tasks.clone()).await.unwrap();
    assert_eq!(results[0].reservation.attempt, 2);
    assert_eq!(results[2].reservation.attempt, 1);
    generator.generate(tasks).await.unwrap();
    assert_eq!(next.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn failed_calls_and_budget_exhaustion_stop_further_dispatch() {
    let (generator, store, backend, tasks) = fixture(1, 2, false);
    backend.failing.store(true, Ordering::SeqCst);
    let error = generator.generate(tasks.clone()).await.unwrap_err();
    assert!(!error.to_string().contains("raw secret"));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    assert!(store.history.lock().unwrap()[0].interrupted);
    assert!(matches!(
        generator.generate(tasks).await,
        Err(OptimizationError::Budget(_))
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
    assert_eq!(store.history.lock().unwrap()[1].reservation.attempt, 2);
}
