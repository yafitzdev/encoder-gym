use super::*;

fn limits() -> TrainingTimeLimits {
    TrainingTimeLimits {
        iteration_millis: 1_000,
        run_millis: 1_500,
    }
}

#[test]
fn interrupted_work_and_unknown_attempts_never_reset_allowance() {
    let iteration = Uuid::new_v4();
    let mut first = limits().reserve(&[], iteration).unwrap();
    assert_eq!(limits().remaining(&[first.clone()], iteration).unwrap(), 0);
    first
        .finish(TrainingAttemptOutcome::Interrupted, 400)
        .unwrap();
    let retry = limits().reserve(&[first.clone()], iteration).unwrap();
    assert_eq!(retry.permit.maximum_millis, 600);
    assert!(limits().reserve(&[first, retry], iteration).is_err());
}

#[test]
fn run_limit_spans_iterations_and_failed_attempts() {
    let mut first = limits().reserve(&[], Uuid::new_v4()).unwrap();
    first.finish(TrainingAttemptOutcome::Failed, 900).unwrap();
    let second = limits().reserve(&[first], Uuid::new_v4()).unwrap();
    assert_eq!(second.permit.maximum_millis, 600);
}

#[test]
fn cleanup_overrun_is_recorded_not_refunded_or_rejected() {
    let iteration = Uuid::new_v4();
    let mut first = limits().reserve(&[], iteration).unwrap();
    first
        .finish(TrainingAttemptOutcome::TimedOut, 1_020)
        .unwrap();
    assert_eq!(first.charge_millis(), 1_020);
    assert_eq!(limits().remaining(&[first], iteration).unwrap(), 0);
}

#[test]
fn duplicate_attempts_and_forged_grants_are_rejected() {
    let iteration = Uuid::new_v4();
    let mut first = limits().reserve(&[], iteration).unwrap();
    assert!(
        limits()
            .remaining(&[first.clone(), first.clone()], iteration)
            .is_err()
    );
    first.permit.maximum_millis += 1;
    assert!(limits().remaining(&[first], iteration).is_err());
}

#[test]
fn completion_retries_are_exact_and_submillisecond_work_is_not_free() {
    let mut attempt = limits().reserve(&[], Uuid::new_v4()).unwrap();
    assert!(attempt.finish(TrainingAttemptOutcome::Failed, 0).is_err());
    attempt
        .finish(TrainingAttemptOutcome::Completed, 1)
        .unwrap();
    attempt
        .finish(TrainingAttemptOutcome::Completed, 1)
        .unwrap();
    assert!(
        attempt
            .finish(TrainingAttemptOutcome::Completed, 2)
            .is_err()
    );
    assert!(
        attempt
            .finish(TrainingAttemptOutcome::Interrupted, 1)
            .is_err()
    );
}
