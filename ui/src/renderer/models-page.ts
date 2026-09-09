import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { candidateDescription, candidateName, candidateRows, comparisonGroups, dateLabel, developmentStatus, initialFilter, modelName, runLabel, runName, score, setupId, suiteName, summaryMetrics, type CatalogFilter } from "./catalog.js";
import { button, empty, icon, metricHeader, pageHeader, scoreStack, sectionHeader, selectControl, status, tag } from "./components.js";
import { h } from "./dom.js";

export interface ModelPageState { filter: CatalogFilter; selected: Set<string>; suiteIndex: number }
export function renderModels(workspace: WorkspaceSnapshot, state: ModelPageState, actions: Actions): HTMLElement {
  const allRows = candidateRows(workspace);
  const newest = workspace.managed ? allRows[0]?.run : workspace.runs[0];
  const importedHistoryOnly = Boolean(workspace.managed && workspace.runs.length && !allRows.length);
  const setups = comparisonGroups(workspace, initialFilter());
  const groups = comparisonGroups(workspace, state.filter, state.suiteIndex);
  const shown = groups.reduce((n, g) => n + g.rows.length, 0);
  const selectedRows = allRows.filter(r => state.selected.has(r.candidate.id));
  const compare = button(`Compare${selectedRows.length ? ` (${selectedRows.length})` : " selected"}`, () => actions.compare(selectedRows), "primary", "models");
  compare.disabled = !selectedRows.length;
  compare.id = "compare-selected";
  if (!selectedRows.length) compare.setAttribute("aria-describedby", "compare-selection-hint");
  const filterChange = (key: keyof CatalogFilter, value: string) => { state.filter[key] = value; actions.render(); };
  const selectedSetup = selectedRows[0] ? setupId(selectedRows[0].run) : undefined;
  return h("div", { class: "page-content" },
    pageHeader("Models", allRows.length ? "Which candidates improve on your reference model?" : "Your starting model and the alternatives you develop from it.", workspace.managed && allRows.length ? button("Start optimization", actions.prepareOptimization, "primary", "runs") : !workspace.managed && allRows.length ? button("How to read this", () => actions.help(), "ghost", "help") : null),
    h("section", { class: "baseline-anchor", "aria-label": "Baseline encoder" },
      h("div", { class: "baseline-heading" }, h("div", { class: "model-symbol", "aria-hidden": "true" }, icon("models")),
        h("div", { class: "baseline-name" }, h("div", { class: "eyebrow" }, "Project baseline"), h("h2", {}, modelName(workspace.baseline.key)), h("p", {}, workspace.baseline.format + " · comparison reference"))),
      h("div", { class: "baseline-action" }, button("Inspect baseline", () => actions.navigate({ page: "baseline" }), "ghost small", "arrow")),
    ),
    newest ? h("div", { class: "project-update" }, icon("runs"), h("span", {}, newest.decision === "retain_baseline" ? "Latest managed candidate run kept the baseline." : "Latest managed candidate run is available.", " ", h("button", { type: "button", class: "inline-link", onClick: () => actions.navigate({ page: "run", id: newest.id }) }, `${runName(newest)} →`)), h("time", {}, dateLabel(newest.updatedAt))) :
      importedHistoryOnly ? h("div", { class: "project-update" }, icon("runs"), h("span", {}, `${workspace.runs.length} imported historical ${workspace.runs.length === 1 ? "run is" : "runs are"} available under Runs and Evaluation. Their checkpoints are not registered project models. `, h("button", { type: "button", class: "inline-link", onClick: () => actions.navigate({ page: "runs" }) }, "Open runs →")), h("time", {}, dateLabel(workspace.runs[0]!.updatedAt))) : null,
    h("section", { class: "candidate-section", "aria-label": "Candidate comparisons" },
      sectionHeader("Candidates", allRows.length ? h("div", { class: "section-tools" }, tag(String(allRows.length)), h("span", { class: "muted" }, "Development results")) : null),
      allRows.length ? h("div", { class: "comparison-toolbar" },
        h("label", { class: "search-control", for: "candidate-search" }, icon("search"), h("span", { class: "sr-only" }, "Find a candidate or run"), h("input", { id: "candidate-search", type: "search", placeholder: "Find a candidate or run…", value: state.filter.query, onInput: (e: Event) => filterChange("query", (e.target as HTMLInputElement).value) })),
        setups.length > 1 ? selectControl("setup-filter", "Evaluation setup", [["all", "All setups"], ...setups.map(g => [g.id, g.label] as [string, string])], state.filter.setup, v => filterChange("setup", v)) : null,
        selectControl("status-filter", "Result", [["all", "All results"], ["rejected", "Below requirements"], ["passed", "Passed development"], ["incomplete", "Incomplete evidence"], ["failed", "Execution failed"]], state.filter.status, v => filterChange("status", v)),
      ) : null,
      allRows.length ? h("div", { class: "comparison-subtoolbar" }, h("div", { class: "inline-group" }, h("span", { class: "muted" }, `${shown} of ${allRows.length} candidates`), selectControl("sort-order", "Order", [["newest", "Newest first"], ["primary", "Best primary score"]], state.filter.sort, v => filterChange("sort", v))),
        h("div", { class: "inline-group" }, state.selected.size ? button("Clear selection", () => { state.selected.clear(); actions.render(); }, "ghost small") : h("span", { id: "compare-selection-hint", class: "selection-hint" }, "Select 1–3 candidates to compare"), compare)) : null,
      ...groups.map((group, gi) => {
        const suite = group.run.baselines[Math.min(state.suiteIndex, group.run.baselines.length - 1)]!;
        const keys = summaryMetrics(group.run, suite);
        return h("section", { class: "comparison-group", "aria-label": group.label },
          h("div", { class: "group-heading" }, h("div", {}, h("h3", {}, group.label), h("span", {}, `${group.rows.length} ${group.rows.length === 1 ? "candidate" : "candidates"} · same benchmarks and scoring rules`)),
            group.run.baselines.length > 1 ? selectControl(`suite-${gi}`, "Benchmark", group.run.baselines.map((b, i) => [String(i), suiteName(b.suite)]), String(Math.min(state.suiteIndex, group.run.baselines.length - 1)), v => { state.suiteIndex = Number(v); actions.render(); }) : tag(suiteName(suite.suite))),
          h("div", { class: "table-scroll", tabindex: "0", "aria-label": `${group.label} comparison table` }, h("table", { class: "candidate-table" },
            h("thead", {}, h("tr", {}, h("th", { class: "select-cell", scope: "col" }, h("span", { class: "sr-only" }, "Select")), h("th", { scope: "col" }, "Encoder"),
              ...keys.map(k => h("th", { scope: "col", class: "numeric" }, metricHeader(k, key => actions.help(key), group.run.directions[k]))), h("th", { scope: "col" }, "Development result"))),
            h("tbody", {},
              h("tr", { class: "baseline-table-row" }, h("td", { class: "select-cell" }, icon("lock")), h("th", { scope: "row" }, h("div", { class: "encoder-cell" }, h("strong", {}, group.run.baseline.fingerprint === workspace.baseline.fingerprint ? "Baseline encoder" : "Historical baseline"), h("small", {}, "Comparison reference"))),
                ...keys.map(k => h("td", { class: "numeric" }, h("span", { class: "score" }, score(suite.metrics[k], k)))), h("td", {}, tag("Baseline", "accent"))),
              ...group.rows.map(row => {
                const c = row.candidate;
                const result = c.development.find(d => d.report.suite === suite.suite);
                const s = developmentStatus(row);
                const selected = state.selected.has(c.id);
                return h("tr", { class: selected ? "selected-row" : "", "data-candidate-id": c.id },
                  h("td", { class: "select-cell" }, h("input", { type: "checkbox", id: `select-${c.id}`, "aria-label": `Select ${candidateName(c)}, ${runLabel(row.run, workspace)}`, checked: selected,
                    onChange: () => {
                      if (state.selected.has(c.id)) state.selected.delete(c.id);
                      else if (selectedSetup && selectedSetup !== group.id) actions.notify("Choose candidates from the same evaluation setup. Clear your selection to switch setups.");
                      else if (state.selected.size === 3) actions.notify("Compare up to 3 candidates at a time. Remove one to choose another.");
                      else state.selected.add(c.id);
                      actions.render();
                    } })),
                  h("th", { scope: "row" }, h("div", { class: "encoder-cell" }, h("button", { id: "candidate-" + c.id, class: "candidate-link", type: "button", onClick: () => actions.navigate({ page: "candidate", id: c.id }) }, candidateName(c), icon("arrow")),
                    h("small", {}, candidateDescription(c)), h("button", { class: "run-link", type: "button", onClick: () => actions.navigate({ page: "run", id: row.run.id }) }, `${runLabel(row.run, workspace)} · ${dateLabel(row.run.createdAt)}`))),
                  ...keys.map(k => h("td", { class: "numeric" }, scoreStack(result?.report.metrics[k], result?.baseline.metrics[k], k, row.run.directions[k]))),
                  h("td", {}, h("div", { class: "result-cell" }, status(s.label, s.tone), h("small", {}, s.detail))),
                );
              }),
            ),
          )),
        );
      }),
      !allRows.length ? h("div", { class: "candidate-empty" }, h("h3", {}, workspace.managed ? "No managed candidates yet" : "No candidates recorded yet"),
        h("p", {}, importedHistoryOnly ? "Imported scientific history remains available under Runs and Evaluation. A checkpoint appears here only after it is registered as an immutable artifact of this managed project." : "A candidate is a trained or transformed alternative to your baseline. Its results belong here once an experiment has produced evidence."),
        workspace.managed ? h("div", { class: "next-step" }, h("div", {}, h("h3", {}, "Prepare the first bounded run"),
          h("p", {}, "Check the active baseline, scientific inputs, evaluations, providers, and finite budgets before reserving any work.")),
          button("Start optimization", actions.prepareOptimization, "primary", "arrow")) : button("Project settings", () => actions.navigate({ page: "project" })),
      ) : !groups.length ? empty("No matching candidates", "Try a different name, result, or evaluation setup.", button("Reset filters", () => { state.filter = initialFilter(); actions.render(); })) : null,
      allRows.length ? h("p", { class: "table-footnote" }, "Changes are relative to the matching baseline. pp = percentage points. A score improvement can still miss a required threshold. ", h("button", { class: "inline-link", type: "button", onClick: () => actions.navigate({ page: "benchmarks" }) }, "Understand the benchmarks →")) : null,
    ),
  );
}
