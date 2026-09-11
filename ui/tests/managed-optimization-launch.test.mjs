import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { access, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import { ManagedOptimizationLaunch } from "../dist/evidence/managed-optimization-launch.js";
import { fingerprint, setupFixture } from "./optimization-setup-fixture.mjs";

function fixture() {
  const base = setupFixture(), providerId = randomUUID(), setupId = randomUUID();
  base.workspace.providerCatalog = { id: providerId, projectId: base.projectId, sequence: 1, providers: [], actor: "operator", reason: "fixture", createdAt: new Date().toISOString(), fingerprint };
  const provider = maximumRequests => ({ maximumRequests, maximumInputTokens: 10_000, maximumOutputTokens: 2_000, maximumCostMicrousd: 0 });
  const scope = { projectId: base.projectId, setup: { id: setupId, fingerprint }, providerCatalog: { id: providerId, fingerprint },
    limits: { maximumIterations: 3, maximumModels: 3, maximumDatasetRowChanges: 5_000, maximumTrainingSeconds: 21_600, maximumDevelopmentEvaluations: 6, maximumFinalEvaluations: 1 },
    generation: provider(11), advisor: provider(7), finalEvaluation: "selected_candidate_once", fingerprint };
  const preview = { scope, modelName: "Baseline", datasetRows: 10, benchmarkNumber: 1 };
  const authorization = id => ({ id, scope, authorizedBy: "local-operator", createdAt: new Date().toISOString(), fingerprint });
  const run = (state = "queued", extra = {}) => ({
    run: { id: randomUUID(), projectId: base.projectId, launch: { id: randomUUID(), fingerprint }, setup: { id: setupId, fingerprint }, createdAt: new Date().toISOString(), fingerprint },
    state, attempt: 0, materializationAttempt: 0, experimentAttempt: 0, executionAttempt: 0, finalAttempt: 0,
    lastSequence: 1, headFingerprint: fingerprint, updatedAt: new Date().toISOString(), ...extra,
  });
  return { ...base, setupId, scope, preview, authorization, run };
}
function ports(f, command, exclusive = async (_id, work) => work()) {
  return {
    open: async id => { assert.equal(id, f.projectId); return f.workspace; }, command, exclusive,
    exclusiveRun: async (_id, _runId, work) => work(new AbortController().signal), abortRun: () => {},
  };
}

test("optimization launch preview and history are exact project-owned reads", async () => {
  const f = fixture(), calls = [], authorization = f.authorization(randomUUID());
  const backend = new ManagedOptimizationLaunch(ports(f, async args => { calls.push(args); return args[2] === "list" ? [authorization] : f.preview; }));
  assert.deepEqual(await backend.list(f.projectId), [authorization]);
  assert.deepEqual(await backend.preview(f.projectId, f.setupId), f.preview);
  assert.deepEqual(calls, [
    ["optimization-launch", "owned-project", "list"],
    ["optimization-launch", "owned-project", "preview", "--setup", f.setupId],
  ]);
  await assert.rejects(() => backend.preview(f.projectId, "../other"));
  const changed = structuredClone(f.preview); changed.scope.providerCatalog.id = randomUUID();
  await assert.rejects(() => new ManagedOptimizationLaunch(ports(f, async () => changed)).preview(f.projectId, f.setupId));
});

test("one-click authorization preserves its retry, removes temp files and rejects injected execution data", async () => {
  const f = fixture(), id = randomUUID(), request = { id, scope: f.scope }, writes = [], files = [];
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    assert.deepEqual(args.slice(0, 4), ["optimization-launch", "owned-project", "authorize", "--file"]);
    files.push(args[4]); const written = JSON.parse(await readFile(args[4], "utf8")); writes.push(written);
    return { actionId: randomUUID(), authorization: f.authorization(written.id) };
  }, async (projectId, work) => { assert.equal(projectId, f.projectId); return work(); }));
  const first = await backend.authorize(f.projectId, request);
  const second = await backend.authorize(f.projectId, request);
  assert.equal(first.authorization.id, id);
  assert.deepEqual(second.authorization.scope, first.authorization.scope);
  assert.deepEqual(writes, [request, request]);
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
  for (const alter of [
    value => value.scope.apiKey = "secret",
    value => value.scope.execute = true,
    value => value.scope.projectId = randomUUID(),
    value => value.scope.limits.maximumModels = 99,
    value => value.scope.finalEvaluation = "unlimited",
    value => value.path = "other-project",
  ]) {
    const invalid = structuredClone(request); alter(invalid);
    await assert.rejects(() => backend.authorize(f.projectId, invalid));
  }
  assert.equal(writes.length, 2);
});

