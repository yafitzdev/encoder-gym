import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export type AssignableProviderRole = "advisor" | "generation";
export interface ProviderModelAssignment { connectionId: string; model: string }
export interface ProviderConnection {
  id: string;
  projectId: string;
  endpoint: string;
  models: string[];
  checkedAt: string;
}
export interface ProviderConnectionStatus extends ProviderConnection { availability: "available" | "missing" | "unavailable" }
export interface ProjectProviderConnections {
  projectId: string;
  connections: ProviderConnectionStatus[];
  assignments: Partial<Record<AssignableProviderRole, ProviderModelAssignment>>;
}
export interface ProviderAssignmentRequest {
  advisor: ProviderModelAssignment;
  generation: ProviderModelAssignment;
}
interface StoredProject {
  connections: ProviderConnection[];
  assignments: Partial<Record<AssignableProviderRole, ProviderModelAssignment>>;
}
interface ConnectionFile { version: 1; projects: Record<string, StoredProject> }

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const roles: AssignableProviderRole[] = ["advisor", "generation"];
function uuid(value: unknown, label: string): string {
  if (typeof value !== "string" || !uuidPattern.test(value)) throw new Error(`Invalid ${label}.`);
  return value.toLowerCase();
}
export function providerEndpoint(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value.length > 2048) throw new Error("Enter a valid provider URL.");
  let parsed: URL;
  try { parsed = new URL(value.trim()); } catch { throw new Error("Enter a valid provider URL."); }
  if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash) throw new Error("Enter an HTTP(S) provider URL without credentials or query parameters.");
  parsed.pathname = parsed.pathname.replace(/\/+$/, "");
  return parsed.toString().replace(/\/$/, "");
}
function model(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value.trim().length > 200 || /[\u0000-\u001f]/.test(value)) throw new Error("The provider returned an invalid model identity.");
  return value.trim();
}
function modelList(value: unknown): string[] {
  if (!Array.isArray(value) || !value.length || value.length > 2000) throw new Error("The provider did not return a usable model list.");
  return [...new Set(value.map(model))].sort((a, b) => a.localeCompare(b));
}
function assignment(value: unknown, project: StoredProject, label: string): ProviderModelAssignment {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Choose a model for ${label}.`);
  const item = value as ProviderModelAssignment;
  const connectionId = uuid(item.connectionId, "provider connection identity"), selectedModel = model(item.model);
  const connection = project.connections.find(candidate => candidate.id === connectionId);
  if (!connection || !connection.models.includes(selectedModel)) throw new Error(`Choose a discovered model for ${label}.`);
  return { connectionId, model: selectedModel };
}
function emptyFile(): ConnectionFile { return { version: 1, projects: {} }; }
function validateFile(value: unknown): ConnectionFile {
  const file = value as ConnectionFile;
  if (!file || file.version !== 1 || !file.projects || typeof file.projects !== "object" || Array.isArray(file.projects)) throw new Error("unsupported format");
  for (const [projectId, project] of Object.entries(file.projects)) {
    uuid(projectId, "project identity");
    if (!project || !Array.isArray(project.connections) || project.connections.length > 50 || !project.assignments || typeof project.assignments !== "object" || Array.isArray(project.assignments)) throw new Error("invalid project connections");
    const ids = new Set<string>();
    for (const connection of project.connections) {
      if (uuid(connection.id, "provider connection identity") !== connection.id || connection.projectId !== projectId || ids.has(connection.id)) throw new Error("invalid provider connection");
      ids.add(connection.id); providerEndpoint(connection.endpoint); modelList(connection.models);
      if (typeof connection.checkedAt !== "string" || !Number.isFinite(Date.parse(connection.checkedAt))) throw new Error("invalid provider check time");
    }
    for (const role of roles) if (project.assignments[role]) assignment(project.assignments[role], project, role);
    if (Object.keys(project.assignments).some(key => !roles.includes(key as AssignableProviderRole))) throw new Error("invalid provider assignment");
  }
  return file;
}

export function providerConnectionSecretId(projectId: string, connectionId: string): string {
  return `${uuid(projectId, "project identity")}:connection:${uuid(connectionId, "provider connection identity")}`;
}

export async function discoverProviderModels(endpointValue: unknown, secret: unknown, request: typeof fetch = fetch): Promise<{ endpoint: string; models: string[] }> {
  const endpoint = providerEndpoint(endpointValue);
  if (typeof secret !== "string" || secret.length < 8 || secret.length > 8192 || /[\u0000-\u001f]/.test(secret)) throw new Error("Enter a valid API key.");
  const url = new URL(endpoint);
  if (!url.pathname.endsWith("/models")) url.pathname = url.pathname.replace(/\/$/, "") + "/models";
  let response: Response;
  try {
    response = await request(url, { method: "GET", headers: { accept: "application/json", authorization: `Bearer ${secret}` }, signal: AbortSignal.timeout(15_000) });
  } catch { throw new Error("Could not reach the provider. Check the URL and try again."); }
  if (!response.ok) throw new Error(`Model discovery failed (HTTP ${response.status}). Check the URL and API key.`);
  const bytes = await response.arrayBuffer();
  if (bytes.byteLength > 1024 * 1024) throw new Error("The provider model list is too large.");
  let body: unknown;
  try { body = JSON.parse(new TextDecoder().decode(bytes)); } catch { throw new Error("The provider returned an invalid model list."); }
  const data = (body as { data?: unknown })?.data;
  if (!Array.isArray(data)) throw new Error("The provider did not return an OpenAI-compatible model list.");
  return { endpoint, models: modelList(data.map(item => (item as { id?: unknown })?.id)) };
}

/** App-local connection metadata. API keys live only in CredentialStore. */
export class ProviderConnectionStore {
  constructor(readonly file: string) {}
  private read(): ConnectionFile {
    let source: string;
    try { source = readFileSync(this.file, "utf8"); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return emptyFile(); throw error; }
    try { return validateFile(JSON.parse(source)); }
    catch { throw new Error("The provider connection index is unreadable. It has not been changed."); }
  }
  private write(value: ConnectionFile): void {
    validateFile(value); mkdirSync(dirname(this.file), { recursive: true });
    const temporary = this.file + "." + randomUUID() + ".tmp";
    try { writeFileSync(temporary, JSON.stringify(value, null, 2) + "\n", { flag: "wx", mode: 0o600 }); renameSync(temporary, this.file); }
    finally { try { unlinkSync(temporary); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; } }
  }
  get(projectIdValue: unknown): { projectId: string; connections: ProviderConnection[]; assignments: StoredProject["assignments"] } {
    const projectId = uuid(projectIdValue, "project identity"), project = this.read().projects[projectId] ?? { connections: [], assignments: {} };
    return { projectId, connections: structuredClone(project.connections), assignments: structuredClone(project.assignments) };
  }
  add(projectIdValue: unknown, endpointValue: unknown, modelsValue: unknown): ProviderConnection {
    const projectId = uuid(projectIdValue, "project identity"), file = this.read();
    const project = file.projects[projectId] ??= { connections: [], assignments: {} };
    if (project.connections.length >= 50) throw new Error("This project already has 50 provider connections.");
    const endpoint = providerEndpoint(endpointValue), models = modelList(modelsValue);
    const connection = { id: randomUUID(), projectId, endpoint, models, checkedAt: new Date().toISOString() };
    project.connections.push(connection); this.write(file); return structuredClone(connection);
  }
  update(projectIdValue: unknown, connectionIdValue: unknown, modelsValue: unknown): ProviderConnection {
    const projectId = uuid(projectIdValue, "project identity"), connectionId = uuid(connectionIdValue, "provider connection identity"), file = this.read();
    const connection = file.projects[projectId]?.connections.find(item => item.id === connectionId);
    if (!connection) throw new Error("Provider connection not found.");
    connection.models = modelList(modelsValue); connection.checkedAt = new Date().toISOString();
    const project = file.projects[projectId]!;
    for (const role of roles) if (project.assignments[role]?.connectionId === connectionId && !connection.models.includes(project.assignments[role]!.model)) delete project.assignments[role];
    this.write(file); return structuredClone(connection);
  }
  remove(projectIdValue: unknown, connectionIdValue: unknown): void {
    const projectId = uuid(projectIdValue, "project identity"), connectionId = uuid(connectionIdValue, "provider connection identity"), file = this.read();
    const project = file.projects[projectId];
    if (!project?.connections.some(item => item.id === connectionId)) throw new Error("Provider connection not found.");
    if (roles.some(role => project.assignments[role]?.connectionId === connectionId)) throw new Error("Assign another model before removing this connection.");
    project.connections = project.connections.filter(item => item.id !== connectionId); this.write(file);
  }
  assign(projectIdValue: unknown, value: unknown): ProviderAssignmentRequest {
    const projectId = uuid(projectIdValue, "project identity"), file = this.read(), project = file.projects[projectId];
    if (!project) throw new Error("Add a provider connection first.");
    const request = value as ProviderAssignmentRequest;
    const assignments = { advisor: assignment(request?.advisor, project, "Agent"), generation: assignment(request?.generation, project, "Data generation") };
    project.assignments = assignments; this.write(file); return structuredClone(assignments);
  }
  replaceAssignments(projectIdValue: unknown, value: StoredProject["assignments"]): void {
    const projectId = uuid(projectIdValue, "project identity"), file = this.read(), project = file.projects[projectId];
    if (!project) return;
    const assignments: StoredProject["assignments"] = {};
    for (const role of roles) if (value[role]) assignments[role] = assignment(value[role], project, role);
    project.assignments = assignments; this.write(file);
  }
}
