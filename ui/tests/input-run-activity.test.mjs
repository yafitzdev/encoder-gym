import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { appendLiveActivity, inputRunActivity, inputRunStageDetail, inputRunStageLabel, mergedActivity } from "../dist/evidence/input-run-activity.js";

test("activity keeps recorded timestamps on redraw and compacts live file counters", () => {
  const at = "2026-09-14T10:00:00.000Z";
  const progress = { phase: "verifying_file", subject: "model.safetensors", completed: 1, total: 4 };
  const activity = { updatedAt: at, events: [{ at, progress }] };
  assert.deepEqual(mergedActivity(activity, [], progress), activity.events);
  const live = [];
  appendLiveActivity(live, progress, Date.parse(at) + 1000);
  appendLiveActivity(live, { ...progress, completed: 4 }, Date.parse(at) + 2000);
  assert.equal(live.length, 1);
  assert.equal(mergedActivity(activity, live).length, 1);
  appendLiveActivity(live, { ...progress, subject: "dataset.jsonl" }, Date.parse(at) + 3000);
  assert.equal(mergedActivity(activity, live).length, 2);
});

test("file identity and counters survive project activity persistence", () => {
  const log = { actions: [{ action_id: "a", operation: "optimization.run", state: "progress", started_at: "2026-09-13T12:00:00Z",
    references: [{ kind: "run", id: "run" }], events: [{ state: "progress", stage: "verifying_file", completed: 1024, total: 2048, created_at: "2026-09-13T12:00:01Z",
      references: [{ kind: "progress_subject", id: "model.safetensors" }, { kind: "progress_unit", id: "bytes" }] }] }] };
  const activity = inputRunActivity(log, "run");
  assert.deepEqual(activity.progress, { phase: "verifying_file", subject: "model.safetensors", unit: "bytes", completed: 1024, total: 2048 });
  assert.equal(inputRunStageDetail(activity.progress, {}), "model.safetensors");
  assert.equal(activity.events[0].progress.subject, "model.safetensors");
});

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
      { state: "failed", created_at: "2026-01-01T00:00:04Z", failure: { code: "training_failed", message: "Checkpoint was not written." } },
    ] },
  ] };
  const activity = inputRunActivity(log, runId);
  assert.equal(activity.actionId, actionId);
  assert.deepEqual(activity.progress, { phase: "training", completed: 40, total: 100 });
  assert.deepEqual(activity.stages, ["loading_model", "training"]);
  assert.deepEqual(activity.failure, { code: "training_failed", message: "Checkpoint was not written." });
  assert.equal(inputRunStageLabel(activity.progress.phase), "Training candidate");
  assert.equal(inputRunStageDetail(activity.progress, {
    model: "Nomos baseline", dataset: "Candidate dataset · v2", datasetRows: 6992,
    evaluation: "Evaluation · Version 1", developmentSuites: ["generic holdout", "agent holdout"], finalEvaluation: false,
  }), "Candidate 1");
});

test("optimization narrative stays inline with compact work across resumed actions", () => {
  const projectId = randomUUID(), runId = randomUUID();
  const narrative = { origin: "system", kind: "reasoning", summary: "Use one conservative candidate before expanding the search." };
  const log = { project_id: projectId, actions: [
    { action_id: "new", operation: "optimization.run", state: "progress", started_at: "2026-01-01T00:10:00Z", references: [{ kind: "run", id: runId }], events: [
      { state: "started", created_at: "2026-01-01T00:10:00Z" },
      { state: "progress", stage: "training", completed: 1, total: 10, created_at: "2026-01-01T00:10:01Z" },
      { state: "progress", stage: "training", completed: 4, total: 10, created_at: "2026-01-01T00:10:02Z" },
    ] },
    { action_id: "old", operation: "optimization.run", state: "failed", started_at: "2026-01-01T00:00:00Z", references: [{ kind: "run", id: runId }], events: [
      { state: "started", created_at: "2026-01-01T00:00:00Z" },
      { state: "progress", stage: "creating_candidate", narrative, created_at: "2026-01-01T00:00:01Z" },
      { state: "failed", failure: { code: "interrupted", message: "Stopped." }, created_at: "2026-01-01T00:00:02Z" },
    ] },
  ] };
  const activity = inputRunActivity(log, runId);
  assert.equal(activity.actionId, "new");
  assert.equal(activity.failure, undefined, "an older failed attempt must not override the active attempt");
  assert.deepEqual(activity.events, [
    { at: "2026-01-01T00:00:01Z", progress: { phase: "creating_candidate", narrative }, narrative },
    { at: "2026-01-01T00:10:02Z", progress: { phase: "training", completed: 4, total: 10 } },
  ]);
});
