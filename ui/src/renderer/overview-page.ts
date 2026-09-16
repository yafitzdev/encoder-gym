import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedRunStatus } from "../managed-control.js";
import { inputOptimizationTerminal } from "../input-optimization.js";
import type { Actions } from "./actions.js";
import type { InputRunsController } from "./input-runs-controller.js";
import type { OptimizationSetupController } from "./optimization-setup-controller.js";
import { button, disclosureIndicator, failureNotice, spinner, workspacePage } from "./components.js";
import { h } from "./dom.js";
import { dateLabel, metricInfo, score, delta, suiteName } from "./catalog.js";
import { inputRunProgress, pendingInputRunProgress, type ActivityView } from "./input-run-progress.js";
import type { OptimizationStage } from "../optimization-stages.js";
import { runContext } from "./workspace-pages.js";
import { comparisonTone, newerInputRun, overviewRecords, reportDecision, type OverviewRecord } from "./overview-records.js";
import { failureReason } from "../presentation-errors.js";
import { iterationReport } from "./iteration-report.js";

// The standalone renderer verification uses the same keyed replacement rule
// as the full application shell.
export { replaceView } from "./dom.js";

type Stage = "setup" | "status" | "report";
export interface OverviewState { expanded?: string | null; tabs: Map<string, Stage>; iterations: Map<string, number>; activityViews: Map<string, ActivityView>; draft: boolean; launching: boolean }
export const newOverviewState = (): OverviewState => ({ tabs: new Map(), iterations: new Map(), activityViews: new Map(), draft: false, launching: false });

