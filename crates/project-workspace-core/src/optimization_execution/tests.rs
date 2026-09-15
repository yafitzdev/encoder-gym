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
