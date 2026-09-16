import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { parseAgentFinalScope, parseAgentFinalView } from "../dist/evidence/optimization-final.js";
import { finalFixture } from "./optimization-final-fixture.mjs";

test("final custody distinguishes authorization, consumed unknown outcome and completed evidence", () => {
  const f = finalFixture();
  assert.equal(parseAgentFinalView(null, f.projectId, f.runId), null);
  for (const state of ["authorized", "outcome_unknown", "completed"]) {
    assert.deepEqual(parseAgentFinalView(f.view(state), f.projectId, f.runId), f.view(state));
  }
  const rejected = f.view(); rejected.execution.result.accepted = false;
  assert.deepEqual(parseAgentFinalView(rejected, f.projectId, f.runId), rejected);
});

test("final boundary rejects foreign ownership, protected payloads and malformed scope", () => {
  const f = finalFixture();
  for (const change of [
    s => { s.run.id = randomUUID(); }, s => { s.dataset.projectId = randomUUID(); },
    s => { s.sealedScores = { mrr: 99 }; }, s => { s.model.path = "private-model"; },
    s => { s.apiKey = "private-value"; }, s => { s.finalSuite = "\nsealed-final"; },
    s => { s.maximumEvaluationSeconds = 0; }, s => { s.maximumEvaluationSeconds = Infinity; },
    s => { s.dataset.number = 1.5; }, s => { s.executionHead = "unverified"; },
    s => { delete s.baselineFinalReport; }, s => { s.adaptiveClosedAt = "not-a-time"; },
  ]) {
    const scope = structuredClone(f.scope); change(scope);
    assert.throws(() => parseAgentFinalScope(scope, f.projectId, f.runId));
  }
});

test("final custody rejects substituted links, incomplete evidence and impossible ordering", () => {
  const f = finalFixture();
  for (const change of [
    v => { v.state = "authorized"; }, v => { v.execution.result.metrics = { mrr: 99 }; },
    v => { v.execution.authorization.createdAt = f.time(-1); },
    v => { v.execution.dispatch.authorization.id = randomUUID(); },
    v => { v.execution.dispatch.authorization.fingerprint = "sha256:" + "b".repeat(64); },
    v => { v.execution.dispatch.reportId = v.execution.dispatch.assessmentId; },
    v => { v.execution.dispatch.createdAt = f.time(0); },
    v => { v.execution.result.dispatch.id = randomUUID(); },
    v => { v.execution.result.report.id = randomUUID(); },
    v => { v.execution.result.assessment.id = randomUUID(); },
    v => { v.execution.result.accepted = "true"; },
    v => { v.execution.result.reportCreatedAt = f.time(1); },
    v => { v.execution.result.createdAt = f.time(2); },
    v => { v.execution.dispatch = null; },
  ]) {
    const view = f.view(); change(view);
    assert.throws(() => parseAgentFinalView(view, f.projectId, f.runId));
  }
});
