import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import type { ManagedOptimizationRequest, ManagedOptimizationResult, ManagedProviderStatus, ManagedReadiness, NativePathChoice, NomosBindingPreview, OptimizationManifestChoice, ProviderInput, ProviderSettingsRequest } from "./managed-control.js";
import type { CreateProjectRequest, DatasetChoice, DatasetPurpose, FolderChoice, LocalModel, ManagedWorkspace, ModelChoice } from "./managed-workspace.js";
import type { ProjectRegistry } from "./project-registry.js";
import type { WorkspaceSnapshot } from "./workspace.js";

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
  const provider = object(value, label, ["kind", "endpoint", "model", "authentication", "environmentFallback", "limits"]);
  if (!(["fake", "openai-compatible"] as unknown[]).includes(provider.kind) || !(["none", "bearer"] as unknown[]).includes(provider.authentication)) throw new Error(`Invalid ${label}.`);
  const endpoint = provider.endpoint === undefined ? undefined : text(provider.endpoint, `${label} endpoint`, 2048);
  const environmentFallback = provider.environmentFallback === undefined ? undefined : text(provider.environmentFallback, `${label} environment fallback`, 128);
  const expectedFallback = role === "generation" ? "SYNTH_OPENAI_API_KEY" : role === "advisor" ? "SYNTH_ADVISOR_API_KEY" : "SYNTH_EVALUATOR_API_KEY";
  if (provider.kind === "fake" && (endpoint !== undefined || provider.authentication !== "none" || environmentFallback !== undefined)) throw new Error(`Invalid ${label}.`);
  if (provider.kind === "openai-compatible" && (!endpoint || provider.authentication !== "bearer" || environmentFallback !== expectedFallback)) throw new Error(`Invalid ${label}.`);
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
    limits: parsedLimits,
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
  });
  return { version: 1, generation: encode(settings.generation), advisor: encode(settings.advisor), ...(settings.evaluator ? { evaluator: encode(settings.evaluator) } : {}) };
}
export function redactBackendError(value: string): string {
  return value
    .replace(/\bBearer\s+[^\s"']+/gi, "Bearer [redacted]")
    .replace(/\b(sk|key|token)-[A-Za-z0-9._-]{8,}\b/g, "[redacted]")
    .trim();
}

export type CommandExecutor = (executable: string, args: string[]) => Promise<string>;
const executeCommand: CommandExecutor = (executable, args) => new Promise((resolve, reject) => {
  execFile(executable, args, { windowsHide: true, shell: false, maxBuffer: 16 * 1024 * 1024 }, (error, stdout, stderr) => {
    if (error) {
      const missing = (error as NodeJS.ErrnoException).code === "ENOENT";
      reject(new Error(missing ? "The local workspace backend is missing. Run npm run build:backend in ui, then retry." : redactBackendError(stderr) || redactBackendError(error.message)));
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
  private busy = false;
  constructor(readonly executable: string, private registry: ProjectRegistry, private executor: CommandExecutor = executeCommand) {}
  private async command<T>(args: string[]): Promise<T> {
    const stdout = await this.executor(this.executable, ["--output", "json", "workspace", ...args]);
    try { return JSON.parse(stdout) as T; } catch { throw new Error("The workspace backend returned an unreadable response."); }
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
    return this.command<ManagedReadiness>(args);
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

  async optimize(projectId: string, value: unknown): Promise<ManagedOptimizationResult> {
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
    const run = () => this.command<ManagedOptimizationResult>(["optimize", workspace.folder, ...args]);
    return ["start", "resume", "authorize-external", "authorize-sealed", "cancel"].includes(request.action)
      ? this.exclusiveProject(projectId, run)
      : run();
  }

  async providerStatus(projectId: string): Promise<ManagedProviderStatus> {
    const workspace = await this.openRegistered(projectId);
    return this.command<ManagedProviderStatus>(["providers", workspace.folder, "show"]);
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
      const current = await this.command<ManagedProviderStatus>(["providers", workspace.folder, "show"]);
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-providers-"));
      const file = join(directory, "settings.json");
      try {
        await writeFile(file, JSON.stringify(providerFile(settings)), { flag: "wx", mode: 0o600 });
        return await this.command<ManagedProviderStatus>(["providers", workspace.folder, "configure", "--file", file,
          ...(current.catalog?.id ? ["--expected-revision-id", current.catalog.id] : []), "--actor", settings.actor ?? "local-operator", "--reason", settings.reason ?? "Configure project providers"]);
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
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), source: "local", folder: managed.folder,
    name: manifest.name, task: manifest.task ?? "Task not configured",
    baseline: active
      ? { id: active.id, key: active.name, format: active.format, bytes: active.bytes, fingerprint: active.fingerprint }
      : { id: model.fingerprint, key: `${manifest.name} baseline`, format: model.format, bytes: model.bytes, fingerprint: model.fingerprint },
    runs: [], databases: ["project.sqlite"], managed,
  };
}
