import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { appendLiveActivity, inputRunActivity, inputRunStageDetail, inputRunStageLabel, mergedActivity, presentedActivity, stagedActivity, optimizationStages } from "../dist/evidence/input-run-activity.js";

test("post-evaluation bookkeeping stays visible in its exact iteration", () => {
  const narrative = { origin: "system", kind: "action", summary: "Recording completed development evaluation results." };
  const event = { state: "progress", stage: "finalizing_iteration", created_at: "2026-09-18T12:00:00Z", narrative,
    references: [{ kind: "iteration", id: "3" }, { kind: "run_stage", id: "evaluating" }] };
  const log = { actions: [{ action_id: "root", operation: "optimization.run", state: "progress", started_at: event.created_at,
    references: [{ kind: "run", id: "run" }], events: [event] }] };
  const activity = inputRunActivity(log, "run");
  assert.equal(activity.progress.phase, "finalizing_iteration");
  assert.equal(activity.progress.iteration, 3);
  assert.deepEqual(activity.progress.narrative, narrative);
  assert.equal(activity.events[0].stage, "evaluating");
  assert.equal(inputRunStageLabel("finalizing_iteration"), "Saving and checking iteration results");
  assert.equal(stagedActivity([{ at: event.created_at, progress: { phase: "finalizing_iteration", iteration: 3 } }])[0].stage, "evaluating");
});

test("iteration identity survives durable and live activity without merging adjacent iterations", () => {
  const live = [];
  appendLiveActivity(live, { phase: "training", iteration: 1 }, 1_000);
  appendLiveActivity(live, { phase: "training", iteration: 2 }, 2_000);
  assert.equal(live.length, 2);
  const log = { actions: [{ action_id: "root", operation: "optimization.run", state: "progress", started_at: "2026-09-16T00:00:00Z",
    references: [{ kind: "run", id: "run" }], events: [1, 2].map(iteration => ({ state: "progress", stage: "training", created_at: `2026-09-16T00:00:0${iteration}Z`,
      references: [{ kind: "iteration", id: String(iteration) }, { kind: "run_stage", id: "training" }] })) }] };
  assert.deepEqual(inputRunActivity(log, "run").events.map(event => event.progress.iteration), [1, 2]);
});

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
    { at: "2026-01-01T00:00:01Z", stage: "starting", progress: { phase: "creating_candidate", narrative }, narrative },
    { at: "2026-01-01T00:10:02Z", stage: "training", progress: { phase: "training", completed: 4, total: 10 } },
  ]);
});

test("project Agent and generation journals appear inline without desktop narration", () => {
  const narrative = { origin: "agent", kind: "reasoning", summary: "Remove the ambiguous row identified by the retrieval failure." };
  const generation = { origin: "generation", kind: "intent", summary: "Generating the requested training example." };
  const qualification = { origin: "system", kind: "intent", summary: "Checking the complete candidate dataset." };
  const action = (id, operation, stage, narrative, time) => ({ action_id: id, operation, source: "cli", state: "succeeded",
    started_at: time, finished_at: time, references: [{ kind: "run", id: "run" }],
    events: [{ state: "progress", stage, narrative, created_at: time }] });
  const activity = inputRunActivity({ actions: [
    action("clearance", "optimization.qualification", "checking_training_data", qualification, "2026-09-15T12:00:04Z"),
    action("generator", "optimization.generation", "data_generation", generation, "2026-09-15T12:00:02Z"),
    action("agent", "optimization.agent", "agent_analysis", narrative, "2026-09-15T12:00:01Z"),
    { ...action("other", "optimization.agent", "agent_analysis", narrative, "2026-09-15T12:00:03Z"), references: [{kind:"run",id:"another"}] },
  ] }, "run");
  assert.equal(activity.actionId, "agent");
  assert.deepEqual(activity.events.map(event => event.narrative), [narrative, generation, qualification]);
  assert.deepEqual(activity.events.map(event => event.stage), ["preparing_data", "preparing_data", "preparing_data"]);
  assert.equal(activity.progress.phase, "checking_training_data");
  assert.equal(inputRunStageLabel("data_generation"), "Generating training examples");
});

