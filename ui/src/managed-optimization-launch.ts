import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ManagedWorkspace } from "./managed-workspace.js";
import type {
  OptimizationExecutionLimits, OptimizationLaunchAuthorization, OptimizationLaunchPreview,
  OptimizationLaunchRequest, OptimizationLaunchSaved, OptimizationLaunchScope,
  OptimizationProviderLimits,
} from "./optimization-launch.js";
import { parseInputOptimizationRun, parseInputOptimizationRuns, parseInputOptimizationStarted, type InputOptimizationPhase, type InputOptimizationRun, type InputOptimizationStarted } from "./input-optimization.js";
import type { NativeProgress } from "./managed-control.js";

interface Ports {
  open(projectId: string): Promise<ManagedWorkspace>;
  command<T>(args: string[], environment?: Readonly<Record<string, string>>, progress?: (value: NativeProgress) => void, signal?: AbortSignal): Promise<T>;
  environment?(projectId: string, workspace: ManagedWorkspace): Readonly<Record<string, string>>;
  exclusive<T>(projectId: string, run: () => Promise<T>): Promise<T>;
  exclusiveRun<T>(projectId: string, runId: string, run: (signal: AbortSignal) => Promise<T>): Promise<T>;
  abortRun(projectId: string, runId: string): void;
}
function record(value: unknown, label: string, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key))) throw new Error(`Invalid ${label}.`);
  return value as Record<string, unknown>;
}
function uuid(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error("Invalid optimization identity.");
  return value.toLowerCase();
}
function fingerprint(value: unknown): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) throw new Error("Invalid optimization fingerprint.");
  return value;
}
function text(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim() || value !== value.trim() || value.length > 240 || /[\u0000-\u001f]/.test(value)) throw new Error(`Invalid ${label}.`);
  return value;
}
function integer(value: unknown, label: string, zero = false): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < (zero ? 0 : 1)) throw new Error(`Invalid ${label}.`);
  return value;
}
function bound(value: unknown): { id: string; fingerprint: string } {
  const item = record(value, "optimization identity", ["id", "fingerprint"]);
  return { id: uuid(item.id), fingerprint: fingerprint(item.fingerprint) };
}
function providerLimits(value: unknown): OptimizationProviderLimits {
  const limits = record(value, "provider limits", ["maximumRequests", "maximumInputTokens", "maximumOutputTokens", "maximumCostMicrousd"]);
  return {
    maximumRequests: integer(limits.maximumRequests, "provider request limit"),
    maximumInputTokens: integer(limits.maximumInputTokens, "provider input-token limit"),
    maximumOutputTokens: integer(limits.maximumOutputTokens, "provider output-token limit"),
    maximumCostMicrousd: integer(limits.maximumCostMicrousd, "provider cost limit", true),
  };
}
function executionLimits(value: unknown): OptimizationExecutionLimits {
  const limits = record(value, "execution limits", ["maximumIterations", "maximumModels", "maximumDatasetRowChanges", "maximumTrainingSeconds", "maximumDevelopmentEvaluations", "maximumFinalEvaluations"]);
  const result = {
    maximumIterations: integer(limits.maximumIterations, "iteration limit"),
    maximumModels: integer(limits.maximumModels, "model limit"),
    maximumDatasetRowChanges: integer(limits.maximumDatasetRowChanges, "dataset-change limit"),
    maximumTrainingSeconds: integer(limits.maximumTrainingSeconds, "training limit"),
    maximumDevelopmentEvaluations: integer(limits.maximumDevelopmentEvaluations, "development-evaluation limit"),
    maximumFinalEvaluations: integer(limits.maximumFinalEvaluations, "final-evaluation limit"),
  };
  if (result.maximumIterations !== 3 || result.maximumModels !== 3 || result.maximumDatasetRowChanges !== 5_000 || result.maximumTrainingSeconds !== 21_600 || result.maximumFinalEvaluations !== 1 || result.maximumDevelopmentEvaluations % result.maximumModels !== 0) throw new Error("Optimization limits do not match the application envelope.");
  return result;
}
function scope(value: unknown, projectId: string): OptimizationLaunchScope {
  const item = record(value, "optimization launch", ["projectId", "setup", "providerCatalog", "limits", "generation", "advisor", "finalEvaluation", "fingerprint"]);
  if (uuid(item.projectId) !== projectId || item.finalEvaluation !== "selected_candidate_once") throw new Error("Optimization launch belongs to another project or final-evaluation policy.");
  return {
    projectId,
    setup: bound(item.setup),
    providerCatalog: bound(item.providerCatalog),
    limits: executionLimits(item.limits),
    generation: providerLimits(item.generation),
    advisor: providerLimits(item.advisor),
    finalEvaluation: "selected_candidate_once",
    fingerprint: fingerprint(item.fingerprint),
  };
}
function authorization(value: unknown, projectId: string): OptimizationLaunchAuthorization {
  const item = record(value, "optimization authorization", ["id", "scope", "authorizedBy", "createdAt", "fingerprint"]);
  const createdAt = text(item.createdAt, "authorization time");
  if (!Number.isFinite(Date.parse(createdAt))) throw new Error("Invalid authorization time.");
  return { id: uuid(item.id), scope: scope(item.scope, projectId), authorizedBy: text(item.authorizedBy, "authorizer"), createdAt, fingerprint: fingerprint(item.fingerprint) };
}
function same(a: unknown, b: unknown): boolean { return JSON.stringify(a) === JSON.stringify(b); }