export function renderOverview(workspace: WorkspaceSnapshot, state: OverviewState, setup: OptimizationSetupController, runs: InputRunsController, actions: Actions, managed?: ManagedRunStatus): HTMLElement {
  const roots = [...(runs.runs ?? [])];
  if (setup.run) {
    const index = roots.findIndex(run => run.id === setup.run!.id);
    if (index < 0) roots.push(setup.run);
    else if (setup.running || newerInputRun(setup.run, roots[index]!)) roots[index] = setup.run;
  }
  const records = overviewRecords(workspace, roots, managed);
  if (state.launching && setup.run) {
    if (state.expanded === "draft") state.expanded = setup.run.id;
    state.tabs.set(setup.run.id, "status"); state.launching = false; state.draft = false;
  }
  const busy = !!setup.preparationId || setup.running || setup.saving || setup.initializingEvaluation || !!runs.runningId;
  if (state.expanded === undefined && runs.runs && setup.data) {
    state.draft = !records.length;
    state.expanded = records[0]?.id ?? "draft";
  }
  const create = button("New run", () => {
    if (busy) return;
    if (setup.run) runs.retain(setup.run);
    setup.newDraft(); state.draft = true; state.expanded = "draft"; state.tabs.set("draft", "setup"); actions.render();
  }, "primary");
  create.id = "overview-new-run"; create.disabled = busy || setup.loading || runs.loading;
  const controllerError = setup.error ?? runs.error;
  const expandedRun = records.find(record => record.id === state.expanded)?.input;
  const expandedActivity = expandedRun
    ? setup.run?.id === expandedRun.id && setup.activity?.failure ? setup.activity : runs.activities.get(expandedRun.id)
    : undefined;
  const error = controllerError && (!expandedActivity?.failure
    || failureReason(controllerError) !== failureReason(expandedActivity.failure.message)) ? controllerError : undefined;
  const page = workspacePage("Overview", create,
    error ? h("div", { class: "operation-failure", role: "alert" }, failureNotice(error), !setup.data || !runs.runs ? button("Retry", () => { setup.refresh(); runs.refresh(); }, "secondary") : null) : null,
    (setup.loading || runs.loading) && !records.length ? h("div", { role: "status", class: "workspace-progress" }, spinner(), "Loading runs") : null,
    h("div", { class: "focus-runs", "aria-label": "Optimization runs" },
      state.draft ? row("draft", `Run ${String(records.length + 1).padStart(2, "0")}`, busy ? "Starting" : "Draft", undefined) : null,
      ...records.map(record => row(record.id, record.name, runLabel(record), record))));
  page.setAttribute("data-live-view", "overview");
  return page;

  function runBusy(record?: OverviewRecord): boolean { return !!record?.input && (runs.runningId === record.id || setup.running && setup.run?.id === record.id); }
  function needsRegistration(record: OverviewRecord): boolean {
    const experimentId = record.input?.experimentRunId ?? record.experiment?.id;
    return !!record.input?.outcome && !!experimentId && !!workspace.managed?.modelCatalog
      && (!record.experiment || record.experiment.candidates.some(candidate => candidate.model))
      && !workspace.managed.modelCatalog.artifacts.some(model => model.producingRun?.id === experimentId);
  }
  function runLabel(record: OverviewRecord): string {
    if (runBusy(record)) return "Running";
    if (needsRegistration(record)) return "Paused";
    const decision = reportDecision(record);
    if (decision) return decision;
    if (record.input?.state === "cancelled") return "Cancelled";
    if (record.input?.state === "agent_completed") return "Complete";
    if (record.input?.state === "agent_budget_exhausted") return "Budget exhausted";
    if (record.input?.state.endsWith("_failed")) return "Failed";
    if (record.experiment?.candidates.some(candidate => candidate.failure)) return "Failed";
    return "Paused";
  }
  function row(id: string, name: string, label: string, record?: OverviewRecord): HTMLElement {
    const expanded = state.expanded === id, running = runBusy(record) || id === "draft" && busy;
    const availableReport = !!record?.input?.iterations?.length || !!reportDecision(record ?? { id, name, createdAt: "" }) && !!record?.experiment;
    const availableStatus = !!record || busy;
    let tab = state.tabs.get(id) ?? (record && needsRegistration(record) ? "status" : availableReport && (!record?.input?.agentExecution || inputOptimizationTerminal(record.input.state)) ? "report" : record ? "status" : "setup");
    if (tab === "report" && !availableReport || tab === "status" && !availableStatus) tab = "setup";
    const panelId = "overview-panel-" + id;
    return h("article", { class: "focus-run" + (expanded ? " expanded" : ""), "data-run-id": id, "data-run-state": record?.input?.state },
      h("button", { type: "button", id: "overview-run-" + id, class: "focus-run-heading", "aria-expanded": String(expanded), "aria-controls": panelId,
        onClick: () => { state.expanded = expanded ? null : id; actions.render(); } },
      disclosureIndicator(expanded), h("strong", {}, name),
      h("span", { class: "focus-run-state " + (label === "KEEP" ? "success" : ["REJECT", "Failed"].includes(label) ? "danger" : "muted") }, running ? spinner(`overview-row:${id}`) : null, label),
      record ? h("time", { datetime: record.createdAt }, dateLabel(record.createdAt)) : h("span", {})),
      expanded ? h("div", { id: panelId, class: "focus-run-body" },
        h("nav", { class: "focus-stages", "aria-label": name + " stages" }, ...(["setup", "status", "report"] as Stage[]).map((stage, index) =>
          h("button", { type: "button", id: `overview-${id}-${stage}`, disabled: stage === "status" && !availableStatus || stage === "report" && !availableReport,
            class: stage === tab ? "active" : "", "aria-current": stage === tab ? "step" : null,
            onClick: () => { state.tabs.set(id, stage); actions.render(); } }, h("span", { class: "focus-stage-number" }, String(index + 1)), { setup: "Setup", status: "Status", report: "Report" }[stage]))),
        h("section", { class: "focus-panel", "aria-label": tab }, tab !== "setup" ? iterationSelector(record) : null,
          tab === "setup" ? setupPanel(record) : tab === "status" ? statusPanel(record) : record?.input?.iterations?.length
            ? iterationReport(selectedIteration(record), actions) : reportPanel(record!))) : null);
  }
  function selectedIteration(record: OverviewRecord) {
    const history = record.input?.iterations;
    return history?.find(item => item.number === (state.iterations.get(record.id) ?? history.at(-1)?.number));
  }
  function iterationSelector(record?: OverviewRecord): HTMLElement | null {
    const history = record?.input?.iterations;
    if (!record || !history?.length) return null;
    const selected = state.iterations.get(record.id) ?? history.at(-1)!.number;
    const selector = h("select", { id: `overview-${record.id}-iteration`, value: String(selected), onChange: (event: Event) => {
      state.iterations.set(record.id, Number((event.target as HTMLSelectElement).value)); actions.render();
    } }, h("option", { value: "0", selected: selected === 0 }, "Run setup"), ...history.map(item => h("option", { value: String(item.number), selected: item.number === selected }, `Iteration ${item.number}${item.selected ? " · Best so far" : ""}`)));
    return h("div", { class: "focus-controls iteration-navigation" }, h("label", { for: selector.id }, "Iteration"), selector,
      state.iterations.has(record.id) ? button(runBusy(record) ? "Live iteration" : "Latest iteration", () => {
        state.iterations.delete(record.id);
        state.activityViews.delete(`${record.id}:${history.at(-1)!.number}`); actions.render();
      }, "ghost small") : null);
  }
  function setupPanel(record?: OverviewRecord): HTMLElement {
    const saved = record?.input ? setup.data?.history.find(item => item.id === record.input!.setupId) : undefined;
    const pinnedModel = saved ? workspace.managed?.modelCatalog?.artifacts.find(item => item.id === saved.inputs.model.id) : undefined;
    const historical = !!record, datasetId = saved?.inputs.dataset.id ?? (historical ? "" : setup.datasetId);
    const benchmarkId = saved?.inputs.benchmark.id ?? (historical ? "" : setup.benchmarkId);
    const datasets: [string, string][] = setup.data?.datasets.flatMap(entry => entry.versions.map(item => [item.version.id, `${entry.dataset.name} · v${item.version.number}`] as [string, string])) ?? [];
    const benchmarks: [string, string][] = setup.data?.benchmarks.filter(item => historical || item.id === setup.benchmarkId).map(item => [item.id, `Benchmark · v${item.number}`]) ?? [];
    const launch = setup.launches?.find(item => item.id === record?.input?.launchId);
    // Never label a historical run using today's provider settings.
    const providers = !historical || launch?.scope.providerCatalog.id === setup.workspace.providerCatalog?.id ? setup.workspace.providerCatalog?.providers : undefined;
    const selector = (id: string, label: string, options: [string, string][], value: string, change?: (value: string) => void) => h("label", { class: "focus-field", for: id },
      h("span", {}, label), h("select", { id, value, disabled: historical || busy || setup.loading, onChange: (event: Event) => change?.((event.target as HTMLSelectElement).value) },
      ...options.map(([key, text]) => h("option", { value: key }, text))));
    const modelName = historical ? pinnedModel?.name ?? record.experiment?.baseline.key ?? "Not recorded" : setup.model?.name ?? "Choose in Models";
    const provider = (role: "advisor" | "generation", label: string) => {
      const configured = providers?.find(item => item.role === role);
      return selector("optimization-" + role, label,
        [[role, configured ? `${configured.endpoint ? providerName(configured.endpoint) + " · " : ""}${configured.model}` : historical ? launch ? "Earlier project settings" : "Not recorded" : "Choose in Project settings"], ...(!historical ? [["settings", "Project settings…"] as [string, string]] : [])], role,
        value => { if (value === "settings") actions.navigate({ page: "project" }); });
    };
    const start = button("Optimize", () => {
      if (!setup.canOptimize || busy) return;
      state.launching = true; state.tabs.set("draft", "status");
      void setup.optimize().finally(() => { if (!setup.run) state.launching = false; actions.render(); });
    }, "primary");
    start.id = "optimization-start"; start.disabled = !setup.canOptimize || busy;
    return h("div", { class: "focus-setup" },
      selector("optimization-baseline", "Baseline", [["baseline", modelName], ...(!historical ? [["models", "Models…"] as [string, string]] : [])], "baseline", value => { if (value === "models") actions.navigate({ page: "models" }); }),
      selector("optimization-dataset", "Starting Dataset", [["", historical ? "Not recorded" : "Choose dataset"], ...datasets], datasetId, value => setup.select("dataset", value)),
      selector("optimization-benchmark", "Evaluation", [...(!benchmarks.length ? [["", historical ? "Not recorded" : setup.workspace.scientificBinding ? "Project evaluation" : "Choose in Evaluation"] as [string, string]] : []), ...benchmarks, ...(!historical ? [["evaluation", "Evaluation…"] as [string, string]] : [])], benchmarkId, value => { if (value === "evaluation") actions.navigate({ page: "benchmarks" }); }),
      provider("advisor", "LLM agent"), provider("generation", "LLM data generator"),
      !historical ? h("div", { class: "focus-controls" }, start) : null);
  }
  function statusPanel(record?: OverviewRecord): HTMLElement {
    const iteration = record?.input?.iterations?.length ? selectedIteration(record) ?? null : undefined;
    const key = (record?.id ?? "draft") + (iteration !== undefined ? `:${iteration?.number ?? 0}` : "");
    let view = state.activityViews.get(key);
    if (!view) { view = { scroll: {} }; state.activityViews.set(key, view); }
    const navigation = { key: `overview-${key}`, view, change: (stage?: OptimizationStage) => {
      view.stage = stage;
      if (record && iteration && stage) state.iterations.set(record.id, iteration.number);
      actions.render();
    } };
    if (!record?.input) {
      if (!record) {
        const stop = button(setup.stopping ? "Stopping…" : "Stop", () => { void setup.stop(); }, "secondary");
        stop.disabled = setup.stopping;
        return h("div", { class: "focus-status" }, pendingInputRunProgress(setup.initializingEvaluation ? setup.benchmarkProgress : setup.liveProgress,
          "overview-draft-progress", setup.liveEvents, busy ? stop : undefined, busy, navigation));
      }
      return h("div", {}, h("h2", {}, runLabel(record)), h("ol", { class: "focus-events" }, ...(record.experiment?.activity.slice(-5).reverse().map(event => h("li", {}, h("time", {}, clock(event.at)), event.kind.replaceAll("_", " "))) ?? [])),
        record.managed && !["completed", "cancelled", "failed"].includes(record.managed.state) ? button("Continue", () => actions.navigate({ page: "optimization" }), "primary") : null);
    }
    const run = record.input, setupBusy = setup.running && setup.run?.id === run.id, running = runBusy(record);
    if (!running) void runs.ensureActivity(run.id);
    const activity = setupBusy ? setup.activity : runs.activities.get(run.id) ?? (setup.run?.id === run.id ? setup.activity : undefined);
    const context = runContext(run, workspace, setup), stopping = setupBusy ? setup.stopping : runs.stoppingId === run.id;
    if (iteration) {
      context.candidate = `Iteration ${iteration.number} candidate`;
      context.model = workspace.managed?.modelCatalog?.artifacts.find(model => model.id === iteration.startingModelId)?.name ?? "Recorded starting model";
      context.dataset = `Iteration ${iteration.number} · ${iteration.trainingDatasetVersionId ? "Training dataset" : "Input dataset"}`;
    }
    const stop = button(stopping ? "Stopping…" : "Stop", () => { void (setupBusy ? setup.stop() : runs.stop(run)); }, "secondary");
    stop.disabled = stopping;
    const resume = button("Resume", () => { void runs.resume(run); }, "primary"); resume.disabled = busy;
    return h("div", { class: "focus-status" },
      runs.activityErrors.has(run.id) ? failureNotice(runs.activityErrors.get(run.id)) : null,
      activity?.failure ? h("div", { class: "operation-failure", role: "alert" }, failureNotice(activity.failure.message)) : null,
      inputRunProgress({ run, running, activity, iteration, startedAt: activity ? Date.parse(activity.startedAt) : setup.startedAt, context,
        liveProgress: setupBusy ? setup.liveProgress : runs.runningId === run.id ? runs.liveProgress : undefined,
        liveProgressAt: setupBusy ? setup.liveProgressAt : runs.runningId === run.id ? runs.liveProgressAt : undefined,
        liveEvents: setupBusy ? setup.liveEvents : runs.runningId === run.id ? runs.liveEvents : [],
        registrationPending: needsRegistration(record), animationKey: `overview-progress:${run.id}`, navigation,
        controls: running ? stop : !inputOptimizationTerminal(run.state) || needsRegistration(record) ? resume : undefined }));
  }
}

