import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { readFile, access } from "node:fs/promises";
import { dirname } from "node:path";
import { ManagedOptimizationSetup } from "../dist/evidence/managed-optimization-setup.js";
import { setupFixture } from "./optimization-setup-fixture.mjs";

test("optimization selection uses project-owned paths and cannot execute or pass extra data", async () => {
  const f = setupFixture(), calls = [];
  const backend = new ManagedOptimizationSetup({ open: async id => { assert.equal(id, f.projectId); return f.workspace; }, command: async args => { calls.push(args); return args[2] === "list" ? [] : f.preview; } });
  const selection = { modelId: f.model.id, datasetVersionId: f.version.id, benchmarkVersionId: f.benchmark.id };
  assert.deepEqual(await backend.list(f.projectId), []);
  assert.deepEqual(await backend.preview(f.projectId, selection), f.preview);
  assert.deepEqual(calls, [["optimization-setup", "owned-project", "list"], ["optimization-setup", "owned-project", "preview", "--model", f.model.id, "--dataset-version", f.version.id, "--benchmark-version", f.benchmark.id]]);
  for (const invalid of [{ ...selection, folder: "other" }, { ...selection, apiKey: "secret" }, { ...selection, start: true }, { ...selection, modelId: "../other" }]) await assert.rejects(() => backend.preview(f.projectId, invalid));
  assert.equal(calls.length, 2);
});
test("optimization preview rejects stale baseline and substituted artifact responses", async () => {
  const f = setupFixture(), selection = { modelId: f.model.id, datasetVersionId: f.version.id, benchmarkVersionId: f.benchmark.id };
  for (const alter of [v => v.inputs.projectId = randomUUID(), v => v.inputs.model.id = randomUUID(), v => v.inputs.dataset.id = randomUUID(), v => v.inputs.baselineRevision.fingerprint = "sha256:" + "b".repeat(64), v => v.inputs.benchmark.fingerprint = "sha256:" + "b".repeat(64)]) {
    const preview = structuredClone(f.preview); alter(preview);
    await assert.rejects(() => new ManagedOptimizationSetup({ open: async () => f.workspace, command: async () => preview }).preview(f.projectId, selection));
  }
});
test("optimization save keeps the exact retry request, cleans temp files and rejects injected settings", async () => {
  const f = setupFixture(), calls = [], files = [], request = { id: randomUUID(), expectedParent: null, inputs: f.inputs };
  let fail = true;
  const backend = new ManagedOptimizationSetup({ open: async () => f.workspace, exclusive: async (id, work) => { assert.equal(id, f.projectId); return work(); }, command: async args => {
    assert.deepEqual(args.slice(0, 4), ["optimization-setup", "owned-project", "save", "--file"]);
    files.push(args[4]); const data = JSON.parse(await readFile(args[4], "utf8")); calls.push(data);
    if (fail) { fail = false; throw new Error("Lost save response"); } return f.saved(data);
  } });
  await assert.rejects(() => backend.save(f.projectId, request));
  await backend.save(f.projectId, request);
  assert.deepEqual(calls, [request, request]);
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
  for (const change of [v => v.inputs.projectId = randomUUID(), v => v.inputs.dataset.rows = [], v => v.inputs.model.apiKey = "secret", v => v.inputs.dataset.number = 0, v => v.execute = true]) {
    const invalid = structuredClone(request); change(invalid); await assert.rejects(() => backend.save(f.projectId, invalid));
  }
  assert.equal(calls.length, 2);
});
