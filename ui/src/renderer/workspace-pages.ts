import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedReadiness, ManagedRunStatus } from "../managed-control.js";
import type { Actions } from "./actions.js";
import { bytesLabel, comparisonGroups, dateLabel, evaluationGroups, initialFilter, metricInfo, primaryMetric, score, setupName, suiteName, summaryMetrics, timeLabel } from "./catalog.js";
import { button, copyField, details, empty, facts, failureNotice, pageHeader, sectionHeader, status, tag, workspacePage } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";
import type { InputRunsController } from "./input-runs-controller.js";
import type { OptimizationSetupController } from "./optimization-setup-controller.js";
import { inputOptimizationPhase, inputOptimizationTerminal, type InputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { inputRunStageLabel, type InputRunStageContext } from "./input-run-activity.js";
import { inputRunProgress, pendingInputRunProgress } from "./input-run-progress.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions, optimization?: ManagedRunStatus, projectRuns?: InputRunsController, setup?: OptimizationSetupController): HTMLElement {
  const knownProjectRuns = projectRuns?.runs ?? [];
  const projectRunValues = setup?.run && !knownProjectRuns.some(run => run.id === setup.run!.id) ? [setup.run, ...knownProjectRuns] : knownProjectRuns;
  const projectRunIds = new Set(projectRunValues.map(run => run.id));
  const experimentRuns = workspace.runs.filter(run => !run.optimizationId || !projectRunIds.has(run.optimizationId));
  const optimizationIsListed = !!optimization && (projectRunIds.has(optimization.run_id) || experimentRuns.some(run => run.optimizationId === optimization.run_id));
  const records: Array<{ kind: "project"; createdAt: string; run: InputOptimizationRun } | { kind: "experiment"; createdAt: string; run: WorkspaceSnapshot["runs"][number] } | { kind: "managed"; createdAt: string; run: ManagedRunStatus }> = [
    ...projectRunValues.map(run => ({ kind: "project" as const, createdAt: run.createdAt, run })),
    ...experimentRuns.map(run => ({ kind: "experiment" as const, createdAt: run.createdAt, run })),
    ...(optimization && !optimizationIsListed ? [{ kind: "managed" as const, createdAt: optimization.created_at, run: optimization }] : []),
  ].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  const entries = records.map((record, index) => {
    const label = `Run ${String(index + 1).padStart(2, "0")}`;
    if (record.kind === "project") return projectRun(record.run, label, workspace, actions, projectRuns!, setup);
    if (record.kind === "managed") return managedRun(record.run, label, actions);
    return runListItem(record.run, workspace, actions, record.run.optimizationId === optimization?.run_id ? () => actions.navigate({ page: "optimization" }) : undefined, label);
  }).reverse();
  const pending = !!setup?.running && !setup.run;
  return workspacePage("Runs", null,
    projectRuns?.error ? h("section", { class: "operation-failure", role: "alert" }, "Could not load runs. ", button("Retry", () => projectRuns.refresh(), "secondary")) : null,
    projectRuns?.loading ? h("div", { class: "workspace-progress", role: "status" }, "Loading runs…") : null,
    pending || entries.length ? h("div", { class: "unified-run-list", "aria-label": "Optimization runs" },
      pending ? h("article", { class: "unified-run-entry is-running pending-run" },
        h("div", { class: "project-run-item" }, h("span", { class: "run-number" }, "New run"), h("span", { class: "run-list-name" }, h("strong", {}, "Optimization"), h("small", {}, "Creating run record")), status("Starting", "neutral")),
        pendingInputRunProgress()) : null,
      ...entries) : empty("No runs"));
}

