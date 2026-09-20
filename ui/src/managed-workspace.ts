/** Row-free workspace metadata. Explicit bounded training-row inspections use
 * the separate dataset-workspace contract, never this scientific projection. */
import type { DatasetVersionRef } from "./dataset-workspace.js";
import type { ProjectBenchmarkVersion } from "./benchmark-workspace.js";
export interface FileIdentity { path: string; bytes: number; fingerprint: string }
export interface LocalModel {
  source: string; format: string; architecture: string; files: FileIdentity[];
  bytes: number; fingerprint: string; execution: "not-configured";
}
export type DatasetPurpose = "unassigned" | "training" | "development" | "sealed";
export interface DatasetImport {
  id: string; name: string; createdAt: string; source: string; purpose: DatasetPurpose;
  format: "jsonl"; artifact: FileIdentity; rows: number;
  trainingSource: null | { baselineFingerprint: string; manifestFingerprint: string; input: string; declaredRows: number };
}
export interface ManagedWorkspace {
  folder: string; verified: boolean;
  manifest: { version: 1; id: string; name: string; createdAt: string; task: string | null; baseline: LocalModel };
  datasets: DatasetImport[];
  modelDatasetLinks?: ModelDatasetLink[];
  benchmarkVersions?: ProjectBenchmarkVersion[];
  modelCatalog?: ModelCatalog;
  scientificBinding?: ScientificBinding;
  providerCatalog?: ProviderCatalog;
}
export interface ModelChoice { token: string; model: LocalModel }
export interface FolderChoice { token: string; path: string }
export interface DatasetChoice {
  token: string; source: string; artifact: FileIdentity; rows: number; partitions: Record<string, number>; purpose: DatasetPurpose;
}
export interface CreateProjectRequest { modelToken: string; parentToken: string; folderName: string; name: string; task: string }

export interface BoundIdentity { id: string; fingerprint: string }
export interface ModelDatasetLink {
  projectId: string; modelId: string; modelFingerprint: string; version: DatasetVersionRef;
  inputs: { key: string; importId: string; fingerprint: string; rows: number }[];
  evidence: { kind: "importedManifest"; manifest: FileIdentity } | { kind: "completedTraining"; manifest: FileIdentity; snapshot: BoundIdentity; run: BoundIdentity } | { kind: "materializedTraining"; manifest: FileIdentity; snapshot: BoundIdentity; run: BoundIdentity; orderedContentFingerprint: string };
  createdAt: string; fingerprint: string;
}
export interface ModelArtifact {
  id: string; projectId: string; name: string; createdAt: string; origin: "imported" | "trained" | "transformed";
  path: string; format: string; bytes: number; fingerprint: string; parentModelId?: string;
  producingRun?: BoundIdentity; trainingSnapshot?: BoundIdentity; trainer?: BoundIdentity;
  sourceModel?: BoundIdentity;
  effectiveConfigurationFingerprint?: string; tokenizerFingerprint?: string; sourceRevision?: string;
}
export interface BaselineRevision {
  id: string; projectId: string; sequence: number; modelArtifactId: string; previousRevisionId?: string;
  change: { kind: "initialization"; source_fingerprint: string } | { kind: "promotion"; decision_id: string; decision_fingerprint: string } | { kind: "restoration"; target_revision_id: string };
  actor: string; reason: string; createdAt: string; fingerprint: string;
}
export interface ModelCatalog { projectId: string; artifacts: ModelArtifact[]; baselineRevisions: BaselineRevision[]; activeBaselineRevisionId: string }

export interface ScientificBinding {
  id: string; projectId: string; baselineRevisionId: string; previousBindingId?: string;
  adapter: { key: string; protocol: string; configurationFingerprint: string };
  runtime: { kind: "managed" | "external-isolated"; location: string; executable?: string; package?: BoundIdentity; projectSnapshot: BoundIdentity };
  store: { databasePath: string; schema: BoundIdentity; snapshotFingerprint?: string; snapshotBytes?: number };
  actor: string; reason: string; createdAt: string; specificationFingerprint: string; fingerprint: string;
}

export type ProviderRole = "generation" | "advisor" | "evaluator";
export interface ProviderConfiguration {
  role: ProviderRole; kind: "fake" | "openai-compatible"; endpoint?: string; model: string;
  authentication: "none" | "bearer"; secret?: { id: string; environmentFallback?: string };
  limits: { maximumRequests: number; maximumInputTokens: number; maximumOutputTokens: number; maximumCostMicrousd: number };
}
export interface ProviderCatalog {
  id: string; projectId: string; sequence: number; previousRevisionId?: string; providers: ProviderConfiguration[];
  actor: string; reason: string; createdAt: string; fingerprint: string;
}
