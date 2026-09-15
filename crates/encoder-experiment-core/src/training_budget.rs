//! Cumulative native-training time. Pending attempts consume their full grant.
use crate::{
    domain::TrainingCandidate,
    ports::{BoxFuture, EncoderTaskAdapterError},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingTimeLimits {
    pub iteration_millis: u64,
    pub run_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingPermit {
    pub id: Uuid,
    pub maximum_millis: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingAttemptOutcome {
    Completed,
    Interrupted,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingAttempt {
    pub iteration_id: Uuid,
    pub permit: TrainingPermit,
    pub completion: Option<TrainingAttemptCompletion>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingAttemptCompletion {
    pub outcome: TrainingAttemptOutcome,
    pub elapsed_millis: u64,
}

impl TrainingAttempt {
    pub fn charge_millis(&self) -> u64 {
        self.completion
            .as_ref()
            .map_or(self.permit.maximum_millis, |c| c.elapsed_millis)
    }

    pub fn finish(
        &mut self,
        outcome: TrainingAttemptOutcome,
        elapsed_millis: u64,
    ) -> Result<(), EncoderTaskAdapterError> {
        let completion = TrainingAttemptCompletion {
            outcome,
            elapsed_millis,
        };
        if self
            .completion
            .as_ref()
            .is_some_and(|old| *old != completion)
            || elapsed_millis == 0
        {
            return Err(invalid(
                "Training completion changed or has zero elapsed time",
            ));
        }
        // Cleanup may exceed the granted deadline. Keep the actual charge.
        self.completion = Some(completion);
        Ok(())
    }
}

impl TrainingTimeLimits {
    pub fn remaining(
        &self,
        attempts: &[TrainingAttempt],
        iteration: Uuid,
    ) -> Result<u64, EncoderTaskAdapterError> {
        if self.iteration_millis == 0 || self.run_millis == 0 || iteration.is_nil() {
            return Err(invalid("Invalid training time authority"));
        }
        let mut ids = BTreeSet::new();
        let mut per_iteration = BTreeMap::<Uuid, u64>::new();
        let mut total = 0_u64;
        for attempt in attempts {
            let charged = per_iteration.entry(attempt.iteration_id).or_default();
            let available = self
                .iteration_millis
                .saturating_sub(*charged)
                .min(self.run_millis.saturating_sub(total));
            if attempt.iteration_id.is_nil()
                || attempt.permit.id.is_nil()
                || !ids.insert(attempt.permit.id)
                || attempt.permit.maximum_millis == 0
                || attempt.permit.maximum_millis > available
                || attempt
                    .completion
                    .as_ref()
                    .is_some_and(|c| c.elapsed_millis == 0)
            {
                return Err(invalid("Training attempt identity or grant is invalid"));
            }
            *charged = charged
                .checked_add(attempt.charge_millis())
                .ok_or_else(|| invalid("Training time overflow"))?;
            total = total
                .checked_add(attempt.charge_millis())
                .ok_or_else(|| invalid("Training time overflow"))?;
        }
        Ok(self
            .iteration_millis
            .saturating_sub(*per_iteration.get(&iteration).unwrap_or(&0))
            .min(self.run_millis.saturating_sub(total)))
    }

    pub fn reserve(
        &self,
        attempts: &[TrainingAttempt],
        iteration: Uuid,
    ) -> Result<TrainingAttempt, EncoderTaskAdapterError> {
        let maximum_millis = self.remaining(attempts, iteration)?;
        if maximum_millis == 0 {
            return Err(EncoderTaskAdapterError::TrainingBudgetExhausted);
        }
        Ok(TrainingAttempt {
            iteration_id: iteration,
            permit: TrainingPermit {
                id: Uuid::new_v4(),
                maximum_millis,
            },
            completion: None,
        })
    }
}

/// Attached by the composition root. Reserve only for fresh native dispatch,
/// never for artifact replay, and settle only after owned work has terminated.
pub trait TrainingAccounting: std::fmt::Debug + Send + Sync {
    fn reserve(
        &self,
        candidate: &TrainingCandidate,
    ) -> BoxFuture<'_, Result<TrainingPermit, EncoderTaskAdapterError>>;
    fn finish(
        &self,
        permit: TrainingPermit,
        outcome: TrainingAttemptOutcome,
        elapsed_millis: u64,
    ) -> BoxFuture<'_, Result<(), EncoderTaskAdapterError>>;
}

fn invalid(message: &str) -> EncoderTaskAdapterError {
    EncoderTaskAdapterError::Failure(message.into())
}

#[cfg(test)]
mod tests;
