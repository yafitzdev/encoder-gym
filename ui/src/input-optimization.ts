export type InputOptimizationState =
  | "queued" | "preparing" | "ready" | "preparation_failed"
  | "materializing" | "materialized" | "materialization_failed"
  | "attaching_experiment" | "ready_to_run" | "experiment_attachment_failed"
  | "optimizing" | "ready_for_final_evaluation" | "baseline_retained"
  | "execution_failed" | "evaluating_final" | "candidate_accepted"
  | "candidate_rejected" | "final_evaluation_failed" | "cancelled";

export type InputOptimizationPhase = "checking_inputs" | "preparing_data" | "starting" | "training" | "saving_candidate" | "evaluating" | "complete";

export interface InputOptimizationRun {
  id: string;
  projectId: string;
  setupId: string;
  launchId?: string;
  experimentRunId?: string;
  createdAt: string;
  state: InputOptimizationState;
  attempt: number;
  materializationAttempt: number;
  experimentAttempt: number;
  executionAttempt: number;
  finalAttempt: number;
  outcome?: { kind: "candidate_ready" | "baseline_retained"; selectedModelId?: string };
  finalResult?: { kind: "candidate_accepted" | "candidate_rejected"; modelId: string; reportId: string };
  failureCode?: string;
  lastSequence: number;
  updatedAt: string;
}

export interface InputOptimizationStarted { actionId: string; run: InputOptimizationRun }

const states = new Set<InputOptimizationState>([
  "queued", "preparing", "ready", "preparation_failed", "materializing", "materialized", "materialization_failed",
  "attaching_experiment", "ready_to_run", "experiment_attachment_failed", "optimizing", "ready_for_final_evaluation",
  "baseline_retained", "execution_failed", "evaluating_final", "candidate_accepted", "candidate_rejected", "final_evaluation_failed",
  "cancelled",
]);
const runKeys = ["run", "state", "attempt", "preparation", "materializationAttempt", "materialization", "experimentAttempt", "experiment", "executionAttempt", "outcome", "finalAttempt", "finalResult", "failureCode", "lastSequence", "headFingerprint", "updatedAt"];