const phaseLabels: Record<InputOptimizationPhase, string> = {
  checking_inputs: "Checking inputs", preparing_data: "Preparing data", starting: "Starting", training: "Training",
  saving_candidate: "Saving candidate", evaluating: "Evaluating", complete: "Complete",
};
function projectRun(run: InputOptimizationRun, label: string, workspace: WorkspaceSnapshot, actions: Actions, controller: InputRunsController, setup?: OptimizationSetupController): HTMLElement {
  const outcome = run.state === "candidate_accepted" ? "Candidate passed" : run.state === "candidate_rejected" ? "Candidate did not pass" : run.state === "baseline_retained" ? "No improvement" : run.state === "cancelled" ? "Cancelled" : undefined;
  const failed = run.state.endsWith("_failed"), terminal = inputOptimizationTerminal(run.state);
  const setupBusy = !!setup?.running && setup.run?.id === run.id;
  const busy = setupBusy || controller.runningId === run.id;
  const setupCancelling = !!setup?.cancelling && setup.run?.id === run.id;
  const cancelling = setupCancelling || controller.cancellingId === run.id;
  const activity = setupBusy ? setup?.activity : controller.activities.get(run.id);
  const phase = phaseLabels[inputOptimizationPhase(run.state)];
  const current = outcome ?? (busy && activity?.progress ? inputRunStageLabel(activity.progress.phase) : failed ? `${phase} failed` : phase);
  return h("article", { class: "unified-run-entry" + (busy ? " is-running" : ""), "data-project-run-id": run.id },
    h("div", { class: "project-run-item" },
      h("span", { class: "run-number" }, label),
      h("span", { class: "run-list-name" }, h("strong", {}, "Optimization"), h("small", {}, current)),
      h("div", { class: "project-run-state" }, cancelling && !terminal ? status("Stopping", "neutral") : busy && !terminal ? status("Running", "neutral") : status(failed ? "Failed" : terminal ? "Complete" : "Paused", failed ? "danger" : "neutral")),
      h("time", {}, dateLabel(run.createdAt)),
      h("div", { class: "project-run-action" },
        button("Open", () => actions.navigate({ page: "run", id: run.id }), "secondary", "arrow"),
        !terminal && busy ? button(cancelling ? "Stopping…" : "Stop", () => { void (setupBusy ? setup!.cancel() : controller.cancel(run)); }, "ghost danger")
          : null)),
    busy && !terminal ? inputRunProgress({ run, running: true, startedAt: setupBusy ? setup?.startedAt : activity ? Date.parse(activity.startedAt) : undefined, activity, context: runContext(run, workspace, setup) }) : null);
}

export function inputRunById(controller: InputRunsController | undefined, setup: OptimizationSetupController | undefined, id: string | undefined): InputOptimizationRun | undefined {
  if (!id) return undefined;
  return setup?.run?.id === id ? setup.run : controller?.runs?.find(run => run.id === id);
}

export function inputRunName(workspace: WorkspaceSnapshot, run: InputOptimizationRun, controller?: InputRunsController, setup?: OptimizationSetupController): string {
  const projectRuns = [...(controller?.runs ?? [])];
  if (setup?.run && !projectRuns.some(item => item.id === setup.run!.id)) projectRuns.push(setup.run);
  const ids = new Set(projectRuns.map(item => item.id));
  const dates = [...workspace.runs.filter(item => !item.optimizationId || !ids.has(item.optimizationId)).map(item => item.createdAt), ...projectRuns.map(item => item.createdAt)].sort();
  return `Run ${String(Math.max(0, dates.findIndex(createdAt => createdAt === run.createdAt)) + 1).padStart(2, "0")}`;
}

