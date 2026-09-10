import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { DatasetController } from "../dist/evidence/dataset-controller.js";

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const entry = projectId => {
  const datasetId = randomUUID(), version = { id: randomUUID(), datasetId, projectId, number: 1, fingerprint: "sha256:" + "a".repeat(64) };
  return { dataset: { id: datasetId, projectId, name: "Base dataset", origin: null }, versions: [{ version, rows: 300 }] };
};
function controller(projectId, bridge) {
  const navigation = [];
  const instance = new DatasetController(projectId, { folder: "original", datasets: [] }, bridge, {
    render() {}, imported() {}, navigate(location) { navigation.push(location); },
    dialog() { throw new Error("This test should not open a dialog"); }, copy() {},
  });
  return { instance, navigation };
}

test("dataset inspection deduplicates pending pages and ignores a late error after refresh", async () => {
  const projectId = randomUUID(), dataset = entry(projectId), pending = deferred();
  let calls = 0;
  const { instance } = controller(projectId, { queryDatasets: async (id, query) => {
    assert.equal(id, projectId); calls++;
    if (query.kind === "list") return { kind: "list", entries: [dataset] };
    return pending.promise;
  } });
  instance.entries = [dataset];
  const location = { page: "dataset", id: dataset.versions[0].version.id };
  const first = instance.ensure(location);
  await instance.ensure(location);
  assert.equal(calls, 1);
  instance.refresh(); await instance.ensure({ page: "datasets" });
  pending.reject(new Error("Old page failed")); await first;
  assert.equal(instance.error, undefined); assert.equal(instance.page(location), undefined);
  assert.deepEqual(instance.entries, [dataset]);
});

test("moving a project invalidates its failed reads and fences the previous folder's responses", async () => {
  const projectId = randomUUID(), current = entry(projectId), pending = deferred();
  let calls = 0;
  const { instance } = controller(projectId, { queryDatasets: async () => ++calls === 1 ? pending.promise : { kind: "list", entries: [current] } });
  const beforeMove = instance.load();
  instance.sync({ folder: "moved", datasets: [] });
  assert.equal(instance.loading, false);
  await instance.ensure({ page: "datasets" });
  pending.reject(new Error("Cannot read original folder")); await beforeMove;
  assert.equal(instance.error, undefined); assert.deepEqual(instance.entries, [current]);
  instance.error = new Error("Folder unavailable");
  instance.sync({ folder: "moved-again", datasets: [] });
  assert.equal(instance.error, undefined); assert.equal(instance.entries, undefined);
});

for (const failure of ["mutation response", "collection read"]) test(`retrying a lost ${failure} reuses the exact dataset version without importing again`, async () => {
  const projectId = randomUUID(), pending = deferred(), requests = [];
  const source = { id: randomUUID(), purpose: "training", rows: 1, artifact: { fingerprint: "sha256:" + "b".repeat(64) } };
  let choices = 0, imports = 0, reads = 0, saved;
  const { instance, navigation } = controller(projectId, {
    chooseDataset: async () => { choices++; return pending.promise; },
    importDataset: async () => { imports++; return { content: { state: "ready", workspace: { managed: { folder: "original", datasets: [source] } } } }; },
    mutateDataset: async (id, request) => {
      assert.equal(id, projectId); requests.push(structuredClone(request));
      saved = { id: request.versionId, datasetId: request.datasetId, projectId, number: 1, fingerprint: "sha256:" + "c".repeat(64) };
      if (failure === "mutation response" && requests.length === 1) throw new Error("Response lost after commit");
      return { actionId: randomUUID(), version: saved, rows: 1 };
    },
    queryDatasets: async () => {
      if (failure === "collection read" && ++reads === 1) throw new Error("Collection read failed after commit");
      return { kind: "list", entries: [{ dataset: { id: saved.datasetId, projectId, name: "Base dataset" }, versions: [{ version: saved, rows: 1 }] }] };
    },
  });
  instance.entries = [];
  const importing = instance.importRows();
  await instance.importRows();
  assert.equal(choices, 1); assert.equal(instance.saving, true);
  pending.resolve({ rows: 1, source: "train.jsonl", artifact: source.artifact, token: "checked-source" });
  await importing;
  assert.ok(instance.error); assert.equal(instance.saving, false); assert.equal(navigation.length, 0);
  await instance.retry();
  assert.equal(instance.error, undefined); assert.equal(instance.saving, false);
  assert.equal(imports, 1); assert.equal(choices, 1); assert.equal(requests.length, 2);
  assert.deepEqual(requests[0], requests[1]);
  assert.deepEqual(navigation, [{ page: "dataset", id: saved.id, tab: "rows" }]);
});

test("an unknown edit target is rejected before picking or importing a file", async () => {
  const projectId = randomUUID(); let choices = 0;
  const { instance } = controller(projectId, { chooseDataset: async () => { choices++; } });
  instance.entries = [entry(projectId)];
  await instance.importRows(randomUUID());
  assert.match(instance.error.message, /version not found/); assert.equal(choices, 0);
  await instance.importRows();
  assert.match(instance.error.message, /Open a dataset version/); assert.equal(choices, 0);
});

test("dataset page caches stay bounded and separate for each project", async () => {
  const projectId = randomUUID(), dataset = entry(projectId), versionId = dataset.versions[0].version.id;
  const { instance } = controller(projectId, { queryDatasets: async (id, query) => {
    assert.equal(id, projectId);
    return { kind: "rows", page: { versionId, offset: query.offset, total: 300, rows: [] } };
  } });
  instance.entries = [dataset];
  for (let offset = 0; offset < 300; offset += 25) await instance.ensure({ page: "dataset", id: versionId, offset });
  assert.equal(instance.page({ page: "dataset", id: versionId, offset: 0 }), undefined);
  assert.equal(instance.page({ page: "dataset", id: versionId, offset: 275 }).page.offset, 275);
  const { instance: other } = controller(randomUUID(), {});
  assert.equal(other.entries, undefined); assert.equal(other.page({ page: "dataset", id: versionId, offset: 275 }), undefined);
});
