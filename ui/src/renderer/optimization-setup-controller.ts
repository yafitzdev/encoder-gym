import type { EncoderGymBridge } from "../preload.js";
import type { DatasetEntry } from "../dataset-workspace.js";
import type { ProjectBenchmarkVersion } from "../benchmark-workspace.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { OptimizationInputs, OptimizationSetup, OptimizationSetupRequest } from "../optimization-setup.js";

/** Per-project input selection. Running experiments have a separate controller. */
export class OptimizationSetupController {
  data?: { datasets: DatasetEntry[]; benchmarks: ProjectBenchmarkVersion[]; history: OptimizationSetup[] };
  datasetId = "";
  benchmarkId = "";
  loading = false;
  saving = false;
  phase?: string;
  startedAt?: number;
  error?: unknown;
  private epoch = 0;
  private retry?: OptimizationSetupRequest;
  constructor(readonly projectId: string, public workspace: ManagedWorkspace, private bridge: EncoderGymBridge, private render: () => void) {}
  get baseline() { const catalog = this.workspace.modelCatalog; return catalog?.baselineRevisions.find(revision => revision.id === catalog.activeBaselineRevisionId); }
  get model() { return this.workspace.modelCatalog?.artifacts.find(model => model.id === this.baseline?.modelArtifactId); }
  get dataset() { return this.data?.datasets.flatMap(entry => entry.versions.map(version => ({ entry, version }))).find(item => item.version.version.id === this.datasetId); }
  get benchmark() { return this.data?.benchmarks.find(version => version.id === this.benchmarkId); }
  get latest() { return this.data?.history.at(-1); }
  get selected(): OptimizationInputs | undefined {
    const model = this.model, baseline = this.baseline, dataset = this.dataset, benchmark = this.benchmark;
    if (!model || !baseline || !dataset || !benchmark || !dataset.version.rows) return undefined;
    return { projectId: this.workspace.manifest.id, baselineRevision: { id: baseline.id, fingerprint: baseline.fingerprint },
      model: { id: model.id, fingerprint: model.fingerprint }, dataset: dataset.version.version, benchmark: { id: benchmark.id, fingerprint: benchmark.fingerprint } };
  }
  get saved(): boolean { return !!this.selected && sameInputs(this.selected, this.latest?.inputs); }
  get canSave(): boolean { return !!this.selected && !this.saved && !this.saving && !this.loading; }
  sync(workspace: ManagedWorkspace): void {
    if (workspace !== this.workspace) { this.workspace = workspace; this.invalidate(); }
  }
  private invalidate(): void { this.epoch++; this.loading = false; this.error = undefined; this.data = undefined; this.retry = undefined; }
  refresh(): void { if (!this.saving) { this.invalidate(); this.render(); } }
  select(kind: "dataset" | "benchmark", id: string): void {
    if (this.saving || this.loading) return;
    if (kind === "dataset") this.datasetId = id; else this.benchmarkId = id;
    this.retry = undefined; this.error = undefined; this.render();
  }
  async ensure(): Promise<void> { if (!this.data && !this.error && !this.loading && !this.saving) await this.load(); }
  private async load(): Promise<void> {
    this.loading = true; const epoch = this.epoch; this.render();
    try {
      const [datasets, benchmarks, history] = await Promise.all([
        this.bridge.queryDatasets(this.projectId, { kind: "list" }), this.bridge.queryBenchmarks(this.projectId, { kind: "list" }), this.bridge.optimizationSetups(this.projectId),
      ]);
      if (datasets.kind !== "list" || benchmarks.kind !== "list") throw new Error("Could not read optimization inputs.");
      if (epoch !== this.epoch) return;
      this.data = { datasets: datasets.entries, benchmarks: benchmarks.versions, history };
      const latest = history.at(-1);
      // Saved versions never float to newly-created versions. For first setup,
      // prefer recorded model provenance, not an arbitrary training population.
      const linked = this.workspace.modelDatasetLinks?.find(link => link.modelId === this.model?.id)?.version.id;
      this.datasetId = latest?.inputs.dataset.id ?? linked ?? (datasets.entries.length === 1 ? datasets.entries[0]?.versions.at(-1)?.version.id : undefined) ?? "";
      this.benchmarkId = latest?.inputs.benchmark.id ?? benchmarks.versions.at(-1)?.id ?? "";
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.render(); } }
  }
  async save(): Promise<void> {
    if (!this.canSave) return;
    const selected = this.selected!, epoch = this.epoch;
    this.saving = true; this.error = undefined; this.phase = this.retry ? "Verifying files and saving…" : "Checking inputs…"; this.startedAt = Date.now(); this.render();
    try {
      if (!this.retry) {
        const preview = await this.bridge.previewOptimizationSetup(this.projectId, { modelId: selected.model.id, datasetVersionId: selected.dataset.id, benchmarkVersionId: selected.benchmark.id });
        if (epoch !== this.epoch) return;
        if (!sameInputs(selected, preview.inputs) || preview.expectedParent !== (this.latest?.id ?? null)) throw new Error("Inputs changed. Refresh the project before saving.");
        this.retry = { id: crypto.randomUUID(), expectedParent: preview.expectedParent, inputs: preview.inputs };
      }
      this.phase = "Verifying files and saving…"; this.render();
      await this.bridge.saveOptimizationSetup(this.projectId, this.retry);
      if (epoch !== this.epoch) return;
      // Read current history even on a successful old retry: do not reactivate it.
      await this.load(); if (this.error) throw this.error;
      this.retry = undefined;
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { this.saving = false; this.phase = undefined; this.startedAt = undefined; this.render(); }
  }
}
function sameInputs(a: OptimizationInputs, b?: OptimizationInputs): boolean {
  return !!b && a.projectId === b.projectId && a.model.id === b.model.id && a.model.fingerprint === b.model.fingerprint
    && a.baselineRevision.id === b.baselineRevision.id && a.baselineRevision.fingerprint === b.baselineRevision.fingerprint
    && a.dataset.id === b.dataset.id && a.dataset.datasetId === b.dataset.datasetId && a.dataset.projectId === b.dataset.projectId
    && a.dataset.number === b.dataset.number && a.dataset.fingerprint === b.dataset.fingerprint
    && a.benchmark.id === b.benchmark.id && a.benchmark.fingerprint === b.benchmark.fingerprint;
}
