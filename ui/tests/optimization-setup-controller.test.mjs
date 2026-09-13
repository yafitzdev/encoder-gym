import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { OptimizationSetupController } from "../dist/evidence/optimization-setup-controller.js";
import { setupFixture } from "./optimization-setup-fixture.mjs";
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
function fixture(overrides = {}) {
  const f = setupFixture(), history = [], writes = [], runs = [];
  const run = (state = "queued") => ({ id: randomUUID(), projectId: f.projectId, createdAt: new Date().toISOString(), state, attempt: 0, materializationAttempt: 0, experimentAttempt: 0, executionAttempt: 0, finalAttempt: 0, lastSequence: 1, updatedAt: new Date().toISOString() });
  const bridge = { queryDatasets: async () => ({ kind: "list", entries: f.datasets }), queryBenchmarks: async () => ({ kind: "list", versions: f.workspace.benchmarkVersions }),
    optimizationSetups: async () => [...history], optimizationLaunches: async () => [], previewOptimizationSetup: async () => f.preview,
    saveOptimizationSetup: async (_, request) => { writes.push(structuredClone(request)); const result = f.saved(request); history.push(result.setup); return result; },
    startInputOptimization: async () => { const value = run(); runs.push(value); return { actionId: randomUUID(), run: value }; },
    driveInputOptimization: async (_, id) => ({ ...runs.find(value => value.id === id), state: "baseline_retained", outcome: { kind: "baseline_retained" } }),
    initializeBenchmark: async () => { throw new Error("Evaluation already exists"); },
    cancelInputOptimization: async (_, id) => ({ ...runs.find(value => value.id === id), state: "cancelled", failureCode: "user_requested" }),
    inputOptimizationRun: async (_, id) => runs.find(value => value.id === id),
    projectActivity: async () => ({ project_id: f.projectId, actions: [] }),
    selectProject: async () => ({ project: { id: f.projectId }, content: { state: "ready", workspace: { managed: f.workspace } } }), ...overrides };
  const controller = new OptimizationSetupController(f.projectId, f.workspace, bridge, () => {});
  return { ...f, controller, bridge, history, writes, runs, run };
}
test("first setup uses model provenance and new runs use the current project evaluation", async () => {
  const f = fixture(); await f.controller.ensure(); assert.equal(f.controller.datasetId, f.version.id); assert.ok(f.controller.canSave);
  await f.controller.save(); assert.ok(f.controller.saved); assert.equal(f.writes.length, 1); await f.controller.save(); assert.equal(f.writes.length, 1);
  f.workspace.benchmarkVersions.push({ ...f.benchmark, id: randomUUID(), number: 2 });
  f.datasets[0].versions.push({ version: { ...f.version, id: randomUUID(), number: 2 }, rows: 11 });
  f.controller.refresh(); await f.controller.ensure();
  assert.equal(f.controller.datasetId, f.version.id); assert.equal(f.controller.benchmarkId, f.workspace.benchmarkVersions[1].id); assert.equal(f.controller.saved, false);
});

