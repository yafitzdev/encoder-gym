//! Row-free progress for the work between native evaluation and loop completion.
pub(super) fn finalizing(iteration: u32, summary: &'static str) {
    eprintln!(
        "ENCODER_GYM_PROGRESS {}",
        serde_json::json!({
            "phase": "finalizing_iteration",
            "runStage": "evaluating",
            "iteration": iteration,
            "narrative": {"origin": "system", "kind": "action", "summary": summary},
        })
    );
}