function reportPanel(record: OverviewRecord): HTMLElement {
  const decision = reportDecision(record), run = record.experiment!;
  return h("div", { class: "focus-report" }, h("h2", { class: decision === "KEEP" ? "success" : "danger" }, decision),
    ...run.candidates.filter(candidate => candidate.development.length).map(candidate => h("section", {},
      run.candidates.length > 1 ? h("h3", {}, `Candidate ${candidate.sequence}`) : null,
      h("div", { class: "focus-table-scroll" }, h("table", { class: "focus-comparison" },
        h("thead", {}, h("tr", {}, ...["Benchmark", "Baseline", "Candidate", "Change"].map(label => h("th", { scope: "col" }, label)))),
        h("tbody", {}, ...candidate.development.flatMap(result => result.checks.map(check => {
          const tone = comparisonTone(check.baseline, check.candidate, run.directions[check.metric]);
          return h("tr", {}, h("th", { scope: "row" }, suiteName(result.report.suite), h("small", {}, metricInfo(check.metric).label)),
            h("td", {}, score(check.baseline, check.metric)), h("td", { class: tone }, score(check.candidate, check.metric)),
            h("td", { class: tone }, delta(check.candidate - check.baseline, check.metric)));
        }))))))));
}
function clock(value: string): string { return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }); }
function providerName(endpoint: string): string { try { const host = new URL(endpoint).hostname; return host === "api.deepseek.com" ? "DeepSeek" : host === "yan.tail85512d.ts.net" ? "Yan" : host; } catch { return "Configured provider"; } }
