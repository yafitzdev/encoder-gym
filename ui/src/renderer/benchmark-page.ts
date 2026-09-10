import type { BenchmarkReport, ProjectBenchmarkResults, ProjectBenchmarkVersion } from "../benchmark-workspace.js";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions, Location } from "./actions.js";
import type { BenchmarkController } from "./benchmark-controller.js";
import { button, copyField, details, empty, facts, failureNotice, selectControl, tabs, tag, workspacePage } from "./components.js";
import { dateLabel, humanize, metricInfo, runLabel, score, suiteName, timeLabel } from "./catalog.js";
import { h } from "./dom.js";

function inspectReport(report: BenchmarkReport, results: ProjectBenchmarkResults, actions: Actions): void {
  const dialog = document.getElementById("project-dialog") as HTMLDialogElement, opener = document.activeElement as HTMLElement | null;
  const modelLink = (id: string) => button(results.models.find(model => model.modelId === id)?.name ?? "Inspect model", () => { dialog.close(); actions.navigate({ page: "model", id }); }, "ghost small");
  dialog.replaceChildren(h("div", { class: "dataset-row-dialog benchmark-report-dialog" },
    h("div", { class: "dialog-heading" }, h("h2", { id: "project-dialog-title" }, "Evaluation result"), button("Close", () => dialog.close(), "ghost small", "close")),
    facts([["Test", suiteName(report.result.suite_key)], ["Recorded", `${dateLabel(report.result.created_at)} · ${timeLabel(report.result.created_at)}`],
      ...Object.entries(report.result.metrics).map(([key, value]): [string, string] => [metricInfo(key).label, score(value, key)])]),
    ...report.contexts.map(context => details("Run " + context.runId.slice(0, 8), h("div", {},
      facts([["Compared against", modelLink(context.baselineModelId)], ["Original result", context.assessment ? context.assessment.verdict === "passed" ? "Passed development checks" : "Failed development checks" : "Baseline measurement"]]),
      context.assessment ? h("div", { class: "table-scroll" }, h("table", { class: "benchmark-rules" },
        h("thead", {}, h("tr", {}, ...["Metric", "Baseline", "Model", "Check"].map(label => h("th", { scope: "col" }, label)))),
        h("tbody", {}, ...context.assessment.gates.map(gate => h("tr", {}, h("th", { scope: "row" }, metricInfo(gate.key).label), h("td", {}, score(gate.baseline, gate.key)), h("td", {}, score(gate.candidate, gate.key)), h("td", { class: gate.passed ? "success" : "danger" }, gate.passed ? "Passed" : "Failed")))))) : null,
      details("Run details", facts([["Run ID", copyField(context.runId, actions.copy)], ["Baseline revision", copyField(context.baselineRevisionId, actions.copy)]]))), report.contexts.length === 1)),
    details("Report details", facts([["Report ID", copyField(report.result.report_id, actions.copy)], ["Fingerprint", copyField(report.result.report_fingerprint, actions.copy)]]))));
  dialog.addEventListener("close", () => { const target = opener?.isConnected ? opener : document.getElementById(`benchmark-report-${report.result.report_id}`); target?.focus(); }, { once: true }); dialog.showModal();
}
function protocol(version: ProjectBenchmarkVersion, actions: Actions): HTMLElement {
  const definition = version.definition;
  return h("div", { class: "benchmark-protocol" },
    h("div", { class: "artifact-list" }, ...definition.suites.map(suite => h("div", { class: "artifact-row" }, h("div", { class: "artifact-row-name" }, h("h2", {}, suiteName(suite.key))), tag(suite.role === "development" ? "Development" : "Final holdout")))),
    h("h2", {}, "Scoring rules"), h("div", { class: "table-scroll" }, h("table", { class: "benchmark-rules" },
      h("thead", {}, h("tr", {}, ...["Metric", "Test", "Requirement"].map(label => h("th", { scope: "col" }, label)))),
      h("tbody", {}, ...definition.metric_contract.gates.map(gate => h("tr", {}, h("th", { scope: "row" }, metricInfo(gate.key).label), h("td", {}, gate.suite_key ? suiteName(gate.suite_key) : gate.role === "development" ? "All development tests" : "Final holdout"),
        h("td", {}, `${humanize(gate.condition.kind)}: ${score(gate.condition.value, gate.key)}`)))))),
    details("Benchmark details", facts([["Version ID", copyField(version.id, actions.copy)], ["Fingerprint", copyField(definition.fingerprint, actions.copy)], ["Evaluator revision", definition.source_revision]])));
}
export function renderBenchmarkPage(workspace: WorkspaceSnapshot, location: Location, controller: BenchmarkController, actions: Actions): HTMLElement {
  const version = controller.selected(location), failure = controller.failure(version?.id);
  const navigate = (changes: Partial<Location>) => actions.navigate({ ...location, page: "benchmarks", id: version?.id, ...changes });
  const tab = ["results", "protocol", "versions"].includes(location.tab ?? "") ? location.tab! : "results";
  const choose = () => controller.choose(workspace.runs.filter(run => run.projectId === workspace.managed?.scientificBinding?.runtime.projectSnapshot.id).map(run => [run.id, `${runLabel(run, workspace)} · ${dateLabel(run.createdAt)}`]));
  const hasSource = workspace.runs.some(run => run.projectId === workspace.managed?.scientificBinding?.runtime.projectSnapshot.id);
  const change = button("Choose recorded benchmark", choose, "secondary"); change.disabled = controller.saving;
  const refresh = button("Refresh", () => controller.refresh(), "ghost", "refresh"); refresh.disabled = controller.saving;
  let body: HTMLElement;
  if (!controller.versions) body = h("div", { role: "status" }, failure ? "" : "Loading benchmark…");
  else if (!controller.versions.length) body = empty("No benchmark", hasSource ? change : button("Project settings", () => actions.navigate({ page: "project" }), "secondary"));
  else if (!version) body = empty("Version not found", button("Latest version", () => actions.navigate({ page: "benchmarks" }), "secondary"));
  else {
    const definition = version.definition, metric = definition.metric_contract.definitions.some(metric => metric.key === location.metric) ? location.metric! : definition.metric_contract.primary_metric;
    const results = controller.result(version.id), suites = definition.suites.filter(suite => suite.role === "development");
    const panel = tab === "protocol" ? protocol(version, actions) : tab === "versions" ? h("div", {},
      hasSource ? h("div", { class: "benchmark-history-actions" }, change) : null,
      h("div", { class: "artifact-list" }, ...[...controller.versions].reverse().map(item => h("div", { class: "artifact-row" }, h("div", { class: "artifact-row-name" }, h("h2", {}, `Version ${item.number}`), h("span", { class: "muted" }, dateLabel(item.createdAt))), button("Inspect version", () => navigate({ id: item.id, tab: "results" }), "ghost small"))))) :
      !results ? h("div", { role: "status" }, failure ? "" : "Loading results…") : h("div", {},
        selectControl("benchmark-metric", "Metric", definition.metric_contract.definitions.map(item => [item.key, `${metricInfo(item.key).label} ${item.direction === "higher_is_better" ? "↑" : "↓"}`]), metric, metric => navigate({ metric })),
        h("div", { class: "table-scroll benchmark-table-scroll", tabindex: "0", "aria-label": "Model comparison" }, h("table", { class: "benchmark-results" },
          h("thead", {}, h("tr", {}, h("th", { scope: "col" }, "Model"), ...suites.map(suite => h("th", { scope: "col" }, suiteName(suite.key))))),
          h("tbody", {}, ...results.models.map(model => h("tr", { "data-benchmark-model": model.modelId }, h("th", { scope: "row" },
            button(model.name, () => actions.navigate({ page: "model", id: model.modelId }), "ghost small benchmark-model-link"), model.isBaseline ? tag("Baseline", "accent") : null),
            ...suites.map(suite => {
              const reports = model.reports.filter(report => report.result.suite_key === suite.key);
              return h("td", {}, reports.length ? reports.map(report => h("div", { class: "benchmark-measurement" },
                h("button", { type: "button", id: `benchmark-report-${report.result.report_id}`, class: "benchmark-score", "data-report-id": report.result.report_id, "aria-label": `Inspect ${metricInfo(metric).label} for ${model.name}`, onClick: () => inspectReport(report, results, actions) }, score(report.result.metrics[metric], metric)),
                reports.length > 1 ? h("small", { class: "muted" }, `${dateLabel(report.result.created_at)} · ${timeLabel(report.result.created_at)}`) : null)) : h("span", { class: "muted" }, "Not evaluated"));
            })))))));
    body = h("div", { class: "benchmark-view" }, selectControl("benchmark-version", "Version", controller.versions.map(item => [item.id, `Version ${item.number}`]), version.id, id => navigate({ id })),
      tabs([["results", "Results"], ["protocol", "Protocol"], ["versions", "Versions"]], tab, tab => navigate({ tab })),
      h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${tab}` }, panel));
  }
  return workspacePage("Evaluation", refresh,
    failure ? h("div", { class: "operation-failure", role: "alert" }, failureNotice(failure), button("Retry", () => { void controller.retry(); }, "secondary")) : null,
    controller.saving ? h("div", { role: "status", class: "dataset-progress" }, "Saving benchmark…") : null, body);
}
