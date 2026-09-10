/** Safe development-only projections produced by the shared benchmark CLI. */
import type { BoundIdentity } from "./managed-workspace.js";
export interface BenchmarkDefinition {
  schema_version: 1;
  task: string;
  backend: { name: string; protocol_version: string; configuration_fingerprint: string };
  source_revision: string;
  evaluation_configuration_fingerprint: string;
  metric_contract: {
    schema_version: number; fingerprint: string; primary_metric: string;
    definitions: { key: string; direction: "higher_is_better" | "lower_is_better" }[];
    gates: { key: string; role: "development" | "sealed_acceptance"; suite_key?: string; condition: { kind: string; value: number } }[];
  };
  suites: { key: string; role: "development" | "sealed_acceptance"; fingerprint: string; support: number }[];
  fingerprint: string;
}
export interface BenchmarkSource { scientificBinding: BoundIdentity; projectSnapshot: BoundIdentity; protocol: BoundIdentity }
export interface ProjectBenchmarkVersion {
  id: string; projectId: string; number: number; parent: BoundIdentity | null;
  definition: BenchmarkDefinition; source: BenchmarkSource; createdAt: string; fingerprint: string;
}
export interface BenchmarkReport {
  result: {
    model: { id: string; key: string; format: string; bytes: number; fingerprint: string };
    report_id: string; report_fingerprint: string; suite_key: string; metrics: Record<string, number>; support: number; created_at: string;
  };
  contexts: {
    source: BenchmarkSource; runId: string; baselineRevisionId: string; baselineModelId: string; candidateId: string | null;
    assessment: null | {
      id: string; project_snapshot_id: string; baseline_report_id: string; candidate_report_id: string;
      evidence_role: "development"; primary_metric: string; primary_improvement: number;
      gates: { key: string; baseline: number; candidate: number; direction_adjusted_improvement: number; condition: { kind: string; value: number }; passed: boolean }[];
      verdict: "passed" | "failed"; created_at: string; fingerprint: string;
    };
  }[];
}
export interface ProjectBenchmarkResults {
  version: ProjectBenchmarkVersion;
  models: { modelId: string; name: string; isBaseline: boolean; reports: BenchmarkReport[] }[];
}
export type BenchmarkQuery = { kind: "list" } | { kind: "results"; versionId: string };
export type BenchmarkQueryResult = { kind: "list"; versions: ProjectBenchmarkVersion[] } | { kind: "results"; results: ProjectBenchmarkResults };
export interface BenchmarkPreview {
  versionId: string; expectedParent: string | null; existingVersion: string | null;
  definition: BenchmarkDefinition; source: BenchmarkSource;
}
export interface BenchmarkAdoption { runId: string; expectedParent: string | null; definitionFingerprint: string }
export interface BenchmarkAdoptionResult { actionId: string; version: ProjectBenchmarkVersion }
