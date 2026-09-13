import type { EncoderGymBridge } from "../preload.js";
import type { DatasetEntry } from "../dataset-workspace.js";
import type { ProjectBenchmarkVersion } from "../benchmark-workspace.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { OptimizationLaunchAuthorization } from "../optimization-launch.js";
import type { NativeProgress } from "../managed-control.js";
import type { OptimizationInputs, OptimizationSetup, OptimizationSetupRequest } from "../optimization-setup.js";
import { inputOptimizationPhase, inputOptimizationTerminal, type InputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { inputRunActivity, inputRunStageLabel, type InputRunActivity } from "./input-run-activity.js";

/** Per-project input selection. Running experiments have a separate controller. */
export class OptimizationSetupController {
  data?: { datasets: DatasetEntry[]; benchmarks: ProjectBenchmarkVersion[]; history: OptimizationSetup[] };
  datasetId = "";
  benchmarkId = "";
  loading = false;
  saving = false;
  running = false;
  cancelling = false;
  stopping = false;
  launches?: OptimizationLaunchAuthorization[];
  initializingEvaluation = false;
  run?: InputOptimizationRun;
  activity?: InputRunActivity;
  phase?: string;
  benchmarkProgress?: NativeProgress;
  liveProgress?: NativeProgress;
  liveProgressAt?: number;
  startedAt?: number;
  error?: unknown;
  private epoch = 0;
  private retry?: OptimizationSetupRequest;
  constructor(readonly projectId: string, public workspace: ManagedWorkspace, private bridge: EncoderGymBridge, private render: () => void, private updated?: (workspace: ManagedWorkspace, snapshot?: WorkspaceSnapshot) => void) {}
  newDraft(): void {
    if (this.running || this.saving || this.initializingEvaluation) return;
    this.run = undefined; this.activity = undefined; this.error = undefined; this.liveProgress = undefined;
  }
  async stop(): Promise<void> {
    if (!this.running || !this.run || this.stopping) return;
    this.stopping = true; this.render();
    try { await this.bridge.stopInputOptimization(this.projectId, this.run.id); }
    catch (error) { this.error = error; this.stopping = false; this.render(); }
  }
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
  get canInitializeBenchmark(): boolean {
    return !!this.data && !this.data.benchmarks.length && !!this.workspace.scientificBinding
      && !this.loading && !this.saving && !this.running && !this.initializingEvaluation;
  }
  get canSave(): boolean { return !!this.selected && !this.saved && !this.saving && !this.loading && !this.running && !this.initializingEvaluation; }
  get canOptimize(): boolean {
    const canCreateInputs = this.canInitializeBenchmark && !!this.model && !!this.dataset?.version.rows;
    return (!!this.selected || canCreateInputs) && !this.loading && !this.saving && !this.running && !this.initializingEvaluation;
  }
  get canCancel(): boolean { return this.running && !!this.run && !inputOptimizationTerminal(this.run.state) && !this.cancelling; }
  get runPhase(): InputOptimizationPhase | undefined { return this.run ? inputOptimizationPhase(this.run.state) : undefined; }
  sync(workspace: ManagedWorkspace): void {
    // A read-only page refresh must not invalidate observation of an active
    // drive. Its completion reloads the exact project before releasing busy.
    if (this.running || this.initializingEvaluation) return;
    if (workspace !== this.workspace) { this.workspace = workspace; this.invalidate(); }
  }
  private invalidate(): void { this.epoch++; this.loading = false; this.error = undefined; this.data = undefined; this.retry = undefined; }
  refresh(): void { if (!this.saving && !this.running && !this.initializingEvaluation) { this.invalidate(); this.render(); } }
  select(kind: "dataset" | "benchmark", id: string): void {
    if (this.saving || this.loading || this.running || this.initializingEvaluation) return;
    if (kind === "dataset") this.datasetId = id; else this.benchmarkId = id;
    this.retry = undefined; this.error = undefined; this.render();
  }
  async ensure(): Promise<void> { if (!this.data && !this.error && !this.loading && !this.saving) await this.load(); }
  private async load(): Promise<void> {
    this.loading = true; const epoch = this.epoch; this.render();
    try {
      const [datasets, benchmarks, history, launches] = await Promise.all([
        this.bridge.queryDatasets(this.projectId, { kind: "list" }), this.bridge.queryBenchmarks(this.projectId, { kind: "list" }), this.bridge.optimizationSetups(this.projectId),
        this.bridge.optimizationLaunches(this.projectId),
      ]);
      if (datasets.kind !== "list" || benchmarks.kind !== "list") throw new Error("Could not read optimization inputs.");
      if (epoch !== this.epoch) return;
      this.data = { datasets: datasets.entries, benchmarks: benchmarks.versions, history };
      this.launches = launches;
      const latest = history.at(-1);
      // Saved versions never float to newly-created versions. For first setup,
      // prefer recorded model provenance, not an arbitrary training population.
      const linked = this.workspace.modelDatasetLinks?.find(link => link.modelId === this.model?.id)?.version.id;
      this.datasetId = latest?.inputs.dataset.id ?? linked ?? (datasets.entries.length === 1 ? datasets.entries[0]?.versions.at(-1)?.version.id : undefined) ?? "";
      // Evaluation is a project input, not a per-run user choice. New setups
      // always pin the current project benchmark; old setup history stays exact.
      this.benchmarkId = benchmarks.versions.at(-1)?.id ?? "";
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.render(); } }
  }
  async save(): Promise<void> {
    if (!this.canSave) return;
    const selected = this.selected!, epoch = this.epoch;
    this.saving = true; this.error = undefined; this.phase = this.retry ? "Verifying files and saving…" : "Checking inputs…"; this.startedAt = Date.now(); this.render();
    const progress = (value: NativeProgress): void => { if (epoch === this.epoch && this.saving) { this.liveProgress = value; this.liveProgressAt = Date.now(); this.render(); } };
    try {
      if (!this.retry) {
        const preview = await this.bridge.previewOptimizationSetup(this.projectId, { modelId: selected.model.id, datasetVersionId: selected.dataset.id, benchmarkVersionId: selected.benchmark.id }, progress);
        if (epoch !== this.epoch) return;
        if (!sameInputs(selected, preview.inputs) || preview.expectedParent !== (this.latest?.id ?? null)) throw new Error("Inputs changed. Refresh the project before saving.");
        this.retry = { id: crypto.randomUUID(), expectedParent: preview.expectedParent, inputs: preview.inputs };
      }
      this.phase = "Verifying files and saving…"; this.render();
      await this.bridge.saveOptimizationSetup(this.projectId, this.retry, progress);
      if (epoch !== this.epoch) return;
      // Read current history even on a successful old retry: do not reactivate it.
      await this.load(); if (this.error) throw this.error;
      this.retry = undefined;
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { this.saving = false; this.phase = undefined; this.startedAt = undefined; this.render(); }
  }

  async optimize(): Promise<void> {
    if (!this.canOptimize) return;
    this.error = undefined;
    if (!this.benchmark) await this.initializeEvaluation();
    if (!this.benchmark || this.error) return;
    if (!this.saved) await this.save();
    if (!this.saved || this.error || !this.latest) return;
    const epoch = this.epoch;
    this.running = true; this.startedAt = Date.now(); this.render();
    const progress = (value: NativeProgress): void => { if (epoch === this.epoch && this.running) { this.liveProgress = value; this.liveProgressAt = Date.now(); this.render(); } };
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      if (!this.run || inputOptimizationTerminal(this.run.state)) {
        this.activity = undefined;
        this.run = undefined;
        this.phase = "Starting run";
        this.render();
        this.run = (await this.bridge.startInputOptimization(this.projectId, this.latest.id, progress)).run;
        if (epoch !== this.epoch) return;
        this.phase = undefined;
        this.render();
      }
      let settled = false;
      const poll = (): void => {
        timer = setTimeout(() => {
          if (settled || epoch !== this.epoch || !this.run) return;
          void Promise.all([this.bridge.inputOptimizationRun(this.projectId, this.run.id), this.bridge.projectActivity(this.projectId, 30)]).then(([run, log]) => {
            if (!settled && epoch === this.epoch) {
              this.run = run;
              const observed = inputRunActivity(log, run.id);
              if (observed) this.activity = observed;
              this.render(); poll();
            }
          }, () => { if (!settled && epoch === this.epoch) poll(); });
        }, 1250);
      };
      poll();
      try { this.run = await this.bridge.driveInputOptimization(this.projectId, this.run.id, progress); }
      finally { settled = true; if (timer) clearTimeout(timer); }
      if (epoch !== this.epoch) return;
      this.activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30), this.run.id);
      const opened = await this.bridge.selectProject(this.projectId);
      if (epoch !== this.epoch) return;
      if (opened.content.state === "ready" && opened.content.workspace.managed) {
        this.workspace = opened.content.workspace.managed;
        this.updated?.(this.workspace, opened.content.workspace);
      }
    } catch (error) {
      if (timer) clearTimeout(timer);
      if (epoch === this.epoch) {
        this.error = error;
        if (this.run) {
          const existing = this.run;
          this.run = await this.bridge.inputOptimizationRun(this.projectId, existing.id).catch(() => existing);
          this.activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30).catch(() => ({ project_id: this.projectId, actions: [] })), existing.id) ?? this.activity;
        }
      }
    } finally {
      if (epoch === this.epoch) { this.running = false; this.stopping = false; this.phase = undefined; this.startedAt = undefined; this.render(); }
    }
  }
  async initializeEvaluation(): Promise<void> {
    if (!this.canInitializeBenchmark) return;
    const epoch = this.epoch;
    this.initializingEvaluation = true; this.error = undefined; this.benchmarkProgress = undefined;
    this.phase = "Checking evaluation files"; this.startedAt = Date.now(); this.render();
    try {
      const saved = await this.bridge.initializeBenchmark(this.projectId, progress => {
        if (epoch !== this.epoch || !this.initializingEvaluation) return;
        this.benchmarkProgress = progress; this.phase = inputRunStageLabel(progress.phase); this.render();
      });
      if (epoch !== this.epoch) return;
      const opened = await this.bridge.selectProject(this.projectId);
      if (epoch !== this.epoch) return;
      if (opened.content.state !== "ready" || !opened.content.workspace.managed) throw new Error("Could not reload the project benchmark.");
      this.workspace = opened.content.workspace.managed;
      this.updated?.(this.workspace);
      this.data = undefined; this.retry = undefined;
      await this.load();
      if (this.error) throw this.error;
      if (this.benchmarkId !== saved.version.id) throw new Error("The current project benchmark changed. Refresh the project.");
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally {
      if (epoch === this.epoch) {
        this.initializingEvaluation = false; this.benchmarkProgress = undefined; this.phase = undefined; this.startedAt = undefined; this.render();
      }
    }
  }
  async cancel(): Promise<void> {
    if (!this.canCancel || !this.run) return;
    const run = this.run; this.cancelling = true; this.error = undefined; this.render();
    try {
      this.run = await this.bridge.cancelInputOptimization(this.projectId, run.id);
      this.render();
      this.activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30), run.id) ?? this.activity;
    } catch (error) { this.error = error; }
    finally { this.cancelling = false; this.render(); }
  }
}
function sameInputs(a: OptimizationInputs, b?: OptimizationInputs): boolean {
  return !!b && a.projectId === b.projectId && a.model.id === b.model.id && a.model.fingerprint === b.model.fingerprint
    && a.baselineRevision.id === b.baselineRevision.id && a.baselineRevision.fingerprint === b.baselineRevision.fingerprint
    && a.dataset.id === b.dataset.id && a.dataset.datasetId === b.dataset.datasetId && a.dataset.projectId === b.dataset.projectId
    && a.dataset.number === b.dataset.number && a.dataset.fingerprint === b.dataset.fingerprint
    && a.benchmark.id === b.benchmark.id && a.benchmark.fingerprint === b.benchmark.fingerprint;
}
