import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { InputRunsController } from "../dist/evidence/input-runs-controller.js";

const run = (projectId, state = "queued", createdAt = new Date().toISOString()) => ({
  id: randomUUID(), projectId, createdAt, state, attempt: 0, materializationAttempt: 0, experimentAttempt: 0,
  executionAttempt: 0, finalAttempt: 0, lastSequence: 1, updatedAt: createdAt,
});

test("opening stage history reads the entire selected run, independently of the recent feed", async () => {
  const projectId = randomUUID(), value = run(projectId); let reads = 0;
  const controller = new InputRunsController(projectId, {
    projectActivity: async (id, _limit, runId) => { reads++; assert.equal(id, projectId); assert.equal(runId, value.id); return { actions: [] }; },
  }, () => {});
  await Promise.all([controller.ensureActivity(value.id), controller.ensureActivity(value.id)]);
  await controller.ensureActivity(value.id);
  assert.equal(reads, 1);
});

test("run history loads newest first and completes the exact selected run", async () => {
  const projectId = randomUUID(), first = run(projectId, "queued", "2026-01-01T00:00:00Z"), second = run(projectId, "execution_failed", "2026-01-02T00:00:00Z"), updates = []; let activityReads = 0;
  const bridge = {
    inputOptimizationRuns: async () => [first, second],
    projectActivity: async () => { activityReads++; return { project_id: projectId, actions: [] }; },
    driveInputOptimization: async (_, id) => ({ ...(id === second.id ? second : first), state: "baseline_retained", outcome: { kind: "baseline_retained" } }),
    inputOptimizationRun: async (_, id) => id === second.id ? second : first,
    cancelInputOptimization: async (_, id) => ({ ...(id === second.id ? second : first), state: "cancelled", failureCode: "user_requested" }),
    selectProject: async () => ({ content: { state: "ready", workspace: { managed: { manifest: { id: projectId } } } } }),
  };
  const controller = new InputRunsController(projectId, bridge, () => {}, workspace => updates.push(workspace));
  await controller.ensure(); assert.deepEqual(controller.runs.map(value => value.id), [second.id, first.id]);
  assert.equal(activityReads, 0, "the run collection must not preload the project-wide activity log");
  await controller.ensureActivity(second.id); assert.equal(activityReads, 1, "activity is loaded only for the expanded run");
  await controller.resume(second);
  assert.equal(controller.runs[0].state, "baseline_retained"); assert.equal(controller.runningId, undefined); assert.equal(updates.length, 1);
});

test("failed continuation remains retryable and refresh never interrupts active work", async () => {
  const projectId = randomUUID(), value = run(projectId, "execution_failed"), stopped = { ...value, failureCode: "candidate_execution_failed" }; let attempts = 0, release;
  const pending = new Promise(resolve => { release = resolve; });
  const bridge = {
    inputOptimizationRuns: async () => [value],
    projectActivity: async () => ({ project_id: projectId, actions: [] }),
    driveInputOptimization: async () => { attempts++; if (attempts === 1) throw new Error("Stopped"); await pending; return { ...value, state: "baseline_retained", outcome: { kind: "baseline_retained" } }; },
    inputOptimizationRun: async () => stopped,
    cancelInputOptimization: async () => ({ ...value, state: "cancelled", failureCode: "user_requested" }),
    selectProject: async () => ({ content: { state: "ready", workspace: {} } }),
  };
  const controller = new InputRunsController(projectId, bridge, () => {}); await controller.ensure();
  await controller.resume(value); assert.ok(controller.error); assert.equal(controller.runs[0].state, "execution_failed");
  const retry = controller.resume(controller.runs[0]); await new Promise(resolve => setImmediate(resolve));
  controller.refresh(); assert.ok(controller.runs); assert.equal(controller.runningId, value.id);
  release(); await retry; assert.equal(controller.runs[0].state, "baseline_retained"); assert.equal(attempts, 2);
});

test("a run can be durably cancelled without starting or resuming it", async () => {
  const projectId = randomUUID(), value = run(projectId, "queued"); let cancelled = 0;
  const bridge = {
    inputOptimizationRuns: async () => [value], projectActivity: async () => ({ project_id: projectId, actions: [] }),
    cancelInputOptimization: async (_, id) => { cancelled++; assert.equal(id, value.id); return { ...value, state: "cancelled", failureCode: "user_requested" }; },
  };
  const controller = new InputRunsController(projectId, bridge, () => {}); await controller.ensure();
  await controller.cancel(value);
  assert.equal(cancelled, 1); assert.equal(controller.runs[0].state, "cancelled"); assert.equal(controller.cancellingId, undefined);
});

test("Resume carries the execution head displayed by the renderer", async () => {
  const projectId = randomUUID(), value = run(projectId, "agent_paused"), head = `sha256:${"c".repeat(64)}`;
  value.agentExecution = { headFingerprint: head };
  let observed;
  const controller = new InputRunsController(projectId, {
    driveInputOptimization: async (id, runId, _progress, resumeHead) => {
      assert.equal(id, projectId); assert.equal(runId, value.id); observed = resumeHead;
      throw new Error("A newer Stop was recorded");
    },
    inputOptimizationRun: async () => ({ ...value, agentExecution: { headFingerprint: `sha256:${"d".repeat(64)}` } }),
    projectActivity: async () => ({ actions: [] }),
  }, () => {});
  await controller.resume(value);
  assert.equal(observed, head);
  assert.match(controller.error.message, /newer Stop/);
  assert.notEqual(controller.runs[0].agentExecution.headFingerprint, observed);
});

test("a late Stop reply refreshes its durable pause after the drive has already settled", async () => {
  const projectId = randomUUID(), value = run(projectId, "agent_running");
  let finishDrive, acknowledgeStop;
  const driving = new Promise(resolve => { finishDrive = resolve; });
  const stopping = new Promise(resolve => { acknowledgeStop = resolve; });
  const paused = { ...value, state: "agent_paused", agentExecution: { headFingerprint: `sha256:${"e".repeat(64)}` } };
  const controller = new InputRunsController(projectId, {
    driveInputOptimization: async () => { await driving; return { ...value, state: "agent_stopping" }; },
    stopInputOptimization: async () => { await stopping; },
    inputOptimizationRun: async () => paused,
    selectProject: async () => ({ content: { state: "ready", workspace: {} } }),
    projectActivity: async () => ({ actions: [] }),
  }, () => {});
  const resume = controller.resume(value);
  const stop = controller.stop(value);
  finishDrive(); await resume;
  assert.equal(controller.stoppingId, value.id, "finishing the drive must not acknowledge an outstanding Stop");
  acknowledgeStop(); await stop;
  assert.equal(controller.runs[0], paused);
  assert.equal(controller.stoppingId, undefined);
});
