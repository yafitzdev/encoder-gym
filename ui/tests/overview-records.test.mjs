import assert from "node:assert/strict";
import test from "node:test";
import { overviewRecords, comparisonTone, reportDecision } from "../dist/evidence/overview-records.js";

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
