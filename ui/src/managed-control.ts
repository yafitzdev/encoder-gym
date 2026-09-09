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
export interface ManagedOptimizationAuthority {
  proposalId: string; selectionId: string; trainingSnapshotId?: string; benchmarkGenerationId: string;
  hypotheses: string[]; candidateCount: number; baseTrainingInputs: number; deltaRows: number;
  budget: { maximum_total_rows: number; maximum_rows_per_target: number; maximum_candidates: number; maximum_training_seconds: number; maximum_evaluation_seconds: number; maximum_development_evaluations: number; maximum_external_calls: number; maximum_sealed_uses: number };
  validUntil: string;
}
export interface ManagedReadiness {
  report: ReadinessReport;
  optimizationAuthority?: ManagedOptimizationAuthority;
  launchPreview?: ManagedLaunchPreview;
  /** Reissued by the main process from durable project-scoped preparation. */
  preparedOptimization?: PreparedOptimizationChoice;
}
export interface OptimizationManifestChoice { token: string; name: string; readiness: ManagedReadiness }
export interface PreparedOptimizationChoice {
  token: string; name: string; launchPreview: ManagedLaunchPreview; authority: ManagedOptimizationAuthority;
  createdTrainingSnapshot: boolean; externalCalls: number;
}

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
  stage: {
    key: string; label: string; detail: string;
    execution: "quick" | "native" | "authorization" | "terminal" | "blocked";
    development?: { completed_units: number; total_units: number; active_candidate_id?: string };
  };
  next_command: "resume" | "authorize-sealed" | "none";
}

export type ManagedOptimizationRequest =
  | { action: "preview" | "start"; manifestToken: string }
  | { action: "status" | "inspect" | "review-repair" | "review-delta" | "resume" | "doctor" | "provenance" | "report"; runId: string }
  | { action: "authorize-external" | "authorize-sealed"; runId: string; authorizedBy?: string }
  | { action: "cancel"; runId: string; reason: string };

export type ManagedOptimizationResult = ManagedRunStatus | Record<string, unknown>;

export interface ManagedPromotionRequest {
  runId: string;
  expectedBaselineRevisionId: string;
  actor?: string;
  reason?: string;
}

export type ProviderRole = "generation" | "advisor" | "evaluator";
export interface ProviderLimitsInput { maximumRequests: number; maximumInputTokens: number; maximumOutputTokens: number; maximumCostMicrousd: number }
export interface ProviderInput {
  kind: "fake" | "openai-compatible"; endpoint?: string; model: string; authentication: "none" | "bearer";
  environmentFallback?: string; limits: ProviderLimitsInput;
}
export interface ProviderSettingsRequest { version: 1; generation: ProviderInput; advisor: ProviderInput; evaluator?: ProviderInput; actor?: string; reason?: string }
export interface CredentialAvailability { role: ProviderRole; authentication: "none" | "bearer"; availability: "available" | "missing" | "unavailable"; source?: "credential_store" | "environment" | "not_required" }
export interface ManagedProviderStatus {
  projectId: string; configured: boolean; catalog?: import("./managed-workspace.js").ProviderCatalog | null;
  credentialAvailability: CredentialAvailability[]; liveProbePerformed: false;
}

export interface NativePathChoice { token: string; path: string }
export interface PythonCapability { key: string; label: string; ready: boolean; missingModules: string[] }
export interface ScientificStoreInventory {
  projects: number; protocols: number; experimentRuns: number; benchmarkGenerations: number;
  diagnoses: number; proposals: number; approvedDeltaSelections: number; trainingSnapshots: number; optimizationRuns: number;
}
export interface NomosBindingPreview {
  token: string; projectId: string; projectName: string; baselineRevisionId: string;
  activeModel: { name: string; format: string; bytes: number; fingerprint: string };
  adapter: { key: string; protocol: string; configurationFingerprint: string };
  runtimeLocation: string; sourceRevision: string; sourceFingerprint: string;
  projectSnapshot: { id: string; fingerprint: string };
  python: { executable: string; version: string; compatibleVersion: boolean; capabilities: PythonCapability[]; ready: boolean };
  store: {
    databasePath: string; action: "initialize_new_store" | "verify_existing_store" | "import_verified_history";
    importedHistory?: { sourceName: string; projectSnapshot: { id: string; fingerprint: string }; inventory: ScientificStoreInventory; verification: string };
  };
  previousBindingId?: string; ready: boolean;
}
