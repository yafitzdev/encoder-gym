//! Bounded generation inside one local optimization run. Finished slots are
//! replayed; only failed/unknown slots obtain a fresh, budgeted call on resume.

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use encoder_optimization_core::{
    OptimizationError,
    agent::AgentTokenUsage,
    generation::{
        GenerationCanaryPolicy, GenerationOutcome, GenerationReservation, GenerationTask,
        canary_passed,
    },
    ports::{OptimizationGenerationAdmission, OptimizationGenerationStore},
};
use generation_core::structured::StructuredGenerationBackend;
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct OptimizationGenerator {
    pub backend: Arc<dyn StructuredGenerationBackend>,
    pub admission: Arc<dyn OptimizationGenerationAdmission>,
    pub store: Arc<dyn OptimizationGenerationStore>,
    pub concurrency: u32,
    pub maximum_cost_microusd_per_call: u64,
}

impl OptimizationGenerator {
    /// Validate every task before spending on the first slot. A saved rejected
    /// canary remains rejected on resume; it does not authorize another attempt.
    pub async fn generate_with_canary(
        &self,
        mut tasks: Vec<GenerationTask>,
        policy: Option<GenerationCanaryPolicy>,
    ) -> Result<Vec<GenerationOutcome>, OptimizationError> {
        self.validate_tasks(&tasks)?;
        if policy.is_none() || tasks.is_empty() {
            return self.generate(tasks).await;
        }
        if policy == Some(GenerationCanaryPolicy::PerCombinationSemanticV3) {
            return Err(OptimizationError::Validation(
                "Protocol-V3 canaries require semantic gating before bulk dispatch".into(),
            ));
        }
        let first = tasks.remove(0);
        if first.target_index != 0 || first.first_row != 0 {
            return Err(OptimizationError::Validation(
                "Generation canary is not the first proposal slot".into(),
            ));
        }
        let outcome = self.one(first.clone()).await?;
        if !canary_passed(&first, &outcome)? {
            return Err(OptimizationError::Validation(format!(
                "Generation canary rejected {} of {} sample rows; remaining batches were not dispatched. The saved rejection will not be retried.",
                outcome
                    .admission
                    .as_ref()
                    .map_or(0, |value| value.rejected.len()),
                first.requested_rows
            )));
        }
        let mut results = vec![outcome];
        results.extend(self.generate(tasks).await?);
        Ok(results)
    }

    pub async fn generate(
        &self,
        tasks: Vec<GenerationTask>,
    ) -> Result<Vec<GenerationOutcome>, OptimizationError> {
        self.validate_tasks(&tasks)?;
        self.dispatch(tasks).await
    }

    fn validate_tasks(&self, tasks: &[GenerationTask]) -> Result<(), OptimizationError> {
        if !(1..=16).contains(&self.concurrency) || tasks.len() > 5000 {
            return Err(OptimizationError::Validation(
                "Invalid generation concurrency or task count".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for task in tasks {
            task.validate()?;
            if !ids.insert(task.id)
                || !slots.insert((task.target_index, task.first_row))
                || tasks.first().is_some_and(|first| {
                    first.run_id != task.run_id
                        || first.iteration != task.iteration
                        || first.proposal_fingerprint != task.proposal_fingerprint
                })
            {
                return Err(OptimizationError::Validation(
                    "Generation tasks must belong to one proposal without duplicate slots".into(),
                ));
            }
        }
        Ok(())
    }

    async fn dispatch(
        &self,
        tasks: Vec<GenerationTask>,
    ) -> Result<Vec<GenerationOutcome>, OptimizationError> {
        let mut pending = tasks.into_iter().enumerate();
        let mut workers = JoinSet::new();
        let mut results = Vec::new();
        let mut failure = None;
        loop {
            while failure.is_none() && workers.len() < (self.concurrency as usize) {
                let Some((index, task)) = pending.next() else {
                    break;
                };
                let worker = self.clone();
                workers.spawn(async move { (index, worker.one(task).await) });
            }
            let Some(result) = workers.join_next().await else {
                break;
            };
            match result {
                Ok((index, Ok(outcome))) => results.push((index, outcome)),
                Ok((_, Err(error))) => {
                    if failure.is_none() {
                        failure = Some(error)
                    }
                }
                Err(_) => {
                    if failure.is_none() {
                        failure = Some(OptimizationError::Adapter(
                            "Generation worker interrupted; its reservation remains charged".into(),
                        ))
                    }
                }
            }
            // Drain already dispatched work so each outcome is persisted. A
            // failure stops new dispatch; user cancellation also stops live calls.
        }
        if let Some(error) = failure {
            return Err(error);
        }
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, result)| result).collect())
    }

