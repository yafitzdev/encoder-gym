import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ManagedWorkspace } from "./managed-workspace.js";
import type { NativeProgress } from "./managed-control.js";
import { parseInputOptimizationRun } from "./input-optimization.js";
import { finalFingerprint, finalObject, finalUuid, parseAgentFinalAuthorization, parseAgentFinalScope, parseAgentFinalView,
  type AgentFinalScope, type AgentFinalReview, type AgentFinalView } from "./optimization-final.js";

interface Ports {
  open(projectId: string): Promise<ManagedWorkspace>;
  command<T>(args: string[], progress?: (value: NativeProgress) => void, signal?: AbortSignal): Promise<T>;
  exclusive<T>(projectId: string, run: () => Promise<T>): Promise<T>;
  exclusiveRun<T>(projectId: string, runId: string, run: (signal: AbortSignal) => Promise<T>): Promise<T>;
  abortRun(projectId: string, runId: string): void;
}
interface Review { projectId: string; runId: string; token: string; authorizationId: string; scope: AgentFinalScope }
const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);

/** Explicit post-loop actions only; never called by Optimize/drive or a read. */
export class ManagedOptimizationFinal {
  private reviews = new Map<string, Review>();
  private active = new Map<string, Promise<void>>();
  constructor(private ports: Ports) {}
  private async owned(projectId: string, value: unknown, signal?: AbortSignal): Promise<{ workspace: ManagedWorkspace; runId: string }> {
    signal?.throwIfAborted();
    const runId = finalUuid(value), workspace = await this.ports.open(projectId);
    signal?.throwIfAborted();
    if (workspace.manifest.id !== projectId) throw new Error("Final acceptance belongs to another project.");
    const run = parseInputOptimizationRun(await this.ports.command(["optimization-run", workspace.folder, "show", runId], undefined, signal), projectId);
    if (run.id !== runId || !["agent_completed", "agent_budget_exhausted"].includes(run.state)) throw new Error("Finish adaptive work before final acceptance.");
    return { workspace, runId };
  }
  async read(projectId: string, value: unknown, signal?: AbortSignal): Promise<AgentFinalView | null> {
    const { workspace, runId } = await this.owned(projectId, value, signal);
    return parseAgentFinalView(await this.ports.command(["optimization-run", workspace.folder, "final-agent-result", runId], undefined, signal), projectId, runId);
  }
  async review(projectId: string, value: unknown): Promise<AgentFinalReview> {
    const { workspace, runId } = await this.owned(projectId, value);
    const scope = parseAgentFinalScope(await this.ports.command(["optimization-run", workspace.folder, "preview-final-agent", runId]), projectId, runId);
    const key = `${projectId}:${runId}`, previous = this.reviews.get(key);
    const record = previous && same(previous.scope, scope) ? previous : { projectId, runId, scope, token: randomUUID(), authorizationId: randomUUID() };
    this.reviews.set(key, record);
    return { token: record.token, scope };
  }
  async authorize(projectId: string, value: unknown, tokenValue: unknown, progress?: (value: NativeProgress) => void): Promise<AgentFinalView> {
    const runId = finalUuid(value), token = finalUuid(tokenValue), review = this.reviews.get(`${projectId}:${runId}`);
    if (!review || review.token !== token) throw new Error("Review this run's final acceptance before authorizing it.");
    return this.execute(projectId, runId, async (workspace, signal) => {
      const scope = parseAgentFinalScope(await this.ports.command(["optimization-run", workspace.folder, "preview-final-agent", runId], undefined, signal), projectId, runId);
      if (!same(scope, review.scope)) throw new Error("Final acceptance changed. Review the selected checkpoint again.");
      signal.throwIfAborted();
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-final-consent-")), file = join(directory, "consent.json");
      try {
        await writeFile(file, JSON.stringify({ id: review.authorizationId, scope: review.scope }), { flag: "wx", mode: 0o600 });
        const grant = parseAgentFinalAuthorization(await this.ports.command(["optimization-run", workspace.folder, "authorize-final-agent", runId, "--file", file, "--authorized-by", "local-operator"], undefined, signal), projectId, runId);
        if (grant.id !== review.authorizationId || !same(grant.scope, scope) || grant.authorizedBy !== "local-operator") throw new Error("Final consent differs from the reviewed request.");
        return grant.id;
      } finally {
        await unlink(file).catch(error => { if (error.code !== "ENOENT") throw error; });
        await rmdir(directory);
      }
    }, progress);
  }
  async recover(projectId: string, value: unknown, authorizationIdValue: unknown, progress?: (value: NativeProgress) => void): Promise<AgentFinalView> {
    const runId = finalUuid(value), authorizationId = finalUuid(authorizationIdValue);
    return this.execute(projectId, runId, async (_workspace, signal) => {
      const saved = await this.read(projectId, runId, signal);
      if (saved?.execution.authorization.id !== authorizationId) throw new Error("Saved final consent changed. Refresh the result.");
      return authorizationId;
    }, progress);
  }
  private execute(projectId: string, runId: string, authority: (workspace: ManagedWorkspace, signal: AbortSignal) => Promise<string>, progress?: (value: NativeProgress) => void): Promise<AgentFinalView> {
    return this.ports.exclusiveRun(projectId, runId, async signal => {
      const key = `${projectId}:${runId}`;
      let finish!: () => void;
      this.active.set(key, new Promise<void>(resolve => { finish = resolve; }));
      try {
        const { workspace } = await this.owned(projectId, runId, signal);
        const authorizationId = await authority(workspace, signal);
        signal.throwIfAborted();
        const result = parseAgentFinalView(await this.ports.command(["optimization-run", workspace.folder, "finalize-agent", runId, "--authorization-id", authorizationId], progress ?? (() => {}), signal), projectId, runId);
        if (result?.execution.authorization.id !== authorizationId || result.state !== "completed") throw new Error("Final outcome is not complete. Refresh its saved custody.");
        return result;
      } finally { this.active.delete(key); finish(); }
    });
  }
  async stop(projectId: string, value: unknown): Promise<void> {
    const runId = finalUuid(value), active = this.active.get(`${projectId}:${runId}`);
    if (!active) throw new Error("No final-evaluation worker is owned by this app. Refresh saved custody.");
    this.ports.abortRun(projectId, runId);
    await active;
  }
  async promote(projectId: string, value: unknown, requestValue: unknown): Promise<ManagedWorkspace> {
    const runId = finalUuid(value), request = finalObject(requestValue, ["baselineRevisionId", "receiptFingerprint"]);
    const baseline = finalUuid(request.baselineRevisionId), fingerprint = finalFingerprint(request.receiptFingerprint);
    return this.ports.exclusive(projectId, async () => {
      const { workspace } = await this.owned(projectId, runId), saved = await this.read(projectId, runId);
      if (!saved?.execution.result?.accepted || saved.execution.result.fingerprint !== fingerprint || saved.execution.authorization.scope.comparisonBaselineRevision.id !== baseline) throw new Error("Promote only the reviewed, final-accepted checkpoint.");
      const result = await this.ports.command<ManagedWorkspace>(["optimization-run", workspace.folder, "promote-final-agent", runId, "--expected-baseline-revision", baseline, "--expected-final-receipt", fingerprint]);
      if (result.manifest.id !== projectId) throw new Error("Promoted model belongs to another project.");
      return result;
    });
  }
}
