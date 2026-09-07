import { DatabaseSync } from "node:sqlite";
import { readdirSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import type { CandidateAttempt, DevelopmentReport, DevelopmentResult, ModelArtifact, RunRecord, WorkspaceSnapshot } from "../workspace.js";
import type { ProjectContent } from "../projects.js";

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

// Candidate contracts are adapter-owned. Project only scalar configuration,
// never credential-like fields, nested native payload, or free-form long text.
function parameters(value: unknown): CandidateAttempt["parameters"] {
  return Object.fromEntries(Object.entries(object(value, "candidate parameters")).filter(([key, v]) =>
    !/(secret|password|credential|api.?key|access.?token|auth.?token|authorization|prompt|payload|row.?text)/i.test(key) &&
    (typeof v === "boolean" || typeof v === "number" && Number.isFinite(v) || typeof v === "string" && v.length <= 500)
  )) as CandidateAttempt["parameters"];
}

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
      if (g.baseline !== baseline.metrics[g.key] || g.candidate !== report.metrics[g.key]) throw new Error("Recorded check values do not match their reports.");
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
  const validateReport = (r: Json) => {
    if (r.project_snapshot_id !== project.id || r.project_snapshot_fingerprint !== project.fingerprint || r.metric_contract_fingerprint !== protocol.metric_contract.fingerprint) throw new Error("Evaluation report belongs to a different project or metric contract.");
  };
  rawBaselines.forEach(validateReport);
  const baselines = rawBaselines.map(development);
  const candidateIds = new Set<string>();
  const candidates: CandidateAttempt[] = protocol.candidates.map((c: Json) => {
    if (candidateIds.has(c.id)) throw new Error("Duplicate candidate identity.");
    candidateIds.add(c.id);
    const ce = events.map(e => e.event).filter(e => e.candidate_id === c.id);
    const trained = ce.find(e => e.kind === "candidate_training_completed");
    const failed = ce.findLast(e => e.kind === "candidate_failed");
    const reports = ce.filter(e => ["candidate_development_completed", "candidate_development_suite_completed"].includes(e.kind));
    for (const e of reports) {
      validateReport(e.report);
      if (trained && e.report.model.fingerprint !== trained.output.model.fingerprint) throw new Error("Candidate evaluation belongs to a different model.");
    }
    if (new Set(reports.map(e => e.report.suite_key)).size !== reports.length) throw new Error("Duplicate development report.");
    return {
      id: string(c.id), sequence: number(c.sequence),
      parameters: parameters(c.parameters),
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
    ...(protocol.metric_contract.primary_metric ? { primaryMetric: string(protocol.metric_contract.primary_metric) } : {}),
    ...(typeof project.task === "string" ? { task: project.task } : {}),
    ...(typeof project.task_configuration?.agent_evaluation?.nomos_top_k === "number" ? { agentTopK: project.task_configuration.agent_evaluation.nomos_top_k } : {}),
    candidates, ...(final ? { decision: string(final.decision) } : {}),
    ...(selected?.candidate_id ? { selectedCandidateId: string(selected.candidate_id) } : {}),
    acceptance: { state: sealedState, ...(sealed?.candidate_id || selected?.candidate_id ? { candidateId: string(sealed?.candidate_id ?? selected.candidate_id) } : {}), failedMetrics: sealed ? sealed.assessment.gates.filter((g: Json) => !g.passed).map((g: Json) => string(g.key)) : [] },
    budget: { candidates: number(protocol.budget.maximum_candidates), trainingSeconds: number(protocol.budget.maximum_training_seconds), development: number(protocol.budget.maximum_development_evaluations), sealed: number(protocol.budget.maximum_sealed_evaluations) },
    inputs: project.inputs.filter((i: Json) => i.role === "training").map((i: Json) => ({ key: string(i.key), bytes: number(i.bytes), fingerprint: string(i.fingerprint) })),
    activity: events.map(e => ({ sequence: number(e.sequence), kind: string(e.event.kind), at: string(e.created_at), ...(e.event.candidate_id ? { candidateId: string(e.event.candidate_id) } : {}) })),
    journalHead: string(events.at(-1)!.fingerprint),
  };
}

export class EmptyWorkspaceError extends Error {
  constructor(readonly databases: string[]) { super("No baseline or experiment records yet."); }
}
export function readProjectContent(folder: string): ProjectContent {
  try { return { state: "ready", workspace: readWorkspace(folder) }; }
  catch (error) {
    if (error instanceof EmptyWorkspaceError) return { state: "empty", databases: error.databases };
    const code = (error as NodeJS.ErrnoException).code;
    return { state: "error", message: code === "ENOENT" ? "The project folder is no longer available. Locate its new folder to reconnect it." : code === "EACCES" || code === "EPERM" ? "The project folder cannot be read. Check its permissions or reconnect an accessible folder." : error instanceof Error ? error.message : "The project evidence could not be read." };
  }
}

/** Reads only SQLite files directly inside the selected folder; never native payloads. */
export function readWorkspace(folder: string): WorkspaceSnapshot {
  const root = resolve(folder);
  const files = readdirSync(root, { withFileTypes: true }).filter(f => f.isFile() && /\.(sqlite3?|db)$/i.test(f.name)).map(f => f.name).sort();
  if (!files.length) throw new EmptyWorkspaceError([]);
  const runs = new Map<string, RunRecord>();
  const projects = new Map<string, Json>();
  const optimizations = new Map<string, string>();
  const databases: string[] = [];
  for (const file of files) {
    const db = new DatabaseSync(join(root, file), { readOnly: true });
    try {
      db.exec("PRAGMA query_only = ON; BEGIN");
      const hasTable = (name: string): boolean => Boolean(db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").get(name));
      if (!hasTable("encoder_experiment_projects")) continue;
      databases.push(file);
      const read = (table: string): Json[] => db.prepare(`SELECT artifact_json FROM ${table}`).all().map(row => object(JSON.parse(string(row.artifact_json)), table));
      const localProjects = new Map<string, Json>();
      for (const p of read("encoder_experiment_projects")) {
        if (projects.has(p.id) && projects.get(p.id)!.fingerprint !== p.fingerprint) throw new Error("Conflicting copies of an immutable project snapshot.");
        projects.set(p.id, p); localProjects.set(p.id, p);
      }
      const protocols = read("encoder_experiment_protocols");
      const events = read("encoder_experiment_events");
      if (hasTable("encoder_production_optimization_runs")) for (const o of read("encoder_production_optimization_runs")) optimizations.set(string(o.reserved_experiment_run_id), string(o.id));
      for (const id of new Set(events.map(e => e.run_id))) {
        const journal = events.filter(e => e.run_id === id).sort((a, b) => a.sequence - b.sequence);
        const protocol = protocols.find(p => p.id === journal[0]!.protocol_id);
        if (!protocol) throw new Error("Run protocol is missing.");
        const project = localProjects.get(protocol.project_snapshot_id);
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
  if (!databases.length) throw new Error("No supported encoder experiment records were found in this folder's databases. Choose the folder containing the experiment journals; existing databases have not been changed.");
  if (!projects.size) throw new EmptyWorkspaceError(databases);
  const newest = ordered[0];
  for (const run of ordered) run.optimizationId = optimizations.get(run.id);
  const project = newest ? projects.get(newest.projectId)! : [...projects.values()].sort((a, b) => string(b.created_at).localeCompare(string(a.created_at)))[0]!;
  const onnx = project.task_configuration?.baseline_evidence?.onnx;
  return { schemaVersion: 1, capturedAt: new Date().toISOString(), source: "local", folder: root,
    name: typeof project.name === "string" ? project.name : basename(root), task: string(project.task),
    baseline: newest?.baseline ?? model(project.baseline_model), baselineEvaluations: newest?.baselines ?? [],
    ...(onnx ? { deployment: { key: string(onnx.path), fingerprint: string(onnx.fingerprint), bytes: number(onnx.bytes) } } : {}),
    runs: ordered, databases };
}
