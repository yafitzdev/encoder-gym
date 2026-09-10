import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { ManagedDatasets } from "../dist/evidence/managed-datasets.js";

test("dataset requests use a fixed project folder and reject renderer paths, commands and invalid pages", async () => {
  const projectId = randomUUID(), versionId = randomUUID(), calls = [];
  const backend = new ManagedDatasets({
    open: async id => { assert.equal(id, projectId); return { folder: "managed-project", manifest: { id: projectId } }; },
    command: async args => { calls.push(args); return { versionId, offset: 5, total: 10, rows: [] }; },
    exclusive: async (_id, run) => run(),
  });
  assert.equal((await backend.query(projectId, { kind: "rows", versionId, offset: 5, limit: 10 })).kind, "rows");
  assert.deepEqual(calls[0], ["dataset", "managed-project", "rows", versionId, "--offset", "5", "--limit", "10"]);
  for (const query of [
    { kind: "rows", versionId, offset: -1, limit: 10 },
    { kind: "changes", versionId, offset: 0, limit: 51 },
    { kind: "rows", versionId: "../other", offset: 0, limit: 10 },
    { kind: "list", folder: "other-project" },
    { kind: "execute", command: "anything" },
  ]) await assert.rejects(() => backend.query(projectId, query));
  assert.equal(calls.length, 1);
});

test("dataset revisions preserve retry IDs, serialize no renderer path, and delete temporary request files", async () => {
  const projectId = randomUUID(), datasetId = randomUUID(), versionId = randomUUID(), parentId = randomUUID();
  let temporary, written, calls = 0;
  const backend = new ManagedDatasets({
    open: async () => ({ folder: "managed-project", manifest: { id: projectId } }),
    exclusive: async (id, work) => { assert.equal(id, projectId); return work(); },
    command: async args => {
      calls++; assert.deepEqual(args.slice(0, 4), ["dataset", "managed-project", "revise", "--file"]);
      temporary = args[4]; written = JSON.parse(readFileSync(temporary, "utf8"));
      return { actionId: randomUUID(), version: { id: versionId, datasetId, projectId }, rows: 1 };
    },
  });
  const request = { kind: "revise", datasetId, versionId, parentId, added: [], removed: ["sha256:" + "a".repeat(64)], replaced: [] };
  await backend.mutate(projectId, request);
  assert.equal(written.versionId, versionId); assert.equal(written.parentId, parentId); assert.equal(written.kind, undefined);
  assert.equal(existsSync(temporary), false);
  await backend.mutate(projectId, request);
  assert.equal(written.versionId, versionId); assert.equal(existsSync(temporary), false);
  await assert.rejects(() => backend.mutate(projectId, { ...request, folder: "foreign" }));
  await assert.rejects(() => backend.mutate(projectId, { ...request, added: [{ importId: randomUUID(), record: 0 }] }));
  assert.equal(calls, 2);
});

test("dataset responses cannot substitute another project or version", async () => {
  const projectId = randomUUID(), versionId = randomUUID(), datasetId = randomUUID();
  const backend = new ManagedDatasets({
    open: async () => ({ folder: "managed-project", manifest: { id: projectId } }),
    exclusive: async (_id, work) => work(),
    command: async args => args[2] === "list" ? [{ dataset: { projectId: randomUUID() }, versions: [] }] : { versionId: randomUUID(), offset: 0, version: { id: versionId, datasetId, projectId: randomUUID() } },
  });
  await assert.rejects(() => backend.query(projectId, { kind: "list" }));
  await assert.rejects(() => backend.query(projectId, { kind: "rows", versionId, offset: 0, limit: 10 }));
  await assert.rejects(() => backend.mutate(projectId, { kind: "fork", datasetId, versionId, parentId: randomUUID(), name: "Variant" }));
});