export function renderInputRun(workspace: WorkspaceSnapshot, run: InputOptimizationRun, label: string, actions: Actions, controller: InputRunsController, setup?: OptimizationSetupController): HTMLElement {
  const terminal = inputOptimizationTerminal(run.state), failed = run.state.endsWith("_failed");
  const setupRun = setup?.run?.id === run.id, setupBusy = !!setup?.running && setupRun;
  const busy = setupBusy || controller.runningId === run.id;
  const cancelling = (!!setup?.cancelling && setupRun) || controller.cancellingId === run.id;
  const activity = setupBusy ? setup?.activity ?? controller.activities.get(run.id) : controller.activities.get(run.id);
  const selected = run.finalResult?.modelId ?? run.outcome?.selectedModelId;
  const model = selected ? workspace.managed?.modelCatalog?.artifacts.find(artifact => artifact.sourceModel?.id === selected || artifact.id === selected) : undefined;
  return h("div", { class: "page-content detail-page input-run-view" },
    button("All runs", () => actions.backTo("runs"), "back-link", "back"),
    pageHeader(label, h("div", { class: "inline-group" }, tag(dateLabel(run.createdAt)), tag(timeLabel(run.createdAt)))),
    activity?.failure ? h("section", { class: "operation-failure", role: "alert" }, failureNotice(activity.failure.message)) : null,
    inputRunProgress({ run, running: busy, startedAt: setupBusy ? setup?.startedAt : activity ? Date.parse(activity.startedAt) : undefined, activity, context: runContext(run, workspace, setup) }),
    h("div", { class: "input-run-actions" },
      !terminal && busy ? button(cancelling ? "Stopping…" : "Stop", () => { void (setupBusy ? setup!.cancel() : controller.cancel(run)); }, "secondary danger") : null,
      !terminal && !busy ? button(failed ? "Retry" : "Continue", () => { void controller.resume(run); }, "primary") : null,
      terminal && model ? button("Inspect candidate", () => actions.navigate({ page: "model", id: model.id }), "secondary", "arrow") : null,
      button("Activity", () => actions.navigate({ page: "activity" }), "ghost", "arrow")),
    details("Run identity", facts([["Run", copyField(run.id, actions.copy)], ["Setup", copyField(run.setupId, actions.copy)]])));
}

function runContext(run: InputOptimizationRun, workspace: WorkspaceSnapshot, controller?: OptimizationSetupController): InputRunStageContext {
  const saved = controller?.data?.history.find(item => item.id === run.setupId);
  const modelId = saved?.inputs.model.id ?? (controller?.run?.id === run.id ? controller.model?.id : undefined);
  const model = modelId ? workspace.managed?.modelCatalog?.artifacts.find(item => item.id === modelId) : controller?.model;
  const datasetId = saved?.inputs.dataset.id ?? (controller?.run?.id === run.id ? controller.datasetId : undefined);
  const dataset = datasetId ? controller?.data?.datasets.flatMap(entry => entry.versions.map(version => ({ entry, version }))).find(item => item.version.version.id === datasetId) : controller?.dataset;
  const benchmarkId = saved?.inputs.benchmark.id ?? (controller?.run?.id === run.id ? controller.benchmarkId : undefined);
  const benchmark = benchmarkId ? controller?.data?.benchmarks.find(item => item.id === benchmarkId) : controller?.benchmark;
  return {
    model: model?.name ?? "Baseline model",
    dataset: dataset ? `${dataset.entry.dataset.name} · v${dataset.version.version.number}` : "Starting dataset",
    datasetRows: dataset?.version.rows ?? 0,
    evaluation: benchmark ? `Evaluation · Version ${benchmark.number}` : "Evaluation",
    developmentSuites: benchmark?.definition.suites.filter(suite => suite.role === "development").map(suite => suite.key.replaceAll("_", " ")) ?? [],
    finalSuite: benchmark?.definition.suites.find(suite => suite.role === "sealed_acceptance")?.key.replaceAll("_", " "),
    finalEvaluation: run.state === "evaluating_final" || run.state === "final_evaluation_failed",
  };
}

function managedRun(run: ManagedRunStatus, label: string, actions: Actions): HTMLElement {
  const active = !["completed", "cancelled", "failed"].includes(run.state);
  return h("article", { class: "unified-run-entry", "data-managed-run-id": run.run_id },
    h("div", { class: "project-run-item" },
      h("span", { class: "run-number" }, label),
      h("span", { class: "run-list-name" }, h("strong", {}, "Optimization"), h("small", {}, run.stage.label)),
      h("div", { class: "project-run-state" }, status(run.decision?.replaceAll("_", " ") ?? run.state.replaceAll("_", " "), "neutral")),
      h("time", {}, dateLabel(run.created_at)),
      h("div", { class: "project-run-action" }, button(active ? "Continue" : "Open", () => actions.navigate({ page: "optimization" }), "secondary", "arrow"))));
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
