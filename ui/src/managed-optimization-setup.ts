import { mkdtemp, rmdir, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ManagedWorkspace } from "./managed-workspace.js";
import type { NativeProgress } from "./managed-control.js";
import type { OptimizationInputs, OptimizationSetup, OptimizationSetupPreview, OptimizationSetupRequest, OptimizationSetupSaved } from "./optimization-setup.js";

interface Ports {
  open(projectId: string): Promise<ManagedWorkspace>;
  command<T>(args: string[], progress?: (value: NativeProgress) => void): Promise<T>;
  exclusive<T>(projectId: string, run: () => Promise<T>): Promise<T>;
}
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key))) throw new Error("Invalid optimization inputs.");
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
function bound(value: unknown): { id: string; fingerprint: string } {
  const input = record(value, ["id", "fingerprint"]);
  return { id: uuid(input.id), fingerprint: fingerprint(input.fingerprint) };
}
function inputs(value: unknown, projectId: string): OptimizationInputs {
  const input = record(value, ["projectId", "baselineRevision", "model", "dataset", "benchmark"]);
  const dataset = record(input.dataset, ["id", "datasetId", "projectId", "number", "fingerprint"]);
  if (uuid(input.projectId) !== projectId || uuid(dataset.projectId) !== projectId) throw new Error("Optimization inputs belong to another project.");
  if (typeof dataset.number !== "number" || !Number.isSafeInteger(dataset.number) || dataset.number < 1) throw new Error("Invalid dataset version.");
  return { projectId, baselineRevision: bound(input.baselineRevision), model: bound(input.model), benchmark: bound(input.benchmark),
    dataset: { ...bound({ id: dataset.id, fingerprint: dataset.fingerprint }), datasetId: uuid(dataset.datasetId), projectId, number: dataset.number } };
}

/** Selection only. This bridge cannot prepare a recipe, start work or grant spending. */
export class ManagedOptimizationSetup {
  constructor(private ports: Ports) {}
  async list(projectId: string): Promise<OptimizationSetup[]> {
    const workspace = await this.ports.open(projectId);
    const history = await this.ports.command<OptimizationSetup[]>(["optimization-setup", workspace.folder, "list"]);
    if (!Array.isArray(history)) throw new Error("Invalid optimization setup history.");
    for (const setup of history) inputs(setup.inputs, workspace.manifest.id);
    return history;
  }
  async preview(projectId: string, value: unknown, progress?: (value: NativeProgress) => void): Promise<OptimizationSetupPreview> {
    const selection = record(value, ["modelId", "datasetVersionId", "benchmarkVersionId"]);
    const modelId = uuid(selection.modelId), datasetId = uuid(selection.datasetVersionId), benchmarkId = uuid(selection.benchmarkVersionId);
    const workspace = await this.ports.open(projectId);
    const preview = await this.ports.command<OptimizationSetupPreview>(["optimization-setup", workspace.folder, "preview", "--model", modelId, "--dataset-version", datasetId, "--benchmark-version", benchmarkId], progress);
    const resolved = inputs(preview.inputs, workspace.manifest.id);
    const catalog = workspace.modelCatalog;
    const baseline = catalog?.baselineRevisions.find(revision => revision.id === catalog.activeBaselineRevisionId);
    const model = catalog?.artifacts.find(model => model.id === modelId);
    const benchmark = workspace.benchmarkVersions?.find(version => version.id === benchmarkId);
    if (resolved.model.id !== modelId || resolved.dataset.id !== datasetId || resolved.benchmark.id !== benchmarkId
      || baseline?.modelArtifactId !== modelId || resolved.baselineRevision.id !== baseline.id || resolved.baselineRevision.fingerprint !== baseline.fingerprint
      || resolved.model.fingerprint !== model?.fingerprint || resolved.benchmark.fingerprint !== benchmark?.fingerprint) throw new Error("Optimization inputs changed. Refresh the project.");
    return { ...preview, inputs: resolved };
  }
  async save(projectId: string, value: unknown, progress?: (value: NativeProgress) => void): Promise<OptimizationSetupSaved> {
    const input = record(value, ["id", "expectedParent", "inputs"]);
    const request: OptimizationSetupRequest = { id: uuid(input.id), expectedParent: input.expectedParent === null ? null : uuid(input.expectedParent), inputs: inputs(input.inputs, projectId) };
    return this.ports.exclusive(projectId, async () => {
      const workspace = await this.ports.open(projectId);
      if (workspace.manifest.id !== request.inputs.projectId) throw new Error("Optimization inputs belong to another project.");
      const directory = await mkdtemp(join(tmpdir(), "encoder-gym-optimization-setup-")), file = join(directory, "inputs.json");
      try {
        await writeFile(file, JSON.stringify(request), { flag: "wx", mode: 0o600 });
        const saved = await this.ports.command<OptimizationSetupSaved>(["optimization-setup", workspace.folder, "save", "--file", file], progress);
        const resolved = inputs(saved.setup.inputs, projectId);
        if (saved.setup.id !== request.id || (saved.setup.parent?.id ?? null) !== request.expectedParent || JSON.stringify(resolved) !== JSON.stringify(request.inputs)) throw new Error("Saved optimization inputs do not match the request.");
        return saved;
      } finally {
        await unlink(file).catch(error => { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; });
        await rmdir(directory);
      }
    });
  }
}
