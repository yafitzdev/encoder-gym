import type { EncoderGymBridge } from "../preload.js";
import { inputOptimizationMayBeActive, newerInputRun, observedInputRun, type InputOptimizationRun } from "../input-optimization.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { NativeProgress } from "../managed-control.js";
import { appendLiveActivity, inputRunActivity, type InputRunActivity, type InputRunActivityEntry } from "./input-run-activity.js";
import { OptimizationFinalController } from "./optimization-final-controller.js";

export class InputRunsController {
  runs?: InputOptimizationRun[];
  activities = new Map<string, InputRunActivity>();
  loading = false;
  runningId?: string;
  cancellingId?: string;
  stoppingId?: string;
  liveProgress?: NativeProgress;
  liveEvents: InputRunActivityEntry[] = [];
  liveProgressAt?: number;
  error?: unknown;
  private epoch = 0;
  private activityLoaded = new Set<string>();
  private activityLoading = new Set<string>();
  activityErrors = new Map<string, unknown>();
  private historyLoadedAt = new Map<string, string>();
  historyLoading = new Set<string>();
  historyErrors = new Map<string, unknown>();
  readonly final: OptimizationFinalController;
  constructor(readonly projectId: string, private bridge: EncoderGymBridge, private render: () => void, private updated?: (workspace: ManagedWorkspace, snapshot?: WorkspaceSnapshot) => void) {
    this.final = new OptimizationFinalController(projectId, bridge, render, updated);
  }
  async stop(run: InputOptimizationRun): Promise<void> {
    if (run.projectId !== this.projectId || this.stoppingId || this.runningId !== run.id && !inputOptimizationMayBeActive(run)) return;
    this.stoppingId = run.id; this.render();
    try {
      await this.bridge.stopInputOptimization(this.projectId, run.id);
      // The drive may have settled before the Stop reply. Refresh the durable
      // acknowledgement instead of leaving a stale stopping view on screen.
      this.replace(await this.bridge.inputOptimizationRun(this.projectId, run.id));
      this.render();
    }
    catch (error) { this.error = error; }
    finally { this.stoppingId = undefined; this.render(); }
  }
  retain(run: InputOptimizationRun): void { this.replace(run); }
  async ensureHistory(runId: string): Promise<void> {
    const run = this.runs?.find(value => value.id === runId);
    if (!run?.agentExecution || this.historyLoadedAt.get(runId) === run.updatedAt
      || this.historyLoading.has(runId) || this.historyErrors.has(runId)) return;
    this.historyLoading.add(runId);
    const epoch = this.epoch;
    try {
      const detailed = await this.bridge.inputOptimizationRun(this.projectId, runId);
      if (epoch === this.epoch) {
        this.replace(detailed);
        this.historyLoadedAt.set(runId, detailed.updatedAt);
      }
    } catch (error) { if (epoch === this.epoch) this.historyErrors.set(runId, error); }
    finally {
      this.historyLoading.delete(runId);
      if (epoch === this.epoch) this.render();
    }
  }
  retryHistory(runId: string): void {
    this.historyErrors.delete(runId);
    this.historyLoadedAt.delete(runId);
    void this.ensureHistory(runId);
    this.render();
  }
  async ensureActivity(runId: string): Promise<void> {
    if (this.activityLoaded.has(runId) || this.activityLoading.has(runId) || this.activityErrors.has(runId)) return;
    this.activityLoading.add(runId);
    const epoch = this.epoch;
    try {
      const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 100, runId), runId);
      if (epoch === this.epoch) { if (activity) this.activities.set(runId, activity); this.activityLoaded.add(runId); }
    } catch (error) { if (epoch === this.epoch) this.activityErrors.set(runId, error); }
    finally { this.activityLoading.delete(runId); this.render(); }
  }
  async ensure(): Promise<void> { if (!this.runs && !this.loading && !this.error) await this.load(); }
  refresh(): void { if (!this.runningId && !this.stoppingId) { this.epoch++; this.runs = undefined; this.error = undefined; this.loading = false; this.activities.clear(); this.activityLoaded.clear(); this.activityErrors.clear(); this.historyLoadedAt.clear(); this.historyErrors.clear(); this.render(); } }
  private async load(): Promise<void> {
    const epoch = this.epoch; this.loading = true; this.error = undefined; this.render();
    try {
      const runs = await this.bridge.inputOptimizationRuns(this.projectId);
      if (epoch === this.epoch) {
        const observed = new Map(this.runs?.map(run => [run.id, run]));
        for (const run of runs) observed.set(run.id, observedInputRun(observed.get(run.id), run));
        this.runs = [...observed.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
      }
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.render(); } }
  }
  async resume(run: InputOptimizationRun): Promise<void> {
    if (this.runningId || run.projectId !== this.projectId) return;
    const epoch = this.epoch; this.runningId = run.id; this.error = undefined; this.liveProgress = undefined; this.liveEvents = []; this.replace(run); this.render();
    let timer: ReturnType<typeof setTimeout> | undefined, settled = false;
    const poll = (): void => {
      timer = setTimeout(() => {
        if (settled || epoch !== this.epoch) return;
        void Promise.all([this.bridge.inputOptimizationRun(this.projectId, run.id), this.bridge.projectActivity(this.projectId, 30, run.id)]).then(([value, log]) => {
          if (!settled && epoch === this.epoch) {
            if (this.replace(value)) { const activity = inputRunActivity(log, run.id); if (activity) this.activities.set(run.id, activity); }
            this.render(); poll();
          }
        }, () => { if (!settled && epoch === this.epoch) poll(); });
      }, 1250);
    };
    try {
      poll(); this.replace(await this.bridge.driveInputOptimization(this.projectId, run.id, value => {
        if (!settled && epoch === this.epoch) { this.liveProgress = value; this.liveProgressAt = Date.now(); appendLiveActivity(this.liveEvents, value, this.liveProgressAt); this.render(); }
      }, run.state === "agent_paused" ? run.agentExecution?.headFingerprint : undefined)); settled = true;
      const opened = await this.bridge.selectProject(this.projectId);
      if (epoch === this.epoch && opened.content.state === "ready" && opened.content.workspace.managed) this.updated?.(opened.content.workspace.managed, opened.content.workspace);
      const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30, run.id), run.id); if (activity) this.activities.set(run.id, activity);
    } catch (error) {
      settled = true;
      if (epoch === this.epoch) {
        this.error = error; this.replace(await this.bridge.inputOptimizationRun(this.projectId, run.id).catch(() => run));
        const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30, run.id).catch(() => ({ project_id: this.projectId, actions: [] })), run.id); if (activity) this.activities.set(run.id, activity);
      }
    } finally {
      settled = true; if (timer) clearTimeout(timer);
      if (epoch === this.epoch) { this.runningId = undefined; this.render(); }
    }
  }
  async cancel(run: InputOptimizationRun): Promise<void> {
    if (this.cancellingId || run.projectId !== this.projectId) return;
    this.cancellingId = run.id; this.error = undefined; this.render();
    try {
      this.replace(await this.bridge.cancelInputOptimization(this.projectId, run.id));
      this.render();
      const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30, run.id), run.id);
      if (activity) this.activities.set(run.id, activity);
    } catch (error) { this.error = error; }
    finally { this.cancellingId = undefined; this.render(); }
  }
  private replace(run: InputOptimizationRun): boolean {
    const values = this.runs ?? [];
    const previous = values.find(value => value.id === run.id);
    if (previous && newerInputRun(previous, run)) return false;
    this.runs = values.some(value => value.id === run.id) ? values.map(value => value.id === run.id ? observedInputRun(value, run) : value) : [run, ...values];
    return true;
  }
}
