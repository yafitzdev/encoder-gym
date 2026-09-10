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
export interface CampaignUsage {
  iterations: number;
  candidates: number;
  training_seconds: number;
  development_evaluations: number;
  sealed_evaluations: number;
  backend_operations: number;
}
export interface RecordedOptimizationUsage {
  models_trained: number;
  candidates_failed: number;
  training_seconds: number;
  development_evaluations: number;
  sealed_evaluations: number;
}
export type OptimizationRunState = "planned" | "campaign_active" | "completed" | "cancelled" | "failed";
export interface ManagedLaunchPreview {
  manifestFingerprint: string;
  specificationFingerprint: string;
  projectId: string;
  projectFingerprint: string;
  trainingSnapshotId: string;
  benchmarkGenerationId: string;
  runName: string;
  developmentSuites: string[];
  sealedSuite: string;
  candidateRecipes: Array<{
    sequence: number;
    maximumTrainingSeconds: number;
    parameters: Record<string, string | number | boolean>;
  }>;
  maximumEvaluationSeconds: number;
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
  worker?: { state: "running" | "idle" | "interrupted" | "unavailable"; started_at?: string; memory_bytes?: number; cpu_milliseconds?: number };
  activity?: RunActivity;
  timeline?: Array<{ at: string; label: string }>;
  run_id: string;
  existing: boolean;
  created_at: string;
  last_transition_at: string;
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
  budgets: { maximum: OptimizationBudget; reserved?: CampaignUsage | null; remaining_unreserved?: CampaignUsage | null };
  recorded_usage: RecordedOptimizationUsage;
  stage: {
    key: string; label: string; detail: string;
    execution: "quick" | "native" | "authorization" | "terminal" | "blocked";
    development?: { completed_units: number; total_units: number; active_candidate_id?: string };
  };
  next_command: "resume" | "authorize-sealed" | "none";
}

export interface NativeProgress {
  phase: "checking_files" | "checking_training_data" | "loading_model" | "preparing_batches" | "training" | "saving_checkpoint" | "evaluating_retrieval" | "evaluating_agent";
  completed?: number;
  total?: number;
}
export interface RunActivity extends NativeProgress {
  running: boolean;
  startedAt: string;
  updatedAt: string;
  events: Array<{ at: string; phase: NativeProgress["phase"] }>;
}

export interface ManagedOptimizationReport {
  schema_version: 1;
  run_id: string;
  name: string;
  state: OptimizationRunState;
  decision?: string | null;
  project: { id: string; revision: string; fingerprint: string; baseline_model: { id: string; key: string; fingerprint: string } };
  diagnosis: { id: string; fingerprint: string; weaknesses: unknown[]; source_campaign_id: string; source_experiment_run_id: string };
  approved_repair: {
    proposal_id: string;
    proposal_fingerprint: string;
    targets: unknown[];
    actions: unknown[];
    candidate_hypotheses: unknown[];
    delta_selection_id: string;
    delta_selection_fingerprint: string;
  };
  training_data_change: { snapshot_id: string; snapshot_fingerprint: string; base_rows: number; delta_rows: number; total_rows: number; combined_membership_fingerprint: string };
  selected_candidate_id?: string | null;
  sealed_evidence: { used: boolean; candidate_exposures: number; generation_id: string; generation_state?: string | null; authorization?: string | null };
  candidate_results: Array<{
    candidate_id: string;
    state: string;
    checkpoint?: { key: string; format: string; bytes: number; fingerprint: string; training_duration_seconds: number } | null;
    development_suites: Array<{
      suite: string;
      baseline_report_id: string;
      baseline_metrics: Record<string, number>;
      candidate_report_id: string;
      candidate_metrics: Record<string, number>;
      assessment_id: string;
      verdict: string;
      primary_improvement?: number | null;
      failed_gates: unknown[];
    }>;
  }>;
  budget_and_recovery: {
    maximum: OptimizationBudget;
    observed_training_seconds: number;
    observed_development_evaluations: number;
    observed_sealed_evaluations: number;
    candidate_failure_events: number;
    adopted_previously_proven_run: boolean;
    optimization_event_count: number;
    campaign_event_count: number;
    experiment_event_count: number;
  };
  known_evidence_limits: string[];
  provenance_head: string;
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
    databasePath: string; action: "initialize_new_store" | "verify_existing_store" | "import_verified_history" | "extend_existing_store_for_promoted_baseline";
    importedHistory?: { sourceName: string; projectSnapshot: { id: string; fingerprint: string }; inventory: ScientificStoreInventory; verification: string };
  };
  previousBindingId?: string; ready: boolean;
}
