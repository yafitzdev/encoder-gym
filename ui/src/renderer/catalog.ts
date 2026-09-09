import type { CandidateAttempt, DevelopmentReport, RunRecord, WorkspaceSnapshot } from "../workspace.js";

export interface CandidateRow { candidate: CandidateAttempt; run: RunRecord; attempts: RunRecord[] }
export interface EvaluationSetup { id: string; label: string; description: string; run: RunRecord; rows: CandidateRow[] }
export interface CatalogFilter { query: string; setup: string; status: string; sort: string }
export const initialFilter = (): CatalogFilter => ({ query: "", setup: "all", status: "all", sort: "newest" });

export const metrics: Record<string, { label: string; short: string; description: string; percent?: boolean }> = {
  accuracy: { label: "Accuracy", short: "Accuracy", percent: true, description: "The fraction of predictions matching the expected label in this evaluation. Higher is better." },
  macro_f1: { label: "Macro F1", short: "F1", percent: true, description: "F1 averaged equally across classes. It balances precision and recall without letting larger classes dominate. Higher is better." },
  loss: { label: "Loss", short: "Loss", description: "The error measure defined by this evaluation contract. Its direction is taken from the recorded contract." },
  mrr: { label: "Ranking score", short: "MRR", description: "Mean reciprocal rank of the first relevant result. A score of 1 means it ranked first every time. Higher is better." },
  recall_at_1: { label: "Top-1 recall", short: "Recall@1", percent: true, description: "Recall measured at the first result, using this task's relevance definition. Higher is better." },
  recall_at_2: { label: "Top-2 recall", short: "Recall@2", percent: true, description: "Recall among the first two results, using this task's relevance definition. Higher is better." },
  recall_at_3: { label: "Top-3 recall", short: "Recall@3", percent: true, description: "Recall among the first three results, using this task's relevance definition. Higher is better." },
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
export const metricInfo = (key: string) => metrics[key] ?? { label: humanize(key), short: key, description: "Recorded metric from this task's evaluation adapter. Its improvement direction and requirements come from the immutable metric contract." };
export const shortId = (id: string) => id.length > 20 ? id.slice(0, 8) + "…" + id.slice(-6) : id;
export const humanize = (value: string) => value.replaceAll("_", " ").replace(/^./, c => c.toUpperCase());
export const suiteName = (suite: string) => humanize(suite);
export const modelName = (key: string) => key.split(/[\\/]/).filter(Boolean).at(-1)?.replaceAll("_", " ") ?? key;
export const dateLabel = (date: string) => new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" }).format(new Date(date));
export const timeLabel = (date: string) => new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit", timeZone: "UTC" }).format(new Date(date)) + " UTC";
export function durationLabel(seconds?: number): string { return seconds === undefined ? "Not recorded" : seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`; }
export function bytesLabel(bytes: number): string { return bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? (bytes / 1024).toFixed(1) + " KB" : (bytes / 1024 / 1024).toFixed(1) + " MB"; }
export function displayPath(path: string): string { return path.startsWith("\\\\?\\") && /^[A-Za-z]:\\/.test(path.slice(4)) ? path.slice(4) : path; }

export function score(value: number | undefined, key: string): string {
  if (value === undefined) return "—";
  return metricInfo(key).percent ? (value * 100).toFixed(2) + "%" : key === "agent_prompt_tokens_per_attempt" ? value.toFixed(1) : value.toFixed(6);
}
export function delta(value: number | undefined, key: string): string {
  if (value === undefined) return "Not evaluated";
  if (value === 0) return "Unchanged";
  const positive = value > 0 ? "+" : "−";
  const percent = metricInfo(key).percent;
  const amount = Math.abs(value) * (percent ? 100 : 1), rounded = amount.toFixed(percent ? 2 : 6);
  const v = (Number(rounded) === 0 ? amount.toExponential(1) : rounded) + (percent ? " pp" : "");
  return positive + v;
}

export function candidateName(candidate: CandidateAttempt): string {
  const p = candidate.parameters;
  if (typeof p.name === "string" && p.name.trim()) return p.name;
  if (p.repair_total_rows) return "Repair fine-tune";
  if (p.strategy === "linear_interpolation") return `${Number(p.specialist_weight) * 100}% ${String(p.reference_model).includes("mnrl") ? "MNRL" : String(p.reference_model).includes("triplet") ? "triplet" : "checkpoint"} blend`;
  if (typeof p.learning_rate === "number") return `${p.loss ? humanize(String(p.loss)) : "Fine-tune"} · LR ${p.learning_rate.toExponential(0)}`;
  return `Candidate ${String(candidate.sequence).padStart(2, "0")}`;
}
export function candidateDescription(candidate: CandidateAttempt): string {
  const p = candidate.parameters;
  if (p.repair_total_rows) return `${Number(p.repair_base_rows).toLocaleString("en")} base + ${p.repair_delta_rows} repair examples`;
  if (p.strategy === "linear_interpolation") return `${100 - Number(p.specialist_weight) * 100}% baseline + ${Number(p.specialist_weight) * 100}% reference checkpoint`;
  return [p.epochs ? `${p.epochs} ${p.epochs === 1 ? "epoch" : "epochs"}` : undefined, p.device, p.margin !== undefined ? `margin ${p.margin}` : undefined].filter(v => v !== undefined).join(" · ") || "Configuration available in details";
}
export function runName(run: RunRecord): string {
  const c = run.candidates[0];
  if (!c) return "Experiment";
  if (c.parameters.repair_total_rows) return "Repair training";
  if (run.baselines.length > 1 && c.parameters.strategy === "linear_interpolation") return "Multi-suite interpolation";
  if (c.parameters.strategy === "linear_interpolation") return run.agentTopK === 1 ? "Interpolation qualification" : String(c.parameters.reference_model).includes("mnrl") ? "MNRL interpolation" : String(c.parameters.reference_model).includes("triplet") ? "Triplet interpolation" : "Checkpoint interpolation";
  return run.candidates.some(c => c.failure) ? "Experiment with execution errors" : "Training experiment";
}
export function runLabel(run: RunRecord, workspace: WorkspaceSnapshot): string {
  return `Run ${String(workspace.runs.length - workspace.runs.findIndex(r => r.id === run.id)).padStart(2, "0")}`;
}
export function setupId(run: RunRecord): string {
  return JSON.stringify([run.baseline.fingerprint, run.agentTopK ?? null, run.baselines.map(b => [b.suite, b.suiteFingerprint, b.contractFingerprint]).sort((a, b) => String(a[0]).localeCompare(String(b[0])))]);
}
export function setupName(run: RunRecord): string {
  return `${run.baselines.length} ${run.baselines.length === 1 ? "benchmark" : "benchmarks"} · ${run.agentTopK ? `top-${run.agentTopK} agent` : "development evaluation"}`;
}
export function candidateRows(workspace: WorkspaceSnapshot): CandidateRow[] {
  const catalog = workspace.managed?.modelCatalog;
  const baselineArtifactIds = catalog
    ? new Set(catalog.baselineRevisions.map(revision => revision.modelArtifactId))
    : undefined;
  const candidateArtifacts = catalog
    ? catalog.artifacts.filter(artifact => !baselineArtifactIds!.has(artifact.id))
    : undefined;
  const rows = new Map<string, CandidateRow>();
  for (const run of [...workspace.runs].sort((a, b) => b.createdAt.localeCompare(a.createdAt))) {
    for (const candidate of run.candidates) {
      // A bound scientific store can contain valid historical experiments whose
      // checkpoints were never brought into this managed project's custody.
      // Runs and Evaluation still expose that evidence, but Models represents
      // only project-catalog artifacts and their baseline/candidate relations.
      if (candidateArtifacts && !candidateArtifacts.some(artifact =>
        candidate.model?.fingerprint === artifact.fingerprint &&
        (!artifact.producingRun || artifact.producingRun.id === run.id)
      )) continue;
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
  if (c.development.some(d => d.verdict === "failed")) {
    const count = c.development.flatMap(d => d.checks).filter(g => !g.passed).length;
    return { label: "Below requirements", tone: "danger", detail: `${count} ${count === 1 ? "check" : "checks"} failed`, key: "rejected" };
  }
  if (c.development.length !== run.baselines.length || c.development.some(d => d.verdict !== "passed")) return { label: "Incomplete evidence", tone: "muted", detail: `${c.development.length} of ${run.baselines.length} suites recorded`, key: "incomplete" };
  return { label: "Passed development", tone: "success", detail: `${c.development.length} of ${run.baselines.length} suites passed`, key: "passed" };
}
export function resultSummary(row: CandidateRow): string {
  const status = developmentStatus(row);
  if (status.key !== "passed") return status.detail;
  if (row.run.acceptance.candidateId === row.candidate.id) return row.run.acceptance.state === "failed" ? "Rejected at final acceptance" : row.run.acceptance.state === "passed" ? "Passed final acceptance" : "Final acceptance pending";
  return row.run.decision ? "Not selected for final acceptance" : "Awaiting selection";
}
export function comparisonGroups(workspace: WorkspaceSnapshot, filter: CatalogFilter, suiteIndex = 0): EvaluationSetup[] {
  const groups = new Map<string, EvaluationSetup>();
  for (const row of candidateRows(workspace)) {
    const id = setupId(row.run);
    const query = filter.query.trim().toLocaleLowerCase();
    if (filter.setup !== "all" && filter.setup !== id) continue;
    if (filter.status !== "all" && developmentStatus(row).key !== filter.status) continue;
    if (query && ![candidateName(row.candidate), candidateDescription(row.candidate), row.candidate.id, runName(row.run), runLabel(row.run, workspace)].some(t => t.toLocaleLowerCase().includes(query))) continue;
    let group = groups.get(id);
    if (!group) { group = { id, label: setupName(row.run), description: "The same baseline artifact, benchmark identities, and scoring requirements are used for these comparisons.", run: row.run, rows: [] }; groups.set(id, group); }
    group.rows.push(row);
  }
  for (const group of groups.values()) if (filter.sort === "primary" || filter.sort === "mrr") {
    const key = primaryMetric(group.run), direction = group.run.directions[key] === "lower_is_better" ? -1 : 1;
    const suite = group.run.baselines[Math.min(suiteIndex, group.run.baselines.length - 1)]!.suite;
    group.rows.sort((a, b) => {
      const av = matchingReport(a, suite)?.report.metrics[key], bv = matchingReport(b, suite)?.report.metrics[key];
      return av === undefined ? bv === undefined ? 0 : 1 : bv === undefined ? -1 : direction * (bv - av) || a.candidate.id.localeCompare(b.candidate.id);
    });
  }
  return [...groups.values()];
}
export function primaryReport(row: CandidateRow): DevelopmentReport | undefined { return row.candidate.development.find(d => d.report.suite === row.run.baselines[0]?.suite)?.report; }
export function matchingReport(row: CandidateRow, suite: string) { return row.candidate.development.find(d => d.report.suite === suite); }
export function primaryMetric(run: RunRecord): string { return run.primaryMetric ?? (run.baselines[0]?.metrics.mrr !== undefined ? "mrr" : Object.keys(run.directions)[0] ?? "score"); }
export function summaryMetrics(run: RunRecord, suite = run.baselines[0]): string[] {
  if (!suite) return [];
  return [...new Set([primaryMetric(run), "accuracy", "macro_f1", "recall_at_2", "agent_success_rate", "loss", ...Object.keys(suite.metrics)])].filter(k => suite.metrics[k] !== undefined && run.directions[k] !== undefined).slice(0, 3);
}
