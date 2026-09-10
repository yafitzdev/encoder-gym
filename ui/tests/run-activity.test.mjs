import assert from "node:assert/strict";
import { test } from "node:test";
import { parseProgress, ProgressLines, recordProgress, executeObservedCommand } from "../dist/evidence/run-activity.js";
import { mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const wire = value => `ENCODER_GYM_PROGRESS ${JSON.stringify(value)}\n`;
test("progress stream handles chunk boundaries and recovers after oversized or untrusted lines", () => {
  const received = [], lines = new ProgressLines(value => received.push(value));
  const counter = { phase: "training", completed: 2, total: 10 }, valid = wire(counter);
  lines.push(valid.slice(0, 30)); lines.push(valid.slice(30));
  lines.push("x".repeat(1500)); lines.push(wire(counter));
  lines.push(wire({ phase: "training", completed: 3, total: 10, secret: "must-not-leave-process" }));
  lines.push(wire({ phase: "evaluating_retrieval", score: 0.9 }));
  lines.push("native output or credentials\n");
  lines.push(wire({ phase: "saving_checkpoint" }));
  assert.deepEqual(received, [counter, { phase: "saving_checkpoint" }]);
  for (const invalid of [{ phase: "unknown" }, { phase: "training", completed: 1 }, { phase: "training", completed: 3, total: 2 }, { phase: "training", completed: 0, total: 0 }]) {
    assert.equal(parseProgress(wire(invalid).trim()), undefined);
  }
});

test("changing tasks clears the previous counter and bounds the activity history", () => {
  const at = "2026-09-10T12:00:00Z";
  const activity = { phase: "loading_model", running: true, startedAt: at, updatedAt: at, events: [] };
  recordProgress(activity, { phase: "training", completed: 2, total: 10 }, at);
  recordProgress(activity, { phase: "training", completed: 3, total: 10 }, at);
  assert.equal(activity.events.length, 1);
  recordProgress(activity, { phase: "evaluating_retrieval" }, at);
  assert.equal(activity.completed, undefined);
  assert.equal(activity.total, undefined);
  for (let n = 0; n < 30; n++) recordProgress(activity, { phase: n % 2 ? "training" : "loading_model" }, at);
  assert.equal(activity.events.length, 20);
});

test("real child progress reaches the observer before exit, without polluting the result", async () => {
  const observed = join(mkdtempSync(join(tmpdir(), "gym-progress-pipe-")), "observed");
  const script = `const fs = require('node:fs');
    process.stderr.write(${JSON.stringify(wire({ phase: "training", completed: 1, total: 5 }))});
    const deadline = setTimeout(() => { process.stderr.write('progress was buffered'); process.exit(1); }, 5000);
    const timer = setInterval(() => { if (fs.existsSync(${JSON.stringify(observed)})) {
      clearTimeout(deadline); clearInterval(timer); process.stdout.write('{"done":true}');
    } }, 10);`;
  const received = [];
  const result = await executeObservedCommand(process.execPath, ["-e", script], undefined, value => {
    received.push(value); writeFileSync(observed, "received-before-exit");
  });
  assert.deepEqual(JSON.parse(result), { done: true });
  assert.deepEqual(received, [{ phase: "training", completed: 1, total: 5 }]);
  await assert.rejects(() => executeObservedCommand(process.execPath, ["-e", `process.stderr.write(${JSON.stringify(wire({ phase: "training" }) + "checkpoint failed")});process.exitCode=1`], undefined, () => {}), error => error.message === "checkpoint failed");
});
