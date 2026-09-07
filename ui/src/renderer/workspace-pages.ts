import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, comparisonGroups, initialFilter, metricInfo, primaryMetric, score, setupName, suiteName, summaryMetrics } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  return h("div", { class: "page-content" }, pageHeader("Runs", "Each run records an experiment and the evidence behind its decision.", tag(workspace.runs.length + " recorded runs")),
    workspace.runs.length ? h("div", { class: "run-list" }, ...workspace.runs.map(run => runListItem(run, workspace, actions))) : empty("No runs recorded yet", "The baseline is registered. Experiments will appear here as the CLI records them.", button("Back to models", () => actions.navigate({ page: "models" }))),
    h("p", { class: "table-footnote" }, "A completed run can keep the baseline. Open a run to inspect its candidates, execution history, and recorded limits."));
}
export function renderBenchmarks(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const groups = comparisonGroups(workspace, initialFilter());
  return h("div", { class: "page-content" }, pageHeader("Benchmarks", "What each evaluation measures, and which candidate comparisons are valid."),
    h("div", { class: "reading-note" }, h("h2", {}, "Compare within the same evaluation setup"), h("p", {}, "A setup pins the baseline artifact, benchmark identities, metric requirements, and evaluation policy. Different setups are kept separate even when they use the same metric names.")),
    !groups.length ? empty("No run evaluations recorded yet", "Baseline and candidate benchmark results will appear with recorded experiment evidence.") : null,
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
  return h("div", { class: "page-content" }, button("All models", () => actions.navigate({ page: "models" }), "back-link", "back"),
    pageHeader("Baseline encoder", "The reference artifact recorded for this project.", tag("Baseline", "accent")),
    h("div", { class: "detail-columns" }, h("section", {}, sectionHeader("Reference artifact"), facts([["Format", workspace.baseline.format], ["Size", bytesLabel(workspace.baseline.bytes)], ["Artifact", copyField(workspace.baseline.key, actions.copy)], ["Fingerprint", copyField(workspace.baseline.fingerprint, actions.copy)]])),
      workspace.deployment ? h("section", {}, sectionHeader("Recorded inference export"), h("p", { class: "section-note" }, "This ONNX export was recorded alongside the reference checkpoint. Its presence does not establish that it is currently deployed."), facts([["Format", "FP32 ONNX"], ["Size", bytesLabel(workspace.deployment.bytes)], ["Artifact", copyField(workspace.deployment.key, actions.copy)], ["Fingerprint", copyField(workspace.deployment.fingerprint, actions.copy)]])) : null),
    sectionHeader("Baseline evaluations"), ...evaluations,
    !evaluations.length ? h("p", { class: "section-note" }, "No evaluations from recorded runs are available for this baseline yet.") : null,
  );
}
