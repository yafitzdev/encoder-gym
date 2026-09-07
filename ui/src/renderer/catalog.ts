import type { CandidateAttempt, DevelopmentReport, RunRecord, WorkspaceSnapshot } from "../workspace.js";

export interface CandidateRow { candidate: CandidateAttempt; run: RunRecord; attempts: RunRecord[] }
export interface EvaluationSetup { id: string; label: string; description: string; run: RunRecord; rows: CandidateRow[] }
export interface CatalogFilter { query: string; setup: string; status: string; sort: string }
export const initialFilter = (): CatalogFilter => ({ query: "", setup: "all", status: "all", sort: "newest" });

export const metrics: Record<string, { label: string; short: string; description: string; percent?: boolean }> = {
  mrr: { label: "Ranking score", short: "MRR", description: "Mean reciprocal rank of the first correct tool. A score of 1 means it ranked first every time. Higher is better." },
  recall_at_1: { label: "Top-1 recall", short: "Recall@1", percent: true, description: "How often the correct tool appears first. Higher is better." },
  recall_at_2: { label: "Top-2 recall", short: "Recall@2", percent: true, description: "How often a correct tool appears among the first two suggestions. Higher is better." },
  recall_at_3: { label: "Top-3 recall", short: "Recall@3", percent: true, description: "How often a correct tool appears among the first three suggestions. Higher is better." },
  agent_success_rate: { label: "Agent completion", short: "Completion", percent: true, description: "The share of evaluated agent sessions completed successfully. An offline retrieval gain alone does not guarantee better agent behavior." },
  mean_positive_margin: { label: "Positive margin", short: "Margin", description: "The score gap between a correct tool and competing tools. Higher is better." },
  agent_wrong_tool_execution_rate: { label: "Wrong tool executions", short: "Wrong executions", percent: true, description: "The share of executions using the wrong tool. Lower is better." },
  agent_invalid_call_rate: { label: "Invalid tool calls", short: "Invalid calls", percent: true, description: "The share of invalid calls. Lower is better." },
  agent_mean_completed_stage_rate: { label: "Completed stages", short: "Stage completion", percent: true, description: "Average fraction of stages completed in an agent session." },
  agent_schema_valid_call_rate: { label: "Valid call schemas", short: "Valid schemas", percent: true, description: "The share of calls matching the required tool schema." },
  agent_successful_execution_rate: { label: "Successful executions", short: "Execution success", percent: true, description: "The share of tool executions completed successfully." },
  agent_tool_selection_accuracy: { label: "Tool selection accuracy", short: "Tool accuracy", percent: true, description: "How often the agent selected the correct tool." },
  agent_visible_oracle_hit_rate: { label: "Correct tool visibility", short: "Oracle visibility", percent: true, description: "How often the correct tool was visible to the agent." },
  agent_prompt_tokens_per_attempt: { label: "Prompt tokens per attempt", short: "Prompt tokens", description: "Mean input tokens used in each agent attempt. Lower is better." },
  agent_tool_description_reduction: { label: "Tool description reduction", short: "Description reduction", percent: true, description: "The reduction in tool description content presented to the agent." },
};
export const metricInfo = (key: string) => metrics[key] ?? { label: key.replaceAll("_", " "), short: key, description: "Recorded metric from the task adapter." };
export const shortId = (id: string) => id.length > 20 ? id.slice(0, 8) + "…" + id.slice(-6) : id;
export const suiteName = (suite: string) => ["development", "generic_holdout"].includes(suite) ? "Generic retrieval" : suite === "retired_post_scaling" ? "Repair scenarios" : suite.replaceAll("_", " ");
export const dateLabel = (date: string) => new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" }).format(new Date(date));
export const timeLabel = (date: string) => new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit", timeZone: "UTC" }).format(new Date(date)) + " UTC";
export function durationLabel(seconds?: number): string { return seconds === undefined ? "Not recorded" : seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`; }
export function bytesLabel(bytes: number): string { return (bytes / 1024 / 1024).toFixed(1) + " MB"; }

export function score(value: number | undefined, key: string): string {
  if (value === undefined) return "—";
  return metricInfo(key).percent ? (value * 100).toFixed(2) + "%" : key === "agent_prompt_tokens_per_attempt" ? value.toFixed(1) : value.toFixed(6);
}
export function delta(value: number | undefined, key: string): string {
  if (value === undefined) return "Not evaluated";
  if (value === 0) return "Unchanged";
  const positive = value > 0 ? "+" : "−";
  const v = metricInfo(key).percent ? (Math.abs(value) * 100).toFixed(2) + " pp" : Math.abs(value).toFixed(6);
  return positive + v;
}

export function candidateName(candidate: CandidateAttempt): string {
  const p = candidate.parameters;
  if (p.repair_total_rows) return "Repair fine-tune";
  if (p.strategy === "linear_interpolation") return `${Number(p.specialist_weight) * 100}% ${String(p.reference_model).includes("mnrl") ? "MNRL" : "triplet"} blend`;
  return `Triplet · LR ${Number(p.learning_rate).toExponential(0)}`;
}
export function candidateDescription(candidate: CandidateAttempt): string {
  const p = candidate.parameters;
  if (p.repair_total_rows) return `${Number(p.repair_base_rows).toLocaleString("en")} base + ${p.repair_delta_rows} repair examples`;
  if (p.strategy === "linear_interpolation") return `${100 - Number(p.specialist_weight) * 100}% baseline + ${Number(p.specialist_weight) * 100}% reference checkpoint`;
  return `${p.epochs ?? "?"} epoch · ${p.device ?? "local"} · margin ${p.margin ?? "?"}`;
}
export function runName(run: RunRecord): string {
  const c = run.candidates[0];
  if (!c) return "Experiment";
  if (c.parameters.repair_total_rows) return "Repair training";
  if (run.baselines.length > 1) return "Multi-suite interpolation";
  if (c.parameters.strategy === "linear_interpolation") return run.agentTopK === 1 ? "Interpolation qualification" : String(c.parameters.reference_model).includes("mnrl") ? "MNRL interpolation" : "Triplet interpolation";
  return run.candidates.some(c => c.failure) ? "Initial training attempt" : "Continued triplet training";
}
export function runLabel(run: RunRecord, workspace: WorkspaceSnapshot): string {
  return `Run ${String(workspace.runs.length - workspace.runs.findIndex(r => r.id === run.id)).padStart(2, "0")}`;
}
export function setupId(run: RunRecord): string {
  return JSON.stringify([run.baseline.fingerprint, run.baselines.map(b => [b.suite, b.suiteFingerprint, b.contractFingerprint]).sort((a, b) => String(a[0]).localeCompare(String(b[0])))]);
}
export function setupName(run: RunRecord): string {
  return `${run.baselines.length > 1 ? "Two-suite" : "Generic"} evaluation · ${run.agentTopK ? `top-${run.agentTopK} agent` : "retrieval only"}`;
}
export function candidateRows(workspace: WorkspaceSnapshot): CandidateRow[] {
  const rows = new Map<string, CandidateRow>();
  for (const run of [...workspace.runs].sort((a, b) => b.createdAt.localeCompare(a.createdAt))) {
    for (const candidate of run.candidates) {
      const previous = rows.get(candidate.id);
      if (previous) previous.attempts.push(run);
      else rows.set(candidate.id, { candidate, run, attempts: [run] });
    }
  }
  return [...rows.values()];
}
export function developmentStatus(row: CandidateRow): { label: string; tone: string; detail: string; key: string } {
  const { candidate: c, run } = row;
  if (c.failure) return { label: "Execution failed", tone: "danger", detail: c.failure.phase, key: "failed" };
  if (c.development.some(d => d.verdict === "failed")) return { label: "Below requirements", tone: "danger", detail: `${c.development.flatMap(d => d.checks).filter(g => !g.passed).length} checks failed`, key: "rejected" };
  if (c.development.length !== run.baselines.length || c.development.some(d => d.verdict !== "passed")) return { label: "Incomplete evidence", tone: "muted", detail: `${c.development.length} of ${run.baselines.length} suites recorded`, key: "incomplete" };
  return { label: "Passed development", tone: "success", detail: `${c.development.length} of ${run.baselines.length} suites passed`, key: "passed" };
}
export function resultSummary(row: CandidateRow): string {
  const status = developmentStatus(row);
  if (status.key !== "passed") return status.detail;
  if (row.run.acceptance.candidateId === row.candidate.id) return row.run.acceptance.state === "failed" ? "Rejected at final acceptance" : row.run.acceptance.state === "passed" ? "Passed final acceptance" : "Final acceptance pending";
  return row.run.decision ? "Not selected for final acceptance" : "Awaiting selection";
}
export function comparisonGroups(workspace: WorkspaceSnapshot, filter: CatalogFilter): EvaluationSetup[] {
  const groups = new Map<string, EvaluationSetup>();
  for (const row of candidateRows(workspace)) {
    const id = setupId(row.run);
    const query = filter.query.trim().toLocaleLowerCase();
    if (filter.setup !== "all" && filter.setup !== id) continue;
    if (filter.status !== "all" && developmentStatus(row).key !== filter.status) continue;
    if (query && ![candidateName(row.candidate), candidateDescription(row.candidate), row.candidate.id, runName(row.run), runLabel(row.run, workspace)].some(t => t.toLocaleLowerCase().includes(query))) continue;
    let group = groups.get(id);
    if (!group) { group = { id, label: setupName(row.run), description: row.run.agentTopK ? "Retrieval and agent behavior, compared with the same baseline." : "Historical setup without agent evaluations.", run: row.run, rows: [] }; groups.set(id, group); }
    group.rows.push(row);
  }
  for (const group of groups.values()) if (filter.sort === "mrr") group.rows.sort((a, b) => (primaryReport(b)?.metrics.mrr ?? -Infinity) - (primaryReport(a)?.metrics.mrr ?? -Infinity) || a.candidate.id.localeCompare(b.candidate.id));
  return [...groups.values()];
}
export function primaryReport(row: CandidateRow): DevelopmentReport | undefined { return row.candidate.development.find(d => d.report.suite === row.run.baselines[0]?.suite)?.report; }
export function matchingReport(row: CandidateRow, suite: string) { return row.candidate.development.find(d => d.report.suite === suite); }
