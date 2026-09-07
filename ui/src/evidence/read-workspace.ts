import { DatabaseSync } from "node:sqlite";
import { readdirSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import type { CandidateAttempt, DevelopmentReport, DevelopmentResult, ModelArtifact, RunRecord, WorkspaceSnapshot } from "../workspace.js";

type Json = Record<string, any>;
// This adapter only projects fields. Gate verdicts are owned by the Rust core.
function object(value: unknown, label: string): Json {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Missing ${label}.`);
  return value as Json;
}
function string(value: unknown): string {
  if (typeof value !== "string") throw new Error("Invalid text in experiment evidence.");
  return value;
}
function number(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("Invalid number in experiment evidence.");
  return value;
}
function model(value: unknown): ModelArtifact {
  const m = object(value, "model");
  return { id: string(m.id), key: string(m.key), format: string(m.format), bytes: number(m.bytes), fingerprint: string(m.fingerprint) };
}
function development(value: unknown): DevelopmentReport {
  const r = object(value, "development report");
  if (r.evidence_role !== "development") throw new Error("Only development evidence may enter model comparison.");
  return {
    id: string(r.id), suite: string(r.suite_key), suiteFingerprint: string(r.suite_fingerprint),
    contractFingerprint: string(r.metric_contract_fingerprint), fingerprint: string(r.fingerprint),
    metrics: Object.fromEntries(Object.entries(object(r.metrics, "metrics")).map(([k, v]) => [k, number(v)])),
  };
}

const parameterKeys = ["strategy", "reference_model", "specialist_weight", "batch_size", "device", "epochs", "learning_rate", "loss", "margin", "mining_batch_size", "positive_strategy", "query_strategy", "seed", "repair_base_rows", "repair_delta_rows", "repair_total_rows", "repair_snapshot_id", "repair_snapshot_fingerprint"];

function result(event: Json, baselines: DevelopmentReport[]): DevelopmentResult {
  const report = development(event.report);
  const a = object(event.assessment, "assessment");
  const baseline = baselines.find(b => b.id === a.baseline_report_id);
  if (!baseline || a.candidate_report_id !== report.id || a.evidence_role !== "development" ||
    baseline.suite !== report.suite || baseline.suiteFingerprint !== report.suiteFingerprint ||
    baseline.contractFingerprint !== report.contractFingerprint) throw new Error("Development report has mismatched comparison authority.");
  if (!Array.isArray(a.gates)) throw new Error("Missing recorded checks.");
  return {
    baseline, report, verdict: string(a.verdict), assessmentId: string(a.id),
    checks: a.gates.map((g: Json) => {
      if (typeof g.passed !== "boolean") throw new Error("Missing recorded check verdict.");
      return { metric: string(g.key), baseline: number(g.baseline), candidate: number(g.candidate),
        improvement: number(g.direction_adjusted_improvement), condition: { kind: string(g.condition.kind), value: number(g.condition.value) }, passed: g.passed };
    }),
  };
}

export function projectRun(protocol: Json, project: Json, events: Json[], sourceDatabase: string): RunRecord {
  if (!events.length) throw new Error("Experiment has no journal events.");
  const first = events[0]!;
  if (protocol.project_snapshot_id !== project.id || protocol.project_snapshot_fingerprint !== project.fingerprint) throw new Error("Project does not match protocol.");
  for (let i = 0; i < events.length; i++) {
    const e = events[i]!;
    if (e.sequence !== i + 1 || e.run_id !== first.run_id || e.protocol_id !== protocol.id || e.protocol_fingerprint !== protocol.fingerprint || (i > 0 && e.previous_event_fingerprint !== events[i - 1]!.fingerprint)) throw new Error("Experiment journal is incomplete or inconsistent.");
  }
  const baseline = model(protocol.baseline_development_report.model);
  const rawBaselines = [protocol.baseline_development_report, ...(protocol.additional_baseline_development_reports ?? [])];
  if (rawBaselines.some(r => r.model.fingerprint !== baseline.fingerprint)) throw new Error("Baseline model changes within a protocol.");
  const baselines = rawBaselines.map(development);
  const candidateIds = new Set<string>();
  const candidates: CandidateAttempt[] = protocol.candidates.map((c: Json) => {
    if (candidateIds.has(c.id)) throw new Error("Duplicate candidate identity.");
    candidateIds.add(c.id);
    const ce = events.map(e => e.event).filter(e => e.candidate_id === c.id);
    const trained = ce.find(e => e.kind === "candidate_training_completed");
    const failed = ce.findLast(e => e.kind === "candidate_failed");
    const reports = ce.filter(e => ["candidate_development_completed", "candidate_development_suite_completed"].includes(e.kind));
    if (new Set(reports.map(e => e.report.suite_key)).size !== reports.length) throw new Error("Duplicate development report.");
    return {
      id: string(c.id), sequence: number(c.sequence),
      parameters: Object.fromEntries(parameterKeys.filter(k => c.parameters[k] !== undefined).map(k => {
        const v = c.parameters[k];
        if (!["string", "number", "boolean"].includes(typeof v)) throw new Error("Invalid candidate parameter.");
        return [k, v];
      })),
      ...(trained ? { model: model(trained.output.model), durationSeconds: number(trained.output.duration_seconds) } : {}),
      development: reports.map(e => result(e, baselines)),
      ...(failed ? { failure: { phase: string(failed.phase), reason: string(failed.reason) } } : {}),
    };
  });
  const ee = events.map(e => e.event);
  const final = ee.findLast(e => e.kind === "finalized");
  const selected = ee.findLast(e => e.kind === "development_selected");
  const sealed = ee.findLast(e => e.kind === "sealed_completed");
  const sealedState = sealed ? (sealed.assessment.verdict === "passed" ? "passed" : "failed") : ee.some(e => e.kind === "sealed_started") ? "started" : ee.some(e => e.kind === "sealed_authorized") ? "authorized" : "unused";
  return {
    id: string(first.run_id), protocolId: string(protocol.id), protocolFingerprint: string(protocol.fingerprint),
    projectId: string(project.id), revision: string(project.source_revision), createdAt: string(first.created_at),
    updatedAt: string(events.at(-1)!.created_at), sourceDatabase, baseline, baselines,
    directions: Object.fromEntries(protocol.metric_contract.definitions.map((m: Json) => [m.key, m.direction])),
    ...(typeof project.task_configuration?.agent_evaluation?.nomos_top_k === "number" ? { agentTopK: project.task_configuration.agent_evaluation.nomos_top_k } : {}),
    candidates, ...(final ? { decision: string(final.decision) } : {}),
    ...(selected?.candidate_id ? { selectedCandidateId: string(selected.candidate_id) } : {}),
    acceptance: { state: sealedState, ...(sealed?.candidate_id ? { candidateId: string(sealed.candidate_id) } : {}), failedMetrics: sealed ? sealed.assessment.gates.filter((g: Json) => !g.passed).map((g: Json) => string(g.key)) : [] },
    budget: { candidates: number(protocol.budget.maximum_candidates), trainingSeconds: number(protocol.budget.maximum_training_seconds), development: number(protocol.budget.maximum_development_evaluations), sealed: number(protocol.budget.maximum_sealed_evaluations) },
    inputs: project.inputs.filter((i: Json) => i.role === "training").map((i: Json) => ({ key: string(i.key), bytes: number(i.bytes), fingerprint: string(i.fingerprint) })),
    activity: events.map(e => ({ sequence: number(e.sequence), kind: string(e.event.kind), at: string(e.created_at), ...(e.event.candidate_id ? { candidateId: string(e.event.candidate_id) } : {}) })),
    journalHead: string(events.at(-1)!.fingerprint),
  };
}

/** Reads only SQLite files explicitly inside the selected workspace; never native payloads. */
export function readWorkspace(folder: string): WorkspaceSnapshot {
  const root = resolve(folder);
  const files = readdirSync(root, { withFileTypes: true }).filter(f => f.isFile() && /^encoder-gym(?:-[a-z0-9-]+)?\.sqlite$/i.test(f.name)).map(f => f.name).sort();
  if (!files.length) throw new Error("No Encoder Gym experiment databases found in this folder. Choose the isolated experiment workspace.");
  const runs = new Map<string, RunRecord>();
  const projects = new Map<string, Json>();
  const optimizations = new Map<string, string>();
  for (const file of files) {
    const db = new DatabaseSync(join(root, file), { readOnly: true });
    try {
      db.exec("PRAGMA query_only = ON; BEGIN");
      const hasTable = (name: string): boolean => Boolean(db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").get(name));
      if (!hasTable("encoder_experiment_projects")) throw new Error(`${file} is not an experiment journal.`);
      const read = (table: string): Json[] => db.prepare(`SELECT artifact_json FROM ${table}`).all().map(row => object(JSON.parse(string(row.artifact_json)), table));
      for (const p of read("encoder_experiment_projects")) projects.set(p.id, p);
      const protocols = read("encoder_experiment_protocols");
      const events = read("encoder_experiment_events");
      if (hasTable("encoder_production_optimization_runs")) for (const o of read("encoder_production_optimization_runs")) optimizations.set(string(o.reserved_experiment_run_id), string(o.id));
      for (const id of new Set(events.map(e => e.run_id))) {
        const journal = events.filter(e => e.run_id === id).sort((a, b) => a.sequence - b.sequence);
        const protocol = protocols.find(p => p.id === journal[0]!.protocol_id);
        if (!protocol) throw new Error("Run protocol is missing.");
        const project = projects.get(protocol.project_snapshot_id);
        if (!project) throw new Error("Run project is missing.");
        const run = projectRun(protocol, project, journal, file);
        const previous = runs.get(id);
        if (previous && previous.journalHead !== run.journalHead) throw new Error("Conflicting copies of a run. Open one unambiguous workspace.");
        runs.set(id, run);
      }
      db.exec("ROLLBACK");
    } finally { db.close(); }
  }
  const ordered = [...runs.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  if (!ordered.length) throw new Error("This workspace has no recorded experiment runs yet.");
  const newest = ordered[0]!;
  if (ordered.some(r => r.baseline.fingerprint !== newest.baseline.fingerprint)) throw new Error("This workspace contains different baseline models. Open a workspace with one baseline lineage.");
  for (const run of ordered) run.optimizationId = optimizations.get(run.id);
  const project = projects.get(newest.projectId)!;
  const onnx = project.task_configuration?.baseline_evidence?.onnx;
  return { schemaVersion: 1, capturedAt: new Date().toISOString(), source: "local", folder: root,
    name: project.backend?.name === "nomos" ? "Nomos repair" : basename(root), task: string(project.task),
    baseline: newest.baseline,
    ...(onnx ? { deployment: { key: string(onnx.path), fingerprint: string(onnx.fingerprint), bytes: number(onnx.bytes) } } : {}),
    runs: ordered, databases: files };
}
