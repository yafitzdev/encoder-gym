import type { EncoderGymBridge } from "../preload.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { DatasetEntry, DatasetMutation, DatasetQueryResult, DatasetVersionSummary, InspectedDatasetRow } from "../dataset-workspace.js";
import type { Location } from "./actions.js";
import { button, copyField, details, facts, failureNotice } from "./components.js";
import { h } from "./dom.js";

interface Hooks {
  render(): void;
  navigate(location: Location): void;
  imported(workspace: ManagedWorkspace): void;
  dialog(): HTMLDialogElement;
  copy(value: string): void;
}
const pageKey = (location: Location) => `${location.id}:${location.tab ?? "rows"}:${location.offset ?? 0}`;

/** One controller per project. Async responses update that project's own state. */
export class DatasetController {
  entries?: DatasetEntry[];
  error?: unknown;
  loading = false;
  saving = false;
  private epoch = 0;
  private pages = new Map<string, DatasetQueryResult>();
  private pending = new Set<string>();
  private failed = new Set<string>();
  private retryRequest?: DatasetMutation;
  constructor(readonly projectId: string, public workspace: ManagedWorkspace, private bridge: EncoderGymBridge, private hooks: Hooks) {}
  sync(workspace: ManagedWorkspace): void {
    const moved = this.workspace.folder !== workspace.folder;
    this.workspace = workspace;
    if (moved) this.invalidate();
  }
  private invalidate(): void {
    this.epoch++; this.loading = false; this.error = undefined; this.entries = undefined;
    this.pages.clear(); this.pending.clear(); this.failed.clear();
  }
  page(location: Location): DatasetQueryResult | undefined { return this.pages.get(pageKey(location)); }
  find(versionId?: string): { entry: DatasetEntry; version: DatasetVersionSummary } | undefined {
    for (const entry of this.entries ?? []) {
      const version = entry.versions.find(v => v.version.id === versionId);
      if (version) return { entry, version };
    }
    return undefined;
  }
  async ensure(location: Location): Promise<void> {
    if (!this.entries) { if (!this.loading && !this.error) await this.load(); return; }
    if (location.page !== "dataset" || !this.find(location.id)) return;
    const tab = location.tab ?? "rows", key = pageKey(location);
    if (!["rows", "changes"].includes(tab) || this.pages.has(key) || this.pending.has(key) || this.failed.has(key)) return;
    this.pending.add(key); const epoch = this.epoch;
    try {
      const result = await this.bridge.queryDatasets(this.projectId, { kind: tab as "rows" | "changes", versionId: location.id!, offset: location.offset ?? 0, limit: 25 });
      if (epoch === this.epoch) {
        this.pages.set(key, result);
        if (this.pages.size > 8) this.pages.delete(this.pages.keys().next().value!);
      }
    } catch (error) { if (epoch === this.epoch) { this.error = error; this.failed.add(key); } }
    finally { if (epoch === this.epoch) { this.pending.delete(key); this.hooks.render(); } }
  }
  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true; this.error = undefined; const epoch = this.epoch;
    this.hooks.render();
    try { const result = await this.bridge.queryDatasets(this.projectId, { kind: "list" }); if (result.kind !== "list") throw new Error("Invalid dataset collection."); if (epoch === this.epoch) this.entries = result.entries; }
    catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.hooks.render(); } }
  }
  refresh(): void { if (this.saving) return; this.invalidate(); this.hooks.render(); }
  private async save(request: DatasetMutation): Promise<void> {
    this.retryRequest = request;
    const result = await this.bridge.mutateDataset(this.projectId, request);
    this.invalidate();
    await this.load();
    if (this.error) throw this.error;
    this.retryRequest = undefined;
    this.hooks.navigate({ page: "dataset", id: result.version.id, tab: request.kind === "revise" ? "changes" : "rows" });
  }
  async retry(): Promise<void> {
    if (!this.retryRequest) { this.refresh(); return; }
    if (this.saving) return;
    this.saving = true; this.error = undefined; this.hooks.render();
    try { await this.save(this.retryRequest); } catch (error) { this.error = error; }
    finally { this.saving = false; this.hooks.render(); }
  }
  private form(title: string, fields: HTMLElement[], label: string, submit: () => Promise<void>): void {
    if (this.saving) return;
    const dialog = this.hooks.dialog(), opener = document.activeElement as HTMLElement | null;
    const error = h("div", { class: "form-error", role: "alert" });
    const body = h("div", { class: "dataset-dialog-fields" }, ...fields);
    const cancel = button("Cancel", () => dialog.close(), "ghost");
    const save = h("button", { id: "dataset-confirm", type: "submit", class: "button primary" }, label) as HTMLButtonElement;
    const form = h("form", { class: "dataset-dialog", onSubmit: async (event: Event) => {
      event.preventDefault(); if (this.saving) return;
      this.saving = true; body.inert = true; save.disabled = true; cancel.disabled = true; save.textContent = "Saving…"; error.replaceChildren(); this.hooks.render();
      try { await submit(); dialog.close(); }
      catch (failure) { this.error = failure; error.replaceChildren(failureNotice(failure)); }
      finally { this.saving = false; body.inert = false; save.disabled = false; cancel.disabled = false; save.textContent = label; this.hooks.render(); }
    } }, h("h2", { id: "project-dialog-title" }, title), body, error, h("div", { class: "dialog-actions" }, cancel, save));
    const onCancel = (event: Event) => { if (this.saving) event.preventDefault(); };
    dialog.addEventListener("cancel", onCancel);
    dialog.addEventListener("close", () => { dialog.removeEventListener("cancel", onCancel); if (opener?.isConnected) opener.focus(); }, { once: true });
    dialog.replaceChildren(form); dialog.showModal(); (body.querySelector("input,select,button") as HTMLElement | null)?.focus();
  }
  create(): void {
    const imports = this.workspace.datasets.filter(source => source.purpose === "training");
    const input = h("input", { id: "dataset-name", class: "text-input", value: "Base dataset", required: true, maxlength: 120 }) as HTMLInputElement;
    const selected = new Set(imports.map(source => source.id));
    const sources = h("fieldset", {}, h("legend", {}, "Training data"), ...imports.map(source => h("label", { class: "dataset-source-choice" }, h("input", { type: "checkbox", checked: true, "data-import-id": source.id, onChange: (e: Event) => { if ((e.target as HTMLInputElement).checked) selected.add(source.id); else selected.delete(source.id); } }), h("span", {}, source.name, h("small", {}, `${source.rows.toLocaleString()} rows`)))));
    const identity = { datasetId: crypto.randomUUID(), versionId: crypto.randomUUID() };
    this.form("Create base dataset", [h("label", { class: "form-field", for: "dataset-name" }, "Name", input), imports.length ? sources : h("p", {}, "Import training data first.")], "Create dataset", async () => {
      if (!selected.size) throw new Error("Select at least one training source.");
      if (!input.value.trim()) throw new Error("Enter a dataset name.");
      await this.save({ kind: "create", ...identity, name: input.value, imports: [...selected] });
    });
  }
  fork(versionId: string): void {
    const found = this.find(versionId); if (!found) return;
    const input = h("input", { id: "dataset-name", class: "text-input", value: `Variant ${(this.entries?.length ?? 1)}`, required: true, maxlength: 120 }) as HTMLInputElement;
    const identity = { datasetId: crypto.randomUUID(), versionId: crypto.randomUUID() };
    this.form("New dataset variant", [h("label", { class: "form-field", for: "dataset-name" }, "Name", input), facts([["From", `${found.entry.dataset.name} · Version ${found.version.version.number}`]])], "Create variant", async () => {
      if (!input.value.trim()) throw new Error("Enter a dataset name.");
      await this.save({ kind: "fork", ...identity, name: input.value, parentId: versionId });
    });
  }
  inspect(row: InspectedDatasetRow): void {
    const dialog = this.hooks.dialog();
    const opener = document.activeElement as HTMLElement | null;
    dialog.replaceChildren(h("div", { class: "dataset-row-dialog" }, h("div", { class: "dialog-heading" }, h("h2", { id: "project-dialog-title" }, "Dataset row"), button("Close", () => dialog.close(), "ghost small", "close")),
      h("pre", { class: "native-row" }, JSON.stringify(row.value, null, 2)), details("Row identity", facts([["Row ID", copyField(row.member.id, this.hooks.copy)], ["Source record", String(row.member.source.record)], ["Content", copyField(row.member.contentFingerprint, this.hooks.copy)]]))));
    dialog.addEventListener("close", () => { if (opener?.isConnected) opener.focus(); }, { once: true }); dialog.showModal();
  }
  remove(versionId: string, row: InspectedDatasetRow): void {
    const found = this.find(versionId); if (!found) return;
    const request: DatasetMutation = { kind: "revise", datasetId: found.entry.dataset.id, versionId: crypto.randomUUID(), parentId: versionId, added: [], removed: [row.member.id], replaced: [] };
    this.form("Remove row", [h("p", {}, "Save a new version without this row? Earlier versions stay intact.")], "Save version", () => this.save(request));
  }
  async importRows(versionId?: string, replacement?: InspectedDatasetRow): Promise<void> {
    if (this.saving) return;
    this.saving = true; this.error = undefined; this.hooks.render();
    try {
      if (!this.entries) await this.load();
      if (this.error) throw this.error;
      const found = this.find(versionId);
      if (versionId && !found) throw new Error("Dataset version not found. Refresh the dataset before editing.");
      if (!versionId && this.entries?.length) throw new Error("Open a dataset version to add rows.");
      const choice = await this.bridge.chooseDataset(this.projectId, "training");
      if (!choice) return;
      if (replacement && choice.rows !== 1) throw new Error("Choose a JSONL file containing exactly one replacement row.");
      if (choice.rows > 100_000) throw new Error("A single edit supports at most 100,000 rows.");
      const name = choice.source.split(/[\\/]/).at(-1) ?? "Imported rows";
      const opened = await this.bridge.importDataset(this.projectId, choice.token, name);
      if (opened.content.state !== "ready" || !opened.content.workspace.managed) throw new Error("The import did not return a managed project.");
      this.workspace = opened.content.workspace.managed; this.hooks.imported(this.workspace);
      const source = this.workspace.datasets.find(source => source.artifact.fingerprint === choice.artifact.fingerprint);
      if (!source) throw new Error("Imported source is missing.");
      if (found) await this.save({ kind: "revise", datasetId: found.entry.dataset.id, versionId: crypto.randomUUID(), parentId: versionId!, added: replacement ? [] : Array.from({ length: source.rows }, (_, i) => ({ importId: source.id, record: i + 1 })), removed: [], replaced: replacement ? [{ id: replacement.member.id, source: { importId: source.id, record: 1 } }] : [] });
      else if (!this.entries?.length) await this.save({ kind: "create", datasetId: crypto.randomUUID(), versionId: crypto.randomUUID(), name: "Base dataset", imports: [source.id] });
    } catch (error) { this.error = error; }
    finally { this.saving = false; this.hooks.render(); }
  }
}
