import { DatabaseSync } from "node:sqlite";

/** Deterministic, synthetic normalized records. Never native rows or real models. */
export function experimentFixture(prefix = "support") {
  const fp = value => "sha256:" + value.repeat(64);
  const model = { id: `${prefix}-baseline`, key: `models/${prefix}-encoder`, format: "test-checkpoint", bytes: 1024, fingerprint: fp("1") };
  const outputModel = { ...model, id: `${prefix}-model`, key: `models/${prefix}-candidate`, fingerprint: fp("2") };
  const project = { id: `${prefix}-project`, name: `${prefix} encoder`, task: "text_classification", fingerprint: fp("3"), source_revision: "fixture-revision", baseline_model: model, backend: { name: "deterministic-fixture" }, task_configuration: {}, inputs: [{ role: "training", key: "training/support.jsonl", bytes: 1024, fingerprint: fp("4") }], created_at: "2026-09-01T12:00:00Z" };
  const report = { id: `${prefix}-baseline-report`, project_snapshot_id: project.id, project_snapshot_fingerprint: project.fingerprint, evidence_role: "development", suite_key: "validation", suite_fingerprint: fp("5"), metric_contract_fingerprint: fp("6"), fingerprint: fp("7"), model, metrics: { accuracy: 0.80, macro_f1: 0.75, loss: 0.32 } };
  const candidateReport = { ...report, id: `${prefix}-candidate-report`, fingerprint: fp("8"), model: outputModel, metrics: { accuracy: 0.84, macro_f1: 0.79, loss: 0.25 } };
  const candidate = { id: `${prefix}-candidate`, sequence: 1, parameters: { epochs: 2, learning_rate: 0.0002, head_width: 64, api_key: "DO-NOT-PROJECT", native_payload: { text: "DO-NOT-PROJECT" } } };
  const protocol = { id: `${prefix}-protocol`, fingerprint: fp("9"), project_snapshot_id: project.id, project_snapshot_fingerprint: project.fingerprint, baseline_development_report: report, additional_baseline_development_reports: [], candidates: [candidate], metric_contract: { fingerprint: fp("6"), primary_metric: "macro_f1", definitions: [{ key: "accuracy", direction: "higher_is_better" }, { key: "macro_f1", direction: "higher_is_better" }, { key: "loss", direction: "lower_is_better" }] }, budget: { maximum_candidates: 1, maximum_training_seconds: 60, maximum_development_evaluations: 1, maximum_sealed_evaluations: 1 } };
  const assessment = { id: `${prefix}-assessment`, baseline_report_id: report.id, candidate_report_id: candidateReport.id, evidence_role: "development", verdict: "failed", gates: [{ key: "macro_f1", baseline: 0.75, candidate: 0.79, direction_adjusted_improvement: 0.04, condition: { kind: "minimum_improvement", value: 0.05 }, passed: false }] };
  const events = [
    { kind: "run_created" },
    { kind: "candidate_training_started", candidate_id: candidate.id },
    { kind: "candidate_training_completed", candidate_id: candidate.id, output: { model: outputModel, duration_seconds: 12 } },
    { kind: "candidate_development_completed", candidate_id: candidate.id, report: candidateReport, assessment },
    { kind: "development_selected", candidate_id: null },
    { kind: "finalized", decision: "retain_baseline" },
  ].map((event, i) => ({ event, run_id: `${prefix}-run`, protocol_id: protocol.id, protocol_fingerprint: protocol.fingerprint, sequence: i + 1, fingerprint: `${prefix}-event-${i}`, previous_event_fingerprint: i ? `${prefix}-event-${i - 1}` : null, created_at: `2026-09-02T12:0${i}:00Z` }));
  return { project, protocol, events };
}
export function writeExperimentDatabase(file, fixture, includeEvents = true) {
  const db = new DatabaseSync(file);
  try {
    for (const table of ["encoder_experiment_projects", "encoder_experiment_protocols", "encoder_experiment_events"]) db.exec(`CREATE TABLE ${table} (artifact_json TEXT NOT NULL)`);
    if (fixture) {
      db.prepare("INSERT INTO encoder_experiment_projects VALUES (?)").run(JSON.stringify(fixture.project));
      db.prepare("INSERT INTO encoder_experiment_protocols VALUES (?)").run(JSON.stringify(fixture.protocol));
      if (includeEvents) for (const event of fixture.events) db.prepare("INSERT INTO encoder_experiment_events VALUES (?)").run(JSON.stringify(event));
    }
  } finally { db.close(); }
}
