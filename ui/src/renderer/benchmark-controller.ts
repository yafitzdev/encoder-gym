import type { EncoderGymBridge } from "../preload.js";
import type { BenchmarkAdoption, BenchmarkPreview, ProjectBenchmarkResults, ProjectBenchmarkVersion } from "../benchmark-workspace.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { Location } from "./actions.js";
import { button, details, failureNotice, facts, selectControl } from "./components.js";
import { humanize, metricInfo, score, suiteName } from "./catalog.js";
import { h } from "./dom.js";

interface Hooks { render(): void; navigate(location: Location): void; dialog(): HTMLDialogElement }
export class BenchmarkController {
  versions?: ProjectBenchmarkVersion[];
  error?: unknown;
  loading = false;
  saving = false;
  private epoch = 0;
  private results = new Map<string, ProjectBenchmarkResults>();
  private pending = new Set<string>();
  private failures = new Map<string, unknown>();
  private retryRequest?: BenchmarkAdoption;
  constructor(readonly projectId: string, public workspace: ManagedWorkspace, private bridge: EncoderGymBridge, private hooks: Hooks) {}
  sync(workspace: ManagedWorkspace): void { if (workspace !== this.workspace) { this.workspace = workspace; this.invalidate(); } }
  selected(location: Location): ProjectBenchmarkVersion | undefined { return location.id ? this.versions?.find(version => version.id === location.id) : this.versions?.at(-1); }
  result(id: string): ProjectBenchmarkResults | undefined { return this.results.get(id); }
  failure(id?: string): unknown { return this.error ?? (id ? this.failures.get(id) : undefined); }
  private invalidate(): void { this.epoch++; this.loading = false; this.error = undefined; this.versions = undefined; this.results.clear(); this.pending.clear(); this.failures.clear(); }
  refresh(): void { if (!this.saving) { this.retryRequest = undefined; this.invalidate(); this.hooks.render(); } }
  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true; this.error = undefined; const epoch = this.epoch; this.hooks.render();
    try { const data = await this.bridge.queryBenchmarks(this.projectId, { kind: "list" }); if (data.kind !== "list") throw new Error("Invalid benchmark collection."); if (epoch === this.epoch) this.versions = data.versions; }
    catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.hooks.render(); } }
  }
  async ensure(location: Location): Promise<void> {
    if (!this.versions) { if (!this.loading && !this.error) await this.load(); return; }
    const version = this.selected(location);
    if (!version || ![undefined, "results"].includes(location.tab) || this.results.has(version.id) || this.pending.has(version.id) || this.failures.has(version.id)) return;
    this.pending.add(version.id); const epoch = this.epoch;
    try {
      const data = await this.bridge.queryBenchmarks(this.projectId, { kind: "results", versionId: version.id });
      if (data.kind !== "results" || data.results.version.id !== version.id) throw new Error("Invalid benchmark result.");
      if (epoch === this.epoch) { this.results.set(version.id, data.results); if (this.results.size > 5) this.results.delete(this.results.keys().next().value!); }
    } catch (error) { if (epoch === this.epoch) this.failures.set(version.id, error); }
    finally { if (epoch === this.epoch) { this.pending.delete(version.id); this.hooks.render(); } }
  }
  async adopt(request: BenchmarkAdoption): Promise<void> {
    if (this.saving) return;
    this.saving = true; this.error = undefined; this.retryRequest = request; this.hooks.render();
    try {
      const saved = await this.bridge.adoptBenchmark(this.projectId, request);
      this.invalidate(); await this.load(); if (this.error) throw this.error;
      this.retryRequest = undefined;
      this.hooks.navigate({ page: "benchmarks", id: saved.version.id, tab: "results" });
    } catch (error) { this.error = error; }
    finally { this.saving = false; this.hooks.render(); }
  }
  async retry(): Promise<void> { if (this.retryRequest) await this.adopt(this.retryRequest); else this.refresh(); }
  choose(runs: [string, string][]): void {
    if (this.saving || !runs.length) return;
    this.retryRequest = undefined;
    const dialog = this.hooks.dialog(), opener = document.activeElement as HTMLElement | null;
    let selection = runs[0]![0], preview: BenchmarkPreview | undefined, generation = 0;
    const body = h("div", { class: "benchmark-preview", role: "status" });
    const save = button("Use benchmark", () => { if (!preview) return; const request = { runId: selection, expectedParent: preview.expectedParent, definitionFingerprint: preview.definition.fingerprint }; const existing = preview.existingVersion; dialog.close(); if (existing) this.hooks.navigate({ page: "benchmarks", id: existing, tab: "results" }); else void this.adopt(request); }, "primary");
    save.id = "benchmark-confirm"; save.disabled = true;
    const read = async () => {
      const request = ++generation; preview = undefined; save.disabled = true; body.textContent = "Loading benchmark…";
      try {
        const result = await this.bridge.previewBenchmark(this.projectId, selection);
        if (request !== generation || !dialog.open) return;
        preview = result; save.disabled = false; save.textContent = result.existingVersion ? "Open version" : "Use benchmark";
        body.replaceChildren(facts([["Tests", result.definition.suites.map(suite => suiteName(suite.key)).join(" · ")], ["Primary metric", metricInfo(result.definition.metric_contract.primary_metric).label]]),
          details("Scoring rules", facts(result.definition.metric_contract.gates.map(gate => [`${metricInfo(gate.key).label} · ${gate.suite_key ? suiteName(gate.suite_key) : gate.role === "development" ? "Development" : "Final holdout"}`, `${humanize(gate.condition.kind)}: ${score(gate.condition.value, gate.key)}`]))));
      } catch (error) { if (request === generation && dialog.open) body.replaceChildren(failureNotice(error), button("Retry", () => { void read(); }, "secondary")); }
    };
    dialog.replaceChildren(h("div", { class: "dataset-dialog" }, h("h2", { id: "project-dialog-title" }, "Choose recorded benchmark"),
      selectControl("benchmark-source-run", "From run", runs, selection, value => { selection = value; void read(); }), body,
      h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "ghost"), save)));
    dialog.addEventListener("close", () => { generation++; if (opener?.isConnected) opener.focus(); }, { once: true });
    dialog.showModal(); void read();
  }
}
