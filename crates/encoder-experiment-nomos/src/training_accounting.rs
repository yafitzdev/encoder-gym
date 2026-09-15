use super::*;
use encoder_experiment_core::training_budget::{TrainingAccounting, TrainingAttemptOutcome};

impl NomosBackend {
    pub fn with_training_accounting(mut self, accounting: Arc<dyn TrainingAccounting>) -> Self {
        self.training_accounting = Some(accounting);
        self
    }

    pub(super) async fn run_accounted_training(
        &self,
        arguments: &[String],
        candidate: &TrainingCandidate,
    ) -> Result<(), EncoderTaskAdapterError> {
        let Some(accounting) = &self.training_accounting else {
            if candidate.parameters.get("receipt_protocol")
                == Some(&ParameterValue::Text("managed-settings-v1".into()))
            {
                return Err(EncoderTaskAdapterError::TrainingAccounting(
                    "Managed training requires durable time accounting".into(),
                ));
            }
            return self
                .run_bounded(arguments, candidate.maximum_training_seconds)
                .await;
        };
        progress::check_stop()?;
        let permit = accounting.reserve(candidate).await?;
        let started = std::time::Instant::now();
        let result = self
            .run_with_deadline(arguments, Duration::from_millis(permit.maximum_millis))
            .await;
        let elapsed_millis = u64::try_from(started.elapsed().as_nanos().div_ceil(1_000_000))
            .map_err(adapter_error)?
            .max(1);
        let outcome = match &result {
            Ok(()) => TrainingAttemptOutcome::Completed,
            Err(EncoderTaskAdapterError::TimeLimitExceeded) => TrainingAttemptOutcome::TimedOut,
            Err(_) if progress::stopped() => TrainingAttemptOutcome::Interrupted,
            Err(_) => TrainingAttemptOutcome::Failed,
        };
        accounting.finish(permit, outcome, elapsed_millis).await?;
        result
    }
}
