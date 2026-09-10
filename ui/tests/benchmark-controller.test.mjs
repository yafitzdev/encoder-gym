import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { BenchmarkController } from "../dist/evidence/benchmark-controller.js";
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const version = () => ({ id: randomUUID(), number: 1 });
function controller(bridge) {
  const locations = [];
  const instance = new BenchmarkController(randomUUID(), { folder: "owned-project" }, bridge, { render() {}, navigate(location) { locations.push(location); }, dialog() { throw new Error("No dialog in this test"); } });
  return { instance, locations };
}
test("benchmark reads deduplicate pending work and isolate results/errors by version", async () => {
  const one = version(), two = version(), pending = deferred(), calls = [];
  const { instance } = controller({ queryBenchmarks: async (_, request) => {
    calls.push(request);
    return request.versionId === one.id ? pending.promise : { kind: "results", results: { version: two, models: [] } };
  } });
  instance.versions = [one, two];
  const first = instance.ensure({ page: "benchmarks", id: one.id });
  await instance.ensure({ page: "benchmarks", id: one.id });
  await instance.ensure({ page: "benchmarks", id: two.id });
  pending.reject(new Error("Earlier version unavailable")); await first;
  assert.equal(calls.length, 2);
  assert.equal(instance.failure(two.id), undefined); assert.ok(instance.result(two.id));
  assert.ok(instance.failure(one.id)); assert.equal(instance.result(one.id), undefined);
});
test("moving/reopening a project fences late responses and refreshes benchmark versions", async () => {
  const one = version(), pending = deferred(); let calls = 0;
  const { instance } = controller({ queryBenchmarks: async () => ++calls === 1 ? pending.promise : { kind: "list", versions: [one] } });
  const opening = instance.load();
  instance.sync({ folder: "moved-project" });
  await instance.ensure({ page: "benchmarks" });
  pending.reject(new Error("Old path disappeared")); await opening;
  assert.deepEqual(instance.versions, [one]); assert.equal(instance.error, undefined); assert.equal(instance.loading, false);
});

test("an explicit refresh abandons the old save retry instead of mutating on a later read error", async () => {
  const one = version(); let writes = 0;
  const { instance } = controller({
    adoptBenchmark: async () => { writes++; throw new Error("Save failed"); },
    queryBenchmarks: async (_, request) => { if (request.kind === "list") return { kind: "list", versions: [one] }; throw new Error("Read failed"); },
  });
  await instance.adopt({ runId: randomUUID(), expectedParent: null, definitionFingerprint: "sha256:" + "a".repeat(64) });
  instance.refresh(); await instance.ensure({ page: "benchmarks" }); await instance.ensure({ page: "benchmarks" });
  assert.ok(instance.failure(one.id)); await instance.retry(); assert.equal(writes, 1);
});
for (const failure of ["save", "reload"]) test(`benchmark retries preserve the reviewed definition after a lost ${failure} response`, async () => {
  const saved = version(), requests = [], pending = deferred(); let reads = 0;
  const { instance, locations } = controller({
    adoptBenchmark: async (_, request) => { requests.push(structuredClone(request)); if (requests.length === 1) await pending.promise; return { version: saved }; },
    queryBenchmarks: async () => { if (failure === "reload" && ++reads === 1) throw new Error("Reload failed"); return { kind: "list", versions: [saved] }; },
  });
  const request = { runId: randomUUID(), expectedParent: null, definitionFingerprint: "sha256:" + "a".repeat(64) };
  const saving = instance.adopt(request); await instance.adopt(request); assert.equal(requests.length, 1);
  if (failure === "save") pending.reject(new Error("Response lost after commit")); else pending.resolve();
  await saving; assert.ok(instance.error); assert.equal(locations.length, 0);
  await instance.retry(); assert.equal(instance.error, undefined); assert.equal(instance.saving, false);
  assert.deepEqual(requests, [request, request]); assert.deepEqual(locations, [{ page: "benchmarks", id: saved.id, tab: "results" }]);
});
