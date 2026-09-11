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
  return { ...base, setupId, scope, preview, authorization };
}
function ports(f, command, exclusive = async (_id, work) => work()) {
  return { open: async id => { assert.equal(id, f.projectId); return f.workspace; }, command, exclusive };
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