test("one-click optimization reserves exact current inputs and removes its private request", async () => {
  const f = fixture(), files = [], wire = f.run();
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    if (args[0] === "optimization-launch") return f.preview;
    assert.deepEqual(args.slice(0, 4), ["optimization-run", "owned-project", "start", "--file"]);
    files.push(args[4]);
    const request = JSON.parse(await readFile(args[4], "utf8"));
    assert.equal(request.scope.setup.id, f.setupId);
    return { actionId: randomUUID(), run: { ...wire, run: { ...wire.run, launch: { id: request.id, fingerprint } } } };
  }));
  const started = await backend.start(f.projectId, f.setupId);
  assert.equal(started.run.id, wire.run.id);
  assert.equal(started.run.state, "queued");
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
});

test("one-click optimization drives recoverable stages, forwards exact work, and withholds credentials from non-execution work", async () => {
  const f = fixture(), calls = [], phases = [], details = [], environment = { SYNTH_OPENAI_API_KEY: "secret" }, wire = f.run("candidate_rejected", {
    outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "candidate_ready", selectedModel: { id: randomUUID(), fingerprint }, createdAt: new Date().toISOString(), fingerprint },
    finalResult: { run: { id: randomUUID(), fingerprint }, outcomeFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "candidate_rejected", model: { id: randomUUID(), fingerprint }, finalReport: { id: randomUUID(), fingerprint }, createdAt: new Date().toISOString(), fingerprint },
  });
  const configured = new ManagedOptimizationLaunch({ ...ports(f, async (args, env, progress) => {
    calls.push({ args, env });
    if (args[2] === "materialize") progress?.({ phase: "writing_training_rows", completed: 40, total: 100 });
    return args[2] === "show" ? wire : { actionId: randomUUID(), run: wire };
  }), environment: () => environment });
  const result = await configured.drive(f.projectId, wire.run.id, (phase, native) => { phases.push(phase); if (native) details.push(native); });
  assert.equal(result.state, "candidate_rejected");
  assert.deepEqual(calls.map(call => call.args[2]), ["prepare", "materialize", "attach", "execute", "register", "show", "finalize", "show"]);
  assert.deepEqual(calls.filter(call => call.env).map(call => call.args[2]), ["execute", "finalize"]);
  assert.deepEqual(phases, ["checking_inputs", "preparing_data", "preparing_data", "starting", "training", "saving_candidate", "evaluating", "complete"]);
  assert.deepEqual(details, [{ phase: "writing_training_rows", completed: 40, total: 100 }]);
});

test("a development result that retains the baseline skips sealed evaluation", async () => {
  const f = fixture(), wire = f.run("baseline_retained", { outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "baseline_retained", createdAt: new Date().toISOString(), fingerprint } }), commands = [];
  const backend = new ManagedOptimizationLaunch({ ...ports(f, async args => { commands.push(args[2]); return args[2] === "show" ? wire : {}; }), environment: () => ({}) });
  assert.equal((await backend.drive(f.projectId, wire.run.id)).state, "baseline_retained");
  assert.ok(!commands.includes("finalize"));
});

test("cancellation is recorded before the active worker is aborted", async () => {
  const f = fixture(), active = f.run("optimizing"), cancelled = { ...active, state: "cancelled", failureCode: "user_requested", lastSequence: 3 };
  const order = [];
  const backend = new ManagedOptimizationLaunch({
    ...ports(f, async args => {
      assert.deepEqual(args, ["optimization-run", "owned-project", "cancel", active.run.id]);
      order.push("recorded"); return { actionId: randomUUID(), run: cancelled };
    }),
    abortRun: (_projectId, runId) => { assert.equal(runId, active.run.id); order.push("aborted"); },
  });
  assert.equal((await backend.cancel(f.projectId, active.run.id)).state, "cancelled");
  assert.deepEqual(order, ["recorded", "aborted"]);
});

test("project optimization history is a typed project-owned read", async () => {
  const f = fixture(), older = f.run("baseline_retained", { outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "baseline_retained", createdAt: new Date().toISOString(), fingerprint } }), newer = f.run();
  const backend = new ManagedOptimizationLaunch(ports(f, async args => { assert.deepEqual(args, ["optimization-run", "owned-project", "list"]); return [older, newer]; }));
  const runs = await backend.runs(f.projectId);
  assert.deepEqual(runs.map(run => run.id), [older.run.id, newer.run.id]);
  const foreign = structuredClone(newer); foreign.run.projectId = randomUUID();
  await assert.rejects(() => new ManagedOptimizationLaunch(ports(f, async () => [foreign])).runs(f.projectId));
});
