import type { EncoderGymBridge } from "../preload.js";
import type { AgentFinalReview, AgentFinalView } from "../optimization-final.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { NativeProgress } from "../managed-control.js";

export interface FinalState {
  loaded: boolean; loading: boolean; revision: number;
  view?: AgentFinalView | null; review?: AgentFinalReview; error?: unknown;
  operation?: "review" | "evaluate" | "promote"; stopping?: boolean;
  progress?: NativeProgress; promoted?: boolean;
}

/** Presentation of explicit post-loop actions; the CLI owns all authority. */
export class OptimizationFinalController {
  private entries = new Map<string, FinalState>();
  busyRun?: string;
  constructor(readonly projectId: string, private bridge: EncoderGymBridge, private render: () => void,
    private updated?: (workspace: ManagedWorkspace, snapshot?: WorkspaceSnapshot) => void) {}
  state(runId: string): FinalState {
    let state = this.entries.get(runId);
    if (!state) { state = { loaded: false, loading: false, revision: 0 }; this.entries.set(runId, state); }
    return state;
  }
  async ensure(runId: string): Promise<void> {
    const state = this.state(runId);
    if (!state.loaded && !state.loading && !state.error && !state.operation) await this.refresh(runId);
  }
  async refresh(runId: string): Promise<void> {
    const state = this.state(runId);
    if (state.loading || state.operation) return;
    const revision = ++state.revision;
    state.loading = true; state.error = undefined; state.review = undefined; this.render();
    try {
      const saved = await this.bridge.agentFinalResult(this.projectId, runId);
      if (revision === state.revision) { state.view = saved; state.loaded = true; }
    } catch (error) { if (revision === state.revision) state.error = error; }
    finally { if (revision === state.revision) { state.loading = false; this.render(); } }
  }
  async review(runId: string): Promise<void> {
    await this.perform(runId, "review", async state => {
      // Recover an uncertain consent reply instead of creating a new approval.
      state.view = await this.bridge.agentFinalResult(this.projectId, runId);
      state.loaded = true;
      state.review = state.view ? undefined : await this.bridge.reviewAgentFinal(this.projectId, runId);
    });
  }
  dismiss(runId: string): void {
    const state = this.state(runId);
    if (!state.operation) { state.review = undefined; this.render(); }
  }
  async authorize(runId: string): Promise<void> {
    const review = this.state(runId).review;
    if (!review || this.state(runId).view) return;
    await this.evaluate(runId, progress => this.bridge.authorizeAgentFinal(this.projectId, runId, review.token, progress));
  }
  async recover(runId: string): Promise<void> {
    const view = this.state(runId).view;
    if (!view || view.state === "completed") return;
    await this.evaluate(runId, progress => this.bridge.recoverAgentFinal(this.projectId, runId, view.execution.authorization.id, progress));
  }
  private async evaluate(runId: string, execute: (progress: (value: NativeProgress) => void) => Promise<AgentFinalView>): Promise<void> {
    await this.perform(runId, "evaluate", async state => {
      try {
        state.view = await execute(value => { state.progress = value; this.render(); });
        state.review = undefined; state.loaded = true;
      } catch (error) {
        const stopped = state.stopping;
        // A Stop or lost reply does not roll back consent or the one-use grant.
        try { state.view = await this.bridge.agentFinalResult(this.projectId, runId); state.loaded = true; if (state.view) state.review = undefined; }
        catch { /* Keep the previous custody, but never claim completion. */ }
        if (!stopped) throw error;
      }
    });
  }
  async stop(runId: string): Promise<void> {
    const state = this.state(runId);
    if (state.operation !== "evaluate" || state.stopping) return;
    state.stopping = true; this.render();
    try { await this.bridge.stopAgentFinal(this.projectId, runId); }
    catch (error) { state.error = error; }
    finally { state.stopping = false; this.render(); }
  }
  async promote(runId: string): Promise<void> {
    const saved = this.state(runId).view;
    if (!saved?.execution.result?.accepted) return;
    await this.perform(runId, "promote", async state => {
      const managed = await this.bridge.promoteAgentFinal(this.projectId, runId, {
        baselineRevisionId: saved.execution.authorization.scope.comparisonBaselineRevision.id,
        receiptFingerprint: saved.execution.result!.fingerprint,
      });
      state.promoted = true; this.updated?.(managed);
      const opened = await this.bridge.selectProject(this.projectId);
      if (opened.content.state === "ready" && opened.content.workspace.managed) this.updated?.(opened.content.workspace.managed, opened.content.workspace);
    });
  }
  private async perform(runId: string, operation: NonNullable<FinalState["operation"]>, work: (state: FinalState) => Promise<void>): Promise<void> {
    if (this.busyRun) return;
    const state = this.state(runId);
    ++state.revision; state.loading = false; state.error = undefined; state.operation = operation; state.progress = undefined;
    this.busyRun = runId; this.render();
    try { await work(state); } catch (error) { state.error = error; }
    finally { state.operation = undefined; state.stopping = false; this.busyRun = undefined; this.render(); }
  }
}
