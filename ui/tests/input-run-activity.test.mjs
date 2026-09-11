import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { inputRunActivity, inputRunStageDetail, inputRunStageLabel } from "../dist/evidence/input-run-activity.js";

test("optimization activity resolves the exact run and latest durable counter", () => {
  const projectId = randomUUID(), runId = randomUUID(), otherRun = randomUUID(), actionId = randomUUID();
  const event = (state, stage, completed, total, at) => ({ state, stage, completed, total, created_at: at });
  const log = { project_id: projectId, actions: [
    { action_id: randomUUID(), operation: "optimization.run", state: "progress", started_at: "2026-01-01T00:00:00Z", references: [{ kind: "run", id: otherRun }], events: [event("progress", "training", 99, 100, "2026-01-01T00:00:02Z")] },
    { action_id: actionId, operation: "optimization.run", state: "progress", started_at: "2026-01-01T00:00:00Z", references: [{ kind: "run", id: runId }], events: [
      event("started", undefined, undefined, undefined, "2026-01-01T00:00:00Z"),
      event("progress", "loading_model", undefined, undefined, "2026-01-01T00:00:01Z"),
      event("progress", "training", 20, 100, "2026-01-01T00:00:02Z"),
      event("progress", "training", 40, 100, "2026-01-01T00:00:03Z"),
    ] },
  ] };
  const activity = inputRunActivity(log, runId);
  assert.equal(activity.actionId, actionId);
  assert.deepEqual(activity.progress, { phase: "training", completed: 40, total: 100 });
  assert.deepEqual(activity.stages, ["loading_model", "training"]);
  assert.equal(inputRunStageLabel(activity.progress.phase), "Training candidate");
  assert.equal(inputRunStageDetail(activity.progress, {
    model: "Nomos baseline", dataset: "Candidate dataset · v2", datasetRows: 6992,
    evaluation: "Evaluation · Version 1", developmentSuites: ["generic holdout", "agent holdout"], finalEvaluation: false,
  }), "Candidate 1 · 40 / 100 steps");
});
