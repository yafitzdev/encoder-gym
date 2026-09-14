export type ProjectActivitySource = "desktop" | "cli" | "system";
export type ProjectActivityState = "started" | "progress" | "succeeded" | "failed";
export type ProjectActivityNarrativeOrigin = "agent" | "generation" | "system";
export type ProjectActivityNarrativeKind = "intent" | "reasoning" | "action" | "observation" | "decision" | "next_step";

export interface ProjectActivityReference { kind: string; id: string }
export interface ProjectActivityFailure { code: string; message: string }
export interface ProjectActivityNarrative {
  origin: ProjectActivityNarrativeOrigin;
  kind: ProjectActivityNarrativeKind;
  summary: string;
}
export interface ProjectActivityEvent {
  schema_version: 1;
  id: string;
  action_id: string;
  project_id: string;
  sequence: number;
  operation: string;
  source: ProjectActivitySource;
  state: ProjectActivityState;
  stage?: string;
  completed?: number;
  total?: number;
  narrative?: ProjectActivityNarrative;
  references?: ProjectActivityReference[];
  failure?: ProjectActivityFailure;
  created_at: string;
  previous_event_fingerprint: string | null;
  fingerprint: string;
}

export interface ProjectActivityAction {
  action_id: string;
  project_id: string;
  operation: string;
  source: ProjectActivitySource;
  state: ProjectActivityState;
  started_at: string;
  finished_at: string | null;
  references: ProjectActivityReference[];
  events: ProjectActivityEvent[];
}

export interface ProjectActivityLog { project_id: string; actions: ProjectActivityAction[] }
export interface ProjectActivityExport { output: string; events: number }

export interface AppendProjectActivity {
  action_id: string;
  operation: string;
  source: ProjectActivitySource;
  state: ProjectActivityState;
  stage?: string;
  completed?: number;
  total?: number;
  narrative?: ProjectActivityNarrative;
  references?: ProjectActivityReference[];
  failure?: ProjectActivityFailure;
  created_at: string;
}
