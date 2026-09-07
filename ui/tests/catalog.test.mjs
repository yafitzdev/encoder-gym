import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { candidateRows, comparisonGroups, developmentStatus, initialFilter, setupId, score, delta } from "../dist/evidence/catalog.js";

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
  assert.equal(comparisonGroups(snapshot, { ...initialFilter(), query: "repair fine-tune" }).flatMap(g => g.rows).length, 1);
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
});
