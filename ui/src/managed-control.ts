export type ReadinessCategory = "workspace" | "models" | "scientific" | "data" | "evaluation" | "optimization" | "providers" | "recovery";
export type ReadinessState = "ready" | "action_required" | "blocked" | "stale" | "unavailable";

export interface ReadinessAction { key: string; label: string }
export interface ReadinessCheck {
  key: string;
  category: ReadinessCategory;
  state: ReadinessState;
  required: boolean;
  summary: string;
  evidence: string;
  nextAction?: ReadinessAction;
}
export interface ReadinessReport {
  projectId: string;
  baselineRevisionId?: string;
  computedAt: string;
  overall: ReadinessState;
  runnable: boolean;
  checks: ReadinessCheck[];
}

export interface OptimizationBudget {
  maximum_iterations: number;
  maximum_candidates: number;
  maximum_training_seconds: number;
  maximum_development_evaluations: number;
  maximum_sealed_evaluations: number;
  maximum_backend_operations: number;
  maximum_external_calls?: number;
}
export type OptimizationRunState = "planned" | "campaign_active" | "completed" | "cancelled" | "failed";
export interface ManagedLaunchPreview {
  manifestFingerprint: string;
  specificationFingerprint: string;
  projectId: string;
  projectFingerprint: string;
  trainingSnapshotId: string;
  benchmarkGenerationId: string;
  candidateCount: number;
  budget: OptimizationBudget;
  existingRun?: { runId: string; state: OptimizationRunState };
}
export interface ManagedReadiness { report: ReadinessReport; launchPreview?: ManagedLaunchPreview }
export interface OptimizationManifestChoice { token: string; name: string; readiness: ManagedReadiness }

export interface ManagedRunStatus {
  run_id: string;
  existing: boolean;
  state: OptimizationRunState;
  campaign_state?: string;
  experiment_state?: string;
  decision?: string;
  selected_candidate_id?: string;
  completed: string[];
  failed_or_uncertain?: string;
  stopped_reason: string;
  human_authorization_required: boolean;
  last_sequence: number;
  head_fingerprint: string;
  artifacts: Record<string, string>;
  budgets: { maximum: OptimizationBudget; reserved?: OptimizationBudget; remaining_unreserved?: OptimizationBudget };
  next_command: "resume" | "authorize-sealed" | "none";
}

export type ManagedOptimizationRequest =
  | { action: "preview" | "start"; manifestToken: string }
  | { action: "status" | "inspect" | "review-repair" | "review-delta" | "resume" | "doctor" | "provenance" | "report"; runId: string }
  | { action: "authorize-external" | "authorize-sealed"; runId: string; authorizedBy?: string }
  | { action: "cancel"; runId: string; reason: string };

export type ManagedOptimizationResult = ManagedRunStatus | Record<string, unknown>;