/** Exact one-click authority only. It cannot execute a run or receive credentials. */
export class ManagedOptimizationLaunch {
  private active = new Map<string, { runId: string; stop: boolean }>();
  constructor(private ports: Ports) {}
  /** Stop at a durable stage boundary; unlike cancel this preserves re-entry. */
  async stop(projectId: string, runIdValue: unknown): Promise<void> {
    const runId = uuid(runIdValue);
    const active = this.active.get(projectId);
    if (active?.runId === runId) active.stop = true;
    else await this.show(projectId, runId);
  }
  async list(projectId: string): Promise<OptimizationLaunchAuthorization[]> {
    const workspace = await this.ports.open(projectId);
    const history = await this.ports.command<unknown[]>(["optimization-launch", workspace.folder, "list"]);
    if (!Array.isArray(history)) throw new Error("Invalid optimization authorization history.");
    return history.map(value => authorization(value, workspace.manifest.id));
  }
  async preview(projectId: string, setupIdValue: unknown): Promise<OptimizationLaunchPreview> {
    const setupId = uuid(setupIdValue), workspace = await this.ports.open(projectId);
    const received = await this.ports.command<unknown>(["optimization-launch", workspace.folder, "preview", "--setup", setupId]);
    const item = record(received, "optimization launch preview", ["scope", "modelName", "datasetRows", "benchmarkNumber"]);
    const resolved = scope(item.scope, workspace.manifest.id), current = workspace.providerCatalog;
    if (resolved.setup.id !== setupId || !current || resolved.providerCatalog.id !== current.id || resolved.providerCatalog.fingerprint !== current.fingerprint) throw new Error("Optimization inputs or providers changed. Refresh the project.");
    return { scope: resolved, modelName: text(item.modelName, "model name"), datasetRows: integer(item.datasetRows, "dataset size"), benchmarkNumber: integer(item.benchmarkNumber, "benchmark version") };
  }
  async authorize(projectId: string, value: unknown): Promise<OptimizationLaunchSaved> {
    const item = record(value, "optimization launch request", ["id", "scope"]);
    const request: OptimizationLaunchRequest = { id: uuid(item.id), scope: scope(item.scope, projectId) };
    return this.ports.exclusive(projectId, async () => {
      const workspace = await this.ports.open(projectId);
      const current = workspace.providerCatalog;
      if (workspace.manifest.id !== request.scope.projectId || !current || current.id !== request.scope.providerCatalog.id || current.fingerprint !== request.scope.providerCatalog.fingerprint) throw new Error("Optimization inputs or providers changed. Refresh the project.");
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-optimization-launch-")), file = join(directory, "launch.json");
      try {
        await writeFile(file, JSON.stringify(request), { flag: "wx", mode: 0o600 });
        const received = await this.ports.command<unknown>(["optimization-launch", workspace.folder, "authorize", "--file", file]);
        const saved = record(received, "saved optimization launch", ["actionId", "authorization"]);
        const result = { actionId: uuid(saved.actionId), authorization: authorization(saved.authorization, projectId) };
        if (result.authorization.id !== request.id || !same(result.authorization.scope, request.scope)) throw new Error("Saved optimization authorization does not match the request.");
        return result;
      } finally {
        await unlink(file).catch(error => { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; });
        await rmdir(directory);
      }
    });
  }

