import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { readWorkspace, readProjectContent } from "../dist/evidence/read-workspace.js";
import { experimentFixture, writeExperimentDatabase } from "./fixtures/experiment.mjs";

const folder = () => mkdtempSync(join(tmpdir(), "encoder-gym-evidence-"));
test("a folder and a valid empty journal are usable before any experiment exists", () => {
  const root = folder();
  assert.deepEqual(readProjectContent(root), { state: "empty", databases: [] });
  writeExperimentDatabase(join(root, "experiments.db"));
  assert.deepEqual(readProjectContent(root), { state: "empty", databases: ["experiments.db"] });
});
test("a registered baseline without runs remains visible without fabricating results", () => {
  const root = folder();
  writeExperimentDatabase(join(root, "experiments.db"), experimentFixture(), false);
  const result = readProjectContent(root);
  assert.equal(result.state, "ready");
  assert.equal(result.workspace.baseline.key, "models/support-encoder");
  assert.equal(result.workspace.runs.length, 0);
  assert.deepEqual(result.workspace.baselineEvaluations, []);
});
test("non-Nomos task, model identity, primary metric and safe parameters come from records", () => {
  const root = folder();
  const file = join(root, "arbitrary-experiment-name.sqlite3");
  writeExperimentDatabase(file, experimentFixture());
  const before = readFileSync(file);
  const result = readWorkspace(root);
  assert.equal(result.name, "support encoder");
  assert.equal(result.task, "text_classification");
  assert.equal(result.runs[0].primaryMetric, "macro_f1");
  assert.equal(result.runs[0].directions.loss, "lower_is_better");
  assert.equal(result.runs[0].candidates[0].parameters.head_width, 64);
  assert.ok(!JSON.stringify(result).includes("DO-NOT-PROJECT"));
  assert.deepEqual(readFileSync(file), before, "reading evidence must leave its database unchanged");
});
test("missing, corrupt and unsupported folders are distinguishable from an empty project", () => {
  const root = folder();
  assert.match(readProjectContent(join(root, "missing")).message, /no longer available/);
  const file = join(root, "foreign.db");
  const db = new DatabaseSync(file); db.exec("CREATE TABLE unrelated (value TEXT)"); db.close();
  assert.match(readProjectContent(root).message, /No supported/);
  writeFileSync(file, "not a database");
  assert.equal(readProjectContent(root).state, "error");
});
test("a candidate report from another model or project is rejected", () => {
  for (const tamper of [f => f.events[3].event.report.model.fingerprint = "foreign-model", f => f.events[3].event.report.project_snapshot_id = "foreign-project"]) {
    const root = folder(), f = experimentFixture();
    // Fixture objects intentionally share references; clone the report before tampering.
    f.events[3].event.report = structuredClone(f.events[3].event.report);
    tamper(f);
    writeExperimentDatabase(join(root, "experiments.db"), f);
    assert.equal(readProjectContent(root).state, "error");
  }
});
