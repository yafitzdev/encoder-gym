import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { basename, join } from "node:path";
import type { ManagedOptimizationRequest, ManagedOptimizationResult, ManagedReadiness, OptimizationManifestChoice } from "./managed-control.js";
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
