export const PROTOCOL_VERSION = 1 as const;

export const RESEARCH_TOOL_NAMES = [
  "search_web",
  "fetch_page",
  "record_evidence",
  "inspect_evidence",
  "draft_profile",
  "finish_research",
] as const;

export type ResearchToolName = (typeof RESEARCH_TOOL_NAMES)[number];

export const ARCHITECT_TOOL_NAMES = [
  "inspect_dataset",
  "inspect_semantics",
  "inspect_authenticity",
  "inspect_coverage",
  "inspect_development_evidence",
  "preview_allocation",
  "estimate_cost",
  "submit_proposal",
  "finish_architecture",
] as const;

export type ArchitectToolName = (typeof ARCHITECT_TOOL_NAMES)[number];

export const SUPERVISOR_TOOL_NAMES = [
  "inspect_quality_contract",
  "inspect_quality_window",
  "inspect_failure_breakdown",
  "inspect_current_prompt_guidance",
  "preview_prompt_revision",
  "submit_prompt_revision",
  "finish_supervision",
] as const;

export type SupervisorToolName = (typeof SUPERVISOR_TOOL_NAMES)[number];
export type AgentToolName = ResearchToolName | ArchitectToolName | SupervisorToolName;

export interface ToolExecutionRequest {
  runId: string;
  callId: string;
  name: AgentToolName;
  arguments: unknown;
}

export interface ToolExecutionResult {
  content: unknown;
  details?: unknown;
  terminate?: boolean;
}

export interface ToolExecutor {
  execute(request: ToolExecutionRequest, signal?: AbortSignal): Promise<ToolExecutionResult>;
}

export interface ScriptedToolCall {
  name: AgentToolName;
  arguments: Record<string, unknown>;
}

export interface ScriptedTurn {
  text?: string;
  toolCalls?: ScriptedToolCall[];
}

export interface PiRunRequest {
  protocolVersion: typeof PROTOCOL_VERSION;
  capabilitySet:
    | "authenticity_research_v1"
    | "dataset_architect_v1"
    | "generation_quality_supervisor_v1";
  runId: string;
  runSpecificationFingerprint: string;
  provider: string;
  model: string;
  apiKeyEnv?: string;
  systemPrompt: string;
  initialPrompt: string;
  maxModelTurns: number;
  scriptedTurns?: ScriptedTurn[];
}

export type PiRunEvent =
  | { type: "agent_started"; runId: string }
  | { type: "turn_started"; runId: string; sequence: number }
  | {
      type: "turn_completed";
      runId: string;
      sequence: number;
      inputTokens: number;
      outputTokens: number;
      costMicrousd: number;
    }
  | { type: "agent_text"; runId: string; text: string }
  | {
      type: "tool_started";
      runId: string;
      callId: string;
      name: string;
      arguments: unknown;
    }
  | {
      type: "tool_completed";
      runId: string;
      callId: string;
      name: string;
      failed: boolean;
    }
  | { type: "agent_finished"; runId: string; turns: number; aborted: boolean };

export type InputMessage =
  | { type: "start"; request: PiRunRequest }
  | { type: "tool_result"; callId: string; result: ToolExecutionResult }
  | { type: "tool_error"; callId: string; message: string }
  | { type: "cancel"; runId: string };

export type OutputMessage =
  | { type: "ready"; protocolVersion: typeof PROTOCOL_VERSION; piPackageVersion: string }
  | { type: "event"; event: PiRunEvent }
  | ({ type: "tool_request" } & ToolExecutionRequest)
  | { type: "completed"; runId: string }
  | { type: "failed"; runId?: string; message: string };
