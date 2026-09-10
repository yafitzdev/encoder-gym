import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { candidateName, candidateRows, comparisonGroups, developmentStatus, evaluationGroups, initialFilter, setupId, score, delta, summaryMetrics } from "../dist/evidence/catalog.js";
import { projectRun } from "../dist/evidence/read-workspace.js";
import { experimentFixture } from "./fixtures/experiment.mjs";

const snapshot = JSON.parse(readFileSync(new URL("../src/evidence/nomos-snapshot.json", import.meta.url), "utf8"));
test("all 15 model identities are present and recovered attempts remain linked", () => {
  const rows = candidateRows(snapshot);
  assert.equal(rows.length, 15);
  assert.equal(snapshot.runs.length, 9);
  const recovered = rows.find(r => r.candidate.id === "63daf605-3d39-4602-b5f2-3fed3971f117");
  assert.equal(recovered.attempts.length, 2);
  assert.ok(recovered.candidate.model);
  assert.equal(recovered.candidate.failure, undefined);
});
test("managed Models exposes only cataloged candidates, not imported historical checkpoints", () => {
  const managed = structuredClone(snapshot);
  const projectId = "00000000-0000-4000-8000-000000000001";
  const baselineId = "00000000-0000-4000-8000-000000000002";
  managed.managed = {
    modelCatalog: {
      projectId,
      activeBaselineRevisionId: "00000000-0000-4000-8000-000000000003",
      baselineRevisions: [{ modelArtifactId: baselineId }],
      artifacts: [{ id: baselineId, projectId, fingerprint: managed.baseline.fingerprint }],
    },
  };
  assert.equal(candidateRows(managed).length, 0);
  assert.equal(evaluationGroups(managed, initialFilter()).flatMap(group => group.rows).length, 15, "historical evaluations remain evidence without becoming managed candidates");

  const row = candidateRows(snapshot).find(item => item.candidate.model);
  assert.ok(row?.candidate.model);
  managed.managed.modelCatalog.artifacts.push({
    id: "00000000-0000-4000-8000-000000000004",
    projectId,
    fingerprint: row.candidate.model.fingerprint,
    producingRun: { id: row.run.id, fingerprint: "sha256:" + "a".repeat(64) },
  });
  assert.deepEqual(candidateRows(managed).map(item => item.candidate.id), [row.candidate.id]);

  managed.managed.modelCatalog.baselineRevisions.push({ modelArtifactId: managed.managed.modelCatalog.artifacts.at(-1).id });
  assert.equal(candidateRows(managed).length, 0, "a former or active baseline is not implicitly a candidate");
});
test("all displayed comparisons share exact baseline, suite, and metric authority within a group", () => {
  const groups = comparisonGroups(snapshot, initialFilter());
  assert.ok(groups.length > 1);
  assert.equal(groups.flatMap(g => g.rows).length, 15);
  for (const g of groups) for (const row of g.rows) assert.equal(setupId(row.run), g.id);
  const changed = structuredClone(snapshot.runs[0]);
  changed.baselines[0].suiteFingerprint = "new-suite";
  assert.notEqual(setupId(changed), setupId(snapshot.runs[0]));
});
test("positive metric delta does not override the persisted failure", () => {
  const row = candidateRows(snapshot).find(r => r.candidate.id === "f972efda-b311-47c1-9ff8-f6969e4903fd");
  assert.equal(developmentStatus(row).key, "rejected");
  assert.ok(row.candidate.development[0].report.metrics.mrr > row.candidate.development[0].baseline.metrics.mrr);
});
test("search, setup and status filters are real and do not alter source data", () => {
  assert.equal(comparisonGroups(snapshot, { ...initialFilter(), query: "6,992" }).flatMap(g => g.rows).length, 1);
  assert.equal(comparisonGroups(snapshot, { ...initialFilter(), query: "does not exist" }).length, 0);
  const passed = comparisonGroups(snapshot, { ...initialFilter(), status: "passed" }).flatMap(g => g.rows);
  assert.equal(passed.length, 2);
  assert.equal(candidateRows(snapshot).length, 15);
});
test("precision, percentages, zero, and missing values remain distinguishable", () => {
  assert.equal(score(undefined, "mrr"), "—");
  assert.equal(score(0, "mrr"), "0.000000");
  assert.equal(score(0.906, "recall_at_2"), "90.60%");
  assert.equal(delta(-0.005, "recall_at_2"), "−0.50 pp");
  assert.equal(delta(-0.000026190476, "mrr"), "−0.000026");
  assert.equal(delta(0.0000000001, "mrr"), "+1.0e-10");
});

test("generic projects use their primary metric and never invent triplet training", () => {
  const f = experimentFixture();
  const run = projectRun(f.protocol, f.project, f.events, "experiment.db");
  assert.deepEqual(summaryMetrics(run), ["macro_f1", "accuracy", "loss"]);
  assert.equal(candidateName(run.candidates[0]), "Candidate 01");
  assert.equal(candidateName({ id: "untouched", sequence: 2, parameters: {}, development: [] }), "Candidate 02");
});
test("sorting follows the declared primary direction and puts missing evidence last", () => {
  const f = experimentFixture();
  const run = projectRun(f.protocol, f.project, f.events, "experiment.db");
  run.primaryMetric = "loss";
  const better = structuredClone(run.candidates[0]);
  better.id = "lower-loss"; better.development[0].report.metrics.loss = 0.1;
  const missing = { id: "missing", sequence: 3, parameters: {}, development: [] };
  run.candidates = [run.candidates[0], missing, better];
  const groups = comparisonGroups({ ...snapshot, runs: [run] }, { ...initialFilter(), sort: "primary" });
  assert.deepEqual(groups[0].rows.map(row => row.candidate.id), ["lower-loss", "support-candidate", "missing"]);
});
test("changing the selected benchmark changes primary-score ordering", () => {
  const f = experimentFixture(), run = projectRun(f.protocol, f.project, f.events, "experiment.db");
  const b = structuredClone(run.candidates[0]); b.id = "alternative";
  b.development[0].report.metrics.macro_f1 = 0.8;
  const extraBaseline = { ...run.baselines[0], id: "second-baseline", suite: "second", suiteFingerprint: "second-fp" };
  run.baselines.push(extraBaseline);
  for (const [candidate, value] of [[run.candidates[0], 0.9], [b, 0.7]]) candidate.development.push({ ...candidate.development[0], baseline: extraBaseline, report: { ...candidate.development[0].report, suite: "second", metrics: { macro_f1: value } } });
  run.candidates.push(b);
  const world = { ...snapshot, runs: [run] }, filter = { ...initialFilter(), sort: "primary" };
  assert.equal(comparisonGroups(world, filter, 0)[0].rows[0].candidate.id, "alternative");
  assert.equal(comparisonGroups(world, filter, 1)[0].rows[0].candidate.id, "support-candidate");
});