  async start(projectId: string, setupIdValue: unknown): Promise<InputOptimizationStarted> {
    const setupId = uuid(setupIdValue);
    return this.ports.exclusive(projectId, async () => {
      const workspace = await this.ports.open(projectId);
      const received = await this.ports.command<unknown>(["optimization-launch", workspace.folder, "preview", "--setup", setupId]);
      const item = record(received, "optimization launch preview", ["scope", "modelName", "datasetRows", "benchmarkNumber"]);
      const resolved = scope(item.scope, workspace.manifest.id), current = workspace.providerCatalog;
      if (resolved.setup.id !== setupId || !current || resolved.providerCatalog.id !== current.id || resolved.providerCatalog.fingerprint !== current.fingerprint) throw new Error("Optimization inputs or providers changed. Refresh the project.");
      const request: OptimizationLaunchRequest = { id: randomUUID(), scope: resolved };
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-optimization-run-")), file = join(directory, "launch.json");
      try {
        await writeFile(file, JSON.stringify(request), { flag: "wx", mode: 0o600 });
        return parseInputOptimizationStarted(await this.ports.command<unknown>(["optimization-run", workspace.folder, "start", "--file", file]), projectId);
      } finally {
        await unlink(file).catch(error => { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; });
        await rmdir(directory);
      }
    });
  }

  async show(projectId: string, runIdValue: unknown): Promise<InputOptimizationRun> {
    const runId = uuid(runIdValue), workspace = await this.ports.open(projectId);
    return parseInputOptimizationRun(await this.ports.command<unknown>(["optimization-run", workspace.folder, "show", runId]), projectId);
  }

  async runs(projectId: string): Promise<InputOptimizationRun[]> {
    const workspace = await this.ports.open(projectId);
    return parseInputOptimizationRuns(await this.ports.command<unknown>(["optimization-run", workspace.folder, "list"]), projectId);
  }

  async cancel(projectId: string, runIdValue: unknown): Promise<InputOptimizationRun> {
    const runId = uuid(runIdValue), workspace = await this.ports.open(projectId);
    const cancelled = parseInputOptimizationStarted(
      await this.ports.command<unknown>(["optimization-run", workspace.folder, "cancel", runId]),
      projectId,
    ).run;
    this.ports.abortRun(projectId, runId);
    return cancelled;
  }

  async drive(projectId: string, runIdValue: unknown, progress?: (phase: InputOptimizationPhase, native?: NativeProgress) => void): Promise<InputOptimizationRun> {
    const runId = uuid(runIdValue);
    return this.ports.exclusiveRun(projectId, runId, async signal => {
      const workspace = await this.ports.open(projectId);
      const execution = { runId, stop: false };
      this.active.set(projectId, execution);
      const stage = async (phase: InputOptimizationPhase, command: string, native = false): Promise<void> => {
        if (execution.stop) throw stopped;
        progress?.(phase);
        await this.ports.command<unknown>(["optimization-run", workspace.folder, command, runId], native ? this.ports.environment?.(projectId, workspace) : undefined,
          value => progress?.(phase, value), signal);
      };
      try {
        await stage("checking_inputs", "prepare");
        await stage("preparing_data", "materialize");
        await stage("starting", "attach");
        await stage("training", "execute", true);
        await stage("saving_candidate", "register");
        let current = parseInputOptimizationRun(await this.ports.command<unknown>(["optimization-run", workspace.folder, "show", runId]), projectId);
        if (current.state !== "baseline_retained") {
          await stage("evaluating", "finalize", true);
          current = parseInputOptimizationRun(await this.ports.command<unknown>(["optimization-run", workspace.folder, "show", runId]), projectId);
        }
        progress?.("complete");
        return current;
      } catch (error) {
        const current = parseInputOptimizationRun(await this.ports.command<unknown>(["optimization-run", workspace.folder, "show", runId]), projectId);
        if (current.state === "cancelled" || error === stopped) return current;
        throw error;
      } finally {
        this.active.delete(projectId);
      }
    });
  }
}
const stopped = Symbol("stopped at durable stage boundary");
