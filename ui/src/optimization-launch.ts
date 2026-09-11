import type { BoundIdentity } from "./managed-workspace.js";

export interface OptimizationExecutionLimits {
  maximumIterations: number;
  maximumModels: number;
  maximumDatasetRowChanges: number;
  maximumTrainingSeconds: number;
  maximumDevelopmentEvaluations: number;
  maximumFinalEvaluations: number;
}

export interface OptimizationProviderLimits {
  maximumRequests: number;
  maximumInputTokens: number;
  maximumOutputTokens: number;
  maximumCostMicrousd: number;
}

export interface OptimizationLaunchScope {
  projectId: string;
  setup: BoundIdentity;
  providerCatalog: BoundIdentity;
  limits: OptimizationExecutionLimits;
  generation: OptimizationProviderLimits;
  advisor: OptimizationProviderLimits;
  finalEvaluation: "selected_candidate_once";
  fingerprint: string;
}

export interface OptimizationLaunchPreview {
  scope: OptimizationLaunchScope;
  modelName: string;
  datasetRows: number;
  benchmarkNumber: number;
}

export interface OptimizationLaunchRequest {
  id: string;
  scope: OptimizationLaunchScope;
}

export interface OptimizationLaunchAuthorization {
  id: string;
  scope: OptimizationLaunchScope;
  authorizedBy: string;
  createdAt: string;
  fingerprint: string;
}

export interface OptimizationLaunchSaved {
  actionId: string;
  authorization: OptimizationLaunchAuthorization;
}
