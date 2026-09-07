/** Presentation read model. No native rows, predictions, or sealed scores. */
export interface ModelArtifact {
  id: string;
  key: string;
  format: string;
  bytes: number;
  fingerprint: string;
}

export interface DevelopmentReport {
  id: string;
  suite: string;
  suiteFingerprint: string;
  contractFingerprint: string;
  fingerprint: string;
  metrics: Record<string, number>;
}

export interface RecordedCheck {
  metric: string;
  baseline: number;
  candidate: number;
  improvement: number;
  condition: { kind: string; value: number };
  passed: boolean;
}

export interface DevelopmentResult {
  baseline: DevelopmentReport;
  report: DevelopmentReport;
  verdict: string;
  assessmentId: string;
  checks: RecordedCheck[];
}

export interface CandidateAttempt {
  id: string;
  sequence: number;
  parameters: Record<string, string | number | boolean>;
  model?: ModelArtifact;
  durationSeconds?: number;
  development: DevelopmentResult[];
  failure?: { phase: string; reason: string };
}

export interface RunRecord {
  id: string;
  protocolId: string;
  protocolFingerprint: string;
  projectId: string;
  revision: string;
  createdAt: string;
  updatedAt: string;
  sourceDatabase: string;
  baseline: ModelArtifact;
  baselines: DevelopmentReport[];
  directions: Record<string, "higher_is_better" | "lower_is_better">;
  primaryMetric?: string;
  task?: string;
  agentTopK?: number;
  candidates: CandidateAttempt[];
  decision?: string;
  selectedCandidateId?: string;
  acceptance: { state: "unused" | "authorized" | "started" | "passed" | "failed"; candidateId?: string; failedMetrics: string[] };
  budget: { candidates: number; trainingSeconds: number; development: number; sealed: number };
  inputs: { key: string; bytes: number; fingerprint: string }[];
  activity: { sequence: number; kind: string; at: string; candidateId?: string }[];
  journalHead: string;
  optimizationId?: string;
}

export interface WorkspaceSnapshot {
  schemaVersion: 1;
  capturedAt: string;
  source: "recorded" | "local";
  folder: string;
  name: string;
  task: string;
  baseline: ModelArtifact;
  baselineEvaluations?: DevelopmentReport[];
  deployment?: { key: string; fingerprint: string; bytes: number };
  runs: RunRecord[];
  databases: string[];
}
