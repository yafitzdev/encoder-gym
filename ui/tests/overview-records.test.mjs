import assert from "node:assert/strict";
import test from "node:test";
import { overviewRecords, comparisonTone, reportDecision, projectOptimizationWorking } from "../dist/evidence/overview-records.js";

test("sidebar activity respects durable Agent closure while mutation custody remains owned", () => {
  const running = { id:"run", state:"agent_running", lastSequence:1, agentExecution:{lastSequence:1} };
  const setup = { running:true, run:running };
  const runs = { runningId:"run", runs:[running], final:{} };
  assert.equal(projectOptimizationWorking(setup,runs),true);
  for (const state of ["agent_paused","agent_interrupted","agent_completed","agent_budget_exhausted","agent_failed"]) {
    runs.runs=[{...running,state,agentExecution:{lastSequence:2}}];
    assert.equal(projectOptimizationWorking(setup,runs),false,state);
  }
  runs.final.busyRun="run"; assert.equal(projectOptimizationWorking(setup,runs),true);
  assert.equal(projectOptimizationWorking({saving:true}),true);
  assert.equal(projectOptimizationWorking(undefined,{runningId:"legacy",runs:[{id:"legacy",state:"baseline_retained"}],final:{}}),true);
});

test("Overview joins scientific children by exact identity and numbers roots consistently", () => {
  const createdAt = "2026-09-13T12:00:00Z";
  const workspace = { runs: [{ id: "experiment", createdAt }, { id: "legacy", createdAt, optimizationId: "old" }] };
  const roots = [{ id: "root", createdAt, experimentRunId: "experiment", lastSequence: 3, state: "baseline_retained" }, { id: "root", createdAt, lastSequence: 1, state: "queued" }];
  const records = overviewRecords(workspace, roots, { run_id: "old", created_at: createdAt });
  assert.equal(records.length, 2);
  assert.equal(records.find(row => row.id === "root").experiment.id, "experiment");
  assert.equal(records.find(row => row.id === "root").input.lastSequence, 3);
  assert.equal(records.find(row => row.id === "legacy").managed.run_id, "old");
  assert.deepEqual(records.map(row => row.name), ["Run 02", "Run 01"]);
  assert.deepEqual(overviewRecords(workspace, [...roots].reverse()).map(row => row.id), records.map(row => row.id));
});
test("report colors respect direction; verdict never follows positive numbers", () => {
  assert.equal(comparisonTone(.5, .6, "higher_is_better"), "success");
  assert.equal(comparisonTone(.5, .6, "lower_is_better"), "danger");
  assert.equal(comparisonTone(.5, .4, "lower_is_better"), "success");
  assert.equal(comparisonTone(.5, .5, "higher_is_better"), "muted");
  assert.equal(comparisonTone(.5, .6), "muted");
  assert.equal(reportDecision({ input: { state: "baseline_retained" } }), "REJECT");
  assert.equal(reportDecision({ input: { state: "candidate_accepted" } }), "KEEP");
  assert.equal(reportDecision({ input: { state: "ready_for_final_evaluation" }, experiment: { acceptance: { state: "unused" } } }), undefined);
});

test("multiple Agent iteration experiments belong to one root without fabricated final approval", () => {
  const createdAt = "2026-09-16T12:00:00Z";
  const root = { id: "root", createdAt, lastSequence: 1, state: "agent_completed", iterations: [{ experimentRunId: "one" }, { experimentRunId: "two" }] };
  const records = overviewRecords({ runs: [{ id: "one", createdAt }, { id: "two", createdAt }, { id: "unrelated", createdAt }] }, [root]);
  assert.equal(records.length, 2);
  assert.equal(reportDecision(records.find(record => record.id === "root")), undefined);
});

test("Overview keeps the newest Agent journal even when the fixed root sequence is unchanged", () => {
  const base = { id: "root", createdAt: "2026-09-16T12:00:00Z", lastSequence: 1 };
  const running = { ...base, state: "agent_running", agentExecution: { lastSequence: 1 } };
  const paused = { ...base, state: "agent_paused", agentExecution: { lastSequence: 3 } };
  for (const roots of [[running, paused], [paused, running]]) {
    assert.equal(overviewRecords({ runs: [] }, roots)[0].input, paused);
  }
  const cancelled = { ...running, state: "cancelled", lastSequence: 2 };
  assert.equal(overviewRecords({ runs: [] }, [cancelled, paused])[0].input, cancelled);
});