test("all recorded and live tasks survive every stage and retries beyond old cutoffs", () => {
  const origin = Date.parse("2026-09-14T12:00:00Z");
  const events = optimizationStages.flatMap((stage, index) => Array.from({ length: 140 }, (_, row) => ({
    state: "progress", stage: "verifying_file", created_at: new Date(origin + (index * 140 + row) * 1000).toISOString(),
    references: [{ kind: "run_stage", id: stage }, { kind: "progress_subject", id: `file-${row}.json` }],
  })));
  const action = { action_id: "run-action", operation: "optimization.run", state: "succeeded", started_at: new Date(origin).toISOString(),
    references: [{ kind: "run", id: "run" }], events };
  const activity = inputRunActivity({ actions: [action] }, "run");
  assert.equal(activity.events.length, 840);
  const live = [];
  for (let row = 0; row < 150; row++) appendLiveActivity(live, { phase: "verifying_file", subject: `retry-${row}.json`, runStage: "checking_inputs" }, origin + (900 + row) * 1000);
  const combined = stagedActivity(mergedActivity(activity, live));
  assert.equal(combined.length, 990);
  for (const stage of optimizationStages) assert.equal(combined.filter(event => event.stage === stage).length, stage === "checking_inputs" ? 290 : 140);
  assert.equal(combined[0].progress.subject, "file-0.json");
});

test("checksum telemetry stays durable but completed bursts become one visible system activity", () => {
  const checksum = (second, subject) => ({ at: `2026-09-14T12:00:0${second}Z`, stage: "training", progress: { phase: "verifying_file", subject } });
  const events = [
    checksum(1, "config.json"), checksum(2, "model.safetensors"),
    { at: "2026-09-14T12:00:03Z", stage: "training", progress: { phase: "loading_model" } },
    checksum(4, "modules.json"), checksum(5, "tokenizer.json"),
  ];
  assert.equal(events.length, 5, "the immutable source is not compacted");
  assert.deepEqual(presentedActivity(events, false), [
    { at: "2026-09-14T12:00:02Z", stage: "training", progress: { phase: "verifying_file" }, label: "Verified required files" },
    events[2],
    { at: "2026-09-14T12:00:05Z", stage: "training", progress: { phase: "verifying_file" }, label: "Verified required files" },
  ]);
  assert.deepEqual(presentedActivity(events, true).at(-1), events.at(-1), "only the current file remains visible while hashing");
});

test("legacy checksums use CLI stage intervals and retain native substages", () => {
  const ref = [{ kind: "run", id: "run" }];
  const at = seconds => new Date(Date.parse("2026-09-14T12:00:00Z") + seconds * 1000).toISOString();
  const activity = inputRunActivity({ actions: [
    { action_id: "drive", operation: "optimization.run", state: "succeeded", started_at: at(0), references: ref, events: [
      { state: "progress", stage: "verifying_file", created_at: at(2) },
      { state: "progress", stage: "loading_model", created_at: at(6) },
      { state: "progress", stage: "saving_checkpoint", created_at: at(7) },
      { state: "progress", stage: "verifying_file", created_at: at(8) },
      { state: "progress", stage: "evaluating_agent", created_at: at(9) },
    ] },
    { action_id: "attach", operation: "optimization.attach_experiment", source: "cli", started_at: at(1), finished_at: at(4), references: ref, events: [] },
    { action_id: "execute", operation: "optimization.execute", source: "cli", started_at: at(5), finished_at: at(10), references: ref, events: [] },
  ] }, "run");
  assert.deepEqual(activity.events.map(event => event.stage), ["starting", "training", "saving_candidate", "saving_candidate", "evaluating"]);
});
