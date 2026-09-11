import type { NativeProgress } from "../managed-control.js";
import type { ProjectActivityAction, ProjectActivityLog } from "../project-activity.js";

export interface InputRunActivity {
  actionId: string;
  state: ProjectActivityAction["state"];
  startedAt: string;
  updatedAt: string;
  progress?: NativeProgress;
  stages: string[];
}

/** Project activity is the durable source for desktop orchestration progress. */
export function inputRunActivity(log: ProjectActivityLog, runId: string): InputRunActivity | undefined {
  const action = log.actions.find(candidate => candidate.operation === "optimization.run"
    && candidate.references.some(reference => reference.kind === "run" && reference.id === runId));
  if (!action) return undefined;
  const progressEvents = action.events.filter(event => event.state === "progress" && event.stage);
  const latest = progressEvents.at(-1);
  return {
    actionId: action.action_id,
    state: action.state,
    startedAt: action.started_at,
    updatedAt: action.events.at(-1)?.created_at ?? action.started_at,
    ...(latest ? { progress: { phase: latest.stage as NativeProgress["phase"], ...(latest.completed !== undefined && latest.total !== undefined ? { completed: latest.completed, total: latest.total } : {}) } } : {}),
    stages: [...new Set(progressEvents.map(event => event.stage!))],
  };
}

export const inputRunStageLabel = (stage: string): string => ({
  checking_files: "Checking inputs",
  checking_training_data: "Preparing data",
  loading_model: "Loading model",
  preparing_batches: "Preparing batches",
  training: "Training candidate",
  saving_checkpoint: "Saving candidate",
  evaluating_retrieval: "Evaluating retrieval",
  evaluating_agent: "Evaluating agent",
}[stage] ?? stage.replaceAll("_", " "));
