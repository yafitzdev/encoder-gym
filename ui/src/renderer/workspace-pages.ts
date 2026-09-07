import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, candidateRows, comparisonGroups, dateLabel, initialFilter, metricInfo, score, setupName, suiteName, timeLabel } from "./catalog.js";
import { button, copyField, details, facts, pageHeader, sectionHeader, tag } from "./components.js";
import { runListItem } from "./detail-pages.js";
import { h } from "./dom.js";

export function renderRuns(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  return h("div", { class: "page-content" }, pageHeader("Runs", "Each run records a bounded experiment and the evidence behind its decision.", tag(`${workspace.runs.length} recorded runs`)),
    h("div", { class: "run-list" }, ...workspace.runs.map(run => runListItem(run, workspace, actions))),
    h("p", { class: "table-footnote" }, "A completed run can keep the baseline. Open a run to inspect its candidates, execution history, and recorded limits."));
}
export function renderBenchmarks(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const groups = comparisonGroups(workspace, initialFilter());
  return h("div", { class: "page-content" }, pageHeader("Benchmarks", "Know what each score measures, and which candidates can be compared."),
    h("div", { class: "reading-note" }, h("h2", {}, "Compare within the same evaluation setup"), h("p", {}, "The dataset, metric requirements, and agent policy are frozen for each run. Changing any of them creates a different comparison context. Models groups candidates by those exact identities.")),
    ...groups.map((group, i) => h("section", { class: "benchmark-section" }, sectionHeader(group.label, tag(i === 0 ? "Latest recorded setup" : "Historical setup")),
      h("p", { class: "section-note" }, group.description),
      ...group.run.baselines.map(base => h("div", { class: "benchmark-row" }, h("div", {}, h("h3", {}, suiteName(base.suite)), h("p", {}, base.suite === "retired_post_scaling" ? "Previously retired post-scaling scenarios, explicitly reassigned to development in this setup." : "General retrieval behavior outside the targeted repair examples."), h("code", { class: "muted" }, base.suite)),
        h("div", {}, h("span", { class: "muted" }, "Baseline ranking score"), h("strong", { class: "score" }, score(base.metrics.mrr, "mrr"))),
      )),
      details("Evaluation identities", facts(group.run.baselines.map(base => [suiteName(base.suite), copyField(base.suiteFingerprint, actions.copy)]))),
      button(`Compare ${group.rows.length} candidates`, () => actions.navigate({ page: "models", id: group.id }), "ghost", "arrow"),
    )),
    h("section", { class: "reading-note" }, h("h2", {}, "Development and final acceptance"), h("p", {}, "Development scores help you compare and diagnose candidates. The separate sealed evaluation is used only for an explicitly selected candidate's final acceptance. Its scores never enter this comparison workspace."), h("p", {}, "The run record shows whether final acceptance was used and the recorded outcome. Use the CLI to verify current authority before starting another experiment.")),
  );
}
export function renderBaseline(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const groups = comparisonGroups(workspace, initialFilter());
  const evaluations = groups.map(g => {
    const keys = ["mrr", "recall_at_2", "agent_success_rate"];
    const head = h("thead", {}, h("tr", {}, h("th", { scope: "col" }, "Benchmark"), ...keys.map(k => h("th", { scope: "col", class: "numeric" }, metricInfo(k).label))));
    const body = h("tbody", {}, ...g.run.baselines.map(b => h("tr", {}, h("th", { scope: "row" }, suiteName(b.suite)), ...keys.map(k => h("td", { class: "numeric score" }, score(b.metrics[k], k))))));
    return h("section", { class: "baseline-evaluation" }, h("h3", {}, setupName(g.run)), h("div", { class: "table-scroll", tabindex: "0", "aria-label": "Baseline evaluations" }, h("table", { class: "evidence-table" }, head, body)));
  });
  return h("div", { class: "page-content" }, button("All models", () => actions.navigate({ page: "models" }), "back-link", "back"),
    pageHeader("Baseline encoder", "The reference model used to assess every candidate in this workspace.", tag("Baseline", "accent")),
    h("div", { class: "detail-columns" }, h("section", {}, sectionHeader("Training checkpoint"), facts([["Format", workspace.baseline.format], ["Size", bytesLabel(workspace.baseline.bytes)], ["Artifact", copyField(workspace.baseline.key, actions.copy)], ["Fingerprint", copyField(workspace.baseline.fingerprint, actions.copy)]])),
      workspace.deployment ? h("section", {}, sectionHeader("Recorded inference artifact"), h("p", { class: "section-note" }, "The FP32 ONNX export is recorded alongside the baseline training checkpoint."), facts([["Format", "FP32 ONNX"], ["Size", bytesLabel(workspace.deployment.bytes)], ["Artifact", copyField(workspace.deployment.key, actions.copy)], ["Fingerprint", copyField(workspace.deployment.fingerprint, actions.copy)]])) : null),
    sectionHeader("Baseline evaluations"), ...evaluations,
  );
}
export function renderProject(workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  return h("div", { class: "page-content" }, pageHeader("Project", "Workspace identity and the source of the evidence you are viewing."),
    h("section", { class: "project-info" }, sectionHeader(workspace.name, tag("Local workspace")),
      facts([["Task", workspace.task.replaceAll("_", " ")], ["Folder", copyField(workspace.folder, actions.copy)], ["Models", `${candidateRows(workspace).length} candidates + 1 baseline`], ["Runs", String(workspace.runs.length)], ["Recorded revision", copyField(workspace.runs[0]!.revision, actions.copy)]])),
    h("section", { class: "project-info" }, sectionHeader("Evidence source", tag(workspace.source === "local" ? "Local journals" : "Recorded snapshot")),
      h("p", { class: "section-note" }, workspace.source === "local" ? "Read from your local experiment databases. Reload to read newly persisted results." : "A portable capture of real experiment journals. Connect the workspace to read the latest local records."),
      facts([["Captured", `${dateLabel(workspace.capturedAt)} at ${timeLabel(workspace.capturedAt)}`], ["Databases", h("div", { class: "stack" }, ...workspace.databases.map(name => h("code", {}, name)))]]),
      h("div", { class: "inline-group" }, button(workspace.source === "local" ? "Reload evidence" : "Connect local workspace", actions.refresh, "primary", "refresh"), button("Choose workspace folder", actions.connect, "secondary", "project"))),
    h("section", { class: "reading-note" }, h("h2", {}, "Inspection workspace"), h("p", {}, "These screens read model and run records. They do not start training, change the baseline, or authorize final acceptance. The CLI remains the execution interface."), h("p", {}, "The data reader checks reference links and journal continuity. A full native artifact verification is performed by CLI Doctor; this page does not claim the workspace is currently verified.")),
  );
}
