import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedReadiness, ManagedRunStatus } from "../managed-control.js";
import type { Actions } from "./actions.js";
import { bytesLabel, comparisonGroups, dateLabel, evaluationGroups, initialFilter, metricInfo, primaryMetric, score, setupName, suiteName, summaryMetrics } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag, workspacePage } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";
import type { InputRunsController } from "./input-runs-controller.js";
import { inputOptimizationPhase, inputOptimizationTerminal, type InputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { inputRunStageLabel } from "./input-run-activity.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions, optimization?: ManagedRunStatus, projectRuns?: InputRunsController): HTMLElement {
  const active = optimization && !["completed", "cancelled", "failed"].includes(optimization.state);
  const projectRunIds = new Set(projectRuns?.runs?.map(run => run.id) ?? []);
  const experimentRuns = workspace.runs.filter(run => !run.optimizationId || !projectRunIds.has(run.optimizationId));
  const experiments = experimentRuns.length
    ? projectRuns?.runs?.length
      ? [details(`Earlier runs · ${experimentRuns.length}`, h("div", { class: "run-list" }, ...experimentRuns.map(run => runListItem(run, workspace, actions))))]
      : [sectionHeader("Runs"), h("div", { class: "run-list" }, ...experimentRuns.map(run => runListItem(run, workspace, actions)))]
    : projectRuns?.runs?.length ? [] : [sectionHeader("Runs"), empty(optimization ? "No run record" : "No runs")];
  return workspacePage("Runs", null,
    projectRuns?.error ? h("section", { class: "operation-failure", role: "alert" }, "Could not load runs. ", button("Retry", () => projectRuns.refresh(), "secondary")) : null,
    projectRuns?.loading ? h("div", { class: "workspace-progress", role: "status" }, "Loading runs…") : null,
    projectRuns?.runs?.length ? h("section", { class: "project-run-section" },
      sectionHeader("Optimization runs"),
      h("div", { class: "project-run-list" }, ...projectRuns.runs.map(run => projectRun(run, workspace, actions, projectRuns)))) : null,
    optimization ? h("section", { class: "optimization-run-row" },
      h("div", {}, h("div", { class: "eyebrow" }, active ? "Current optimization" : "Latest optimization"), h("h2", {}, optimization.stage.label), facts([["Optimization run", optimization.run_id], ["State", optimization.state.replaceAll("_", " ")], ["Durable transitions", String(optimization.last_sequence)], ...(optimization.decision ? [["Decision", optimization.decision.replaceAll("_", " ")] as [string, string]] : [])])),
      button(active ? "Continue run" : "Inspect run", () => actions.navigate({ page: "optimization" }), "secondary", "arrow")) : null,
    ...experiments);
}

const phaseLabels: Record<InputOptimizationPhase, string> = {
  checking_inputs: "Checking inputs", preparing_data: "Preparing data", starting: "Starting", training: "Training",
  saving_candidate: "Saving candidate", evaluating: "Evaluating", complete: "Complete",
};
function projectRun(run: InputOptimizationRun, workspace: WorkspaceSnapshot, actions: Actions, controller: InputRunsController): HTMLElement {
  const outcome = run.state === "candidate_accepted" ? "Candidate passed" : run.state === "candidate_rejected" ? "Candidate did not pass" : run.state === "baseline_retained" ? "No improvement" : undefined;
  const failed = run.state.endsWith("_failed"), terminal = inputOptimizationTerminal(run.state), busy = controller.runningId === run.id;
  const activity = controller.activities.get(run.id), progress = activity?.progress;
  const selected = run.finalResult?.modelId ?? run.outcome?.selectedModelId;
  const model = selected ? workspace.managed?.modelCatalog?.artifacts.find(artifact => artifact.sourceModel?.id === selected) : undefined;
  const current = busy && progress ? inputRunStageLabel(progress.phase) : outcome ?? phaseLabels[inputOptimizationPhase(run.state)];
  return h("article", { class: "project-run-item", "data-project-run-id": run.id },
    h("div", { class: "project-run-main" }, h("strong", {}, current), h("time", {}, dateLabel(run.createdAt))),
    h("code", {}, run.id.slice(0, 8)),
    progress?.completed !== undefined && progress.total !== undefined && busy ? h("progress", { class: "project-run-progress", value: progress.completed, max: progress.total }) : null,
    h("div", { class: "project-run-action" }, busy ? tag("Running", "accent")
      : !terminal ? button(failed ? "Retry" : "Continue", () => { void controller.resume(run); }, "secondary", "arrow")
      : model ? button("Candidate", () => actions.navigate({ page: "model", id: model.id }), "ghost", "arrow")
      : button("Activity", () => actions.navigate({ page: "activity" }), "ghost", "arrow")));
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
