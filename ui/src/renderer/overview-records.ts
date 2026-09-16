import type { InputOptimizationRun } from "../input-optimization.js";
import type { RunRecord, WorkspaceSnapshot } from "../workspace.js";
import type { ManagedRunStatus } from "../managed-control.js";

export interface OverviewRecord { id: string; name: string; createdAt: string; input?: InputOptimizationRun; experiment?: RunRecord; managed?: ManagedRunStatus }

/** Root and Agent journals advance independently; compare both persisted heads. */
export function newerInputRun(candidate: InputOptimizationRun, previous: InputOptimizationRun): boolean {
  return candidate.lastSequence > previous.lastSequence || candidate.lastSequence === previous.lastSequence
    && (candidate.agentExecution?.lastSequence ?? 0) > (previous.agentExecution?.lastSequence ?? 0);
}

/** Join by persisted identity, never by dates, display names, or list position. */
export function overviewRecords(workspace: WorkspaceSnapshot, roots: InputOptimizationRun[], managed?: ManagedRunStatus): OverviewRecord[] {
  const unique = new Map<string, InputOptimizationRun>();
  for (const run of roots) if (!unique.has(run.id) || newerInputRun(run, unique.get(run.id)!)) unique.set(run.id, run);
  const linked = new Set<string>();
  const rows: Omit<OverviewRecord, "name">[] = [...unique.values()].map(input => {
    for (const iteration of input.iterations ?? []) if (iteration.experimentRunId) linked.add(iteration.experimentRunId);
    const experiment = workspace.runs.find(run => run.id === input.experimentRunId || run.optimizationId === input.id);
    if (experiment) linked.add(experiment.id);
    return { id: input.id, createdAt: input.createdAt, input, experiment };
  });
  for (const experiment of workspace.runs) if (!linked.has(experiment.id)) rows.push({ id: experiment.id, createdAt: experiment.createdAt, experiment });
  if (managed) {
    const existing = rows.find(row => row.id === managed.run_id || row.experiment?.optimizationId === managed.run_id);
    if (existing) existing.managed = managed;
    else rows.push({ id: managed.run_id, createdAt: managed.created_at, managed });
  }
  return rows.sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id))
    .map((row, index) => ({ ...row, name: `Run ${String(index + 1).padStart(2, "0")}` })).reverse();
}

export function reportDecision(record: OverviewRecord): "KEEP" | "REJECT" | undefined {
  if (record.input) {
    if (record.input.state === "candidate_accepted") return "KEEP";
    if (["candidate_rejected", "baseline_retained"].includes(record.input.state)) return "REJECT";
    return undefined;
  }
  if (record.experiment?.acceptance.state === "passed") return "KEEP";
  if (record.experiment?.acceptance.state === "failed" || record.experiment?.decision === "retain_baseline") return "REJECT";
  return undefined;
}

export function comparisonTone(baseline: number, candidate: number, direction?: "higher_is_better" | "lower_is_better"): "success" | "danger" | "muted" {
  if (!direction || !Number.isFinite(baseline) || !Number.isFinite(candidate) || baseline === candidate) return "muted";
  return (candidate - baseline) * (direction === "lower_is_better" ? -1 : 1) > 0 ? "success" : "danger";
}
