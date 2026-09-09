import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, comparisonGroups, initialFilter, metricInfo, primaryMetric, score, setupName, suiteName, summaryMetrics } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  return h("div", { class: "page-content" }, pageHeader("Runs", workspace.managed ? "Start, continue, and inspect this project's bounded optimization work." : "Each run records an experiment and the evidence behind its decision.", workspace.managed ? button("Start optimization", actions.prepareOptimization, "primary", "runs") : tag(workspace.runs.length + " recorded runs")),
    workspace.managed ? h("p", { class: "section-note" }, workspace.runs.length ? `${workspace.runs.length} persisted experiment ${workspace.runs.length === 1 ? "run is" : "runs are"} available from this project's bound scientific store.` : "A run will appear here as soon as its immutable experiment record is reserved in the bound scientific store.") : null,
    workspace.runs.length ? h("div", { class: "run-list" }, ...workspace.runs.map(run => runListItem(run, workspace, actions))) : empty("No runs recorded yet", workspace.managed ? "Check the project's launch requirements, prepare its reviewed definition, and reserve the first bounded optimization run." : "The baseline is registered. Reload after a compatible CLI experiment writes records to this legacy folder.", workspace.managed ? button("Check launch requirements", actions.prepareOptimization, "primary", "arrow") : button("Back to models", () => actions.navigate({ page: "models" }))),
    workspace.runs.length ? h("p", { class: "table-footnote" }, "A completed run can keep the baseline. Open a run to inspect its candidates, execution history, and recorded limits.") : null);
}
export function renderBenchmarks(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const groups = comparisonGroups(workspace, initialFilter());
  if (!groups.length) return h("div", { class: "page-content" }, pageHeader("Benchmarks", "The tests used to measure baseline and candidate quality."),
    empty("No run evaluations recorded yet", workspace.managed ? "Importing a model or dataset does not create evaluation results. Start a bounded optimization, or explicitly bring matching Encoder Gym scientific history through Project settings." : "No comparison setup has been recorded in this folder. Reload after a compatible experiment produces evaluation evidence.", button("Back to models", () => actions.navigate({ page: "models" }))));
  return h("div", { class: "page-content" }, pageHeader("Benchmarks", "What each evaluation measures, and which candidate comparisons are valid."),
    h("p", { class: "section-note" }, "Compare candidates within a group: they share the same baseline, tests, and scoring rules. Identical metric names alone do not make different groups comparable."),
    ...groups.map((group, i) => {
      const primary = primaryMetric(group.run);
      return h("section", { class: "benchmark-section" }, sectionHeader(group.label, tag(i === 0 ? "Latest recorded setup" : "Historical setup")),
        h("p", { class: "section-note" }, group.description),
        ...group.run.baselines.map(base => h("div", { class: "benchmark-row" }, h("div", {}, h("h3", {}, suiteName(base.suite)), h("p", {}, "Development evidence · " + Object.keys(base.metrics).length + " recorded metrics"), h("code", { class: "muted" }, base.suite)),
          h("div", {}, h("span", { class: "muted" }, "Baseline " + metricInfo(primary).label.toLowerCase()), h("strong", { class: "score" }, score(base.metrics[primary], primary))),
        )),
        details("Evaluation identities", facts(group.run.baselines.flatMap(base => [[suiteName(base.suite) + " · benchmark", copyField(base.suiteFingerprint, actions.copy)], [suiteName(base.suite) + " · metric contract", copyField(base.contractFingerprint, actions.copy)]]))),
        button("Compare " + group.rows.length + (group.rows.length === 1 ? " candidate" : " candidates"), () => actions.navigate({ page: "models", id: group.id }), "ghost", "arrow"),
      );
    }),
    h("section", { class: "reading-note" }, h("h2", {}, "Development and final acceptance"), h("p", {}, "Development scores help you compare and diagnose candidates. Separate sealed evaluation is used only for an explicitly selected candidate's final acceptance. Its scores never enter this comparison workspace."), h("p", {}, "The run record shows whether final acceptance was used and its recorded outcome. Use CLI Doctor to verify current authority before starting another experiment.")),
  );
}
export function renderBaseline(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const groups = comparisonGroups(workspace, initialFilter()).filter(g => g.run.baseline.fingerprint === workspace.baseline.fingerprint);
  const evaluations = groups.map(g => {
    const keys = summaryMetrics(g.run);
    const head = h("thead", {}, h("tr", {}, h("th", { scope: "col" }, "Benchmark"), ...keys.map(k => h("th", { scope: "col", class: "numeric" }, metricInfo(k).label))));
    const body = h("tbody", {}, ...g.run.baselines.map(b => h("tr", {}, h("th", { scope: "row" }, suiteName(b.suite)), ...keys.map(k => h("td", { class: "numeric score" }, score(b.metrics[k], k))))));
    return h("section", { class: "baseline-evaluation" }, h("h3", {}, setupName(g.run)), h("div", { class: "table-scroll", tabindex: "0", "aria-label": "Baseline evaluations" }, h("table", { class: "evidence-table" }, head, body)));
  });
  return h("div", { class: "page-content detail-page" }, button("All models", () => actions.backTo("models"), "back-link", "back"),
    pageHeader("Baseline encoder", "The reference artifact recorded for this project.", tag("Baseline", "accent")),
    h("div", { class: "detail-columns" }, h("section", {}, sectionHeader("Reference artifact"), facts([["Format", workspace.baseline.format], ["Size", bytesLabel(workspace.baseline.bytes)], ["Artifact", copyField(workspace.baseline.key, actions.copy)], ["Fingerprint", copyField(workspace.baseline.fingerprint, actions.copy)]])),
      workspace.deployment ? h("section", {}, sectionHeader("Recorded inference export"), h("p", { class: "section-note" }, "This ONNX export was recorded alongside the reference checkpoint. Its presence does not establish that it is currently deployed."), facts([["Format", "FP32 ONNX"], ["Size", bytesLabel(workspace.deployment.bytes)], ["Artifact", copyField(workspace.deployment.key, actions.copy)], ["Fingerprint", copyField(workspace.deployment.fingerprint, actions.copy)]])) : null),
    sectionHeader("Baseline evaluations"), ...evaluations,
    workspace.managed ? details("Imported checkpoint files", facts(workspace.managed.manifest.baseline.files.map(file => [file.path, `${bytesLabel(file.bytes)} · ${file.fingerprint}`]))) : null,
    !evaluations.length ? h("p", { class: "section-note" }, "No evaluations from recorded runs are available for this baseline yet.") : null,
  );
}
