import type { EncoderGymBridge } from "../preload.js";
import type { InputOptimizationRun } from "../input-optimization.js";
import type { ManagedWorkspace } from "../managed-workspace.js";

export class InputRunsController {
  runs?: InputOptimizationRun[];
  loading = false;
  runningId?: string;
  error?: unknown;
  private epoch = 0;
  constructor(readonly projectId: string, private bridge: EncoderGymBridge, private render: () => void, private updated?: (workspace: ManagedWorkspace) => void) {}
  async ensure(): Promise<void> { if (!this.runs && !this.loading && !this.error) await this.load(); }
  refresh(): void { if (!this.runningId) { this.epoch++; this.runs = undefined; this.error = undefined; this.loading = false; this.render(); } }
  private async load(): Promise<void> {
    const epoch = this.epoch; this.loading = true; this.error = undefined; this.render();
    try {
      const runs = await this.bridge.inputOptimizationRuns(this.projectId);
      if (epoch === this.epoch) this.runs = runs.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    } catch (error) { if (epoch === this.epoch) this.error = error; }
    finally { if (epoch === this.epoch) { this.loading = false; this.render(); } }
  }
  async resume(run: InputOptimizationRun): Promise<void> {
    if (this.runningId || run.projectId !== this.projectId) return;
    const epoch = this.epoch; this.runningId = run.id; this.error = undefined; this.replace(run); this.render();
    let timer: ReturnType<typeof setTimeout> | undefined, settled = false;
    const poll = (): void => {
      timer = setTimeout(() => {
        if (settled || epoch !== this.epoch) return;
        void this.bridge.inputOptimizationRun(this.projectId, run.id).then(value => { if (!settled && epoch === this.epoch) { this.replace(value); this.render(); poll(); } }, () => { if (!settled && epoch === this.epoch) poll(); });
      }, 750);
    };
    try {
      poll(); this.replace(await this.bridge.driveInputOptimization(this.projectId, run.id)); settled = true;
      const opened = await this.bridge.selectProject(this.projectId);
      if (epoch === this.epoch && opened.content.state === "ready" && opened.content.workspace.managed) this.updated?.(opened.content.workspace.managed);
    } catch (error) {
      settled = true;
      if (epoch === this.epoch) { this.error = error; this.replace(await this.bridge.inputOptimizationRun(this.projectId, run.id).catch(() => run)); }
    } finally {
      settled = true; if (timer) clearTimeout(timer);
      if (epoch === this.epoch) { this.runningId = undefined; this.render(); }
    }
  }
  private replace(run: InputOptimizationRun): void {
    const values = this.runs ?? [];
    this.runs = values.some(value => value.id === run.id) ? values.map(value => value.id === run.id ? run : value) : [run, ...values];
  }
}
