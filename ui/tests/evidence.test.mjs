import assert from "node:assert/strict";
import { test } from "node:test";
import { projectRun } from "../dist/evidence/read-workspace.js";

function fixture() {
  const model = { id: "base", key: "models/base", format: "test", bytes: 4, fingerprint: "base-fp" };
  const report = { id: "baseline-report", evidence_role: "development", suite_key: "generic", suite_fingerprint: "suite", metric_contract_fingerprint: "contract", fingerprint: "report-fp", metrics: { mrr: 0.9 }, model };
  const protocol = { id: "protocol", fingerprint: "protocol-fp", project_snapshot_id: "project", project_snapshot_fingerprint: "project-fp", baseline_development_report: report, additional_baseline_development_reports: [], candidates: [{ id: "candidate", sequence: 1, parameters: { epochs: 1, secret: "never-project-this" } }], metric_contract: { definitions: [{ key: "mrr", direction: "higher_is_better" }] }, budget: { maximum_candidates: 1, maximum_training_seconds: 20, maximum_development_evaluations: 1, maximum_sealed_evaluations: 1 } };
  const project = { id: "project", fingerprint: "project-fp", source_revision: "revision", inputs: [], task_configuration: {} };
  const assessment = { id: "assessment", baseline_report_id: "baseline-report", candidate_report_id: "candidate-report", evidence_role: "development", verdict: "failed", gates: [{ key: "mrr", baseline: 0.9, candidate: 0.9001, direction_adjusted_improvement: 0.0001, condition: { kind: "minimum_improvement", value: 0.0005 }, passed: false }] };
  const events = [
    { kind: "run_created" },
    { kind: "candidate_development_completed", candidate_id: "candidate", report: { ...report, id: "candidate-report", metrics: { mrr: 0.9001 } }, assessment },
    { kind: "sealed_completed", candidate_id: "candidate", report: { metrics: { secret_sealed_metric: 0.123456789 } }, assessment: { verdict: "failed", gates: [{ key: "mrr", passed: false, candidate: 0.123456789 }] } },
    { kind: "finalized", decision: "retain_baseline" },
  ].map((event, i) => ({ event, run_id: "run", protocol_id: "protocol", protocol_fingerprint: "protocol-fp", sequence: i + 1, fingerprint: `event-${i}`, previous_event_fingerprint: i ? `event-${i - 1}` : null, created_at: `2026-09-02T10:0${i}:00Z` }));
  return { project, protocol, events };
}

test("projection preserves a failed gate even when its score improves", () => {
  const f = fixture();
  const r = projectRun(f.protocol, f.project, f.events, "test.sqlite");
  assert.equal(r.candidates[0].development[0].verdict, "failed");
  assert.equal(r.candidates[0].development[0].checks[0].passed, false);
  assert.equal(r.candidates[0].development[0].checks[0].improvement, 0.0001);
});

test("sealed scores and arbitrary parameters never reach the read model", () => {
  const f = fixture();
  const r = projectRun(f.protocol, f.project, f.events, "test.sqlite");
  assert.deepEqual(r.acceptance, { state: "failed", candidateId: "candidate", failedMetrics: ["mrr"] });
  assert.ok(!JSON.stringify(r).includes("0.123456789"));
  assert.ok(!JSON.stringify(r).includes("never-project-this"));
});

test("foreign baseline, suite, role, and journal identities fail closed", () => {
  for (const tamper of [
    f => f.events[1].event.assessment.baseline_report_id = "foreign",
    f => f.events[1].event.report.suite_fingerprint = "foreign",
    f => f.events[1].event.report.metric_contract_fingerprint = "foreign",
    f => f.events[1].event.report.evidence_role = "sealed_acceptance",
    f => f.events[1].previous_event_fingerprint = "foreign",
    f => f.events[1].sequence = 5,
    f => f.protocol.project_snapshot_fingerprint = "foreign",
  ]) {
    const f = fixture(); tamper(f);
    assert.throws(() => projectRun(f.protocol, f.project, f.events, "test.sqlite"));
  }
});

test("missing development evidence remains missing", () => {
  const f = fixture();
  const r = projectRun(f.protocol, f.project, f.events.slice(0, 1), "test.sqlite");
  assert.deepEqual(r.candidates[0].development, []);
  assert.equal(r.decision, undefined);
  assert.equal(r.acceptance.state, "unused");
});
