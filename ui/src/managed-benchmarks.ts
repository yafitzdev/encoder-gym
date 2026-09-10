import type { BenchmarkQueryResult, ProjectBenchmarkResults, ProjectBenchmarkVersion } from "./benchmark-workspace.js";
import type { ManagedWorkspace } from "./managed-workspace.js";

interface Ports {
  open(projectId: string): Promise<ManagedWorkspace>;
  command<T>(args: string[]): Promise<T>;
}
/** Read-only, fixed CLI grammar. Neither paths nor report contents come from the renderer. */
export class ManagedBenchmarks {
  constructor(private ports: Ports) {}
  async query(projectId: string, value: unknown): Promise<BenchmarkQueryResult> {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid benchmark request.");
    const input = value as Record<string, unknown>;
    const keys = input.kind === "list" ? ["kind"] : ["kind", "versionId"];
    if (!["list", "results"].includes(String(input.kind)) || Object.keys(input).some(key => !keys.includes(key))) throw new Error("Invalid benchmark request.");
    if (input.kind === "results" && (typeof input.versionId !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(input.versionId))) throw new Error("Invalid benchmark version.");
    const workspace = await this.ports.open(projectId);
    const args = ["benchmark", workspace.folder];
    if (input.kind === "list") {
      const versions = await this.ports.command<ProjectBenchmarkVersion[]>([...args, "list"]);
      if (!Array.isArray(versions) || versions.some(version => version.projectId !== workspace.manifest.id)) throw new Error("Benchmark belongs to another project.");
      return { kind: "list", versions };
    }
    const versionId = (input.versionId as string).toLowerCase();
    const expected = workspace.benchmarkVersions?.find(version => version.id === versionId);
    if (!expected) throw new Error("Benchmark version is missing. Refresh the project.");
    const results = await this.ports.command<ProjectBenchmarkResults>([...args, "results", versionId]);
    if (results.version.id !== versionId || results.version.projectId !== workspace.manifest.id || results.version.fingerprint !== expected.fingerprint) throw new Error("Benchmark results belong to another version.");
    const catalog = workspace.modelCatalog;
    const baseline = catalog?.baselineRevisions.find(revision => revision.id === catalog.activeBaselineRevisionId)?.modelArtifactId;
    const actual = new Set(results.models.map(model => model.modelId));
    if (!catalog || !baseline || actual.size !== results.models.length || actual.size !== catalog.artifacts.length || catalog.artifacts.some(model => !actual.has(model.id))) throw new Error("Comparison model inventory changed. Refresh the project.");
    const development = expected.definition.suites.filter(suite => suite.role === "development").map(suite => suite.key);
    for (const model of results.models) {
      if (model.isBaseline !== (model.modelId === baseline)) throw new Error("Comparison baseline changed. Refresh the project.");
      for (const report of model.reports) {
        if (!development.includes(report.result.suite_key) || Object.values(report.result.metrics).some(value => !Number.isFinite(value))
          || report.contexts.some(context => context.assessment !== null && context.assessment.evidence_role !== "development")) throw new Error("Comparison contains incompatible evaluation evidence.");
      }
    }
    return { kind: "results", results };
  }
}
