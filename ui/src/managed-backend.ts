import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve, sep } from "node:path";
import type { ManagedBaselineRestorationRequest, ManagedLaunchPreview, ManagedOptimizationAuthority, ManagedOptimizationRequest, ManagedOptimizationResult, ManagedPromotionRequest, ManagedProviderStatus, ManagedReadiness, NativePathChoice, NomosBindingPreview, OptimizationManifestChoice, PreparedOptimizationChoice, ProviderInput, ProviderSettingsRequest } from "./managed-control.js";
import type { CreateProjectRequest, DatasetChoice, DatasetPurpose, FolderChoice, LocalModel, ManagedWorkspace, ModelChoice } from "./managed-workspace.js";
import type { ProjectRegistry } from "./project-registry.js";
import type { WorkspaceSnapshot } from "./workspace.js";
import { readWorkspaceDatabase } from "./evidence/read-workspace.js";
import type { NativeProgress, RunActivity, ManagedRunStatus } from "./managed-control.js";
import { executeObservedCommand, recordProgress } from "./run-activity.js";
import { ManagedDatasets } from "./managed-datasets.js";
import { ManagedBenchmarks } from "./managed-benchmarks.js";
import { ManagedOptimizationSetup } from "./managed-optimization-setup.js";
import { ManagedOptimizationLaunch } from "./managed-optimization-launch.js";
import { providerCredentialBinding } from "./provider-credentials.js";
import type { AppendProjectActivity, ProjectActivityEvent, ProjectActivityExport, ProjectActivityLog, ProjectActivityNarrative, ProjectActivityReference, ProjectActivitySource } from "./project-activity.js";

