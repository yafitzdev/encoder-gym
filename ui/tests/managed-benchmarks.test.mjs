import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { ManagedBenchmarks } from "../dist/evidence/managed-benchmarks.js";

function fixture() {
  const projectId = randomUUID(), baselineId = randomUUID(), candidateId = randomUUID(), revisionId = randomUUID();
  const version = { id: randomUUID(), projectId, fingerprint: "sha256:" + "a".repeat(64), definition: { suites: [{ key: "development", role: "development" }, { key: "holdout", role: "sealed_acceptance" }] } };
  const workspace = { folder: "owned-project", manifest: { id: projectId }, benchmarkVersions: [version], modelCatalog: {
    artifacts: [{ id: baselineId }, { id: candidateId }], baselineRevisions: [{ id: revisionId, modelArtifactId: baselineId }], activeBaselineRevisionId: revisionId,
  } };
  const results = { version, models: [
    { modelId: baselineId, isBaseline: true, reports: [{ result: { suite_key: "development", metrics: { mrr: 0.7 } }, contexts: [{ assessment: null }] }] },
    { modelId: candidateId, isBaseline: false, reports: [] },
  ] };
  return { projectId, version, workspace, results };
}
test("benchmark reads use fixed project scope and cannot execute evaluation or supply paths", async () => {
  const { projectId, version, workspace, results } = fixture(), calls = [];
  const backend = new ManagedBenchmarks({ open: async id => { assert.equal(id, projectId); return workspace; }, command: async args => { calls.push(args); return args[2] === "list" ? [version] : results; } });
  assert.deepEqual(await backend.query(projectId, { kind: "list" }), { kind: "list", versions: [version] });
  assert.deepEqual(await backend.query(projectId, { kind: "results", versionId: version.id }), { kind: "results", results });
  assert.deepEqual(calls, [["benchmark", "owned-project", "list"], ["benchmark", "owned-project", "results", version.id]]);
  for (const request of [{ kind: "list", folder: "other" }, { kind: "results", versionId: "../other" }, { kind: "results", versionId: randomUUID() }, { kind: "execute", versionId: version.id }, { kind: "results", versionId: version.id, role: "sealed_acceptance" }]) await assert.rejects(() => backend.query(projectId, request));
  assert.equal(calls.length, 2);
});
test("benchmark responses reject foreign versions, substituted models and protected scores", async () => {
  const { projectId, version, workspace, results } = fixture();
  const response = async value => new ManagedBenchmarks({ open: async () => workspace, command: async () => value }).query(projectId, { kind: "results", versionId: version.id });
  for (const alter of [
    v => { v.version.projectId = randomUUID(); },
    v => { v.version.fingerprint = "changed"; },
    v => { v.models[1].modelId = randomUUID(); },
    v => { v.models[1].isBaseline = true; },
    v => { v.models.push(v.models[0]); },
    v => { v.models[0].reports[0].result.suite_key = "holdout"; },
    v => { v.models[0].reports[0].result.metrics.mrr = Infinity; },
    v => { v.models[0].reports[0].contexts[0].assessment = { evidence_role: "sealed_acceptance" }; },
  ]) { const changed = structuredClone(results); alter(changed); await assert.rejects(() => response(changed)); }
  const backend = new ManagedBenchmarks({ open: async () => workspace, command: async () => [{ ...version, projectId: randomUUID() }] });
  await assert.rejects(() => backend.query(projectId, { kind: "list" }));
});
