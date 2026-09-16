use super::*;
use crate::optimization_loop::AgentLoopEnd;

fn identity() -> BoundIdentity {
    BoundIdentity {
        id: Uuid::new_v4().to_string(),
        fingerprint: format!("sha256:{}", "a".repeat(64)),
    }
}

fn event(
    run: &BoundIdentity,
    history: &[AgentExecutionEvent],
    attempt: Uuid,
    change: AgentExecutionChange,
) -> AgentExecutionEvent {
    AgentExecutionEvent::create(run.clone(), history.last(), attempt, change, Utc::now()).unwrap()
}

#[test]
fn stop_requires_acknowledgement_and_resume_preserves_attempt_history() {
    let run = identity();
    let initial = Uuid::new_v4();
    let mut events = vec![event(
        &run,
        &[],
        initial,
        AgentExecutionChange::StopRequested,
    )];
    assert_eq!(replay(&run, &events, &[]).unwrap().unwrap().attempts, 0);
    let attempt = Uuid::new_v4();
    let premature = event(&run, &events, attempt, AgentExecutionChange::Started);
    assert!(replay(&run, &[events.clone(), vec![premature]].concat(), &[]).is_err());
    events.push(event(&run, &events, initial, AgentExecutionChange::Paused));
    events.push(event(&run, &events, attempt, AgentExecutionChange::Started));
    events.push(event(
        &run,
        &events,
        attempt,
        AgentExecutionChange::StopRequested,
    ));
    let late = event(&run, &events, attempt, AgentExecutionChange::Failed);
    assert!(replay(&run, &[events.clone(), vec![late]].concat(), &[]).is_err());
    events.push(event(&run, &events, attempt, AgentExecutionChange::Paused));
    assert_eq!(replay(&run, &events, &[]).unwrap().unwrap().attempts, 1);
    events.push(event(
        &run,
        &events,
        Uuid::new_v4(),
        AgentExecutionChange::Started,
    ));
    let view = replay(&run, &events, &[]).unwrap().unwrap();
    assert_eq!(view.attempts, 2);
    assert_eq!(view.state, AgentExecutionState::Running);
}

#[test]
fn distinct_stop_commands_change_the_head_even_while_paused() {
    let run = identity();
    let attempt = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let first = AgentExecutionEvent::create_with_id(
        request_id,
        run.clone(),
        None,
        attempt,
        AgentExecutionChange::StopRequested,
        Utc::now(),
    )
    .unwrap();
    let mut events = vec![first];
    events.push(event(&run, &events, attempt, AgentExecutionChange::Paused));
    let paused = replay(&run, &events, &[]).unwrap().unwrap();
    events.push(event(
        &run,
        &events,
        attempt,
        AgentExecutionChange::StopRequested,
    ));
    let newer = replay(&run, &events, &[]).unwrap().unwrap();
    assert_eq!(newer.state, AgentExecutionState::StopRequested);
    assert_ne!(newer.head_fingerprint, paused.head_fingerprint);
    assert_eq!(newer.attempts, 0);
    // Retrying one command is handled by persistence, never by appending it
    // again under the same action identity.
    let duplicate = AgentExecutionEvent::create_with_id(
        request_id,
        run.clone(),
        events.last(),
        attempt,
        AgentExecutionChange::StopRequested,
        Utc::now(),
    )
    .unwrap();
    events.push(duplicate);
    assert!(replay(&run, &events, &[]).is_err());
    assert!(
        AgentExecutionEvent::create_with_id(
            Uuid::nil(),
            run,
            None,
            attempt,
            AgentExecutionChange::StopRequested,
            Utc::now(),
        )
        .is_err()
    );
}

#[test]
fn failure_and_interruption_retain_history_and_fence_old_attempts() {
    let run = identity();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut events = vec![event(&run, &[], first, AgentExecutionChange::Started)];
    let duplicate = event(&run, &events, second, AgentExecutionChange::Started);
    assert!(replay(&run, &[events.clone(), vec![duplicate]].concat(), &[]).is_err());
    events.push(event(
        &run,
        &events,
        first,
        AgentExecutionChange::Interrupted,
    ));
    events.push(event(&run, &events, second, AgentExecutionChange::Started));
    let stale = event(&run, &events, first, AgentExecutionChange::Failed);
    assert!(replay(&run, &[events.clone(), vec![stale]].concat(), &[]).is_err());
    events.push(event(&run, &events, second, AgentExecutionChange::Failed));
    let failed = replay(&run, &events, &[]).unwrap().unwrap();
    assert_eq!(failed.state, AgentExecutionState::Failed);
    assert_eq!(failed.attempts, 2);
    events.push(event(
        &run,
        &events,
        Uuid::new_v4(),
        AgentExecutionChange::Started,
    ));
    assert_eq!(replay(&run, &events, &[]).unwrap().unwrap().attempts, 3);
    events[1].previous = Some(identity().fingerprint);
    events[1].fingerprint = events[1].reproduce().unwrap();
    assert!(replay(&run, &events, &[]).is_err());
}

#[test]
fn budget_exhaustion_is_terminal_and_cannot_be_retried_or_stopped() {
    let run = identity();
    let attempt = Uuid::new_v4();
    let mut events = vec![event(&run, &[], attempt, AgentExecutionChange::Started)];
    events.push(event(
        &run,
        &events,
        attempt,
        AgentExecutionChange::BudgetExhausted,
    ));
    let exhausted = replay(&run, &events, &[]).unwrap().unwrap();
    assert_eq!(exhausted.state, AgentExecutionState::BudgetExhausted);
    assert_eq!(exhausted.attempts, 1);

    let restarted = event(&run, &events, Uuid::new_v4(), AgentExecutionChange::Started);
    assert!(replay(&run, &[events.clone(), vec![restarted]].concat(), &[]).is_err());
    let stopped = event(&run, &events, attempt, AgentExecutionChange::StopRequested);
    assert!(replay(&run, &[events, vec![stopped]].concat(), &[]).is_err());
}

#[test]
fn completion_requires_exact_terminal_iteration_and_cannot_restart() {
    let run = identity();
    let attempt = Uuid::new_v4();
    let mut completion = IterationCompletion {
        run: run.clone(),
        iteration: identity(),
        number: 1,
        proposal_call_id: Uuid::new_v4(),
        proposal_fingerprint: identity().fingerprint,
        result: None,
        selected: None,
        row_changes: 0,
        total_row_changes: 0,
        end: Some(AgentLoopEnd::NoChange),
        created_at: Utc::now(),
        fingerprint: String::new(),
    };
    completion.fingerprint = completion.reproduce().unwrap();
    let mut events = vec![event(&run, &[], attempt, AgentExecutionChange::Started)];
    events.push(event(
        &run,
        &events,
        attempt,
        AgentExecutionChange::Completed {
            completion: completion.identity(),
        },
    ));
    assert!(replay(&run, &events, &[]).is_err());
    let view = replay(&run, &events, &[completion.clone()])
        .unwrap()
        .unwrap();
    assert_eq!(view.state, AgentExecutionState::Completed);
    assert_eq!(view.completion, Some(completion.identity()));
    let mut foreign = completion.clone();
    foreign.run = identity();
    foreign.fingerprint = foreign.reproduce().unwrap();
    assert!(replay(&run, &events, &[foreign]).is_err());
    events.push(event(
        &run,
        &events,
        Uuid::new_v4(),
        AgentExecutionChange::Started,
    ));
    assert!(replay(&run, &events, &[completion]).is_err());
}
