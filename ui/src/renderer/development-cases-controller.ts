import type { OptimizationCases } from "../optimization-cases.js";
import type { OptimizationIteration } from "../optimization-history.js";

export type DevelopmentCasesState = { status: "idle" | "loading" } | { status: "error" } | { status: "ready"; value: OptimizationCases };

/** Explicit, read-only loading. Late replies cannot cross project refreshes,
 * iterations or newer retries; native diagnostics never ride status polling. */
export class DevelopmentCasesController {
  private entries = new Map<string, DevelopmentCasesState>();
  private epoch = 0;
  constructor(private projectId: string, private read: (runId: string, iteration: number) => Promise<OptimizationCases>, private render: () => void) {}
  state(runId: string, iterationId: string): DevelopmentCasesState { return this.entries.get(`${runId}:${iterationId}`) ?? { status: "idle" }; }
  clear(): void { this.epoch++; this.entries.clear(); }
  async load(runId: string, iteration: OptimizationIteration, retry = false): Promise<void> {
    if (!iteration.completed || !iteration.modelId || iteration.noChange) return;
    const key = `${runId}:${iteration.id}`, previous = this.entries.get(key);
    if (previous?.status === "loading" || previous && !retry) return;
    while (this.entries.size >= 5 && !this.entries.has(key)) this.entries.delete(this.entries.keys().next().value!);
    const pending: DevelopmentCasesState = { status: "loading" }, epoch = this.epoch;
    this.entries.set(key, pending); this.render();
    const current = () => epoch === this.epoch && this.entries.get(key) === pending;
    try {
      const value = await this.read(runId, iteration.number);
      if (current()) {
        if (value.projectId !== this.projectId || value.runId !== runId || value.iterationId !== iteration.id || value.iteration !== iteration.number
          || value.baselineModelId !== iteration.startingModelId || value.candidateModelId !== iteration.modelId) throw new Error("Saved comparison custody changed.");
        this.entries.set(key, { status: "ready", value }); this.render();
      }
    } catch { if (current()) { this.entries.set(key, { status: "error" }); this.render(); } }
  }
}
