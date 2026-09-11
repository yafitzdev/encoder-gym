import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { InputRunsController } from "../dist/evidence/input-runs-controller.js";

const run = (projectId, state = "queued", createdAt = new Date().toISOString()) => ({
  id: randomUUID(), projectId, createdAt, state, attempt: 0, materializationAttempt: 0, experimentAttempt: 0,
  executionAttempt: 0, finalAttempt: 0, lastSequence: 1, updatedAt: createdAt,
});

test("run history loads newest first and completes the exact selected run", async () => {
  const projectId = randomUUID(), first = run(projectId, "queued", "2026-01-01T00:00:00Z"), second = run(projectId, "execution_failed", "2026-01-02T00:00:00Z"), updates = [];
  const bridge = {
    inputOptimizationRuns: async () => [first, second],
    driveInputOptimization: async (_, id) => ({ ...(id === second.id ? second : first), state: "baseline_retained", outcome: { kind: "baseline_retained" } }),
    inputOptimizationRun: async (_, id) => id === second.id ? second : first,
    selectProject: async () => ({ content: { state: "ready", workspace: { managed: { manifest: { id: projectId } } } } }),
  };
  const controller = new InputRunsController(projectId, bridge, () => {}, workspace => updates.push(workspace));
  await controller.ensure(); assert.deepEqual(controller.runs.map(value => value.id), [second.id, first.id]);
  await controller.resume(second);
  assert.equal(controller.runs[0].state, "baseline_retained"); assert.equal(controller.runningId, undefined); assert.equal(updates.length, 1);
});

test("failed continuation remains retryable and refresh never interrupts active work", async () => {
  const projectId = randomUUID(), value = run(projectId, "execution_failed"), stopped = { ...value, failureCode: "candidate_execution_failed" }; let attempts = 0, release;
  const pending = new Promise(resolve => { release = resolve; });
  const bridge = {
    inputOptimizationRuns: async () => [value],
    driveInputOptimization: async () => { attempts++; if (attempts === 1) throw new Error("Stopped"); await pending; return { ...value, state: "baseline_retained", outcome: { kind: "baseline_retained" } }; },
    inputOptimizationRun: async () => stopped,
    selectProject: async () => ({ content: { state: "ready", workspace: {} } }),
  };
  const controller = new InputRunsController(projectId, bridge, () => {}); await controller.ensure();
  await controller.resume(value); assert.ok(controller.error); assert.equal(controller.runs[0].state, "execution_failed");
  const retry = controller.resume(controller.runs[0]); await new Promise(resolve => setImmediate(resolve));
  controller.refresh(); assert.ok(controller.runs); assert.equal(controller.runningId, value.id);
  release(); await retry; assert.equal(controller.runs[0].state, "baseline_retained"); assert.equal(attempts, 2);
});
