import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, realpathSync, renameSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import type { ProjectCollection, ProjectEntry } from "./projects.js";
import type { ManagedWorkspace } from "./managed-workspace.js";

function name(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value.trim().length > 120 || /[\u0000-\u001f]/.test(value)) throw new Error("Use a project name between 1 and 120 characters.");
  return value.trim();
}
function folder(value: string): string {
  if (!statSync(value).isDirectory()) throw new Error("Choose a project folder, not a file.");
  return realpathSync.native(resolve(value));
}
function pathKey(value: string): string { return process.platform === "win32" ? value.toLowerCase() : value; }
export function validateCollection(value: unknown): ProjectCollection {
  const c = value as ProjectCollection;
  if (!c || c.version !== 1 || !Array.isArray(c.projects) || c.projects.length > 10000) throw new Error("The project collection has an unsupported format. Its file has not been changed.");
  const ids = new Set<string>();
  const sources = new Set<string>();
  for (const p of c.projects) {
    if (!p || typeof p.id !== "string" || !p.id || ids.has(p.id) || name(p.name) !== p.name || typeof p.createdAt !== "string" || !Number.isFinite(Date.parse(p.createdAt))) throw new Error("The project collection contains an invalid project. Its file has not been changed.");
    ids.add(p.id);
    const source = p.source;
    if (source?.kind === "folder" && source.workspaceId !== undefined && (source.workspaceId !== p.id || !/^[a-f0-9-]{36}$/.test(source.workspaceId))) throw new Error("The managed project identity is invalid. Its collection has not been changed.");
    const key = source?.kind === "folder" && typeof source.path === "string" && source.path && resolve(source.path) === source.path ? "folder:" + pathKey(source.path) : source?.kind === "example" && typeof source.key === "string" && source.key ? "example:" + source.key : undefined;
    if (!key || sources.has(key)) throw new Error("The project collection contains an invalid or duplicate source. Its file has not been changed.");
    sources.add(key);
  }
  if (c.selectedId !== null && !ids.has(c.selectedId)) throw new Error("The selected project is missing from the collection. Its file has not been changed.");
  return c;
}

/** Only this app-owned metadata file is mutable. No experiment DB or model is written. */
export class ProjectRegistry {
  constructor(readonly file: string) {}
  read(): ProjectCollection {
    let content: string;
    try { content = readFileSync(this.file, "utf8"); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return { version: 1, selectedId: null, projects: [] }; throw error; }
    try { return validateCollection(JSON.parse(content)); }
    catch (error) { throw new Error(`Cannot read project collection at ${this.file}: ${error instanceof Error ? error.message : String(error)}`); }
  }
  private write(collection: ProjectCollection): ProjectCollection {
    validateCollection(collection);
    mkdirSync(dirname(this.file), { recursive: true });
    const temporary = this.file + "." + randomUUID() + ".tmp";
    try {
      writeFileSync(temporary, JSON.stringify(collection, null, 2) + "\n", { flag: "wx", mode: 0o600 });
      renameSync(temporary, this.file);
    } finally {
      try { unlinkSync(temporary); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    }
    return collection;
  }
  get(id: string): ProjectEntry {
    const project = this.read().projects.find(p => p.id === id);
    if (!project) throw new Error("This project is no longer in the collection.");
    return project;
  }
  private register(source: ProjectEntry["source"], displayName: string): ProjectCollection {
    const collection = this.read();
    const existing = collection.projects.find(p => p.source.kind === source.kind && (source.kind === "folder" ? p.source.kind === "folder" && pathKey(p.source.path) === pathKey(source.path) : p.source.kind === "example" && p.source.key === source.key));
    const project = existing ?? { id: randomUUID(), name: name(displayName), createdAt: new Date().toISOString(), source };
    if (!existing) collection.projects.push(project);
    collection.selectedId = project.id;
    return this.write(collection);
  }
  addFolder(path: string, displayName?: string): ProjectCollection {
    const canonical = folder(path);
    return this.register({ kind: "folder", path: canonical }, displayName === undefined ? basename(canonical) || canonical : name(displayName));
  }
  addExample(key: string, displayName: string): ProjectCollection { return this.register({ kind: "example", key }, displayName); }
  /** Called only after the backend has validated the manifest/database binding. */
  addManaged(workspace: ManagedWorkspace, replaceLegacyId?: string): ProjectCollection {
    const canonical = folder(workspace.folder), { manifest } = workspace;
    const collection = this.read();
    const existing = collection.projects.find(p => p.id === manifest.id);
    if (existing && (existing.source.kind !== "folder" || existing.source.workspaceId !== manifest.id)) throw new Error("Project identity conflicts with a different library entry.");
    if (collection.projects.some(p => p.id !== manifest.id && p.id !== replaceLegacyId && p.source.kind === "folder" && pathKey(p.source.path) === pathKey(canonical))) throw new Error("This folder already belongs to another library entry.");
    if (replaceLegacyId) {
      const legacy = collection.projects.find(p => p.id === replaceLegacyId);
      if (!legacy || legacy.source.kind !== "folder" || legacy.source.workspaceId) throw new Error("Only an explicitly selected legacy connection can be replaced.");
      collection.projects = collection.projects.filter(p => p.id !== replaceLegacyId);
    }
    if (existing) existing.source = { kind: "folder", path: canonical, workspaceId: manifest.id };
    else collection.projects.push({ id: manifest.id, name: name(manifest.name), createdAt: manifest.createdAt, source: { kind: "folder", path: canonical, workspaceId: manifest.id } });
    collection.selectedId = manifest.id;
    return this.write(collection);
  }
  select(id: string): ProjectCollection {
    const collection = this.read();
    if (!collection.projects.some(p => p.id === id)) throw new Error("This project is no longer in the collection.");
    collection.selectedId = id;
    return this.write(collection);
  }
  rename(id: string, displayName: string): ProjectCollection {
    const collection = this.read();
    const project = collection.projects.find(p => p.id === id);
    if (!project) throw new Error("This project is no longer in the collection.");
    project.name = name(displayName);
    return this.write(collection);
  }
  relocate(id: string, path: string): ProjectCollection {
    const canonical = folder(path);
    const collection = this.read();
    const project = collection.projects.find(p => p.id === id);
    if (!project || project.source.kind !== "folder") throw new Error("Only a local folder project can be reconnected.");
    if (project.source.workspaceId) throw new Error("Managed projects must be reconnected using their verified workspace identity.");
    if (collection.projects.some(p => p.id !== id && p.source.kind === "folder" && pathKey(p.source.path) === pathKey(canonical))) throw new Error("That folder already belongs to another project in the collection.");
    project.source = { kind: "folder", path: canonical };
    return this.write(collection);
  }
  remove(id: string): ProjectCollection {
    const collection = this.read();
    if (!collection.projects.some(p => p.id === id)) throw new Error("This project is no longer in the collection.");
    collection.projects = collection.projects.filter(p => p.id !== id);
    if (collection.selectedId === id) collection.selectedId = collection.projects[0]?.id ?? null;
    return this.write(collection);
  }
}
