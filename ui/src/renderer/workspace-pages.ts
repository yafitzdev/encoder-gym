import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedReadiness, ManagedRunStatus } from "../managed-control.js";
import type { Actions } from "./actions.js";
import { bytesLabel, comparisonGroups, evaluationGroups, initialFilter, metricInfo, primaryMetric, score, setupName, suiteName, summaryMetrics } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag, workspacePage } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions, optimization?: ManagedRunStatus): HTMLElement {
  const active = optimization && !["completed", "cancelled", "failed"].includes(optimization.state);
  return workspacePage("Runs", null,
    optimization ? h("section", { class: "optimization-run-row" },
      h("div", {}, h("div", { class: "eyebrow" }, active ? "Current optimization" : "Latest optimization"), h("h2", {}, optimization.stage.label), facts([["Optimization run", optimization.run_id], ["State", optimization.state.replaceAll("_", " ")], ["Durable transitions", String(optimization.last_sequence)], ...(optimization.decision ? [["Decision", optimization.decision.replaceAll("_", " ")] as [string, string]] : [])])),
      button(active ? "Continue run" : "Inspect run", () => actions.navigate({ page: "optimization" }), "secondary", "arrow")) : null,
    sectionHeader("Experiment runs", tag(String(workspace.runs.length))),
    workspace.runs.length ? h("div", { class: "run-list" }, ...workspace.runs.map(run => runListItem(run, workspace, actions))) : empty(optimization ? "No experiment record" : "No runs"));
}
export function renderBenchmarks(workspace: WorkspaceSnapshot, actions: Actions, readiness?: ManagedReadiness): HTMLElement {
  const groups = evaluationGroups(workspace, initialFilter());
  const preview = readiness?.preparedOptimization?.launchPreview ?? readiness?.launchPreview;
  const planned = preview ? h("section", { class: "evaluation-plan" },
    sectionHeader("Bound evaluation plan", tag(preview.existingRun ? "Run reserved" : "Prepared", "accent")),
    h("div", { class: "evaluation-plan-list" },
      ...preview.developmentSuites.map(suite => h("div", { class: "evaluation-plan-row" }, h("strong", {}, suiteName(suite)), tag("Development"))),
      h("div", { class: "evaluation-plan-row sealed-plan" }, h("strong", {}, suiteName(preview.sealedSuite)), tag("Sealed · approval required", "warning"))),
    details("Exact plan identity and limits", facts([
      ["Baseline model", copyField(workspace.baseline.fingerprint, actions.copy)],
      ["Training snapshot", copyField(preview.trainingSnapshotId, actions.copy)],
      ["Benchmark generation", copyField(preview.benchmarkGenerationId, actions.copy)],
      ["Development evaluations", `At most ${preview.budget.maximum_development_evaluations}`],
      ["Sealed evaluations", `At most ${preview.budget.maximum_sealed_evaluations}; separately authorized`],
      ["Evaluation time", `At most ${preview.maximumEvaluationSeconds.toLocaleString()} seconds`],
    ])),
    button(preview.existingRun ? "Open optimization run" : "Review prepared run", actions.prepareOptimization, "secondary", "arrow")) : null;
  if (!groups.length) return workspacePage("Evaluation", null, planned,
    sectionHeader("Recorded evaluation evidence"),
    empty("No evaluations"));
  return workspacePage("Evaluation", null, planned,
    sectionHeader("Recorded evaluation evidence"),
    ...groups.map((group, i) => {
      const primary = primaryMetric(group.run);
      return h("section", { class: "benchmark-section" }, sectionHeader(group.label, tag(i === 0 ? "Latest recorded setup" : "Historical setup")),
        ...group.run.baselines.map(base => h("div", { class: "benchmark-row" }, h("div", {}, h("h3", {}, suiteName(base.suite)), tag(Object.keys(base.metrics).length + " metrics"), h("code", { class: "muted" }, base.suite)),
          h("div", {}, h("span", { class: "muted" }, "Baseline " + metricInfo(primary).label.toLowerCase()), h("strong", { class: "score" }, score(base.metrics[primary], primary))),
        )),
        details("Evaluation identities", facts(group.run.baselines.flatMap(base => [[suiteName(base.suite) + " · benchmark", copyField(base.suiteFingerprint, actions.copy)], [suiteName(base.suite) + " · metric contract", copyField(base.contractFingerprint, actions.copy)]]))),
        button("Compare " + group.rows.length + (group.rows.length === 1 ? " candidate" : " candidates"), () => actions.navigate({ page: "models", id: group.id }), "ghost", "arrow"),
      );
    }),
  );
}