function record(value: unknown, label: string, keys?: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Invalid ${label}.`);
  const item = value as Record<string, unknown>;
  if (keys && Object.keys(item).some(key => !keys.includes(key))) throw new Error(`Invalid ${label}.`);
  return item;
}
function uuid(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error("Invalid optimization identity.");
  return value.toLowerCase();
}
function fingerprint(value: unknown): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) throw new Error("Invalid optimization fingerprint.");
  return value;
}
function integer(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("Invalid optimization sequence.");
  return value;
}
function instant(value: unknown): string {
  if (typeof value !== "string" || !Number.isFinite(Date.parse(value))) throw new Error("Invalid optimization time.");
  return value;
}
function bound(value: unknown): { id: string; fingerprint: string } {
  const item = record(value, "optimization reference", ["id", "fingerprint"]);
  return { id: uuid(item.id), fingerprint: fingerprint(item.fingerprint) };
}

export function parseInputOptimizationRun(value: unknown, expectedProjectId: string): InputOptimizationRun {
  const item = record(value, "optimization run", runKeys);
  const identity = record(item.run, "optimization run identity", ["id", "projectId", "launch", "setup", "createdAt", "fingerprint"]);
  const id = uuid(identity.id), projectId = uuid(identity.projectId);
  if (projectId !== expectedProjectId) throw new Error("Optimization run belongs to another project.");
  const launch = bound(identity.launch); const setup = bound(identity.setup); const createdAt = instant(identity.createdAt); fingerprint(identity.fingerprint); fingerprint(item.headFingerprint);
  if (typeof item.state !== "string" || !states.has(item.state as InputOptimizationState)) throw new Error("Invalid optimization state.");
  const run: InputOptimizationRun = {
    id, projectId, setupId: setup.id, launchId: launch.id, createdAt, state: item.state as InputOptimizationState,
    attempt: integer(item.attempt), materializationAttempt: integer(item.materializationAttempt), experimentAttempt: integer(item.experimentAttempt),
    executionAttempt: integer(item.executionAttempt), finalAttempt: integer(item.finalAttempt), lastSequence: integer(item.lastSequence), updatedAt: instant(item.updatedAt),
  };
  if (item.failureCode !== undefined) {
    if (typeof item.failureCode !== "string" || !/^[a-z0-9_.-]{1,80}$/.test(item.failureCode)) throw new Error("Invalid optimization failure.");
    run.failureCode = item.failureCode;
  }
  if (item.experiment !== undefined) {
    const experiment = record(item.experiment, "optimization experiment", ["run", "materializationFingerprint", "scientificProject", "benchmark", "sourceProtocol", "candidate", "protocol", "experimentRun", "createdAt", "fingerprint"]);
    if (bound(experiment.run).id !== id) throw new Error("Optimization experiment belongs to another run.");
    fingerprint(experiment.materializationFingerprint);
    for (const key of ["scientificProject", "benchmark", "sourceProtocol", "candidate", "protocol"]) bound(experiment[key]);
    instant(experiment.createdAt); fingerprint(experiment.fingerprint);
    run.experimentRunId = bound(experiment.experimentRun).id;
  }
  if (item.outcome !== undefined) {
    const outcome = record(item.outcome, "optimization outcome", ["run", "experimentFingerprint", "experimentRun", "kind", "selectedModel", "createdAt", "fingerprint"]);
    bound(outcome.run); fingerprint(outcome.experimentFingerprint); bound(outcome.experimentRun); instant(outcome.createdAt); fingerprint(outcome.fingerprint);
    if (outcome.kind !== "candidate_ready" && outcome.kind !== "baseline_retained") throw new Error("Invalid optimization outcome.");
    const selected = outcome.selectedModel === undefined ? undefined : bound(outcome.selectedModel);
    if ((outcome.kind === "candidate_ready") !== !!selected) throw new Error("Invalid optimization candidate result.");
    run.outcome = { kind: outcome.kind, ...(selected ? { selectedModelId: selected.id } : {}) };
    const experimentId = bound(outcome.experimentRun).id;
    if (run.experimentRunId && run.experimentRunId !== experimentId) throw new Error("Mismatched optimization experiment.");
    run.experimentRunId = experimentId;
  }
  if (item.finalResult !== undefined) {
    const result = record(item.finalResult, "final optimization result", ["run", "outcomeFingerprint", "experimentRun", "kind", "model", "finalReport", "createdAt", "fingerprint"]);
    bound(result.run); fingerprint(result.outcomeFingerprint); bound(result.experimentRun); instant(result.createdAt); fingerprint(result.fingerprint);
    if (result.kind !== "candidate_accepted" && result.kind !== "candidate_rejected") throw new Error("Invalid final optimization result.");
    run.finalResult = { kind: result.kind, modelId: bound(result.model).id, reportId: bound(result.finalReport).id };
    const experimentId = bound(result.experimentRun).id;
    if (run.experimentRunId && run.experimentRunId !== experimentId) throw new Error("Mismatched optimization experiment.");
    run.experimentRunId = experimentId;
  }
  return run;
}

export function parseInputOptimizationStarted(value: unknown, projectId: string): InputOptimizationStarted {
  const item = record(value, "started optimization", ["actionId", "run"]);
  return { actionId: uuid(item.actionId), run: parseInputOptimizationRun(item.run, projectId) };
}

export function parseInputOptimizationRuns(value: unknown, projectId: string): InputOptimizationRun[] {
  if (!Array.isArray(value)) throw new Error("Invalid optimization run history.");
  return value.map(item => parseInputOptimizationRun(item, projectId));
}

export function inputOptimizationPhase(state: InputOptimizationState): InputOptimizationPhase {
  if (state === "cancelled") return "complete";
  if (["queued", "preparing", "preparation_failed"].includes(state)) return "checking_inputs";
  if (["ready", "materializing", "materialized", "materialization_failed"].includes(state)) return "preparing_data";
  if (["attaching_experiment", "ready_to_run", "experiment_attachment_failed"].includes(state)) return "starting";
  if (["optimizing", "execution_failed"].includes(state)) return "training";
  if (state === "ready_for_final_evaluation" || state === "evaluating_final" || state === "final_evaluation_failed") return "evaluating";
  if (state === "baseline_retained" || state === "candidate_accepted" || state === "candidate_rejected") return "complete";
  return "saving_candidate";
}

export function inputOptimizationTerminal(state: InputOptimizationState): boolean {
  return state === "baseline_retained" || state === "candidate_accepted" || state === "candidate_rejected" || state === "cancelled";
}
