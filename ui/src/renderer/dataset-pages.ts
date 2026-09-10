import type { WorkspaceSnapshot } from "../workspace.js";
import type { DatasetVersionSummary, InspectedDatasetRow } from "../dataset-workspace.js";
import type { Actions, Location } from "./actions.js";
import type { DatasetController } from "./dataset-controller.js";
import { button, copyField, details, empty, facts, failureNotice, pageHeader, selectControl, tabs, tag, workspacePage } from "./components.js";
import { dateLabel } from "./catalog.js";
import { modelInventory } from "./model-inventory.js";
import { h } from "./dom.js";

function preview(row: InspectedDatasetRow): string {
  for (const key of ["text", "question", "query", "input", "instruction", "user_message", "objective", "decision_state_id"]) if (typeof row.value[key] === "string") return String(row.value[key]).slice(0, 180);
  return JSON.stringify(row.value).slice(0, 180);
}
function delta(version: DatasetVersionSummary): string {
  return [version.added ? `+${version.added.toLocaleString()}` : "", version.removed ? `−${version.removed.toLocaleString()}` : "", version.replaced ? `${version.replaced.toLocaleString()} changed` : ""].filter(Boolean).join(" · ") || "No row changes";
}
function pager(location: Location, total: number, count: number, actions: Actions): HTMLElement {
  const offset = location.offset ?? 0;
  const previous = button("Previous", () => actions.navigate({ ...location, offset: Math.max(0, offset - 25) }), "ghost small", "back"); previous.disabled = offset === 0;
  const next = button("Next", () => actions.navigate({ ...location, offset: offset + 25 }), "ghost small", "arrow"); next.disabled = offset + count >= total;
  return h("div", { class: "dataset-pagination" }, previous, h("span", { class: "muted" }, total ? `${offset + 1}–${offset + count} of ${total.toLocaleString()}` : "0 rows"), next);
}
export function renderDatasetPage(workspace: WorkspaceSnapshot, location: Location, controller: DatasetController, actions: Actions): HTMLElement {
  const mutationButton = (...args: Parameters<typeof button>) => {
    const control = button(...args); control.disabled = controller.saving || controller.loading; return control;
  };
  const failure = controller.error ? h("div", { class: "operation-failure", role: "alert" }, failureNotice(controller.error), mutationButton("Retry", () => { void controller.retry(); }, "secondary")) : null;
  const progress = controller.saving ? h("div", { role: "status", class: "dataset-progress" }, "Saving dataset…") : null;
  const create = () => controller.workspace.datasets.some(d => d.purpose === "training") ? controller.create() : void controller.importRows();
  if (location.page === "datasets") {
    const entries = controller.entries;
    return workspacePage("Data", entries?.length ? mutationButton("New variant", () => controller.fork(entries[0]!.versions[0]!.version.id), "primary", "dataset") : mutationButton("Create dataset", create, "primary", "dataset"),
      failure, progress, !entries ? h("div", { role: "status" }, controller.error ? "" : "Loading datasets…") : !entries.length ? empty("No datasets", mutationButton("Import dataset", () => { void controller.importRows(); }, "secondary", "project")) :
        h("div", { class: "artifact-list dataset-collection" }, ...entries.map(entry => {
          const version = entry.versions[0]!;
          const inspect = button("Inspect dataset", () => actions.navigate({ page: "dataset", id: version.version.id }), "ghost small"); inspect.id = `dataset-${entry.dataset.id}`;
          return h("section", { class: "artifact-row", "data-dataset-id": entry.dataset.id }, h("div", { class: "artifact-row-name" }, h("h2", {}, entry.dataset.name), h("div", { class: "inline-group muted" }, tag(entry.dataset.origin ? "Variant" : "Base", entry.dataset.origin ? "neutral" : "accent"), `${version.rows.toLocaleString()} rows · Version ${version.version.number}`)), inspect);
        })));
  }
  if (!controller.entries) return workspacePage("Dataset", null, failure, h("div", { role: "status" }, controller.error ? "" : "Loading dataset…"));
  const found = controller.find(location.id);
  if (!found) return workspacePage("Dataset", null, failure, empty("Dataset version not found", button("All datasets", () => actions.backTo("datasets"))));
  const { entry, version } = found;
  const tab = ["rows", "changes", "versions", "models"].includes(location.tab ?? "") ? location.tab! : "rows";
  const page = controller.page(location), current = entry.versions[0]!.version.id === version.version.id;
  const rowActions = (row: InspectedDatasetRow) => h("div", { class: "inline-group" }, button("Inspect", () => controller.inspect(row), "ghost small"), current ? mutationButton("Replace", () => { void controller.importRows(version.version.id, row); }, "ghost small") : null, current ? mutationButton("Remove", () => controller.remove(version.version.id, row), "ghost small") : null);
  let body: HTMLElement;
  if (tab === "rows") body = page?.kind === "rows" ? h("div", {},
    h("div", { class: "collection-toolbar" }, h("span", { class: "muted" }, `${version.rows.toLocaleString()} rows`), current ? mutationButton("Add rows", () => { void controller.importRows(version.version.id); }, "secondary", "project") : null),
    h("div", { class: "dataset-rows" }, ...page.page.rows.map((row, index) => h("div", { class: "dataset-row-entry", "data-row-id": row.member.id }, h("small", { class: "muted" }, `Row ${(location.offset ?? 0) + index + 1}`), h("span", { class: "row-preview" }, preview(row)), rowActions(row)))), pager(location, page.page.total, page.page.rows.length, actions)) : h("div", { role: "status" }, controller.error ? "" : "Loading rows…");
  else if (tab === "changes") body = page?.kind === "changes" ? page.page.total ? h("div", {}, h("div", { class: "collection-toolbar muted" }, delta(version)),
    h("div", { class: "dataset-changes" }, ...page.page.changes.map(change => h("section", { class: "dataset-change-row", "data-change-kind": change.kind },
      h("div", { class: "inline-group" }, tag(change.kind === "added" ? "Added" : change.kind === "removed" ? "Removed" : "Changed", change.kind === "added" ? "accent" : change.kind === "removed" ? "warning" : "neutral")),
      h("div", { class: "dataset-diff-values" }, change.before ? h("div", {}, h("small", { class: "muted" }, "Before"), h("p", {}, preview(change.before)), button("Inspect before", () => controller.inspect(change.before!), "ghost small")) : null,
        change.after ? h("div", {}, h("small", { class: "muted" }, "After"), h("p", {}, preview(change.after)), button("Inspect after", () => controller.inspect(change.after!), "ghost small")) : null)))), pager(location, page.page.total, page.page.changes.length, actions)) : empty("No row changes") : h("div", { role: "status" }, controller.error ? "" : "Loading changes…");
  else if (tab === "versions") body = h("div", { class: "dataset-versions" }, ...entry.versions.map(v => h("section", { class: "artifact-row" }, h("div", { class: "artifact-row-name" }, h("h2", {}, `Version ${v.version.number}`), h("p", { class: "muted" }, `${v.rows.toLocaleString()} rows · ${delta(v)} · ${dateLabel(v.createdAt)}`)), button("Inspect version", () => actions.navigate({ page: "dataset", id: v.version.id, tab: "rows" }), "ghost small"))));
  else {
    const links = workspace.managed?.modelDatasetLinks ?? [];
    const models = modelInventory(workspace).filter(model => links.some(link => link.projectId === version.version.projectId && link.version.id === version.version.id && link.version.fingerprint === version.version.fingerprint && link.modelId === model.id && link.modelFingerprint === model.catalogArtifact?.fingerprint));
    body = models.length ? h("div", { class: "artifact-list" }, ...models.map(model => h("div", { class: "artifact-row" }, h("div", { class: "artifact-row-name" }, h("h2", {}, model.name), tag(model.role)), button("Inspect model", () => actions.navigate({ page: "model", id: model.id }), "ghost small")))) : empty("No linked models");
  }
  const origin = entry.dataset.origin ? controller.find(entry.dataset.origin.id) : undefined;
  return h("div", { class: "page-content detail-page dataset-view" }, button("All datasets", () => actions.backTo("datasets"), "back-link", "back"),
    pageHeader(entry.dataset.name, mutationButton("Create variant", () => controller.fork(version.version.id), "secondary", "dataset")),
    h("div", { class: "dataset-version-toolbar" }, selectControl("dataset-version", "Version", entry.versions.map(v => [v.version.id, `Version ${v.version.number}`]), version.version.id, id => actions.navigate({ page: "dataset", id, tab })), origin ? button(`From ${origin.entry.dataset.name} · Version ${origin.version.version.number}`, () => actions.navigate({ page: "dataset", id: origin.version.version.id }), "ghost small") : null),
    tabs([["rows", "Rows"], ["changes", "Changes"], ["versions", "Versions"], ["models", "Models"]], tab, tab => actions.navigate({ page: "dataset", id: version.version.id, tab })), failure, progress,
    h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${tab}`, inert: controller.saving }, body),
    details("Version details", facts([["Version ID", copyField(version.version.id, actions.copy)], ["Fingerprint", copyField(version.version.fingerprint, actions.copy)], ["Created", dateLabel(version.createdAt)]])));
}
