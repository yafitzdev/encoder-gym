import type { DatasetVersionRef } from "./dataset-workspace.js";
import type { BoundIdentity } from "./managed-workspace.js";

export interface OptimizationInputs {
  projectId: string; baselineRevision: BoundIdentity; model: BoundIdentity;
  dataset: DatasetVersionRef; benchmark: BoundIdentity;
}
export interface OptimizationSetup {
  id: string; number: number; parent: BoundIdentity | null; inputs: OptimizationInputs;
  createdAt: string; fingerprint: string;
}
export interface OptimizationSelection { modelId: string; datasetVersionId: string; benchmarkVersionId: string }
export interface OptimizationSetupPreview {
  expectedParent: string | null; inputs: OptimizationInputs; modelName: string;
  datasetRows: number; benchmarkNumber: number;
}
export interface OptimizationSetupRequest { id: string; expectedParent: string | null; inputs: OptimizationInputs }
export interface OptimizationSetupSaved { actionId: string; setup: OptimizationSetup }