test("Optimize creates a missing project evaluation and continues in the same click", async () => {
  const f = fixture(), phases = [];
  f.workspace.scientificBinding = { id: randomUUID() };
  f.workspace.benchmarkVersions = [];
  f.bridge.initializeBenchmark = async (_, receive) => {
    for (const phase of ["checking_files", "evaluating_retrieval"]) { const value = { phase }; phases.push(phase); receive(value); }
    f.workspace.benchmarkVersions.push(f.benchmark);
    return { actionId: randomUUID(), version: f.benchmark };
  };
  await f.controller.ensure();
  assert.equal(f.controller.benchmark, undefined); assert.equal(f.controller.canOptimize, true);
  await f.controller.optimize();
  assert.deepEqual(phases, ["checking_files", "evaluating_retrieval"]);
  assert.equal(f.writes.length, 1); assert.equal(f.runs.length, 1);
  assert.equal(f.controller.benchmarkId, f.benchmark.id);
  assert.equal(f.controller.run.state, "baseline_retained");
});
for (const failure of ["save", "reload"]) test(`lost ${failure} responses retry the same identity without duplicate submission`, async () => {
  const f = fixture(), pending = deferred(); let reads = 0;
  f.bridge.optimizationSetups = async () => { if (failure === "reload" && ++reads === 2) throw new Error("Read failed"); return [...f.history]; };
  f.bridge.saveOptimizationSetup = async (_, request) => {
    f.writes.push(structuredClone(request)); const result = f.saved(request);
    if (!f.history.length) f.history.push(result.setup);
    if (f.writes.length === 1) await pending.promise;
    return result;
  };
  await f.controller.ensure(); const operation = f.controller.save();
  await new Promise(resolve => setImmediate(resolve));
  await f.controller.save(); f.controller.select("dataset", randomUUID()); assert.equal(f.controller.datasetId, f.version.id);
  if (failure === "save") pending.reject(new Error("Reply lost")); else pending.resolve();
  await operation; assert.ok(f.controller.error); assert.equal(f.writes.length, 1);
  await f.controller.save(); assert.equal(f.controller.error, undefined); assert.ok(f.controller.saved); assert.deepEqual(f.writes[0], f.writes[1]);
});
test("stale previews cannot save and a refresh abandons uncertain mutation retries", async () => {
  const f = fixture(); await f.controller.ensure();
  f.preview.expectedParent = randomUUID(); await f.controller.save(); assert.ok(f.controller.error); assert.equal(f.writes.length, 0);
  f.controller.refresh(); await f.controller.ensure(); f.preview.expectedParent = null;
  f.preview.inputs = { ...f.inputs, baselineRevision: { ...f.inputs.baselineRevision, id: randomUUID() } };
  await f.controller.save(); assert.equal(f.writes.length, 0);
});
test("reopening or moving a project discards late reads and late previews", async () => {
  const f = fixture(), pending = deferred(); let reads = 0;
  f.bridge.optimizationSetups = async () => ++reads === 1 ? pending.promise : [];
  const opening = f.controller.ensure(); f.controller.sync({ ...f.workspace, folder: "moved-project" }); await f.controller.ensure();
  pending.reject(new Error("Old folder missing")); await opening; assert.equal(f.controller.error, undefined); assert.ok(f.controller.data);
  const preview = deferred(); f.bridge.previewOptimizationSetup = async () => preview.promise;
  const saving = f.controller.save(); f.controller.sync({ ...f.workspace, folder: "moved-again" }); preview.resolve(f.preview); await saving;
  assert.equal(f.writes.length, 0); assert.equal(f.controller.saving, false); await f.controller.ensure(); assert.ok(f.controller.data);
});
test("old retry loads the actual current setup instead of reactivating its inputs", async () => {
  const f = fixture(); await f.controller.ensure();
  f.bridge.saveOptimizationSetup = async (_, request) => { f.writes.push(request); if (f.writes.length === 1) throw new Error("Lost reply"); return f.saved(request); };
  await f.controller.save();
  const other = { ...f.benchmark, id: randomUUID(), number: 2 }; f.workspace.benchmarkVersions.push(other);
  f.history.push(f.saved({ id: randomUUID(), expectedParent: null, inputs: { ...f.inputs, benchmark: { id: other.id, fingerprint: other.fingerprint } } }).setup);
  await f.controller.save(); assert.equal(f.controller.benchmarkId, other.id); assert.ok(f.controller.saved); assert.deepEqual(f.writes[0], f.writes[1]);
});
test("new selections and explicit refresh cannot replay an old uncertain save", async () => {
  const f = fixture(); await f.controller.ensure();
  f.bridge.saveOptimizationSetup = async (_, request) => { f.writes.push(structuredClone(request)); throw new Error("Connection lost"); };
  await f.controller.save(); const first = f.writes[0].id;
  const variant = { ...f.version, id: randomUUID(), number: 2 };
  f.datasets[0].versions.push({ version: variant, rows: 11 }); f.preview.inputs = { ...f.inputs, dataset: variant };
  f.controller.select("dataset", variant.id); await f.controller.save();
  assert.notEqual(f.writes[1].id, first); assert.equal(f.writes[1].inputs.dataset.id, variant.id);
  const second = f.writes[1].id; f.controller.refresh(); await f.controller.ensure(); f.preview.inputs = f.inputs;
  await f.controller.save(); assert.notEqual(f.writes[2].id, second); assert.equal(f.writes[2].inputs.dataset.id, f.version.id);
});
test("a baseline revision change requires a new saved selection even for the same model", async () => {
  const f = fixture(); await f.controller.ensure(); await f.controller.save();
  const revision = { ...f.baseline, id: randomUUID() };
  const workspace = { ...f.workspace, modelCatalog: { ...f.workspace.modelCatalog, baselineRevisions: [f.baseline, revision], activeBaselineRevisionId: revision.id } };
  f.controller.sync(workspace); await f.controller.ensure();
  assert.equal(f.controller.saved, false); assert.equal(f.controller.selected.model.id, f.model.id); assert.equal(f.controller.selected.baselineRevision.id, revision.id);
});

