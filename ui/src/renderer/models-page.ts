import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { dateLabel, developmentStatus, runLabel, setupId, type CatalogFilter } from "./catalog.js";
import { button, empty, status, tag, workspacePage } from "./components.js";
import { h } from "./dom.js";
import { modelInventory } from "./model-inventory.js";

export interface ModelPageState { filter: CatalogFilter; selected: Set<string>; suiteIndex: number }
export function renderModels(workspace: WorkspaceSnapshot, state: ModelPageState, actions: Actions): HTMLElement {
  const inventory = modelInventory(workspace);
  const query = state.filter.query.trim().toLowerCase();
  const models = inventory.filter(m => [m.name, m.id, m.evidence ? runLabel(m.evidence.run, workspace) : ""].some(v => v.toLowerCase().includes(query)));
  const selected = inventory.flatMap(m => m.evidence && state.selected.has(m.evidence.candidate.id) ? [m.evidence] : []);
  return workspacePage("Models", null,
    inventory.length > 1 ? h("div", { class: "collection-toolbar" },
      h("input", { id: "candidate-search", type: "search", class: "text-input", placeholder: "Find a model", "aria-label": "Find a model", value: state.filter.query,
        onInput: (e: Event) => { state.filter.query = (e.target as HTMLInputElement).value; actions.render(); } }),
      selected.length ? h("div", { class: "inline-group" }, button("Clear selection", () => { state.selected.clear(); actions.render(); }, "ghost small"),
        h("button", { id: "compare-selected", type: "button", class: "button secondary", onClick: () => actions.compare(selected) }, `Compare (${selected.length})`)) : null) : null,
    h("div", { class: "artifact-list", "aria-label": "Models" }, ...models.map(model => {
      const row = model.evidence;
      const result = row ? developmentStatus(row) : undefined;
      return h("article", { class: "artifact-row", "data-model-id": model.id, ...(row ? { "data-candidate-id": row.candidate.id } : {}) },
        row && model.role === "Candidate" ? h("input", { type: "checkbox", "aria-label": "Compare " + model.name, checked: state.selected.has(row.candidate.id),
          onChange: () => {
            if (state.selected.has(row.candidate.id)) state.selected.delete(row.candidate.id);
            else if (selected.length && setupId(selected[0]!.run) !== setupId(row.run)) actions.notify("Select models evaluated against the same benchmark and baseline.");
            else if (selected.length === 3) actions.notify("Select up to three models.");
            else state.selected.add(row.candidate.id);
            actions.render();
          } }) : null,
        h("div", { class: "artifact-row-name" }, h("h2", {}, model.name),
          h("div", { class: "inline-group" }, tag(model.role, model.role === "Baseline" ? "accent" : "neutral"),
            row ? h("span", { class: "muted" }, runLabel(row.run, workspace), " · ", dateLabel(row.run.createdAt)) : null)),
        result && model.role !== "Baseline" ? status(result.key === "rejected" ? "Rejected" : result.label, result.tone) : null,
        h("button", { type: "button", class: "button ghost small candidate-link", id: "model-" + model.id,
          onClick: () => actions.navigate({ page: "model", id: model.id }) }, "Inspect model"));
    })),
    !models.length ? empty("No matching models") : null,
  );
}