    async fn one(&self, task: GenerationTask) -> Result<GenerationOutcome, OptimizationError> {
        let history = self.store.history(task.clone()).await?;
        let mut attempt = 0;
        for outcome in history {
            outcome.validate(&task)?;
            if outcome.reservation.attempt != attempt + 1 {
                return Err(OptimizationError::Validation(
                    "Generation attempt history is not contiguous".into(),
                ));
            }
            attempt += 1;
            if !outcome.interrupted {
                return Ok(outcome);
            }
        }
        if self.store.stopped(task.run_id).await? {
            return Err(OptimizationError::Stopped);
        }
        let reservation = GenerationReservation {
            id: Uuid::new_v4(),
            task_id: task.id,
            task_fingerprint: task.fingerprint()?,
            attempt: attempt
                .checked_add(1)
                .ok_or_else(|| OptimizationError::Budget("Generation attempts exhausted".into()))?,
            input_token_ceiling: (task.request.system_prompt.len()
                + task.request.user_prompt.len()
                + 1024) as u64,
            output_token_ceiling: u64::from(task.request.maximum_output_tokens),
            cost_ceiling_microusd: self.maximum_cost_microusd_per_call,
        };
        self.store
            .reserve(task.clone(), reservation.clone())
            .await?;
        let mut outcome = GenerationOutcome {
            reservation,
            usage: AgentTokenUsage::default(),
            admission: None,
            interrupted: true,
        };
        let result = {
            let operation = async {
                let response = self
                    .backend
                    .generate_structured(task.request.clone())
                    .await
                    .map_err(|_| {
                        OptimizationError::Adapter(
                            "Data generation failed; the reserved attempt remains recorded".into(),
                        )
                    })?;
                if let Some(usage) = response.usage {
                    outcome.usage.input_tokens = usage.input_tokens;
                    outcome.usage.output_tokens = usage.output_tokens;
                }
                if outcome.usage.exceeds(
                    outcome.reservation.input_token_ceiling,
                    outcome.reservation.output_token_ceiling,
                    outcome.reservation.cost_ceiling_microusd,
                ) {
                    return Err(OptimizationError::Budget(
                        "Generator reported usage above its reserved ceiling".into(),
                    ));
                }
                // Custom endpoint prices are not known from a missing/zero SDK price.
                let admission = self.admission.admit(task.clone(), response.content).await?;
                admission.validate(&task)?;
                outcome.admission = Some(admission);
                outcome.interrupted = false;
                Ok::<_, OptimizationError>(())
            };
            tokio::pin!(operation);
            let stop = async {
                loop {
                    if self.store.stopped(task.run_id).await? {
                        return Err::<(), _>(OptimizationError::Stopped);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            };
            tokio::select! {
                result=&mut operation=>result,
                result=stop=>result,
                ()=tokio::time::sleep(Duration::from_secs(120))=>Err(OptimizationError::Adapter("Data generation timed out; outcome is unknown".into())),
            }
        };
        // Drop cancels the local request, not necessarily the provider's billing.
        self.store.finish(task.clone(), outcome.clone()).await?;
        result?;
        Ok(outcome)
    }
}
