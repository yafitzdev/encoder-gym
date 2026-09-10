/** Collection metadata is row-free. Native values are returned only by explicit
 * bounded training-row or change inspections, never by scientific projections. */
export interface DatasetVersionRef { id: string; datasetId: string; projectId: string; number: number; fingerprint: string }
export interface DatasetBranch { id: string; projectId: string; name: string; origin: DatasetVersionRef | null; createdAt: string }
export interface DatasetVersionSummary { version: DatasetVersionRef; parentId: string | null; createdAt: string; rows: number; added: number; removed: number; replaced: number }
export interface DatasetEntry { dataset: DatasetBranch; versions: DatasetVersionSummary[] }
export interface DatasetMember { id: string; source: { importId: string; artifactFingerprint: string; record: number }; contentFingerprint: string; split: "train" | "validation" | "test" }
export interface InspectedDatasetRow { member: DatasetMember; value: Record<string, unknown> }
export interface DatasetRowPage { versionId: string; offset: number; total: number; rows: InspectedDatasetRow[] }
export interface DatasetChange { id: string; kind: "added" | "removed" | "replaced"; before: InspectedDatasetRow | null; after: InspectedDatasetRow | null }
export interface DatasetChangePage { versionId: string; offset: number; total: number; changes: DatasetChange[] }
export interface RecordSelection { importId: string; record: number }
export type DatasetQuery = { kind: "list" } | { kind: "rows" | "changes"; versionId: string; offset: number; limit: number };
export type DatasetQueryResult = { kind: "list"; entries: DatasetEntry[] } | { kind: "rows"; page: DatasetRowPage } | { kind: "changes"; page: DatasetChangePage };
export type DatasetMutation =
  | { kind: "create"; datasetId: string; versionId: string; name: string; imports: string[] }
  | { kind: "fork"; datasetId: string; versionId: string; name: string; parentId: string }
  | { kind: "revise"; datasetId: string; versionId: string; parentId: string; added: RecordSelection[]; removed: string[]; replaced: { id: string; source: RecordSelection }[] };
export interface DatasetMutationResult { actionId: string; version: DatasetVersionRef; rows: number }
