import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { OptimizationSetupController } from "../dist/evidence/optimization-setup-controller.js";
import { setupFixture } from "./optimization-setup-fixture.mjs";
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
function fixture(overrides = {}) {
  const f = setupFixture(), history = [], writes = [];
  const bridge = { queryDatasets: async () => ({ kind: "list", entries: f.datasets }), queryBenchmarks: async () => ({ kind: "list", versions: f.workspace.benchmarkVersions }),
    optimizationSetups: async () => [...history], previewOptimizationSetup: async () => f.preview,
    saveOptimizationSetup: async (_, request) => { writes.push(structuredClone(request)); const result = f.saved(request); history.push(result.setup); return result; }, ...overrides };
  const controller = new OptimizationSetupController(f.projectId, f.workspace, bridge, () => {});
  return { ...f, controller, bridge, history, writes };
}
test("first setup uses model provenance; saved versions never follow latest dataset or benchmark", async () => {
  const f = fixture(); await f.controller.ensure(); assert.equal(f.controller.datasetId, f.version.id); assert.ok(f.controller.canSave);
  await f.controller.save(); assert.ok(f.controller.saved); assert.equal(f.writes.length, 1); await f.controller.save(); assert.equal(f.writes.length, 1);
  f.workspace.benchmarkVersions.push({ ...f.benchmark, id: randomUUID(), number: 2 });
  f.datasets[0].versions.push({ version: { ...f.version, id: randomUUID(), number: 2 }, rows: 11 });
  f.controller.refresh(); await f.controller.ensure();
  assert.equal(f.controller.datasetId, f.version.id); assert.equal(f.controller.benchmarkId, f.benchmark.id); assert.ok(f.controller.saved);
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
