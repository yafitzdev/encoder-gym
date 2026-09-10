import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ManagedWorkspace } from "./managed-workspace.js";
import type { DatasetChangePage, DatasetEntry, DatasetMutation, DatasetMutationResult, DatasetQueryResult, DatasetRowPage, RecordSelection } from "./dataset-workspace.js";

interface Ports {
  open(projectId: string): Promise<ManagedWorkspace>;
  command<T>(args: string[]): Promise<T>;
  exclusive<T>(projectId: string, run: () => Promise<T>): Promise<T>;
}
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key))) throw new Error("Invalid dataset request.");
  return value as Record<string, unknown>;
}
function id(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error("Invalid dataset identity.");
  return value.toLowerCase();
}
function integer(value: unknown, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) throw new Error("Invalid dataset page or record number.");
  return value;
}
function rowId(value: unknown): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) throw new Error("Invalid row identity.");
  return value;
}
function name(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || [...value.trim()].length > 120 || /[\u0000-\u001f\u007f]/.test(value)) throw new Error("Enter a dataset name.");
  return value.trim();
}
function array<T>(value: unknown, convert: (value: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > 100_000) throw new Error("Invalid dataset selection.");
  return value.map(convert);
}
function selection(value: unknown): RecordSelection {
  const input = record(value, ["importId", "record"]);
  return { importId: id(input.importId), record: integer(input.record, 1, Number.MAX_SAFE_INTEGER) };
}
function mutation(value: unknown): DatasetMutation {
  const input = record(value, ["kind", "datasetId", "versionId", "name", "imports", "parentId", "added", "removed", "replaced"]);
  const common = { datasetId: id(input.datasetId), versionId: id(input.versionId) };
  if (input.kind === "create") {
    record(value, ["kind", "datasetId", "versionId", "name", "imports"]);
    const imports = array(input.imports, id);
    if (!imports.length) throw new Error("Choose training data.");
    return { kind: "create", ...common, name: name(input.name), imports };
  }
  if (input.kind === "fork") {
    record(value, ["kind", "datasetId", "versionId", "name", "parentId"]);
    return { kind: "fork", ...common, name: name(input.name), parentId: id(input.parentId) };
  }
  if (input.kind !== "revise") throw new Error("Unknown dataset operation.");
  record(value, ["kind", "datasetId", "versionId", "parentId", "added", "removed", "replaced"]);
  return { kind: "revise", ...common, parentId: id(input.parentId), added: array(input.added, selection), removed: array(input.removed, rowId),
    replaced: array(input.replaced, value => { const input = record(value, ["id", "source"]); return { id: rowId(input.id), source: selection(input.source) }; }) };
}

/** Fixed dataset CLI grammar. Renderer input never supplies paths or commands. */
export class ManagedDatasets {
  constructor(private ports: Ports) {}
  async query(projectId: string, value: unknown): Promise<DatasetQueryResult> {
    const input = record(value, ["kind", "versionId", "offset", "limit"]);
    if (input.kind !== "list" && input.kind !== "rows" && input.kind !== "changes") throw new Error("Unknown dataset inspection.");
    const workspace = await this.ports.open(projectId);
    const args = ["dataset", workspace.folder];
    if (input.kind === "list") {
      record(value, ["kind"]);
      const entries = await this.ports.command<DatasetEntry[]>([...args, "list"]);
      if (!Array.isArray(entries) || entries.some(entry => entry.dataset.projectId !== workspace.manifest.id || entry.versions.some(version => version.version.projectId !== workspace.manifest.id || version.version.datasetId !== entry.dataset.id))) throw new Error("Dataset collection belongs to another project.");
      return { kind: "list", entries };
    }
    const versionId = id(input.versionId), offset = integer(input.offset, 0, Number.MAX_SAFE_INTEGER), limit = integer(input.limit, 1, input.kind === "rows" ? 100 : 50);
    const page = await this.ports.command<DatasetRowPage | DatasetChangePage>([...args, input.kind, versionId, "--offset", String(offset), "--limit", String(limit)]);
    if (page.versionId !== versionId || page.offset !== offset) throw new Error("Dataset inspection returned another version.");
    return input.kind === "rows" ? { kind: "rows", page: page as DatasetRowPage } : { kind: "changes", page: page as DatasetChangePage };
  }
  async mutate(projectId: string, value: unknown): Promise<DatasetMutationResult> {
    const request = mutation(value);
    const workspace = await this.ports.open(projectId);
    return this.ports.exclusive(projectId, async () => {
      const args = ["dataset", workspace.folder];
      let result: DatasetMutationResult;
      if (request.kind === "create") result = await this.ports.command([...args, "create", "--name", request.name, "--dataset-id", request.datasetId, "--version-id", request.versionId, ...request.imports.flatMap(id => ["--source", id])]);
      else if (request.kind === "fork") result = await this.ports.command([...args, "fork", request.parentId, "--name", request.name, "--dataset-id", request.datasetId, "--version-id", request.versionId]);
      else {
        const directory = await mkdtemp(join(tmpdir(), "encoder-gym-dataset-change-"));
        const file = join(directory, "changes.json");
        try {
          const { kind: _kind, ...changes } = request;
          await writeFile(file, JSON.stringify(changes), { flag: "wx", mode: 0o600 });
          result = await this.ports.command([...args, "revise", "--file", file]);
        } finally {
          await unlink(file).catch(error => { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; });
          await rmdir(directory);
        }
      }
      if (result.version.projectId !== workspace.manifest.id || result.version.id !== request.versionId || result.version.datasetId !== request.datasetId) throw new Error("Dataset save returned a mismatched version.");
      return result;
    });
  }
}
