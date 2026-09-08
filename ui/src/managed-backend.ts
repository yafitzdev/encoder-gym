import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { basename, join } from "node:path";
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
/** Main-process-only bridge. Executable and argument grammar are app-owned. */
export class ManagedBackend {
  private models = new Map<string, LocalModel>();
  private parents = new Map<string, string>();
  private datasets = new Map<string, DatasetChoice & { projectId: string }>();
  private busy = false;
  constructor(readonly executable: string, private registry: ProjectRegistry) {}
  command<T>(args: string[]): Promise<T> {
    return new Promise((resolve, reject) => {
      execFile(this.executable, ["--output", "json", "workspace", ...args], { windowsHide: true, shell: false, maxBuffer: 16 * 1024 * 1024 }, (error, stdout, stderr) => {
        if (error) { reject(new Error((error as NodeJS.ErrnoException).code === "ENOENT" ? "The local workspace backend is missing. Run npm run build:backend in ui, then retry." : stderr.trim() || error.message)); return; }
        try { resolve(JSON.parse(stdout) as T); } catch { reject(new Error("The workspace backend returned an unreadable response.")); }
      });
    });
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
}

export function managedSnapshot(managed: ManagedWorkspace): WorkspaceSnapshot {
  const { manifest } = managed, model = manifest.baseline;
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), source: "local", folder: managed.folder,
    name: manifest.name, task: manifest.task ?? "Task not configured",
    baseline: { id: model.fingerprint, key: `${manifest.name} baseline`, format: model.format, bytes: model.bytes, fingerprint: model.fingerprint },
    runs: [], databases: ["project.sqlite"], managed,
  };
}