const purposes = new Set<DatasetPurpose>(["unassigned", "training", "development", "sealed"]);
export function datasetPurpose(value: unknown): DatasetPurpose {
  if (typeof value !== "string" || !purposes.has(value as DatasetPurpose)) throw new Error("Choose a dataset purpose.");
  return value as DatasetPurpose;
}
function text(value: unknown, label: string, max = 120): string {
  if (typeof value !== "string" || !value.trim() || value.length > max || /[\u0000-\u001f]/.test(value)) throw new Error(`Invalid ${label}.`);
  return value.trim();
}
function uuid(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error(`Invalid ${label}.`);
  return value.toLowerCase();
}
function object(value: unknown, label: string, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Invalid ${label}.`);
  const record = value as Record<string, unknown>;
  if (Object.keys(record).some(key => !keys.includes(key))) throw new Error(`${label} contains an unsupported setting.`);
  return record;
}
function integer(value: unknown, label: string, allowZero = false, maximum = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < (allowZero ? 0 : 1) || value > maximum) throw new Error(`Invalid ${label}.`);
  return value;
}
function providerInput(value: unknown, role: "generation" | "advisor" | "evaluator"): ProviderInput {
  const label = `${role} provider`;
  const provider = object(value, label, ["kind", "endpoint", "model", "authentication", "environmentFallback", "connectionId", "limits"]);
  if (!(["fake", "openai-compatible"] as unknown[]).includes(provider.kind) || !(["none", "bearer"] as unknown[]).includes(provider.authentication)) throw new Error(`Invalid ${label}.`);
  const endpoint = provider.endpoint === undefined ? undefined : text(provider.endpoint, `${label} endpoint`, 2048);
  const environmentFallback = provider.environmentFallback === undefined ? undefined : text(provider.environmentFallback, `${label} environment fallback`, 128);
  const connectionId = provider.connectionId === undefined ? undefined : uuid(provider.connectionId, `${label} connection`);
  const expectedFallback = role === "generation" ? "SYNTH_OPENAI_API_KEY" : role === "advisor" ? "SYNTH_ADVISOR_API_KEY" : "SYNTH_EVALUATOR_API_KEY";
  if (provider.kind === "fake" && (endpoint !== undefined || provider.authentication !== "none" || environmentFallback !== undefined || connectionId !== undefined)) throw new Error(`Invalid ${label}.`);
  if (provider.kind === "openai-compatible" && (!endpoint || provider.authentication !== "bearer" || (connectionId ? environmentFallback !== undefined : environmentFallback !== expectedFallback))) throw new Error(`Invalid ${label}.`);
  const limits = object(provider.limits, `${label} limits`, ["maximumRequests", "maximumInputTokens", "maximumOutputTokens", "maximumCostMicrousd"]);
  const parsedLimits = {
    maximumRequests: integer(limits.maximumRequests, `${label} request limit`, false, 1_000_000),
    maximumInputTokens: integer(limits.maximumInputTokens, `${label} input-token limit`),
    maximumOutputTokens: integer(limits.maximumOutputTokens, `${label} output-token limit`),
    maximumCostMicrousd: integer(limits.maximumCostMicrousd, `${label} cost limit`, true),
  };
  if (provider.kind === "fake" && parsedLimits.maximumCostMicrousd !== 0) throw new Error(`Invalid ${label}.`);
  return {
    kind: provider.kind as ProviderInput["kind"], endpoint, model: text(provider.model, `${label} model`), authentication: provider.authentication as ProviderInput["authentication"], environmentFallback,
    connectionId, limits: parsedLimits,
  };
}
function providerSettings(value: unknown): ProviderSettingsRequest {
  const settings = object(value, "provider settings", ["version", "generation", "advisor", "evaluator", "actor", "reason"]);
  if (settings.version !== 1) throw new Error("Unsupported provider settings version.");
  return {
    version: 1, generation: providerInput(settings.generation, "generation"), advisor: providerInput(settings.advisor, "advisor"),
    ...(settings.evaluator === undefined ? {} : { evaluator: providerInput(settings.evaluator, "evaluator") }),
    ...(settings.actor === undefined ? {} : { actor: text(settings.actor, "provider settings actor") }),
    ...(settings.reason === undefined ? {} : { reason: text(settings.reason, "provider settings reason") }),
  };
}
function providerFile(settings: ProviderSettingsRequest): object {
  const encode = (provider: ProviderInput) => ({
    kind: provider.kind, ...(provider.endpoint ? { endpoint: provider.endpoint } : {}), model: provider.model, authentication: provider.authentication,
    ...(provider.environmentFallback ? { environment_fallback: provider.environmentFallback } : {}), limits: provider.limits,
    ...(provider.connectionId ? { connection_id: provider.connectionId } : {}),
  });
  return { version: 1, generation: encode(settings.generation), advisor: encode(settings.advisor), ...(settings.evaluator ? { evaluator: encode(settings.evaluator) } : {}) };
}
export function redactBackendError(value: string): string {
  return value
    .replace(/\bBearer\s+[^\s"']+/gi, "Bearer [redacted]")
    .replace(/\b(sk|key|token)-[A-Za-z0-9._-]{8,}\b/g, "[redacted]")
    .replace(/([?&](?:api[_-]?key|token|authorization)=)[^&\s]+/gi, "$1[redacted]")
    .replace(/("(?:api[_-]?key|token|authorization|secret|password)"\s*:\s*")[^"]*/gi, "$1[redacted]")
    .replace(/(https?:\/\/)[^/@\s]+:[^/@\s]+@/gi, "$1[redacted]@")
    .trim();
}

export type CommandEnvironment = Readonly<Record<string, string>>;
export type CommandExecutor = (executable: string, args: string[], environment?: CommandEnvironment, progress?: (value: NativeProgress) => void, signal?: AbortSignal) => Promise<string>;
export interface ManagedBackendOptions {
  /** Main-process-only resolver. Returned values are placed only in a child process environment. */
  resolveCredential?: (id: string, environmentFallback?: string) => string | undefined;
}
interface PreparedOptimizationWire {
  manifestPath: string; manifestName: string; readiness: ManagedLaunchPreview;
  authority: ManagedOptimizationAuthority; createdTrainingSnapshot: boolean; externalCalls: number;
}
type ManagedReadinessWire = Omit<ManagedReadiness, "preparedOptimization"> & { preparedOptimization?: PreparedOptimizationWire };
const executeCommand: CommandExecutor = (executable, args, environment, progress, signal) => new Promise((resolve, reject) => {
  if (progress) {
    void executeObservedCommand(executable, args, environment, progress, signal).then(resolve, error => reject(new Error(redactBackendError(String(error.message)))));
    return;
  }
  const statusRead = args[3] === "optimize" && args[5] === "status";
  execFile(executable, args, { windowsHide: true, shell: false, maxBuffer: 64 * 1024 * 1024, ...(statusRead ? { timeout: 15000 } : {}), ...(environment ? { env: { ...process.env, ...environment } } : {}), ...(signal ? { signal } : {}) }, (error, stdout, stderr) => {
    if (error) {
      const missing = (error as NodeJS.ErrnoException).code === "ENOENT";
      const overflow = (error as NodeJS.ErrnoException).code === "ERR_CHILD_PROCESS_STDIO_MAXBUFFER";
      reject(new Error(missing ? "The local workspace backend is missing. Run npm run build:backend in ui, then retry." : overflow ? "The requested activity history is too large to display at once." : statusRead && error.killed ? "Status check timed out. Retrying…" : redactBackendError(stderr) || redactBackendError(error.message)));
      return;
    }
    resolve(stdout);
  });
});

/** Main-process-only bridge. Executable and argument grammar are app-owned. */
export class ManagedBackend {
  private models = new Map<string, LocalModel>();
  private parents = new Map<string, string>();
  private datasets = new Map<string, DatasetChoice & { projectId: string }>();
  private manifests = new Map<string, { projectId: string; path: string; name: string }>();
  private runtimes = new Map<string, { projectId: string; path: string }>();
  private pythons = new Map<string, { projectId: string; path: string }>();
  private histories = new Map<string, { projectId: string; path: string }>();
  private bindingPreviews = new Map<string, { projectId: string; runtime: string; python: string; history?: string; ready: boolean }>();
  private activeProjects = new Set<string>();
  private activeOptimizationRuns = new Map<string, { runId: string; controller: AbortController }>();
  private runActivity = new Map<string, { runId: string; activity: RunActivity }>();
  private activityInitializers = new Map<string, Promise<void>>();
  private busy = false;
  readonly datasetVersions: ManagedDatasets;
  readonly benchmarks: ManagedBenchmarks;
  readonly optimizationSetup: ManagedOptimizationSetup;
  readonly optimizationLaunch: ManagedOptimizationLaunch;
  constructor(readonly executable: string, private registry: ProjectRegistry, private executor: CommandExecutor = executeCommand, private options: ManagedBackendOptions = {}) {
    this.datasetVersions = new ManagedDatasets({ open: id => this.openRegistered(id), command: args => this.command(args), exclusive: (id, run) => this.exclusiveProject(id, run) });
    this.benchmarks = new ManagedBenchmarks({ open: id => this.openRegistered(id), command: (args, progress, signal) => this.command(args, undefined, progress, signal), exclusive: (id, run) => this.exclusiveProject(id, run) });
    this.optimizationSetup = new ManagedOptimizationSetup({ open: id => this.openRegistered(id), command: (args, progress, signal) => this.command(args, undefined, progress, signal), exclusive: (id, run) => this.exclusiveProject(id, run) });
    this.optimizationLaunch = new ManagedOptimizationLaunch({
      open: id => this.openRegistered(id),
      command: (args, environment, progress, signal) => this.command(args, environment, progress, signal),
      environment: async (id, workspace, runId) => {
        const catalog = await this.command<NonNullable<ManagedWorkspace["providerCatalog"]>>(["optimization-run", workspace.folder, "providers", runId]);
        if (catalog.projectId !== id || !Array.isArray(catalog.providers)) throw new Error("Pinned provider catalog belongs to another project.");
        return this.providerEnvironment(id, { ...workspace, providerCatalog: catalog });
      },
      exclusive: (id, run) => this.exclusiveProject(id, run),
      exclusiveRun: (id, runId, run) => this.exclusiveOptimizationRun(id, runId, run),
      abortRun: (id, runId) => this.abortOptimizationRun(id, runId),
    });
  }
  private async command<T>(args: string[], environment?: CommandEnvironment, progress?: (value: NativeProgress) => void, signal?: AbortSignal): Promise<T> {
    const stdout = await this.executor(this.executable, ["--output", "json", "workspace", ...args], environment, progress, signal);
    try { return JSON.parse(stdout) as T; } catch { throw new Error("The workspace backend returned an unreadable response."); }
  }
  private activityFolder(projectId: string): string {
    const project = this.registry.get(projectId);
    if (project.source.kind !== "folder" || project.source.workspaceId !== projectId) throw new Error("Project activity is available for managed projects.");
    return project.source.path;
  }
  private async appendActivityDirect(folder: string, projectId: string, request: AppendProjectActivity): Promise<ProjectActivityEvent> {
    const directory = await mkdtemp(join(tmpdir(), "encoder-gym-activity-"));
    const file = join(directory, "event.json");
    try {
      await writeFile(file, JSON.stringify(request), { flag: "wx", mode: 0o600 });
      const event = await this.command<ProjectActivityEvent>(["activity", folder, "append", "--file", file]);
      if (event.project_id !== projectId || event.action_id !== request.action_id || event.operation !== request.operation || event.state !== request.state) throw new Error("The activity journal returned a mismatched event.");
      return event;
    } finally {
      try { await unlink(file); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
      try { await rmdir(directory); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    }
  }
  private async ensureActivity(projectId: string): Promise<void> {
    const existing = this.activityInitializers.get(projectId);
    if (existing) return existing;
    const initializing = (async () => {
      const folder = this.activityFolder(projectId);
      const result = await this.command<{ projectId: string; created: boolean }>(["activity", folder, "init"]);
      if (result.projectId !== projectId) throw new Error("The activity journal belongs to a different project.");
      if (result.created) {
        const actionId = randomUUID(), createdAt = new Date().toISOString();
        await this.appendActivityDirect(folder, projectId, { action_id: actionId, operation: "activity.logging_enabled", source: "system", state: "started", created_at: createdAt });
        await this.appendActivityDirect(folder, projectId, { action_id: actionId, operation: "activity.logging_enabled", source: "system", state: "succeeded", created_at: new Date().toISOString() });
      }
    })();
    this.activityInitializers.set(projectId, initializing);
    try { await initializing; }
    catch (error) { this.activityInitializers.delete(projectId); throw error; }
  }
  async appendProjectActivity(projectId: string, request: AppendProjectActivity): Promise<ProjectActivityEvent> {
    await this.ensureActivity(projectId);
    return this.appendActivityDirect(this.activityFolder(projectId), projectId, request);
  }
  async startProjectActivity(projectId: string, operation: string, references: ProjectActivityReference[] = [], source: ProjectActivitySource = "desktop", createdAt = new Date().toISOString()): Promise<string> {
    const actionId = randomUUID();
    await this.appendProjectActivity(projectId, { action_id: actionId, operation, source, state: "started", ...(references.length ? { references } : {}), created_at: createdAt });
    return actionId;
  }
  progressProjectActivity(projectId: string, actionId: string, operation: string, stage: string, completed?: number, total?: number, subject?: string, unit?: string, narrative?: ProjectActivityNarrative, runStage?: NativeProgress["runStage"], iteration?: number): Promise<ProjectActivityEvent> {
    const references = [...(subject ? [{ kind: "progress_subject", id: subject }] : []), ...(unit ? [{ kind: "progress_unit", id: unit }] : [])];
    if (runStage) references.push({ kind: "run_stage", id: runStage });
    if (iteration !== undefined) references.push({ kind: "iteration", id: String(iteration) });
    return this.appendProjectActivity(projectId, { action_id: actionId, operation, source: "desktop", state: "progress", stage, references, ...(completed !== undefined && total !== undefined ? { completed, total } : {}), ...(narrative ? { narrative } : {}), created_at: new Date().toISOString() });
  }
  succeedProjectActivity(projectId: string, actionId: string, operation: string, references: ProjectActivityReference[] = []): Promise<ProjectActivityEvent> {
    return this.appendProjectActivity(projectId, { action_id: actionId, operation, source: "desktop", state: "succeeded", ...(references.length ? { references } : {}), created_at: new Date().toISOString() });
  }
  failProjectActivity(projectId: string, actionId: string, operation: string, error: unknown): Promise<ProjectActivityEvent> {
    const raw = error instanceof Error ? error.message : String(error);
    const message = (redactBackendError(raw).replace(/[\u0000-\u001f]+/g, " ").trim() || "Operation failed.").slice(0, 1000);
    return this.appendProjectActivity(projectId, { action_id: actionId, operation, source: "desktop", state: "failed", failure: { code: "operation_failed", message }, created_at: new Date().toISOString() });
  }
  async projectActivity(projectId: string, limit = 100, runId?: string): Promise<ProjectActivityLog> {
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 1000) throw new Error("Activity limit must be between 1 and 1000.");
    await this.ensureActivity(projectId);
    if (runId !== undefined && !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(runId)) throw new Error("Invalid run identity.");
    const log = await this.command<ProjectActivityLog>(["activity", this.activityFolder(projectId), ...(runId ? ["run", runId] : ["list", "--limit", String(limit)])]);
    if (log.project_id !== projectId) throw new Error("The activity journal belongs to a different project.");
    return log;
  }
  async exportProjectActivity(projectId: string, destination: string): Promise<ProjectActivityExport> {
    await this.ensureActivity(projectId);
    return this.command<ProjectActivityExport>(["activity", this.activityFolder(projectId), "export", "--destination", destination]);
  }
  async chooseModel(path: string): Promise<ModelChoice> {
    const model = await this.command<LocalModel>(["inspect-model", path]);
    this.models.clear(); const token = randomUUID(); this.models.set(token, model); return { token, model };
  }
  chooseParent(path: string): FolderChoice {
    this.parents.clear(); const token = randomUUID(); this.parents.set(token, path); return { token, path };
  }
  async open(path: string, verify = false): Promise<ManagedWorkspace> {
    return this.command([verify ? "verify" : "open", path]);
  }
  async openRegistered(id: string, verify = false): Promise<ManagedWorkspace> {
    const project = this.registry.get(id);
    if (project.source.kind !== "folder" || !project.source.workspaceId) throw new Error("Select a managed project first.");
    const workspace = await this.open(project.source.path, verify);
    if (workspace.manifest.id !== project.source.workspaceId) throw new Error("That folder contains a different Gym project. Locate the original project folder.");
    return workspace;
  }
  private async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (this.busy) throw new Error("A local import is already running. Wait for it to finish.");
    this.busy = true; try { return await operation(); } finally { this.busy = false; }
  }
  private async exclusiveProject<T>(projectId: string, operation: () => Promise<T>): Promise<T> {
    if (this.activeProjects.has(projectId)) throw new Error("An operation for this project is already running. Wait for its durable result.");
    this.activeProjects.add(projectId);
    try { return await operation(); } finally { this.activeProjects.delete(projectId); }
  }
  private async exclusiveOptimizationRun<T>(projectId: string, runId: string, operation: (signal: AbortSignal) => Promise<T>): Promise<T> {
    if (this.activeOptimizationRuns.has(projectId)) throw new Error("An optimization run for this project is already active.");
    const controller = new AbortController();
    this.activeOptimizationRuns.set(projectId, { runId, controller });
    try { return await this.exclusiveProject(projectId, () => operation(controller.signal)); }
    finally { this.activeOptimizationRuns.delete(projectId); }
  }
  private abortOptimizationRun(projectId: string, runId: string): void {
    const active = this.activeOptimizationRuns.get(projectId);
    if (active?.runId === runId) active.controller.abort();
  }
  private retainPrepared(projectId: string, workspace: ManagedWorkspace, prepared: PreparedOptimizationWire): PreparedOptimizationChoice {
    const scientificProjectId = workspace.scientificBinding?.runtime.projectSnapshot.id;
    if (typeof prepared.manifestPath !== "string" || typeof prepared.manifestName !== "string" || !scientificProjectId || prepared.readiness?.projectId !== scientificProjectId) {
      throw new Error("The managed optimization preparation does not match this project.");
    }
    const root = resolve(workspace.folder), manifest = resolve(prepared.manifestPath);
    if (manifest !== root && !manifest.startsWith(root + sep)) throw new Error("The managed optimization definition escaped its project workspace.");
    const token = randomUUID();
    this.manifests.set(token, { projectId, path: manifest, name: prepared.manifestName });
    return { token, name: prepared.manifestName, launchPreview: prepared.readiness, authority: prepared.authority, createdTrainingSnapshot: prepared.createdTrainingSnapshot, externalCalls: prepared.externalCalls };
  }
  async create(value: unknown) {
    const request = value as CreateProjectRequest;
    const model = this.models.get(request?.modelToken), parent = this.parents.get(request?.parentToken);
    if (!model || !parent) throw new Error("Choose the checkpoint and parent folder again.");
    const folderName = text(request.folderName, "folder name");
    if (basename(folderName) !== folderName || /[\\/:<>"|?*]/.test(folderName) || [".", ".."].includes(folderName)) throw new Error("Use a single new folder name, not a path.");
    const name = text(request.name, "project name"), task = request.task ? text(request.task, "task") : undefined;
    return this.exclusive(async () => {
      const workspace = await this.command<ManagedWorkspace>(["create", join(parent, folderName), "--name", name, "--model", model.source, "--expected-fingerprint", model.fingerprint, ...(task ? ["--task", task] : [])]);
      this.models.delete(request.modelToken); this.parents.delete(request.parentToken);
      return this.registry.addManaged(workspace);
    });
  }
  async chooseDataset(projectId: string, path: string, purpose: DatasetPurpose): Promise<DatasetChoice> {
    await this.openRegistered(projectId);
    const preview = await this.command<Omit<DatasetChoice, "token" | "purpose">>(["inspect-dataset", path, "--purpose", purpose]);
    this.datasets.clear(); const token = randomUUID();
    const choice = { ...preview, token, purpose, projectId }; this.datasets.set(token, choice); return choice;
  }
  async importDataset(projectId: string, token: unknown, name: unknown): Promise<ManagedWorkspace> {
    const choice = typeof token === "string" ? this.datasets.get(token) : undefined;
    if (!choice || choice.projectId !== projectId) throw new Error("Choose and inspect a dataset for this project first.");
    const displayName = text(name, "dataset name");
    return this.exclusive(async () => {
      const workspace = await this.openRegistered(projectId);
      const imported = await this.command<ManagedWorkspace>(["import-dataset", workspace.folder, "--source", choice.source, "--name", displayName, "--purpose", choice.purpose, "--expected-fingerprint", choice.artifact.fingerprint]);
      this.datasets.delete(choice.token); return imported;
    });
  }

  async upgradeRegistered(projectId: string): Promise<ManagedWorkspace> {
    const workspace = await this.openRegistered(projectId);
    return this.exclusiveProject(projectId, () => this.command<ManagedWorkspace>(["upgrade", workspace.folder]));
  }

  async readiness(projectId: string, manifestToken?: unknown): Promise<ManagedReadiness> {
    const workspace = await this.openRegistered(projectId);
    const args = ["readiness", workspace.folder];
    if (manifestToken !== undefined) {
      const selected = typeof manifestToken === "string" ? this.manifests.get(manifestToken) : undefined;
      if (!selected || selected.projectId !== projectId) throw new Error("Choose the optimization definition for this project again.");
      args.push("--manifest", selected.path);
    }
    const received = await this.command<ManagedReadinessWire>(args);
    const { preparedOptimization, ...readiness } = received;
    return {
      ...readiness,
      ...(preparedOptimization ? { preparedOptimization: this.retainPrepared(projectId, workspace, preparedOptimization) } : {}),
    };
  }

  async chooseOptimizationManifest(projectId: string, path: string): Promise<OptimizationManifestChoice> {
    await this.openRegistered(projectId);
    const token = randomUUID();
    this.manifests.set(token, { projectId, path, name: basename(path) });
    try {
      const readiness = await this.readiness(projectId, token);
      return { token, name: basename(path), readiness };
    } catch (error) {
      this.manifests.delete(token);
      throw error;
    }
  }

  async prepareOptimization(projectId: string): Promise<PreparedOptimizationChoice> {
    const workspace = await this.openRegistered(projectId);
    return this.exclusiveProject(projectId, async () => {
      const prepared = await this.command<PreparedOptimizationWire>(["prepare-optimization", workspace.folder]);
      return this.retainPrepared(projectId, workspace, prepared);
    });
  }

  async optimize(projectId: string, value: unknown, activityProgress?: (value: NativeProgress) => void): Promise<ManagedOptimizationResult> {
    if ((value as { action?: string })?.action === "resume") {
      const runId = uuid((value as { runId?: unknown }).runId, "run identity");
      return this.exclusiveProject(projectId, async () => {
        const at = new Date().toISOString();
        const activity: RunActivity = { running: true, phase: "checking_files", startedAt: at, updatedAt: at, events: [{ phase: "checking_files", at }] };
        this.runActivity.set(projectId, { runId, activity });
        try {
          const result = await this.optimizeCommand(projectId, value, progress => {
            recordProgress(activity, progress);
            activityProgress?.(progress);
          });
          activity.running = false;
          // The CLI emits its final status before releasing its execution lease.
          // Its successful exit is authoritative for this worker's completion.
          return { ...result, worker: { state: "idle" }, activity: structuredClone(activity) };
        }
        finally { activity.running = false; }
      });
    }
    const result = await this.optimizeCommand(projectId, value);
    const current = this.runActivity.get(projectId);
    if (current?.runId === (result as ManagedRunStatus).run_id) return { ...result, activity: structuredClone(current.activity) };
    return result;
  }

  private async optimizeCommand(projectId: string, value: unknown, progress?: (value: NativeProgress) => void): Promise<ManagedOptimizationResult> {
    const workspace = await this.openRegistered(projectId);
    if (!value || typeof value !== "object" || typeof (value as { action?: unknown }).action !== "string") throw new Error("Choose a supported optimization action.");
    const request = value as ManagedOptimizationRequest;
    let args: string[];
    if (request.action === "preview" || request.action === "start") {
      const selected = this.manifests.get(request.manifestToken);
      if (!selected || selected.projectId !== projectId) throw new Error("Choose the optimization definition for this project again.");
      args = [request.action, "--manifest", selected.path];
    } else {
      const runId = uuid((request as { runId?: unknown }).runId, "run identity");
      if (["status", "inspect", "review-repair", "review-delta", "resume", "doctor", "provenance", "report"].includes(request.action)) args = [request.action, runId];
      else if (request.action === "authorize-external" || request.action === "authorize-sealed") args = [request.action, runId, "--authorized-by", text(request.authorizedBy ?? "local-operator", "operator")];
      else if (request.action === "cancel") args = [request.action, runId, "--reason", text(request.reason, "cancellation reason", 240)];
      else throw new Error("Choose a supported optimization action.");
    }
    // Only commands that can enter native/provider execution receive secrets.
    // Preview, reservation, status, authorization recording, and other passive
    // commands neither resolve nor inherit desktop-managed credentials.
    const environment = request.action === "resume" || request.action === "authorize-sealed"
      ? this.providerEnvironment(projectId, workspace)
      : undefined;
    const run = () => this.command<ManagedOptimizationResult>(["optimize", workspace.folder, ...args], environment, progress);
    return ["start", "authorize-external", "authorize-sealed", "cancel"].includes(request.action)
      ? this.exclusiveProject(projectId, run)
      : run();
  }

  private providerEnvironment(projectId: string, workspace: ManagedWorkspace): CommandEnvironment {
    const resolveCredential = this.options.resolveCredential;
    if (!resolveCredential || !workspace.providerCatalog) return {};
    const environment: Record<string, string> = {};
    for (const provider of workspace.providerCatalog.providers) {
      if (provider.authentication !== "bearer") continue;
      const binding = providerCredentialBinding(projectId, provider);
      const secret = resolveCredential(binding.id, binding.connectionId ? undefined : binding.environment);
      if (!secret && binding.connectionId) throw new Error(`The pinned ${provider.role} connection is unavailable. Restore its key in Project settings before resuming.`);
      if (secret && binding.environment) environment[binding.environment] = secret;
    }
    return environment;
  }

  async promoteAccepted(projectId: string, value: unknown): Promise<ManagedWorkspace> {
    const request = object(value, "promotion request", ["runId", "expectedBaselineRevisionId", "actor", "reason"]) as unknown as ManagedPromotionRequest;
    const runId = uuid(request.runId, "run identity");
    const expected = uuid(request.expectedBaselineRevisionId, "baseline revision identity");
    const actor = text(request.actor ?? "local-operator", "promotion actor");
    const reason = text(request.reason ?? "Promote sealed-accepted optimization candidate", "promotion reason");
    const workspace = await this.openRegistered(projectId);
    return this.exclusiveProject(projectId, () => this.command<ManagedWorkspace>([
      "promote", workspace.folder, "--run-id", runId,
      "--expected-baseline-revision-id", expected, "--actor", actor, "--reason", reason,
    ]));
  }

  async restoreBaseline(projectId: string, value: unknown): Promise<ManagedWorkspace> {
    const request = object(value, "baseline restoration", ["targetRevisionId", "expectedBaselineRevisionId"]) as unknown as ManagedBaselineRestorationRequest;
    const targetRevisionId = uuid(request.targetRevisionId, "previous baseline revision");
    const expectedBaselineRevisionId = uuid(request.expectedBaselineRevisionId, "active baseline revision");
    const workspace = await this.openRegistered(projectId, true);
    const catalog = workspace.modelCatalog;
    if (!catalog || catalog.activeBaselineRevisionId !== expectedBaselineRevisionId) throw new Error("The active baseline changed. Reload Models and try again.");
    const target = catalog.baselineRevisions.find(revision => revision.id === targetRevisionId);
    const active = catalog.baselineRevisions.find(revision => revision.id === expectedBaselineRevisionId);
    if (!target || !active || target.modelArtifactId === active.modelArtifactId) throw new Error("Choose a previous baseline model that is not already active.");
    const revisionId = randomUUID();
    const result = await this.exclusiveProject(projectId, () => this.command<ManagedWorkspace>([
      "restore-baseline", workspace.folder, "--revision-id", revisionId,
      "--target-revision-id", targetRevisionId,
      "--expected-baseline-revision-id", expectedBaselineRevisionId,
    ]));
    const restored = result.modelCatalog;
    const revision = restored?.baselineRevisions.find(candidate => candidate.id === revisionId);
    if (result.manifest.id !== projectId || restored?.activeBaselineRevisionId !== revisionId || revision?.modelArtifactId !== target.modelArtifactId
      || revision.change.kind !== "restoration" || revision.change.target_revision_id !== targetRevisionId) {
      throw new Error("The baseline restoration returned mismatched project history.");
    }
    return result;
  }

  async providerStatus(projectId: string): Promise<ManagedProviderStatus> {
    const workspace = await this.openRegistered(projectId);
    const catalog = workspace.providerCatalog ?? null;
    return {
      projectId: workspace.manifest.id, configured: catalog !== null, catalog,
      credentialAvailability: catalog?.providers.map(provider => ({
        role: provider.role, authentication: provider.authentication,
        availability: provider.authentication === "none" ? "available" : "missing",
      })) ?? [],
      liveProbePerformed: false,
    };
  }

  async chooseNomosRuntime(projectId: string, path: string): Promise<NativePathChoice> {
    await this.openRegistered(projectId);
    this.runtimes.clear();
    const token = randomUUID();
    this.runtimes.set(token, { projectId, path });
    return { token, path };
  }

  async chooseNomosPython(projectId: string, path: string): Promise<NativePathChoice> {
    await this.openRegistered(projectId);
    this.pythons.clear();
    const token = randomUUID();
    this.pythons.set(token, { projectId, path });
    return { token, path };
  }

  async chooseNomosHistory(projectId: string, path: string): Promise<NativePathChoice> {
    await this.openRegistered(projectId);
    this.histories.clear();
    const token = randomUUID();
    this.histories.set(token, { projectId, path });
    return { token, path };
  }

  async previewNomosBinding(projectId: string, runtimeToken: unknown, pythonToken: unknown, historyToken?: unknown): Promise<NomosBindingPreview> {
    const runtime = typeof runtimeToken === "string" ? this.runtimes.get(runtimeToken) : undefined;
    const python = typeof pythonToken === "string" ? this.pythons.get(pythonToken) : undefined;
    if (!runtime || !python || runtime.projectId !== projectId || python.projectId !== projectId) throw new Error("Choose the isolated runtime and Python executable for this project again.");
    const history = historyToken === undefined ? undefined : typeof historyToken === "string" ? this.histories.get(historyToken) : undefined;
    if (historyToken !== undefined && (!history || history.projectId !== projectId)) throw new Error("Choose the existing scientific history for this project again.");
    const workspace = await this.openRegistered(projectId);
    const inspected = await this.command<Omit<NomosBindingPreview, "token">>(["preview-nomos-binding", workspace.folder, "--runtime", runtime.path, "--python", python.path, ...(history ? ["--history-database", history.path] : [])]);
    if (inspected.projectId !== projectId) throw new Error("The runtime preview belongs to a different project.");
    this.bindingPreviews.clear();
    const token = randomUUID();
    this.bindingPreviews.set(token, { projectId, runtime: runtime.path, python: python.path, ...(history ? { history: history.path } : {}), ready: inspected.ready === true });
    return { ...inspected, token };
  }

  async bindNomos(projectId: string, previewToken: unknown): Promise<ManagedWorkspace> {
    const selected = typeof previewToken === "string" ? this.bindingPreviews.get(previewToken) : undefined;
    if (!selected || selected.projectId !== projectId) throw new Error("Preview the scientific runtime for this project again.");
    if (!selected.ready) throw new Error("Resolve every missing runtime capability and preview the setup again before connecting.");
    const workspace = await this.openRegistered(projectId);
    return this.exclusiveProject(projectId, async () => {
      const bound = await this.command<ManagedWorkspace>(["bind-nomos", workspace.folder, "--runtime", selected.runtime, "--python", selected.python, ...(selected.history ? ["--history-database", selected.history] : []), "--actor", "local-operator", "--reason", selected.history ? "Import verified scientific history and bind desktop runtime" : "Bind verified desktop scientific runtime"]);
      this.bindingPreviews.delete(previewToken as string);
      return bound;
    });
  }

  async prepareNomosPython(projectId: string, previewToken: unknown): Promise<void> {
    const selected = typeof previewToken === "string" ? this.bindingPreviews.get(previewToken) : undefined;
    if (!selected || selected.projectId !== projectId) throw new Error("Preview the scientific runtime for this project again.");
    if (selected.ready) throw new Error("The selected Python environment already provides every required capability.");
    const workspace = await this.openRegistered(projectId);
    await this.exclusiveProject(projectId, () => this.command(["prepare-nomos-python", workspace.folder, "--runtime", selected.runtime, "--python", selected.python, "--allow-network-install"]));
    this.bindingPreviews.delete(previewToken as string);
  }

  async configureProviders(projectId: string, value: unknown): Promise<ManagedProviderStatus> {
    const settings = providerSettings(value);
    const workspace = await this.openRegistered(projectId);
    return this.exclusiveProject(projectId, async () => {
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-providers-"));
      const file = join(directory, "settings.json");
      try {
        await writeFile(file, JSON.stringify(providerFile(settings)), { flag: "wx", mode: 0o600 });
        return await this.command<ManagedProviderStatus>(["providers", workspace.folder, "configure", "--file", file,
          ...(workspace.providerCatalog?.id ? ["--expected-revision-id", workspace.providerCatalog.id] : []), "--actor", settings.actor ?? "local-operator", "--reason", settings.reason ?? "Configure project providers"]);
      } finally {
        try { await unlink(file); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
        try { await rmdir(directory); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
      }
    });
  }
}

export function managedSnapshot(managed: ManagedWorkspace): WorkspaceSnapshot {
  const { manifest } = managed, model = manifest.baseline;
  const catalog = managed.modelCatalog;
  const activeRevision = catalog?.baselineRevisions.find(revision => revision.id === catalog.activeBaselineRevisionId);
  const active = catalog?.artifacts.find(artifact => artifact.id === activeRevision?.modelArtifactId);
  const root = resolve(managed.folder);
  let scientific: WorkspaceSnapshot | undefined;
  if (managed.scientificBinding?.store?.databasePath) {
    const database = resolve(root, managed.scientificBinding.store.databasePath);
    if (database === root || !database.startsWith(root + sep)) throw new Error("The bound scientific store escaped its managed project workspace.");
    scientific = readWorkspaceDatabase(database);
  }
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), source: "local", folder: managed.folder,
    name: manifest.name, task: manifest.task ?? "Task not configured",
    baseline: active
      ? { id: active.id, key: active.name, format: active.format, bytes: active.bytes, fingerprint: active.fingerprint }
      : { id: model.fingerprint, key: `${manifest.name} baseline`, format: model.format, bytes: model.bytes, fingerprint: model.fingerprint },
    baselineEvaluations: scientific?.runs.find(run =>
      run.projectId === managed.scientificBinding?.runtime.projectSnapshot.id &&
      managed.scientificBinding?.baselineRevisionId === catalog?.activeBaselineRevisionId
    )?.baselines ?? [],
    deployment: scientific?.deployment,
    runs: scientific?.runs ?? [], databases: ["project.sqlite", ...(scientific?.databases ?? [])], managed,
  };
}
