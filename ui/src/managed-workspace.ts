/** Row-free CLI contract. Never send source dataset records to the renderer. */
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
}
export interface ModelChoice { token: string; model: LocalModel }
export interface FolderChoice { token: string; path: string }
export interface DatasetChoice {
  token: string; source: string; artifact: FileIdentity; rows: number; partitions: Record<string, number>; purpose: DatasetPurpose;
}
export interface CreateProjectRequest { modelToken: string; parentToken: string; folderName: string; name: string; task: string }
