import assert from "node:assert/strict";
import test from "node:test";
import { observedInputRun, inputOptimizationWorking } from "../dist/evidence/input-optimization.js";

test("root cancellation and newer Agent controls outrank late reads while retaining report history", () => {
  const running = { id: "run", projectId: "project", state: "agent_running", lastSequence: 4, agentExecution: { lastSequence: 2 }, iterations: [{ number: 1 }] };
  const paused = { ...running, state: "agent_paused", agentExecution: { lastSequence: 4 } };
  assert.equal(observedInputRun(paused, running), paused);
  const cancelled = { ...running, state: "cancelled", lastSequence: 5 };
  assert.equal(observedInputRun(cancelled, paused), cancelled);
  assert.equal(observedInputRun(paused, cancelled), cancelled);
  assert.deepEqual(observedInputRun(running, { ...paused, iterations: undefined }).iterations, running.iterations);
  assert.throws(() => observedInputRun(running, { ...paused, projectId: "other" }), /another project/);
});

test("durably stopped states outrank unsettled local work; stored running alone proves no live worker", () => {
  for (const state of ["agent_paused", "agent_interrupted", "agent_failed", "agent_budget_exhausted", "agent_completed", "cancelled", "candidate_accepted", "candidate_rejected"]) {
    assert.equal(inputOptimizationWorking({ state }, true), false, state);
  }
  assert.equal(inputOptimizationWorking({ state: "agent_running" }, false), false);
  assert.equal(inputOptimizationWorking({ state: "agent_running" }, true), true);
  assert.equal(inputOptimizationWorking({ state: "agent_stopping" }, true), true, "Stop intent does not acknowledge stopped work");
});