test("Optimize saves changed inputs and completes one project run without a separate launch step", async () => {
  const f = fixture(); await f.controller.ensure();
  await f.controller.optimize();
  assert.equal(f.writes.length, 1); assert.equal(f.runs.length, 1);
  assert.equal(f.controller.run.state, "baseline_retained"); assert.equal(f.controller.runPhase, "complete");
  assert.equal(f.controller.running, false); assert.equal(f.controller.error, undefined);
});

test("a failed execution retries the same durable run instead of reserving another", async () => {
  let attempts = 0;
  const f = fixture({
    driveInputOptimization: async (_, id) => {
      attempts++;
      if (attempts === 1) throw new Error("Training stopped");
      return { ...f.runs.find(value => value.id === id), state: "baseline_retained", outcome: { kind: "baseline_retained" } };
    },
    inputOptimizationRun: async (_, id) => ({ ...f.runs.find(value => value.id === id), state: "execution_failed", failureCode: "candidate_execution_failed" }),
  });
  await f.controller.ensure(); await f.controller.optimize();
  const first = f.controller.run.id; assert.ok(f.controller.error); assert.equal(f.runs.length, 1);
  await f.controller.optimize();
  assert.equal(f.controller.run.id, first); assert.equal(f.runs.length, 1); assert.equal(f.controller.run.state, "baseline_retained");
});

test("legacy Cancel permanently cancels the active optimization", async () => {
  const active = deferred();
  const f = fixture({ driveInputOptimization: async () => active.promise });
  await f.controller.ensure(); const optimizing = f.controller.optimize();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(f.controller.canCancel, true);
  await f.controller.cancel();
  active.resolve(f.controller.run); await optimizing;
  assert.equal(f.controller.run.state, "cancelled"); assert.equal(f.controller.running, false); assert.equal(f.controller.canOptimize, true);
});

test("Overview Stop waits for worker acknowledgement and preserves the resumable run identity", async () => {
  const active = deferred(); let stopRequests = 0;
  const f = fixture({ driveInputOptimization: async () => active.promise,
    stopInputOptimization: async (_, id) => { assert.equal(id, f.controller.run.id); stopRequests++; } });
  await f.controller.ensure(); const optimizing = f.controller.optimize();
  await new Promise(resolve => setImmediate(resolve));
  const id = f.controller.run.id;
  f.controller.sync({ ...f.workspace });
  await f.controller.stop(); await f.controller.stop();
  assert.equal(stopRequests, 1); assert.equal(f.controller.stopping, true); assert.equal(f.controller.running, true);
  active.resolve({ ...f.controller.run, state: "ready", lastSequence: 2 }); await optimizing;
  assert.equal(f.controller.run.id, id); assert.equal(f.controller.run.state, "ready");
  assert.equal(f.controller.stopping, false); assert.equal(f.controller.running, false);
});

test("verification details arrive during setup before any run exists", async () => {
  const pending = deferred(); let entered;
  const started = new Promise(resolve => { entered = resolve; });
  const f = fixture({ previewOptimizationSetup: async (_id, _request, receive) => {
    receive({ phase: "verifying_file", subject: "model.safetensors", completed: 512, total: 1024, unit: "bytes" });
    entered(); return pending.promise;
  } });
  await f.controller.ensure(); const saving = f.controller.save(); await started;
  assert.equal(f.controller.saving, true); assert.equal(f.controller.run, undefined);
  assert.equal(f.controller.liveProgress.subject, "model.safetensors");
  pending.reject(new Error("fixture complete")); await saving;
});
