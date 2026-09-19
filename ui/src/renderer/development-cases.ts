import type { SavedCaseChange, SavedCasePrediction, SavedCaseSource } from "../optimization-cases.js";
import type { DevelopmentCasesState } from "./development-cases-controller.js";
import { button, selectControl } from "./components.js";
import { h } from "./dom.js";

export interface DevelopmentCasesView { state: DevelopmentCasesState; load(retry?: boolean): void }
const labels: Record<SavedCaseChange, string> = { rank_improved: "Rank improved", rank_regressed: "Rank regressed", rank_unchanged: "Rank unchanged", not_comparable: "Not comparable" };
const views = new Map<string, { suite: string; filter: "all" | SavedCaseChange; page: number }>();

export function developmentCasesPanel(view: DevelopmentCasesView, render: () => void): HTMLElement {
  const state = view.state;
  const panel = h("section", { class: "development-cases", "aria-label": "Saved development cases" }, h("h3", {}, "Saved development cases"),
    h("p", { class: "muted" }, "Compare saved original-baseline and exact-candidate diagnostics. No inference or sealed evaluation is run."));
  if (state.status !== "ready") {
    panel.append(h("p", {}, state.status === "loading" ? "Reading saved diagnostics…" : state.status === "error" ? "Saved diagnostics could not be read or verified. No predictions were reconstructed." : "Load this iteration’s recorded diagnostic samples on demand."));
    if (state.status !== "loading") panel.append(button(state.status === "error" ? "Retry saved cases" : "Compare saved cases", () => view.load(state.status === "error"), "secondary"));
    return panel;
  }
  const value = state.value;
  if (views.size >= 100 && !views.has(value.iterationId)) views.delete(views.keys().next().value!);
  const selected = views.get(value.iterationId) ?? { suite: value.comparisons[0]!.suite, filter: "all" as const, page: 0 }; views.set(value.iterationId, selected);
  const comparison = value.comparisons.find(item => item.suite === selected.suite) ?? value.comparisons[0]!;
  const rows = comparison.cases.filter(pair => selected.filter === "all" || pair.change === selected.filter);
  selected.page = Math.min(selected.page, Math.max(0, Math.ceil(rows.length / 20) - 1));
  panel.append(h("p", { class: "diagnostic-notice" }, "Each report saves at most 50 disagreements—not all predictions. Absence from one sample is unknown, not proof of a fix or regression. Rank changes are diagnostic only; aggregate development gates still decide acceptance."),
    h("div", { class: "focus-controls" },
      selectControl(`cases-${value.iterationId}-suite`, "Development suite", value.comparisons.map(item => [item.suite, item.suite]), comparison.suite, next => { selected.suite = next; selected.page = 0; render(); }),
      selectControl(`cases-${value.iterationId}-filter`, "Cases", [["all", "All saved cases"], ...Object.entries(labels)], selected.filter, next => { selected.filter = next as typeof selected.filter; selected.page = 0; render(); }),
      button("Reload saved cases", () => view.load(true), "ghost small")),
    h("p", { class: "muted" }, `Original baseline model ${value.baselineModelId} · Candidate model ${value.candidateModelId}`),
    h("div", { class: "case-predictions" }, sourceCard("Original baseline", comparison.baseline), sourceCard("Exact candidate", comparison.candidate)),
    h("p", {}, `${comparison.cases.filter(pair => pair.change === "rank_improved").length} rank improved · ${comparison.cases.filter(pair => pair.change === "rank_regressed").length} rank regressed · ${comparison.cases.filter(pair => pair.change === "rank_unchanged").length} rank unchanged · ${comparison.cases.filter(pair => pair.change === "not_comparable").length} not comparable in the saved samples`));
  for (const pair of rows.slice(selected.page * 20, (selected.page + 1) * 20)) {
    panel.append(h("article", { class: "saved-case", "data-key": `${comparison.candidate.reportId}:${pair.sourceRowId}` },
      h("h4", { class: pair.change === "rank_improved" ? "success" : pair.change === "rank_regressed" ? "danger" : "muted" }, labels[pair.change]),
      h("p", { class: "muted" }, "Case ", h("code", {}, pair.sourceRowId)),
      h("div", { class: "case-predictions" }, predictionCard("Original baseline", pair.baseline, comparison.baseline), predictionCard("Exact candidate", pair.candidate, comparison.candidate))));
  }
  if (!rows.length) panel.append(h("p", { class: "muted" }, "No saved cases match this filter. An empty disagreement sample is not a complete prediction record."));
  if (rows.length > 20) {
    const previous = button("Previous cases", () => { selected.page--; render(); }, "ghost small"); previous.disabled = selected.page === 0;
    const next = button("Next cases", () => { selected.page++; render(); }, "ghost small"); next.disabled = (selected.page + 1) * 20 >= rows.length;
    panel.append(h("div", { class: "focus-controls" }, previous, h("span", {}, `${selected.page * 20 + 1}–${Math.min((selected.page + 1) * 20, rows.length)} of ${rows.length}`), next));
  }
  return panel;
}

function sourceCard(label: string, source: SavedCaseSource): HTMLElement {
  return h("div", {}, h("h4", {}, label), h("p", {}, source.sampleSize === null ? "Saved diagnostics missing or unverifiable." : `${source.sampleSize} saved disagreements · ${source.support.toLocaleString()} evaluated cases`),
    h("p", { class: "muted" }, "Report ", h("code", {}, source.reportId)));
}
function predictionCard(label: string, value: SavedCasePrediction | null, source: SavedCaseSource): HTMLElement {
  const card = h("div", {}, h("h5", {}, label));
  if (!value) { card.append(h("p", { class: "muted" }, source.sampleSize === null ? "Diagnostics unavailable; prediction unknown." : "Not present in this saved disagreement sample; prediction unknown.")); return card; }
  card.append(h("p", {}, value.question ?? "Question not recorded"),
    h("p", { class: "muted" }, `Task: ${value.taskKind ?? "not recorded"}`),
    h("p", {}, `Expected capabilities: ${value.expectedCapabilities?.join(", ") ?? "not recorded"}`),
    h("p", {}, `Predicted capabilities: ${value.predictedCapabilities?.join(", ") ?? "not recorded"}`),
    h("p", {}, `Expected-item rank: ${value.expectedRank ?? "not recorded"}`));
  return card;
}
