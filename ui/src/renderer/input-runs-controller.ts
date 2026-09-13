import type { EncoderGymBridge } from "../preload.js";
import type { InputOptimizationRun } from "../input-optimization.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { NativeProgress } from "../managed-control.js";
import { inputRunActivity, type InputRunActivity } from "./input-run-activity.js";

export class InputRunsController {
  runs?: InputOptimizationRun[];
  activities = new Map<string, InputRunActivity>();
  loading = false;
  runningId?: string;
  cancellingId?: string;
  stoppingId?: string;
  liveProgress?: NativeProgress;
  liveProgressAt?: number;
  error?: unknown;
  private epoch = 0;
  constructor(readonly projectId: string, private bridge: EncoderGymBridge, private render: () => void, private updated?: (workspace: ManagedWorkspace, snapshot?: WorkspaceSnapshot) => void) {}
  async stop(run: InputOptimizationRun): Promise<void> {
    if (this.runningId !== run.id || this.stoppingId) return;
    this.stoppingId = run.id; this.render();
    try { await this.bridge.stopInputOptimization(this.projectId, run.id); }
    catch (error) { this.error = error; this.stoppingId = undefined; this.render(); }
  }
  retain(run: InputOptimizationRun): void { this.replace(run); }
  async ensure(): Promise<void> { if (!this.runs && !this.loading && !this.error) await this.load(); }
  refresh(): void { if (!this.runningId) { this.epoch++; this.runs = undefined; this.error = undefined; this.loading = false; this.render(); } }
  private async load(): Promise<void> {
    const epoch = this.epoch; this.loading = true; this.error = undefined; this.render();
    try {
      const [runs, log] = await Promise.all([this.bridge.inputOptimizationRuns(this.projectId), this.bridge.projectActivity(this.projectId, 100)]);
      if (epoch === this.epoch) {
        this.runs = runs.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
        this.activities = new Map(runs.flatMap(run => { const activity = inputRunActivity(log, run.id); return activity ? [[run.id, activity] as const] : []; }));
      }
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.render(); } }
  }
  async resume(run: InputOptimizationRun): Promise<void> {
    if (this.runningId || run.projectId !== this.projectId) return;
    const epoch = this.epoch; this.runningId = run.id; this.error = undefined; this.liveProgress = undefined; this.replace(run); this.render();
    let timer: ReturnType<typeof setTimeout> | undefined, settled = false;
    const poll = (): void => {
      timer = setTimeout(() => {
        if (settled || epoch !== this.epoch) return;
        void Promise.all([this.bridge.inputOptimizationRun(this.projectId, run.id), this.bridge.projectActivity(this.projectId, 30)]).then(([value, log]) => {
          if (!settled && epoch === this.epoch) { this.replace(value); const activity = inputRunActivity(log, run.id); if (activity) this.activities.set(run.id, activity); this.render(); poll(); }
        }, () => { if (!settled && epoch === this.epoch) poll(); });
      }, 1250);
    };
    try {
      poll(); this.replace(await this.bridge.driveInputOptimization(this.projectId, run.id, value => {
        if (!settled && epoch === this.epoch) { this.liveProgress = value; this.liveProgressAt = Date.now(); this.render(); }
      })); settled = true;
      const opened = await this.bridge.selectProject(this.projectId);
      if (epoch === this.epoch && opened.content.state === "ready" && opened.content.workspace.managed) this.updated?.(opened.content.workspace.managed, opened.content.workspace);
      const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30), run.id); if (activity) this.activities.set(run.id, activity);
    } catch (error) {
      settled = true;
      if (epoch === this.epoch) {
        this.error = error; this.replace(await this.bridge.inputOptimizationRun(this.projectId, run.id).catch(() => run));
        const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30).catch(() => ({ project_id: this.projectId, actions: [] })), run.id); if (activity) this.activities.set(run.id, activity);
      }
    } finally {
      settled = true; if (timer) clearTimeout(timer);
      if (epoch === this.epoch) { this.runningId = undefined; this.stoppingId = undefined; this.render(); }
    }
  }
  async cancel(run: InputOptimizationRun): Promise<void> {
    if (this.cancellingId || run.projectId !== this.projectId) return;
    this.cancellingId = run.id; this.error = undefined; this.render();
    try {
      this.replace(await this.bridge.cancelInputOptimization(this.projectId, run.id));
      this.render();
      const activity = inputRunActivity(await this.bridge.projectActivity(this.projectId, 30), run.id);
      if (activity) this.activities.set(run.id, activity);
    } catch (error) { this.error = error; }
    finally { this.cancellingId = undefined; this.render(); }
  }
  private replace(run: InputOptimizationRun): void {
    const values = this.runs ?? [];
    this.runs = values.some(value => value.id === run.id) ? values.map(value => value.id === run.id ? run : value) : [run, ...values];
  }
}
